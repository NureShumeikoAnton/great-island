//! The text scanner on the island.
//!
//! 1. Its key or the control center's button: the island leaves the captures, the monitor with
//!    it is grabbed, and the grab stays on the screen as a frozen picture (`scanner.rs`). The
//!    island stays above it with the status (`view/scan.rs`): "recognizing" for a moment.
//! 2. The text is read on a thread of its own, the QR codes on another (`crate::text`). When the
//!    text comes, a glint runs over it once, links are underlined, codes framed, and the status
//!    asks to select.
//! 3. The text is selected as ordinary text (`text::session`); the status shows the start of the
//!    selection, and a hint while the pointer is over a link. Ctrl+C or Copy puts it in the
//!    clipboard and the status says so for a second. A click on a code copies its content,
//!    Ctrl+click on a link (or a code with one) opens it.
//! 4. The language and the line breaks are switched on the status and remembered. Another
//!    language reads the same frozen picture again.
//! 5. Alt+drag copies the words in the rectangle and closes the scanner; the status says
//!    "copied" for a second. Esc, a right click or leaving for another window closes it.

use super::{
    App,
    scanner::{self, Input, Scanner},
    timers,
    view::{CopyLook, ScanHit, ScanLine, ScanModel},
    window::WM_SCAN,
};
use crate::{
    capture::{
        Image,
        adapters::{self as system, SystemClipboard},
    },
    fault::{Cause, Failed, Fault, What},
    files::adapters as shell,
    journal,
    lang::tr,
    platform,
    stage::NoticeKind,
    text::{
        Language, Page, Reader,
        adapters::WindowsOcr,
        choose,
        compose::Breaks,
        qr::{self, Code},
        script,
        session::{Cursor, Outcome, Session},
    },
};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
    UI::{Input::KeyboardAndMouse::GetDoubleClickTime, WindowsAndMessaging::PostMessageW},
};

const COPIED_MS: u32 = 1000;

enum Found {
    Text {
        read: u64,
        page: Result<Page, Failed>,
    },
    Codes {
        frame: u64,
        codes: Vec<Code>,
    },
}

struct Open {
    window: Scanner,
    frame: Arc<Image>,
    /// Which grab this is: codes of an earlier one are dropped.
    frame_id: u64,
    /// Which read of the text is awaited: a language switch starts another.
    read: u64,
    languages: Vec<Language>,
    language: Language,
    /// `None` while the text is being read.
    session: Option<Session>,
    /// Codes that came before the text.
    codes: Option<Vec<Code>>,
    list: bool,
    pointer: (f32, f32),
}

pub(super) struct Scan {
    open: Option<Open>,
    copied: bool,
    next_id: u64,
    tx: Sender<Found>,
    rx: Receiver<Found>,
}

impl Default for Scan {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: None,
            copied: false,
            next_id: 0,
            tx,
            rx,
        }
    }
}

/// What the status shows, owned: the model borrows it.
pub(super) struct ScanInput {
    line: Line,
    language: String,
    join: bool,
    copy: CopyLook,
    list: Vec<(String, bool)>,
}

enum Line {
    Reading,
    Hint,
    Empty,
    Link,
    Copied,
    Languages,
    Selected(String),
}

impl ScanInput {
    pub(super) fn model(&self) -> ScanModel<'_> {
        ScanModel {
            line: match &self.line {
                Line::Reading => ScanLine::Reading,
                Line::Hint => ScanLine::Hint,
                Line::Empty => ScanLine::Empty,
                Line::Link => ScanLine::Link,
                Line::Copied => ScanLine::Copied,
                Line::Languages => ScanLine::Languages,
                Line::Selected(text) => ScanLine::Selected(text),
            },
            language: &self.language,
            join: self.join,
            copy: self.copy,
            list: &self.list,
        }
    }
}

fn post(island: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(island as *mut _)), WM_SCAN, WPARAM(0), LPARAM(0));
    }
}

impl App {
    pub(super) fn scanning(&self) -> bool {
        self.scan.open.is_some()
    }

    pub(super) fn is_scanner(&self, hwnd: HWND) -> bool {
        self.scan
            .open
            .as_ref()
            .is_some_and(|o| o.window.hwnd() == hwnd)
    }

    /// The status is on: the scanner is open or has just copied and closed.
    pub(super) fn scan_shown(&self) -> bool {
        self.scan.open.is_some() || self.scan.copied
    }

