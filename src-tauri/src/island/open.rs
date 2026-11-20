//! A photo or a video opened from outside the island: "Open with" (a second start hands the
//! file over, `--open`), or Space in Explorer (`explorer.rs`).
//!
//! 1. The file is read on a thread of its own: a large photo takes a fraction of a second, a
//!    video's size and length a moment. Only the file asked for last gets a window.
//! 2. One window at a time: a photo in the screenshot viewer, a video in the video window;
//!    following Explorer onto anything else, the small card of the file (`viewer/file_card.rs`).
//!    A new file takes the window's place around its middle; the first appears with Windows'
//!    short fade. "Open with" brings it to the front; Explorer keeps the focus otherwise.
//! 3. The island has no part in it: no status, no card.
//! 4. In the window: Copy (the picture, or the video file), the name copies the path, Show in
//!    folder, Delete sends the file to the Recycle Bin; Ctrl+X does nothing (the file is the
//!    user's). The photo editor saves in the file's own format (`files::picture`), the video
//!    editor through `watch_save.rs`.

use super::{
    App,
    view::format_size,
    viewer::{
        FileCard, Opening, Request, Spot, Trouble, VideoRequest, VideoViewer, Viewer, center_of,
    },
    window::WM_OPENED,
};
use crate::{
    capture::{Image, adapters::SystemClipboard},
    fault::{Cause, Failed, Fault, What},
    files::{
        adapters::{Disk, association, read_photo, recycle, show_in_folder},
        follow::{Item, Show},
        open::Window,
        picture::{Deleted, Picture},
    },
    journal,
    lang::tr,
    montage::adapters::{Session, reader},
    platform,
    shot::Saved,
};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    time::SystemTime,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Arrival {
    /// "Open with": the window comes to the front.
    Asked,
    /// Space or following in Explorer: the focus stays there.
    Explorer,
}

enum Content {
    Photo(Image),
    Video { size: (u32, u32), duration_ms: u64 },
    Card(Facts),
}

struct Facts {
    folder: bool,
    bytes: Option<u64>,
    created: Option<SystemTime>,
}

struct Loaded {
    token: u64,
    path: PathBuf,
    content: Result<Content, Failed>,
}

enum Shown {
    Photo {
        window: Box<Viewer>,
        picture: Picture,
    },
    Video {
        window: Box<VideoViewer>,
        path: PathBuf,
    },
    Card {
        window: FileCard,
        path: PathBuf,
    },
}

impl Shown {
    fn hwnd(&self) -> HWND {
        match self {
            Self::Photo { window, .. } => window.hwnd(),
            Self::Video { window, .. } => window.hwnd(),
            Self::Card { window, .. } => window.hwnd(),
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Photo { picture, .. } => picture.path(),
            Self::Video { path, .. } | Self::Card { path, .. } => path,
        }
    }
}

pub(super) struct Opened {
    shown: Option<Shown>,
    /// How the window on screen came: a file opened from Explorer follows it.
    arrival: Arrival,
    /// The file asked for last; an older one read later is dropped.
    token: u64,
    asked: Option<(u64, Arrival, bool)>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl Default for Opened {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shown: None,
            arrival: Arrival::Asked,
            token: 0,
            asked: None,
            tx,
            rx,
        }
    }
}

