//! Downloads on the island (`crate::download`).
//!
//! 1. At start, and when the setting is turned on: the Downloads folder is watched on a thread
//!    of its own (`download::adapters::Watch`); turned off, the watch ends.
//! 2. While a browser downloads: the small size with a spinner and how much has come, in the
//!    collapsed island's place (`NoticeKind::Downloading`), until nothing comes any more. A
//!    swipe hides it until then.
//! 3. A file has arrived: its name and size in the small size (`NoticeKind::Download`). Files
//!    arriving while it is on screen join it and start its time again. Under the cursor it
//!    opens into the card.
//! 4. The card: a click on the picture opens the file, on the text (or the pile) shows the
//!    files in Explorer; either closes the notice.
//! 5. Both go through the system notifications' gate: quiet in Do Not Disturb and over a
//!    fullscreen app. A recording hides the progress and holds the arrived files until it
//!    ends, while they are fresh.

use super::{App, window::WM_DOWNLOAD};
use crate::{
    download::{
        Progress,
        adapters::{self as system, Arrival, Event, Watch},
    },
    fault::{Fault, What},
    files::adapters as files,
    journal,
    lang::tr,
    media::Icon,
    notify::{self, Kind, STALE_MS, Verdict},
    stage::NoticeKind,
};
use std::{path::PathBuf, sync::Arc};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

pub(super) struct File {
    pub(super) name: String,
    path: PathBuf,
    pub(super) bytes: u64,
    pub(super) source: Option<String>,
    pub(super) picture: Option<Arc<Icon>>,
}

#[derive(Default)]
pub(super) struct Downloads {
    watch: Option<Watch>,
    pub(super) coming: Option<Progress>,
    /// The progress is on the stage: it was posted and not yet ended.
    posted: bool,
    /// The progress was swiped away (or Do Not Disturb took it): it comes back with the next download.
    hidden: bool,
    /// What the notice shows, newest last.
    pub(super) shelf: Vec<File>,
    /// Arrived during a recording, with the time each came.
    held: Vec<(File, u64)>,
}

impl From<Arrival> for File {
    fn from(arrival: Arrival) -> Self {
        Self {
            name: arrival.name,
            path: arrival.path,
            bytes: arrival.bytes,
            source: arrival.source,
            picture: arrival.thumb.map(Arc::new),
        }
    }
}

impl App {
    /// At start and after the settings changed: the watch runs exactly while the setting is on.
    pub(super) fn sync_downloads(&mut self) {
        let want = self.settings.notify_downloads;
        if want == self.downloads.watch.is_some() {
            return;
        }
        if !want {
            self.downloads.watch = None;
            self.downloads.coming = None;
            self.downloads.held.clear();
            let now = self.now_ms();
            let mut changed = self.stage.end(NoticeKind::Downloading, now);
            changed |= self.stage.end(NoticeKind::Download, now);
            self.after_stage(changed);
            return;
        }
        let Some(dir) = system::folder() else {
            journal::warn(tr!(
                "journal.downloads_unwatched",
                error = "no Downloads folder"
            ));
            return;
        };
        // HWND can't cross threads, a number can.
        let service = self.service.0 as isize;
        let notify = move || unsafe {
            let _ = PostMessageW(
                Some(HWND(service as *mut _)),
                WM_DOWNLOAD,
                WPARAM(0),
                LPARAM(0),
            );
        };
        match Watch::start(dir, notify) {
            Ok(watch) => self.downloads.watch = Some(watch),
            Err(failed) => journal::warn(tr!("journal.downloads_unwatched", error = failed.detail)),
        }
    }

    pub(super) fn stop_downloads(&mut self) {
        self.downloads.watch = None;
    }