    pub(super) fn scan_list_open(&self) -> bool {
        self.scan.open.as_ref().is_some_and(|o| o.list)
    }

    pub(super) fn start_scan(&mut self) {
        if self.scanning() {
            return;
        }
        if self.selecting() {
            self.cancel_selection();
        }
        self.close_menu();
        self.kill_timer(timers::SCAN_COPIED);
        self.scan.copied = false;
        let languages = WindowsOcr::new().languages();
        let Some(language) = choose(&languages, &self.settings.scan_language).cloned() else {
            self.report(Fault::of(What::ScanRead, Cause::Unsupported));
            return;
        };
        if !self.settings.scan_language.is_empty() && language.tag != self.settings.scan_language {
            journal::warn(tr!(
                "journal.scan_language_gone",
                tag = self.settings.scan_language,
                used = language.tag
            ));
        }
        // The island leaves the captures before the grab, and stays above the frozen screen.
        self.stage.post(NoticeKind::Scan, self.now_ms());
        self.sync_capture_hiding();
        system::wait_for_hiding();
        let Some((image, _)) = system::capture(self.capture_source()) else {
            self.report(Fault::of(What::ScanOpen, Cause::Graphics));
            self.end_scan();
            return;
        };
        let monitor = platform::monitor_rect(self.monitor);
        let window = match Scanner::open(&self.scene.comp, monitor, self.scale, &image) {
            Ok(window) => window,
            Err(e) => {
                self.report(Fault::new(What::ScanOpen, e.into()));
                self.end_scan();
                return;
            }
        };
        if self.visible {
            platform::show_without_focus(self.island);
        }
        self.scan.next_id += 1;
        let frame = Arc::new(image);
        let id = self.scan.next_id;
        self.find_codes(frame.clone(), id);
        self.scan.open = Some(Open {
            window,
            frame,
            frame_id: id,
            read: 0,
            languages,
            language,
            session: None,
            codes: None,
            list: false,
            pointer: (-1.0, -1.0),
        });
        self.read_text();
        self.after_stage(true);
    }