impl Opened {
    /// The video window of `path`, for the save of its edit.
    pub(super) fn video(&mut self, path: &Path) -> Option<&mut VideoViewer> {
        match &mut self.shown {
            Some(Shown::Video { window, path: own }) if own == path => Some(window.as_mut()),
            _ => None,
        }
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn load(show: &Show) -> Result<Content, Failed> {
    match show {
        Show::File(Window::Photo, path) => read_photo(path).map(Content::Photo),
        Show::File(Window::Video, path) => {
            let _session = Session::start()?;
            let info = reader::info(path)?;
            Ok(Content::Video {
                size: (info.width, info.height),
                duration_ms: (info.duration * 1000.0).round() as u64,
            })
        }
        Show::Card(item) => {
            let meta = std::fs::metadata(&item.path).ok();
            Ok(Content::Card(Facts {
                folder: item.folder,
                bytes: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
                created: meta.and_then(|m| m.created().ok()),
            }))
        }
    }
}

impl App {
    /// The island in Windows' "Open with" for photos and videos, pointing at this exe. Not
    /// something the user asked for at this moment: the journal only.
    pub(super) fn register_viewer(&self) {
        if let Err(failed) = association::register() {
            journal::warn(tr!("journal.register_failed", error = failed.detail));
        }
    }

    /// "Open with": a photo or a video; anything else is said to be unsupported.
    pub(super) fn open_path(&mut self, path: PathBuf, arrival: Arrival) {
        match Window::of(&path.to_string_lossy()) {
            Some(window) => self.show_file(Show::File(window, path), arrival),
            None => {
                self.report(Fault::of(What::FileOpen, Cause::Unsupported).about(name_of(&path)))
            }
        }
    }

    /// Reads the file on a thread of its own; the window comes on `WM_OPENED`.
    pub(super) fn show_file(&mut self, show: Show, arrival: Arrival) {
        let opened = &mut self.opened;
        opened.token += 1;
        let token = opened.token;
        let card = matches!(show, Show::Card(_));
        opened.asked = Some((token, arrival, card));
        let tx = opened.tx.clone();
        let island = self.island.0 as isize;
        let spawned = std::thread::Builder::new()
            .name("open file".into())
            .spawn(move || {
                let content = load(&show);
                let path = show.path().clone();
                let _ = tx.send(Loaded {
                    token,
                    path,
                    content,
                });
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(HWND(island as *mut _)),
                        WM_OPENED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
        if let Err(e) = spawned {
            self.report(Fault::new(What::FileOpen, e.into()));
        }
    }

    /// `WM_OPENED`: a file has been read.
    pub(super) fn on_opened(&mut self) {
        let Some((token, arrival, card)) = self.opened.asked else {
            let _ = self.opened.rx.try_iter().count();
            return;
        };
        // Files asked for earlier and read later are dropped with the rest of the queue.
        let Some(loaded) = self
            .opened
            .rx
            .try_iter()
            .filter(|l| l.token == token)
            .last()
        else {
            return;
        };
        self.opened.asked = None;
        let content = match loaded.content {
            Ok(content) => content,
            // Following Explorer, a file Windows can't read is shown as what it is: a file.
            Err(failed) if arrival == Arrival::Explorer && self.follows_explorer() && !card => {
                journal::warn(tr!(
                    "journal.open_failed",
                    file = name_of(&loaded.path),
                    error = failed.detail
                ));
                let item = Item {
                    path: loaded.path,
                    folder: false,
                };
                return self.show_file(Show::Card(item), arrival);
            }
            Err(failed) => {
                let what = match Window::of(&loaded.path.to_string_lossy()) {
                    Some(Window::Video) => What::VideoOpen,
                    _ => What::PhotoOpen,
                };
                return self.report(Fault::new(what, failed).about(name_of(&loaded.path)));
            }
        };
        let (monitor, center) = match &self.opened.shown {
            Some(shown) => (
                platform::window_monitor(shown.hwnd()),
                Some(center_of(shown.hwnd())),
            ),
            None => (platform::cursor_monitor(), None),
        };
        let spot = Spot {
            work: platform::work_rect(monitor),
            center,
        };
        let k = platform::monitor_scale(monitor);
        let shown = match self.build_window(loaded.path.clone(), content, spot, k) {
            Ok(shown) => shown,
            Err(failed) => return self.report(Fault::new(What::FileOpen, failed)),
        };
        let first = self.opened.shown.is_none();
        let hwnd = shown.hwnd();
        // The new window stands in the old one's place before that goes: no gap between them.
        match (&shown, arrival) {
            (Shown::Photo { window, .. }, Arrival::Asked) => window.show(first),
            (Shown::Photo { window, .. }, Arrival::Explorer) => window.show_quietly(first),
            (Shown::Video { window, .. }, Arrival::Asked) => window.show(first),
            (Shown::Video { window, .. }, Arrival::Explorer) => window.show_quietly(first),
            (Shown::Card { window, .. }, _) => window.show_quietly(first),
        }
        if let Shown::Video { window, .. } = &shown {
            window.start();
        }
        self.close_opened_window();
        self.opened.shown = Some(shown);
        self.opened.arrival = arrival;
        self.after_opened(hwnd, (arrival == Arrival::Explorer).then_some(loaded.path));
    }

    fn build_window(
        &mut self,
        path: PathBuf,
        content: Content,
        spot: Spot,
        k: f32,
    ) -> Result<Shown, Failed> {
        let comp = &self.scene.comp;
        let bar = self.settings.viewer_bar;
        let name = name_of(&path);
        Ok(match content {
            Content::Photo(image) => {
                let picture =
                    Picture::new(path, image).ok_or_else(|| Failed::new(Cause::Unsupported))?;
                let mut window = Box::new(Viewer::new(
                    comp,
                    spot,
                    k,
                    picture.image(),
                    true,
                    bar,
                    &name,
                )?);
                window.set_file(comp, Some(name));
                Shown::Photo { window, picture }
            }
            Content::Video { size, duration_ms } => {
                // Black until the video draws its first frame a moment later.
                let still = Image {
                    width: 1,
                    height: 1,
                    pixels: vec![0, 0, 0, 255],
                };
                let opening = Opening {
                    path: &path,
                    name: name.clone(),
                    title: name,
                    still: &still,
                    size,
                    duration_ms,
                    volume: (
                        f64::from(self.settings.video_volume),
                        self.settings.video_muted,
                    ),
                    remaining: self.settings.video_remaining,
                };
                let window = Box::new(VideoViewer::new(comp, spot, k, opening, bar)?);
                Shown::Video { window, path }
            }
            Content::Card(facts) => {
                let first = if facts.folder {
                    tr!("file_card.folder").into_owned()
                } else {
                    facts.bytes.map(size_text).unwrap_or_default()
                };
                let created = facts
                    .created
                    .map(|at| tr!("file_card.created", date = platform::date_time_text(at)))
                    .unwrap_or_default();
                let window = FileCard::new(comp, spot, k, &name, [&first, &created])?;
                Shown::Card { window, path }
            }
        })
    }

    /// The window goes; a video keeps its sound and length choices for the next one.
    fn close_opened_window(&mut self) {
        if let Some(Shown::Video { window, .. }) = self.opened.shown.take() {
            self.keep_choices(&window);
        }
    }

    pub(super) fn close_opened(&mut self) {
        self.opened.asked = None;
        self.close_opened_window();
        self.after_opened(HWND::default(), None);
    }

    pub(super) fn opened_window(&self) -> Option<HWND> {
        self.opened.shown.as_ref().map(Shown::hwnd)
    }

    pub(super) fn opened_from_explorer(&self) -> bool {
        self.opened.shown.is_some() && self.opened.arrival == Arrival::Explorer
    }

    /// The editor of the window is open: following waits until it closes.
    pub(super) fn opened_editing(&self) -> bool {
        match &self.opened.shown {
            Some(Shown::Photo { window, .. }) => window.is_editing(),
            Some(Shown::Video { window, .. }) => window.is_editing(),
            _ => false,
        }
    }

    pub(super) fn is_opened_window(&self, hwnd: HWND) -> bool {
        self.opened.shown.as_ref().is_some_and(|s| s.hwnd() == hwnd)
    }

    pub(super) fn on_opened_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let comp = &self.scene.comp;
        match self.opened.shown.as_mut()? {
            Shown::Photo { window, picture } => {
                let (result, request) = window.handle(comp, picture.image(), msg, wparam, lparam);
                if let Some(request) = request {
                    self.on_photo_request(request);
                }
                result
            }
            Shown::Video { window, .. } => {
                let (result, request) = window.handle(comp, msg, wparam, lparam);
                self.on_opened_video(request);
                result
            }
            Shown::Card { window, .. } => {
                let (result, close) = window.handle(msg, wparam, lparam);
                if close {
                    self.close_opened();
                }
                result
            }
        }
    }

    /// A press on the video's picture left Windows' move loop.
    pub(super) fn on_opened_pressed(&mut self, hwnd: HWND) {
        if !self.is_opened_window(hwnd) {
            return;
        }
        if let Some(Shown::Video { window, .. }) = &mut self.opened.shown {
            let request = window.pressed(&self.scene.comp);
            self.on_opened_video(request);
        }
    }

    fn on_photo_request(&mut self, request: Request) {
        match request {
            Request::Close => self.close_opened(),
            Request::Copy => self.copy_opened_photo(),
            // The file is the user's: it is never left only in the clipboard.
            Request::Cut => {}
            Request::Delete => self.delete_opened(),
            Request::Folder => self.show_opened_in_folder(),
            Request::CopyPath => self.copy_opened_path(),
            Request::Save(target) => self.save_opened_photo(target),
        }
    }

    fn on_opened_video(&mut self, request: Option<VideoRequest>) {
        match request {
            Some(VideoRequest::Close) => self.close_opened(),
            Some(VideoRequest::Copy) => {
                let mut clipboard = SystemClipboard::new(self.island);
                if let Some(Shown::Video { window, path }) = &self.opened.shown {
                    match clipboard.put_file(path) {
                        Ok(()) => window.show_copied(&self.scene.comp),
                        Err(failed) => self.report(Fault::new(What::Copy, failed)),
                    }
                }
            }
            Some(VideoRequest::Delete) => self.delete_opened(),
            Some(VideoRequest::Folder) => self.show_opened_in_folder(),
            Some(VideoRequest::CopyPath) => self.copy_opened_path(),
            Some(VideoRequest::Frame(Ok(frame))) => {
                if self.shot_from(frame)
                    && let Some(Shown::Video { window, .. }) = &self.opened.shown
                {
                    window.show_shot_done(&self.scene.comp);
                }
            }
            Some(VideoRequest::Frame(Err(failed))) => {
                self.report(Fault::new(What::ShotTake, failed));
            }
            Some(VideoRequest::Trouble(trouble)) => match trouble {
                Trouble::Open(failed) => {
                    self.close_opened();
                    self.report(Fault::new(What::VideoOpen, failed));
                }
                Trouble::Stopped(failed) => {
                    self.close_opened();
                    self.report(Fault::new(What::VideoStopped, failed));
                }
                Trouble::Sound => self.report(Fault::of(What::VideoSound, Cause::Unsupported)),
                Trouble::Control(failed) => self.report(Fault::new(What::VideoControl, failed)),
            },
            Some(VideoRequest::Save(order)) => {
                if let Some(Shown::Video { path, .. }) = &self.opened.shown {
                    let source = path.clone();
                    self.start_video_save(order, source);
                }
            }
            None => {}
        }
    }

    /// The edit of the video was saved as `target`: the window plays it from now on.
    pub(super) fn opened_video_saved(&mut self, source: &Path, target: &Path) {
        let comp = &self.scene.comp;
        let request = match &mut self.opened.shown {
            Some(Shown::Video { window, path }) if path == source => {
                *path = target.to_path_buf();
                window.saved(comp, target, name_of(target))
            }
            _ => return,
        };
        self.on_opened_video(request);
        if let Some(path) = self.opened_from_explorer().then(|| target.to_path_buf()) {
            self.after_opened(self.opened_window().unwrap_or_default(), Some(path));
        }
    }

    fn show_opened_in_folder(&mut self) {
        let Some(shown) = &self.opened.shown else {
            return;
        };
        if let Err(failed) = show_in_folder(shown.path()) {
            self.report(Fault::new(What::FolderOpen, failed));
        }
    }

    fn copy_opened_path(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let comp = &self.scene.comp;
        let Some(shown) = &mut self.opened.shown else {
            return;
        };
        let copied = clipboard.put_text(&shown.path().to_string_lossy());
        match (copied, shown) {
            (Ok(()), Shown::Photo { window, .. }) => window.show_path_copied(comp),
            (Ok(()), Shown::Video { window, .. }) => window.show_path_copied(comp),
            (Ok(()), Shown::Card { .. }) => {}
            (Err(failed), _) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    /// In the editor: the picture with the edits; the file isn't touched.
    fn copy_opened_photo(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let Some(Shown::Photo { window, picture }) = &self.opened.shown else {
            return;
        };
        let copied = if window.is_editing() {
            match window.flatten(picture.image()) {
                Some(edited) => clipboard.put_image(&edited),
                None => Err(Failed::new(Cause::Unknown)),
            }
        } else {
            clipboard
                .put_image(picture.image())
                .map(|()| window.show_copied(&self.scene.comp))
        };
        if let Err(failed) = copied {
            self.report(Fault::new(What::Copy, failed));
        }
    }

    fn save_opened_photo(&mut self, target: crate::shot::Target) {
        let comp = &self.scene.comp;
        let Some(Shown::Photo { window, picture }) = &mut self.opened.shown else {
            return;
        };
        let saved = match window.flatten(picture.image()) {
            Some(edited) => picture.save_edited(edited, &mut Disk, target),
            None => Saved::Failed(Failed::new(Cause::Unknown)),
        };
        match saved {
            Saved::Written => {
                window.finish_edit(comp, picture.image(), Some(picture.name()));
                let path = picture.path().to_path_buf();
                if self.opened_from_explorer() {
                    self.after_opened(self.opened_window().unwrap_or_default(), Some(path));
                }
            }
            Saved::Unchanged => window.close_editor(comp),
            Saved::Failed(failed) => {
                window.save_failed(comp);
                self.report(Fault::new(What::ShotEdit, failed));
            }
        }
    }

    /// To the Recycle Bin. A video's engine holds the file open: its window goes first.
    fn delete_opened(&mut self) {
        match self.opened.shown.take() {
            Some(Shown::Photo { window, picture }) => match picture.delete(&mut Disk) {
                Deleted::Gone => {
                    drop(window);
                    self.close_opened();
                }
                Deleted::Kept(picture) => {
                    self.opened.shown = Some(Shown::Photo { window, picture })
                }
                Deleted::Failed(picture, failed) => {
                    self.opened.shown = Some(Shown::Photo { window, picture });
                    self.report(Fault::new(What::FileDelete, failed));
                }
            },
            Some(Shown::Video { window, path }) => {
                let arrival = self.opened.arrival;
                self.keep_choices(&window);
                drop(window);
                self.close_opened();
                match recycle(&path) {
                    Ok(true) => {}
                    Ok(false) => self.show_file(Show::File(Window::Video, path), arrival),
                    Err(failed) => self.report(Fault::new(What::FileDelete, failed)),
                }
            }
            other => self.opened.shown = other,
        }
    }
}

/// Below a megabyte a file is told in kilobytes: "0.0 MB" says nothing.
fn size_text(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        tr!("size.kb", n = bytes.div_ceil(1024)).into_owned()
    } else {
        format_size(bytes)
    }
}