    /// WM_DOWNLOAD: the watch has news.
    pub(super) fn on_downloads(&mut self) {
        let events = match &self.downloads.watch {
            Some(watch) => watch.drain(),
            None => return,
        };
        let mut changed = false;
        let mut moved = false;
        for event in events {
            match event {
                Event::Coming(progress) if progress.files == 0 => {
                    self.downloads.coming = None;
                    self.downloads.hidden = false;
                    changed |= self.stage.end(NoticeKind::Downloading, self.now_ms());
                }
                Event::Coming(progress) => {
                    self.downloads.coming = Some(progress);
                    changed |= self.post_coming();
                    moved = true;
                }
                Event::Arrived(arrival) => changed |= self.arrive(arrival.into()),
                Event::Stopped(failed) => {
                    journal::warn(tr!("journal.downloads_unwatched", error = failed.detail));
                    self.downloads.watch = None;
                    self.downloads.coming = None;
                    changed |= self.stage.end(NoticeKind::Downloading, self.now_ms());
                }
            }
        }
        self.after_stage(changed);
        // How much has come changes in place: the stage stays as it was.
        if moved && !changed && self.visible {
            self.render();
        }
    }

    fn post_coming(&mut self) -> bool {
        let d = &self.downloads;
        if d.posted || d.hidden || self.downloads_verdict() != Verdict::Show {
            return false;
        }
        self.downloads.posted = true;
        self.stage.post(NoticeKind::Downloading, self.now_ms())
    }

    fn arrive(&mut self, file: File) -> bool {
        match self.downloads_verdict() {
            Verdict::Show => self.show_arrived(vec![file]),
            Verdict::Wait => {
                let now = self.now_ms();
                self.downloads.held.push((file, now));
                false
            }
            Verdict::Drop => false,
        }
    }

    /// Joins the files on screen, or starts a new notice.
    fn show_arrived(&mut self, files: Vec<File>) -> bool {
        if !self.stage.has(NoticeKind::Download) {
            self.downloads.shelf.clear();
        }
        self.downloads.shelf.extend(files);
        let changed = self.stage.post(NoticeKind::Download, self.now_ms());
        // A file joining the shown notice keeps the stage: the content still changed.
        changed || self.stage.shown().notice.map(|n| n.kind) == Some(NoticeKind::Download)
    }

    fn downloads_verdict(&self) -> Verdict {
        notify::allow(
            Kind::Downloads,
            false,
            self.notify_enabled(),
            self.notice_context(),
        )
    }

    /// After the stage changed: a swiped progress stays away until the next download, a
    /// notice gone forgets its files, a recording that ended lets the held ones out.
    pub(super) fn sync_downloads_stage(&mut self) {
        if self.downloads.posted && !self.stage.has(NoticeKind::Downloading) {
            self.downloads.posted = false;
            self.downloads.hidden = self.downloads.coming.is_some();
        }
        if !self.stage.has(NoticeKind::Download) {
            self.downloads.shelf.clear();
        }
        if !self.downloads.held.is_empty() && !self.notice_context().recording {
            let now = self.now_ms();
            let fresh: Vec<File> = std::mem::take(&mut self.downloads.held)
                .into_iter()
                .filter(|(_, at)| now.saturating_sub(*at) < STALE_MS)
                .map(|(file, _)| file)
                .collect();
            if !fresh.is_empty() && self.downloads_verdict() == Verdict::Show {
                let changed = self.show_arrived(fresh);
                self.after_stage(changed);
            }
        }
    }

    pub(super) fn open_download(&mut self) {
        let Some(file) = self.downloads.shelf.last() else {
            return;
        };
        let opened = files::open(&file.path)
            .map_err(|failed| Fault::new(What::FileOpen, failed).about(file.name.clone()));
        self.close_downloads(opened);
    }

    pub(super) fn show_downloads(&mut self) {
        let paths: Vec<PathBuf> = self
            .downloads
            .shelf
            .iter()
            .map(|f| f.path.clone())
            .collect();
        let shown =
            files::show_together(&paths).map_err(|failed| Fault::new(What::FolderOpen, failed));
        self.close_downloads(shown);
    }

    fn close_downloads(&mut self, done: Result<(), Fault>) {
        if let Err(fault) = done {
            self.report(fault);
        }
        let changed = self.stage.end(NoticeKind::Download, self.now_ms());
        self.after_stage(changed);
    }
}