    fn read_text(&mut self) {
        let Some(open) = &mut self.scan.open else {
            return;
        };
        self.scan.next_id += 1;
        open.read = self.scan.next_id;
        open.session = None;
        let (read, frame, tag) = (open.read, open.frame.clone(), open.language.tag.clone());
        let tx = self.scan.tx.clone();
        let island = self.island.0 as isize;
        let spawned = thread::Builder::new()
            .name("scan text".into())
            .spawn(move || {
                // The reader waits on WinRT's async calls: the multithreaded apartment.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                let page = WindowsOcr::new().read(&frame, &tag).map(|mut page| {
                    script::tidy(&mut page);
                    page
                });
                let _ = tx.send(Found::Text { read, page });
                post(island);
            });
        if let Err(e) = spawned {
            self.report(Fault::new(What::ScanRead, e.into()));
            self.close_scan();
        }
    }

    /// No code is not worth a word: a failed thread only leaves the codes out.
    fn find_codes(&self, frame: Arc<Image>, id: u64) {
        let tx = self.scan.tx.clone();
        let island = self.island.0 as isize;
        let spawned = thread::Builder::new()
            .name("scan codes".into())
            .spawn(move || {
                let codes = qr::find(&frame);
                let _ = tx.send(Found::Codes { frame: id, codes });
                post(island);
            });
        if let Err(e) = spawned {
            journal::warn(tr!("journal.scan_codes_failed", error = e));
        }
    }

    /// `WM_SCAN`: the text or the codes have been read.
    pub(super) fn on_scan_found(&mut self) {
        let found: Vec<Found> = self.scan.rx.try_iter().collect();
        let glint = self.settings.animations;
        for found in found {
            let Some(open) = &mut self.scan.open else {
                return;
            };
            let shown = match found {
                Found::Codes { frame, codes } if frame == open.frame_id => {
                    match &mut open.session {
                        Some(session) => {
                            session.set_codes(codes);
                            Some(false)
                        }
                        None => {
                            open.codes = Some(codes);
                            None
                        }
                    }
                }
                Found::Text { read, page } if read == open.read => match page {
                    Ok(page) => {
                        let codes = open.codes.clone().unwrap_or_default();
                        let double = u64::from(unsafe { GetDoubleClickTime() });
                        open.session = Some(Session::new(&page, codes, double));
                        Some(glint)
                    }
                    Err(failed) => {
                        self.report(Fault::new(What::ScanRead, failed));
                        self.close_scan();
                        return;
                    }
                },
                _ => None,
            };
            if let Some(glint) = shown {
                self.show_found(glint);
            }
        }
        self.render();
    }

    fn show_found(&mut self, glint: bool) {
        let Some(open) = &mut self.scan.open else {
            return;
        };
        let Some(session) = &open.session else {
            return;
        };
        let found = glint.then(|| session.found());
        let shown = open.window.show_found(
            &self.scene.comp,
            &session.underlines(),
            &session.code_boxes(),
            found.as_deref(),
        );
        if let Err(e) = shown {
            journal::warn(tr!("journal.render_failed", error = e));
        }
        self.show_scan_selection();
    }

    fn show_scan_selection(&mut self) {
        let Some(open) = &mut self.scan.open else {
            return;
        };
        let Some(session) = &open.session else {
            return;
        };
        if let Err(e) =
            open.window
                .show_selection(&self.scene.comp, &session.highlight(), session.area())
        {
            journal::warn(tr!("journal.render_failed", error = e));
        }
    }

    pub(super) fn on_scan_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let breaks = self.settings.scan_breaks;
        let now = self.now_ms();
        let open = self
            .scan
            .open
            .as_mut()
            .filter(|o| o.window.hwnd() == hwnd)?;
        let (result, input) = open.window.handle(msg, wparam, lparam);
        let Some(input) = input else {
            return result;
        };
        let session = &mut open.session;
        let outcome = match input {
            Input::Cancel => {
                self.close_scan();
                return result;
            }
            Input::Copy => {
                self.copy_scan();
                return result;
            }
            Input::Cursor | Input::Keys => {
                self.sync_scan_cursor();
                return result;
            }
            Input::Move(x, y) => {
                let (px, py) = std::mem::replace(&mut open.pointer, (x, y));
                let link = |s: &Session| (s.over_link(px, py), s.over_link(x, y));
                let hint_changed = session.as_ref().map(link).is_some_and(|(a, b)| a != b);
                let outcome = session.as_mut().map_or(Outcome::Nothing, |s| s.moved(x, y));
                self.sync_scan_cursor();
                // The link hint follows the pointer even when the selection stays.
                if hint_changed && outcome == Outcome::Nothing {
                    self.render();
                }
                outcome
            }
            Input::Press(x, y, keys) => {
                open.pointer = (x, y);
                let closed_list = std::mem::take(&mut open.list);
                let outcome = session
                    .as_mut()
                    .map_or(Outcome::Nothing, |s| s.press(x, y, keys, now));
                if closed_list && outcome == Outcome::Nothing {
                    self.render();
                }
                outcome
            }
            Input::Release(x, y) => session
                .as_mut()
                .map_or(Outcome::Nothing, |s| s.release(x, y, breaks)),
            Input::SelectAll => session
                .as_mut()
                .map_or(Outcome::Nothing, Session::select_all),
        };
        self.on_scan_outcome(outcome);
        result
    }

    fn on_scan_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Changed => {
                self.show_scan_selection();
                self.render();
            }
            Outcome::Open(target) => self.open_link(&target),
            Outcome::Copy(text) => {
                self.put_scanned(&text);
            }
            Outcome::CopyAndClose(text) => {
                if self.put_scanned(&text) {
                    self.close_scan_copied();
                }
            }
        }
    }

    fn sync_scan_cursor(&self) {
        let Some(open) = &self.scan.open else {
            return;
        };
        let (x, y) = open.pointer;
        let cursor = match &open.session {
            Some(session) => session.cursor(x, y, scanner::keys()),
            None => Cursor::Arrow,
        };
        Scanner::set_cursor(cursor);
    }

    fn open_link(&mut self, target: &str) {
        match shell::open(std::path::Path::new(target)) {
            // The browser comes to the front: the scanner, left behind, closes on its own.
            Ok(()) => {}
            Err(failed) => self.report(Fault::new(What::LinkOpen, failed).about(target)),
        }
    }

    /// Ctrl+C or the Copy circle: the selection in the current line-break mode.
    pub(super) fn copy_scan(&mut self) {
        let breaks = self.settings.scan_breaks;
        let Some(text) = self
            .scan
            .open
            .as_ref()
            .and_then(|o| o.session.as_ref())
            .and_then(|s| s.selected(breaks))
        else {
            return;
        };
        self.put_scanned(&text);
    }

    /// True = it is in the clipboard; the status says so for a second.
    fn put_scanned(&mut self, text: &str) -> bool {
        let mut clipboard = SystemClipboard::new(self.island);
        match clipboard.put_text(text) {
            Ok(()) => {
                self.scan.copied = true;
                self.set_timer(timers::SCAN_COPIED, COPIED_MS);
                self.render();
                true
            }
            Err(failed) => {
                self.report(Fault::new(What::Copy, failed));
                false
            }
        }
    }

    pub(super) fn on_scan_copied_timer(&mut self) {
        self.kill_timer(timers::SCAN_COPIED);
        self.scan.copied = false;
        if self.scan.open.is_none() {
            self.end_scan();
        } else {
            self.render();
        }
    }

    pub(super) fn scan_click(&mut self, hit: ScanHit) {
        let Some(open) = &mut self.scan.open else {
            return;
        };
        let language = match hit {
            ScanHit::Pick(i) => open
                .languages
                .get(i)
                .filter(|l| **l != open.language)
                .cloned(),
            _ => None,
        };
        open.list = hit == ScanHit::Language && !open.list;
        match hit {
            ScanHit::Breaks => {
                self.settings.scan_breaks = self.settings.scan_breaks.toggled();
                self.save_settings();
            }
            ScanHit::Copy => self.copy_scan(),
            ScanHit::Language | ScanHit::Pick(_) => {}
        }
        if let Some(language) = language {
            self.settings.scan_language = language.tag.clone();
            self.save_settings();
            self.switch_language(language);
        }
        self.render();
    }

    /// The same frozen picture is read again; what was found in the old language goes.
    fn switch_language(&mut self, language: Language) {
        let Some(open) = &mut self.scan.open else {
            return;
        };
        open.language = language;
        let comp = &self.scene.comp;
        let cleared = open
            .window
            .show_found(comp, &[], &[], None)
            .and_then(|()| open.window.show_selection(comp, &[], None));
        if let Err(e) = cleared {
            journal::warn(tr!("journal.render_failed", error = e));
        }
        self.read_text();
    }

    pub(super) fn scan_input(&self) -> Option<ScanInput> {
        let join = self.settings.scan_breaks == Breaks::Join;
        let Some(open) = &self.scan.open else {
            // Closed by Alt-drag: "copied" for a moment.
            return self.scan.copied.then(|| ScanInput {
                line: Line::Copied,
                language: String::new(),
                join,
                copy: CopyLook::Done,
                list: Vec::new(),
            });
        };
        let session = open.session.as_ref();
        let selected = session.filter(|s| s.has_selection());
        let (x, y) = open.pointer;
        let line = if self.scan.copied {
            Line::Copied
        } else if open.list {
            Line::Languages
        } else {
            match session {
                None => Line::Reading,
                Some(s) if s.over_link(x, y) => Line::Link,
                Some(s) if s.has_selection() => Line::Selected(s.preview()),
                Some(s) if s.has_text() => Line::Hint,
                Some(_) => Line::Empty,
            }
        };
        let copy = match (self.scan.copied, selected.is_some()) {
            (true, _) => CopyLook::Done,
            (false, true) => CopyLook::Ready,
            (false, false) => CopyLook::Off,
        };
        let list = if open.list {
            open.languages
                .iter()
                .map(|l| (l.name.clone(), *l == open.language))
                .collect()
        } else {
            Vec::new()
        };
        Some(ScanInput {
            line,
            language: open.language.short(),
            join,
            copy,
            list,
        })
    }

    /// Esc, a right click, another window: the scanner closes with nothing copied.
    pub(super) fn close_scan(&mut self) {
        if self.scan.open.take().is_none() {
            return;
        }
        self.scan.copied = false;
        self.kill_timer(timers::SCAN_COPIED);
        self.end_scan();
    }

    /// The Alt rectangle was copied: the window goes, "copied" stays a moment on the status.
    fn close_scan_copied(&mut self) {
        self.scan.open = None;
        self.sync_capture_hiding();
        self.render();
    }

    fn end_scan(&mut self) {
        let changed = self.stage.end(NoticeKind::Scan, self.now_ms());
        self.sync_capture_hiding();
        self.after_stage(changed);
    }
}
