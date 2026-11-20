//! Saving the edit made in a video window (`viewer/montage/`): a recording's (`watch.rs`) or a
//! video opened from the disk (`open.rs`).
//!
//! 1. Save or Save copy in the editor: the island starts the save on a thread of its own, into a
//!    file beside the video; the window shows it on Save and its edits wait.
//! 2. The window may close meanwhile: the save goes on, the island finishes it.
//! 3. Done: Save replaces an MP4 with it (the window's engines let the file go first); Save
//!    copy, and Save of any other format (the editor writes MP4 only), keep it beside the video
//!    under a free name. The window, if open, plays the new file and its video is the new file
//!    from now on.
//! 4. Failed: the reason; the half-written file goes, the edits stay in the editor.

use super::viewer::{Order, VideoRequest, VideoViewer};
use super::{App, open::Opened, watch::Watch, window::WM_SAVE};
use crate::{
    capture::adapters::free_name,
    fault::{Failed, Fault, What},
    files::open::{folder, stem, video_replaced},
    montage::adapters::{Session, video_device, writer},
    record::Take,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

/// The save tells its progress in steps this fine.
const STEPS: f64 = 200.0;
/// Replacing the video waits this long for the last handle on it to go.
const REPLACE_TRIES: u32 = 10;
const REPLACE_WAIT_MS: u64 = 50;

enum News {
    Progress(f64),
    Done(Result<(), Failed>),
}

pub(super) struct Saving {
    news: Receiver<News>,
    source: PathBuf,
    /// Where the save writes, and what it becomes.
    temp: PathBuf,
    copy: bool,
    size: (u32, u32),
    duration_ms: u64,
}

impl App {
    pub(super) fn start_video_save(&mut self, order: Order, source: PathBuf) {
        if self.watch.saving.is_some() {
            return;
        }
        let temp = free_name(
            &folder(&source),
            &format!("{}.saving", stem(&source)),
            "mp4",
        );
        let (tx, news) = mpsc::channel();
        let address = self.island.0 as isize;
        let post = move || unsafe {
            let _ = PostMessageW(Some(HWND(address as *mut _)), WM_SAVE, WPARAM(0), LPARAM(0));
        };
        let (from, to) = (source.clone(), temp.clone());
        let montage = order.montage.clone();
        let output = order.output;
        let spawned = std::thread::Builder::new()
            .name("video save".into())
            .spawn(move || {
                let mut step = 0;
                let done = (|| -> Result<(), Failed> {
                    let _session = Session::start()?;
                    let device = video_device()?;
                    writer::save(&device, &from, &to, &montage, &output, |share| {
                        let now = (share * STEPS) as i32;
                        if now != step {
                            step = now;
                            let _ = tx.send(News::Progress(share));
                            post();
                        }
                    })
                })();
                let _ = tx.send(News::Done(done));
                post();
            });
        let comp = &self.scene.comp;
        if let Err(e) = spawned {
            if let Some(window) = video_window(&mut self.watch, &mut self.opened, &source) {
                window.save_failed(comp);
            }
            self.report(Fault::new(What::VideoSave, e.into()));
            return;
        }
        if let Some(window) = video_window(&mut self.watch, &mut self.opened, &source) {
            window.set_saving(comp, Some(0.0));
        }
        self.watch.saving = Some(Saving {
            news,
            source,
            temp,
            copy: order.copy,
            size: (order.output.width, order.output.height),
            duration_ms: (order.montage.duration() * 1000.0).round() as u64,
        });
    }

    /// `WM_SAVE`: the save moved on or ended.
    pub(super) fn on_video_save(&mut self) {
        let Some(saving) = &self.watch.saving else {
            return;
        };
        let mut done = None;
        let mut share = None;
        while let Ok(news) = saving.news.try_recv() {
            match news {
                News::Progress(s) => share = Some(s),
                News::Done(result) => done = Some(result),
            }
        }
        let source = saving.source.clone();
        let comp = &self.scene.comp;
        if let Some(share) = share
            && let Some(window) = video_window(&mut self.watch, &mut self.opened, &source)
        {
            window.set_saving(comp, Some(share));
        }
        let Some(result) = done else {
            return;
        };
        let Some(saving) = self.watch.saving.take() else {
            return;
        };
        match result {
            Ok(()) => self.finish_video_save(saving),
            Err(failed) => {
                let _ = std::fs::remove_file(&saving.temp);
                if let Some(window) = video_window(&mut self.watch, &mut self.opened, &source) {
                    window.save_failed(&self.scene.comp);
                }
                self.report(Fault::new(What::VideoSave, failed));
            }
        }
    }

    fn finish_video_save(&mut self, saving: Saving) {
        let replaced = video_replaced(&saving.source, saving.copy);
        let target = if replaced {
            saving.source.clone()
        } else {
            free_name(&folder(&saving.source), &stem(&saving.source), "mp4")
        };
        if replaced
            && let Some(window) = video_window(&mut self.watch, &mut self.opened, &saving.source)
        {
            window.let_go();
        }
        if let Err(failed) = replace(&saving.temp, &target) {
            self.report(Fault::new(What::VideoSave, failed));
            // The video stays as it was; the edit keeps its file beside it.
            self.video_saved(&saving.source, &saving.source);
            return;
        }
        let bytes = std::fs::metadata(&target).map_or(0, |m| m.len());
        let take = Take::new(target.clone(), saving.duration_ms, bytes, saving.size);
        let watched = self
            .watch
            .watching
            .as_ref()
            .is_some_and(|w| w.clip.take.path() == saving.source);
        if watched && let Some(watching) = &mut self.watch.watching {
            watching.clip.take = take;
        } else if replaced
            && let Some(clip) = &mut self.recording.clip
            && clip.take.path() == saving.source
        {
            clip.take = take;
        }
        self.video_saved(&saving.source, &target);
    }

    /// The window of `source`, if open, plays `target` from now on.
    fn video_saved(&mut self, source: &Path, target: &Path) {
        let comp = &self.scene.comp;
        if let Some(watching) = &mut self.watch.watching
            && (watching.clip.take.path() == source || watching.clip.take.path() == target)
        {
            let name = file_name(target);
            let request = watching.window.saved(comp, target, name);
            self.after_save_request(request);
            return;
        }
        self.opened_video_saved(source, target);
    }

    fn after_save_request(&mut self, request: Option<VideoRequest>) {
        if request.is_some() {
            self.on_video_request(request);
        }
    }
}

/// The video window showing `path`, whichever feature opened it.
fn video_window<'a>(
    watch: &'a mut Watch,
    opened: &'a mut Opened,
    path: &Path,
) -> Option<&'a mut VideoViewer> {
    if let Some(watching) = &mut watch.watching
        && watching.clip.take.path() == path
    {
        return Some(&mut watching.window);
    }
    opened.video(path)
}

/// The saved file takes `to`'s place. A handle closing a moment late (the engines shut down on
/// threads of their own) is waited for.
fn replace(from: &Path, to: &Path) -> Result<(), Failed> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(_) if tries < REPLACE_TRIES => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(REPLACE_WAIT_MS));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
