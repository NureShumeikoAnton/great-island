//! A recording in the video window (`viewer/video.rs`).
//!
//! 1. A click on the recording card's thumbnail: the card closes, the last frame flies from it into
//!    the window, and the window shows under it. The video loads meanwhile and plays when the frame
//!    has landed.
//! 2. While the window is open the island shows its status, as for a screenshot (`capture.rs`
//!    decides whose it is when both windows are open): ▶/❚❚ plays or pauses the video, a click on
//!    the thumbnail raises the window, the arrow pauses the video and sends its frame back into the
//!    island, where the recording goes on as its card.
//! 3. In the window: Copy puts the file into the clipboard, the name copies its path, Show in folder,
//!    Delete deletes the file and closes the window (if the file can't go, the recording comes back
//!    as its card); frame to screenshot puts the frame into the clipboard and the screenshots folder.
//! 4. A video Windows can't play: the reason, and the window closes; the card comes back.
//!
//! The window remembers nothing; closing it keeps its volume and whether the length shows the time
//! left in the settings for the next video.

use super::{
    App,
    flight::{GLIDE_LANDING_MS, GLIDE_START_MS, SLOT_RADIUS},
    record::Clip,
    timers,
    viewer::{CORNER_RADIUS, Opening, Spot, Trouble, VideoRequest, VideoViewer},
};
use crate::{
    capture::adapters::{self as system, SystemClipboard},
    fault::{Cause, Fault, What},
    lang::tr,
    platform,
    record::adapters as files,
    stage::NoticeKind,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

pub(super) struct Watching {
    pub(super) window: VideoViewer,
    pub(super) clip: Clip,
    /// Island ms: the island's status belongs to the window opened last.
    pub(super) opened_at: u64,
}

#[derive(Default)]
pub(super) struct Watch {
    pub(super) watching: Option<Watching>,
    /// The window on its way out, after its frame left for the island.
    vanishing: Option<VideoViewer>,
    /// An edit being saved; it outlives the window.
    pub(super) saving: Option<super::watch_save::Saving>,
}

impl App {
    pub(super) fn open_clip(&mut self) {
        let from = self.card_place();
        let Some(clip) = self.recording.clip.take() else {
            return;
        };
        self.kill_timer(timers::CLIP_CLOSE);
        self.sync_card_keys();
        let changed = self.stage.end(NoticeKind::Clip, self.now_ms());
        self.after_stage(changed);
        // A window already open goes silently: the status stays, now this one's.
        self.close_watch_window();

        let name = clip
            .take
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let opening = Opening {
            path: clip.take.path(),
            name,
            title: tr!("window.video").into_owned(),
            still: &clip.still,
            size: clip.take.size(),
            duration_ms: clip.take.duration_ms(),
            volume: (
                f64::from(self.settings.video_volume),
                self.settings.video_muted,
            ),
            remaining: self.settings.video_remaining,
        };
        let work = platform::work_rect(self.monitor);
        let window = match VideoViewer::new(
            &self.scene.comp,
            Spot::middle(work),
            self.scale,
            opening,
            self.settings.viewer_bar,
        ) {
            Ok(window) => window,
            Err(failed) => {
                self.report(Fault::new(What::VideoOpen, failed));
                self.put_back(clip);
                return;
            }
        };
        let monitor = platform::monitor_rect(self.monitor);
        let to = window.place(monitor);
        let landing = from
            .filter(|from| self.start_glide(&clip.still, (*from, to), (SLOT_RADIUS, CORNER_RADIUS)))
            .map(|_| GLIDE_LANDING_MS);
        self.watch.watching = Some(Watching {
            window,
            clip,
            opened_at: self.now_ms(),
        });
        let changed = self.stage.post(NoticeKind::Viewing, self.now_ms());
        self.after_stage(changed);
        match landing {
            Some(ms) => self.set_timer(timers::WATCH_SHOW, ms),
            None => self.on_watch_show(),
        }
    }

    /// The frame landed: the window takes its place under it and the video starts.
    pub(super) fn on_watch_show(&mut self) {
        self.kill_timer(timers::WATCH_SHOW);
        let animated = self.shots.flight.is_none();
        if let Some(watching) = &self.watch.watching {
            watching.window.show(animated);
            watching.window.start();
        }
    }

    pub(super) fn on_watch_hide(&mut self) {
        self.kill_timer(timers::WATCH_HIDE);
        self.watch.vanishing = None;
    }

    pub(super) fn is_video_window(&self, hwnd: HWND) -> bool {
        self.watch
            .watching
            .as_ref()
            .is_some_and(|w| w.window.hwnd() == hwnd)
    }

    pub(super) fn on_video_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let watching = self.watch.watching.as_mut()?;
        let was = watching.window.running();
        let (result, request) = watching
            .window
            .handle(&self.scene.comp, msg, wparam, lparam);
        self.after_video(was, request);
        result
    }

    /// A press on the picture left Windows' move loop (see `VideoViewer::pressed`).
    pub(super) fn on_video_pressed(&mut self, hwnd: HWND) {
        if !self.is_video_window(hwnd) {
            return;
        }
        let Some(watching) = self.watch.watching.as_mut() else {
            return;
        };
        let was = watching.window.running();
        let request = watching.window.pressed(&self.scene.comp);
        self.after_video(was, request);
    }

    fn after_video(&mut self, was: bool, request: Option<VideoRequest>) {
        let turned = self.watched_running().is_some_and(|running| running != was);
        self.on_video_request(request);
        if turned && self.watch.watching.is_some() {
            self.render();
        }
    }

    pub(super) fn toggle_watched(&mut self) {
        let Some(watching) = &mut self.watch.watching else {
            return;
        };
        let request = watching.window.toggle(&self.scene.comp);
        self.on_video_request(request);
        self.render();
    }

    pub(super) fn watched_running(&self) -> Option<bool> {
        self.watch.watching.as_ref().map(|w| w.window.running())
    }

    pub(super) fn on_video_request(&mut self, request: Option<VideoRequest>) {
        match request {
            Some(VideoRequest::Close) => self.close_watch(),
            Some(VideoRequest::Copy) => self.copy_watched(),
            Some(VideoRequest::Delete) => self.delete_watched(),
            Some(VideoRequest::Folder) => {
                let shown = self
                    .watch
                    .watching
                    .as_ref()
                    .map(|w| system::show_in_folder(w.clip.take.path()));
                if let Some(Err(failed)) = shown {
                    self.report(Fault::new(What::FolderOpen, failed));
                }
            }
            Some(VideoRequest::CopyPath) => self.copy_watched_path(),
            Some(VideoRequest::Frame(Ok(frame))) => {
                if self.shot_from(frame)
                    && let Some(watching) = &self.watch.watching
                {
                    watching.window.show_shot_done(&self.scene.comp);
                }
            }
            Some(VideoRequest::Frame(Err(failed))) => {
                self.report(Fault::new(What::ShotTake, failed));
            }
            Some(VideoRequest::Trouble(trouble)) => self.on_trouble(trouble),
            Some(VideoRequest::Save(order)) => {
                if let Some(watching) = &self.watch.watching {
                    let source = watching.clip.take.path().to_path_buf();
                    self.start_video_save(order, source);
                }
            }
            None => {}
        }
    }

    fn on_trouble(&mut self, trouble: Trouble) {
        match trouble {
            Trouble::Open(failed) => {
                self.report(Fault::new(What::VideoOpen, failed));
                self.give_back();
            }
            Trouble::Stopped(failed) => {
                self.report(Fault::new(What::VideoStopped, failed));
                self.give_back();
            }
            Trouble::Sound => self.report(Fault::of(What::VideoSound, Cause::Unsupported)),
            Trouble::Control(failed) => self.report(Fault::new(What::VideoControl, failed)),
        }
    }

    pub(super) fn sync_watch_settings(&mut self) {
        if let Some(watching) = &mut self.watch.watching {
            watching
                .window
                .set_bar(&self.scene.comp, self.settings.viewer_bar);
        }
    }

    pub(super) fn raise_watched(&self) {
        if let Some(watching) = &self.watch.watching {
            watching.window.raise();
        }
    }

    /// The window closes, the recording is done with: the file stays.
    pub(super) fn close_watch(&mut self) {
        if self.close_watch_window().is_some() {
            let changed = self.end_viewing();
            self.after_stage(changed);
        }
    }

    /// The window goes without touching the status; its sound and length are kept for the next video.
    fn close_watch_window(&mut self) -> Option<Clip> {
        self.kill_timer(timers::WATCH_SHOW);
        let watching = self.watch.watching.take()?;
        self.keep_choices(&watching.window);
        Some(watching.clip)
    }

    pub(super) fn keep_choices(&mut self, window: &VideoViewer) {
        let (volume, muted) = window.volume();
        let (volume, remaining) = (volume as f32, window.remaining());
        let s = &self.settings;
        if volume != s.video_volume || muted != s.video_muted || remaining != s.video_remaining {
            self.settings.video_volume = volume;
            self.settings.video_muted = muted;
            self.settings.video_remaining = remaining;
            self.save_settings();
        }
    }

    /// The window can't show the video: it closes and the recording is its card again.
    fn give_back(&mut self) {
        if let Some(clip) = self.close_watch_window() {
            let changed = self.end_viewing();
            self.after_stage(changed);
            self.put_back(clip);
        }
    }

    fn put_back(&mut self, clip: Clip) {
        self.recording.clip = Some(clip);
        let changed = self.stage.post(NoticeKind::Clip, self.now_ms());
        self.sync_card_keys();
        self.after_stage(changed);
    }

    fn copy_watched(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let Some(watching) = &mut self.watch.watching else {
            return;
        };
        match watching.clip.take.copy(&mut clipboard) {
            Ok(()) => watching.window.show_copied(&self.scene.comp),
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    fn copy_watched_path(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let Some(watching) = &mut self.watch.watching else {
            return;
        };
        let path = watching.clip.take.path().to_string_lossy().into_owned();
        match clipboard.put_text(&path) {
            Ok(()) => watching.window.show_path_copied(&self.scene.comp),
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    /// The engine holds the file open: the window closes first, then the file goes.
    fn delete_watched(&mut self) {
        let Some(clip) = self.close_watch_window() else {
            return;
        };
        match files::remove(clip.take.path()) {
            Ok(()) => {
                let changed = self.end_viewing();
                self.after_stage(changed);
            }
            Err(failed) => {
                let changed = self.end_viewing();
                self.after_stage(changed);
                self.report(Fault::new(What::RecordDelete, failed));
                self.put_back(clip);
            }
        }
    }

    /// The arrow in the island's status: the video pauses, its frame flies into the island and the
    /// recording goes on as its card.
    pub(super) fn return_watched(&mut self) {
        if let Some(watching) = &mut self.watch.watching
            && watching.window.keeps_edits(&self.scene.comp)
        {
            watching.window.raise();
            return;
        }
        let Some(mut watching) = self.watch.watching.take() else {
            return;
        };
        self.kill_timer(timers::WATCH_SHOW);
        watching.window.pause(&self.scene.comp);
        self.keep_choices(&watching.window);
        self.close_menu();
        let monitor = platform::monitor_rect(self.monitor);
        let from = watching.window.place(monitor);
        let frame = watching.window.frame();
        let changed = self.end_viewing();
        self.after_stage(changed);
        let preview = frame.unwrap_or_else(|| (*watching.clip.still).clone());
        self.put_back(watching.clip);
        let to = self.card_place();
        let flown = self.island_takes_cards()
            && to.is_some_and(|to| {
                self.start_glide(&preview, (from, to), (CORNER_RADIUS, SLOT_RADIUS))
            });
        if flown {
            watching.window.vanish();
            self.watch.vanishing = Some(watching.window);
            self.set_timer(timers::WATCH_HIDE, GLIDE_START_MS);
        }
    }
}
