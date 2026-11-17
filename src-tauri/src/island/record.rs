//! Screen recording on the island.
//!
//! 1. The record key or the start in the pre-start panel starts it — at once, or after a
//!    countdown; the control center's record button opens that panel. What the recording
//!    is like (sound, countdown, quality, codec, frames per second) is the panel's choice
//!    (`record_panel.rs`). The
//!    monitor with the island is recorded, or the area picked in the panel for this one
//!    recording; red corner marks show that area from the panel to the stop, unless the
//!    settings turn them off. The island and the marks are hidden from capture for the
//!    whole time.
//!    When the microphone is wanted but can't be opened, the press only says so and how to
//!    go on; the next press records without it (the journal keeps the reason), until it
//!    works again. A chosen microphone that isn't plugged in gives way to the Windows default.
//!    Losing the microphone or the computer's sound mid-way is a warning; the recording goes on.
//! 2. While it runs the island shows REC, time, pause and stop; on hover — file size
//!    and a cancel knob to slide. The control center's button shows the time and stops it too.
//! 3. Stop: the recording thread closes the file and sends the last frame; the island
//!    shows the card, like a screenshot's: the thumbnail opens the video window (`watch.rs`),
//!    text or Ctrl+C copies the file, Ctrl+X leaves it only in the clipboard.
//!
//! What happens to a finished recording is decided by [`Take`]; this is the island side.
//! What it shares with the screenshot (the source, hiding the island, the card) is `capture.rs`,
//! its keys are in `hotkeys.rs`.

use super::{
    App,
    capture::{PREVIEW_WIDTH, QUICK_CLOSE_MS, THUMB_WIDTH},
    marks::{LEAVE_MS, Marks},
    timers,
};
use crate::{
    capture::{
        Image, Source,
        adapters::{self as system, SystemClipboard},
    },
    fault::{Fault, What},
    journal,
    lang::tr,
    record::{
        Clock, Take,
        adapters::{self as files, Ending, Engine, Event, Note, Setup, TempShelf},
        choice::Choice,
        file_stem,
        sound::{Input, Sound},
    },
    stage::NoticeKind,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

pub(super) struct Live {
    engine: Engine,
    /// Island ms, pauses excluded: the time on the island.
    clock: Clock,
    bytes: u64,
    closing: bool,
}

impl Live {
    pub(super) fn elapsed(&self, now: u64) -> u64 {
        self.clock.elapsed(now)
    }

    pub(super) fn still(&self) -> bool {
        self.clock.is_paused() || self.closing
    }

    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }
}

pub(super) struct Clip {
    pub(super) take: Take,
    pub(super) thumb: Arc<Image>,
    /// The last frame at the flight's size: it flies into the video window.
    pub(super) still: Arc<Image>,
}

pub(super) struct Recording {
    pub(super) live: Option<Live>,
    /// Seconds left before the recording starts.
    pub(super) countdown: Option<u32>,
    pub(super) clip: Option<Clip>,
    /// Since when the recording counts (island ms), for the control center's button.
    since: Rc<Cell<Option<u64>>>,
    /// What the recording being started will hear: decided at the press, before the countdown.
    sound: Sound,
    /// The missing microphone was already said: the next press records without it.
    mic_said: bool,
    /// The area picked in the panel for the recording being started or running; `None` —
    /// the whole monitor.
    pub(super) area: Option<Source>,
    marks: Option<Marks>,
    /// Marks flying off, closed by a timer.
    leaving: Option<Marks>,
}

impl Recording {
    pub(super) fn new(since: Rc<Cell<Option<u64>>>) -> Self {
        Self {
            live: None,
            countdown: None,
            clip: None,
            since,
            sound: Sound::Off,
            mic_said: false,
            area: None,
            marks: None,
            leaving: None,
        }
    }
}

impl App {
    /// Recording or counting down to it.
    pub(super) fn recording_on(&self) -> bool {
        self.recording.live.is_some() || self.recording.countdown.is_some()
    }

    pub(super) fn toggle_recording(&mut self) {
        if self.recording.countdown.is_some() {
            self.cancel_countdown();
        } else if self.recording.live.is_some() {
            self.stop_recording();
        } else if let Some(sound) = self.sound_to_record() {
            self.recording.sound = sound;
            if self.settings.record.countdown.seconds() > 0 {
                self.start_countdown();
            } else {
                self.begin_recording();
            }
        } else {
            // Only a warning this time; the area was for this press.
            self.recording.area = None;
            self.sync_marks();
        }
    }

    /// The marks show the area of the next recording: picked in the panel (a preview), then
    /// through the countdown and the recording. A new area gets new marks.
    pub(super) fn sync_marks(&mut self) {
        let area = self
            .recording
            .area
            .map(|source| source.area)
            .or_else(|| self.panel_area())
            .filter(|_| self.settings.area_marks);
        if self.recording.marks.as_ref().map(Marks::area) == area {
            return;
        }
        if let Some(old) = self.recording.marks.take() {
            self.let_marks_go(old);
        }
        let Some(area) = area else {
            return;
        };
        let monitor = crate::platform::monitor_rect(self.monitor);
        match Marks::new(&self.scene.comp, monitor, self.scale, area) {
            Ok(marks) => {
                system::hide_from_capture(marks.hwnd(), true);
                marks.show();
                self.recording.marks = Some(marks);
            }
            Err(e) => journal::warn(tr!("journal.marks_failed", error = e)),
        }
    }

    fn let_marks_go(&mut self, marks: Marks) {
        match marks.leave(&self.scene.comp) {
            Ok(()) => {
                self.recording.leaving = Some(marks);
                self.set_timer(timers::MARKS_GONE, LEAVE_MS);
            }
            Err(e) => journal::warn(tr!("journal.marks_failed", error = e)),
        }
    }

    pub(super) fn on_marks_gone(&mut self) {
        self.kill_timer(timers::MARKS_GONE);
        self.recording.leaving = None;
    }

    /// `None` — the microphone is missing and this press only says so.
    fn sound_to_record(&mut self) -> Option<Sound> {
        let wanted = self.settings.record.sound;
        if !wanted.microphone() {
            return Some(wanted);
        }
        let Err(failed) = files::microphone_ready(self.settings.record.microphone.as_deref())
        else {
            self.recording.mic_said = false;
            return Some(wanted);
        };
        let fault = Fault::new(What::RecordMic, failed);
        if std::mem::replace(&mut self.recording.mic_said, true) {
            journal::fault(&fault);
            return Some(wanted.without_microphone());
        }
        self.report(fault);
        None
    }

    fn start_countdown(&mut self) {
        self.recording.countdown = Some(self.settings.record.countdown.seconds());
        self.sync_escape();
        let now = self.now_ms();
        self.stage.post(NoticeKind::Countdown, now);
        self.close_menu();
        self.set_timer(timers::COUNTDOWN, 1000);
        self.sync_marks();
    }

    pub(super) fn on_countdown_timer(&mut self) {
        match self.recording.countdown {
            Some(left) if left > 1 => {
                self.recording.countdown = Some(left - 1);
                self.render();
            }
            Some(_) => self.begin_recording(),
            None => self.kill_timer(timers::COUNTDOWN),
        }
    }

    pub(super) fn cancel_countdown(&mut self) {
        if self.end_countdown() {
            self.recording.area = None;
            self.sync_marks();
            let changed = self.stage.end(NoticeKind::Countdown, self.now_ms());
            self.after_stage(changed);
        }
    }

    /// False — there was no countdown.
    fn end_countdown(&mut self) -> bool {
        self.kill_timer(timers::COUNTDOWN);
        let was = self.recording.countdown.take().is_some();
        self.sync_escape();
        was
    }

    pub(super) fn begin_recording(&mut self) {
        let now = self.now_ms();
        if self.end_countdown() {
            self.stage.end(NoticeKind::Countdown, now);
        }
        if self.recording.live.is_some() {
            return;
        }
        // The previous card goes: a new recording is more important.
        self.stage.end(NoticeKind::Clip, now);
        self.sync_clip();

        let source = self.recording.area.unwrap_or_else(|| self.capture_source());
        let (dir, fallback) = files::usable_dir(&self.settings.recordings_dir);
        let stem = file_stem(&system::now());
        // HWND can't cross threads, a number can.
        let service = self.service.0 as isize;
        let notify = move || unsafe {
            let _ = PostMessageW(
                Some(HWND(service as *mut _)),
                super::window::WM_RECORD,
                WPARAM(0),
                LPARAM(0),
            );
        };
        // Before the first frame is captured; from here on `sync_capture_hiding` keeps it.
        system::hide_from_capture(self.island, true);
        let setup = Setup {
            thumb_width: PREVIEW_WIDTH,
            choice: Choice {
                sound: self.recording.sound,
                ..self.settings.record.clone()
            },
        };
        let engine = match Engine::start(source, &dir, &stem, setup, notify) {
            Ok(engine) => engine,
            Err(failed) => {
                self.sync_capture_hiding();
                self.recording.area = None;
                self.sync_marks();
                let changed = self.stage.tick(now);
                self.after_stage(changed);
                self.report(Fault::new(What::RecordStart, failed));
                return;
            }
        };
        self.recording.live = Some(Live {
            engine,
            clock: Clock::start(now),
            bytes: 0,
            closing: false,
        });
        self.recording.since.set(Some(now));
        self.sync_marks();
        self.close_menu();
        let changed = self.stage.post(NoticeKind::Recording, now);
        self.after_stage(changed);
        self.render();
        if let Some(failed) = fallback {
            self.report(Fault::new(What::RecordFolder, failed));
        }
    }

    pub(super) fn toggle_pause(&mut self) {
        let now = self.now_ms();
        let Some(live) = self.recording.live.as_mut().filter(|l| !l.closing) else {
            return;
        };
        if live.clock.is_paused() {
            live.clock.resume(now);
            live.engine.resume();
        } else {
            live.clock.pause(now);
            live.engine.pause();
        }
        self.sync_record_time();
        self.render();
    }

    pub(super) fn stop_recording(&mut self) {
        self.close_recording(false);
    }

    pub(super) fn cancel_recording(&mut self) {
        self.close_recording(true);
    }

    fn close_recording(&mut self, discard: bool) {
        let now = self.now_ms();
        let Some(live) = self.recording.live.as_mut().filter(|l| !l.closing) else {
            return;
        };
        live.clock.pause(now);
        live.closing = true;
        if discard {
            live.engine.cancel();
        } else {
            live.engine.stop();
        }
        self.render();
    }

    pub(super) fn sync_record_time(&mut self) {
        let now = self.now_ms();
        let Some(live) = self.recording.live.as_mut() else {
            self.recording.since.set(None);
            return;
        };
        live.bytes = live.engine.bytes();
        self.recording
            .since
            .set(Some(now - live.clock.elapsed(now)));
    }

    pub(super) fn on_record_events(&mut self) {
        let events = match &self.recording.live {
            Some(live) => live.engine.drain(),
            None => return,
        };
        for event in events {
            if let Event::Started = event {
                // Capture and encoder took a moment to come up: the time counts from here.
                let now = self.now_ms();
                if let Some(live) = self.recording.live.as_mut().filter(|l| !l.still()) {
                    live.clock = Clock::start(now);
                }
                journal::info(tr!("journal.record_started"));
                continue;
            }
            if let Event::Sound(note) = event {
                self.on_record_sound(note);
                continue;
            }
            if let Event::FellBack(failed) = event {
                self.report(Fault::new(What::RecordHevc, failed));
                continue;
            }
            // The thread is done; dropping the engine just joins it.
            self.recording.live = None;
            self.recording.since.set(None);
            self.recording.area = None;
            self.sync_marks();
            self.sync_capture_hiding();
            let now = self.now_ms();
            let mut changed = self.stage.end(NoticeKind::Recording, now);
            match event {
                Event::Started | Event::Sound(_) | Event::FellBack(_) => {}
                Event::Finished {
                    path,
                    duration_ms,
                    bytes,
                    size,
                    thumb,
                    ending,
                } => {
                    journal::info(tr!(
                        "journal.record_saved",
                        seconds = duration_ms / 1000,
                        kb = bytes / 1024
                    ));
                    // Reported before the card is posted: the reason shows first, and the
                    // card then gets its full time.
                    match ending {
                        Ending::Asked => {}
                        Ending::Stopped(cause) => {
                            self.report(Fault::of(What::RecordStopped, cause));
                        }
                        Ending::Broken(failed) => {
                            self.report(Fault::new(What::RecordBroken, failed));
                        }
                    }
                    let still = thumb.unwrap_or_else(blank_thumb);
                    self.recording.clip = Some(Clip {
                        take: Take::new(path, duration_ms, bytes, size),
                        thumb: Arc::new(still.preview(THUMB_WIDTH)),
                        still: Arc::new(still),
                    });
                    changed |= self.stage.post(NoticeKind::Clip, now);
                    self.sync_card_keys();
                }
                Event::Cancelled => changed |= self.stage.post(NoticeKind::Discarded, now),
                Event::Failed { started, failed } => {
                    let what = if started {
                        What::RecordLost
                    } else {
                        What::RecordStart
                    };
                    self.report(Fault::new(what, failed));
                }
            }
            self.after_stage(changed);
        }
    }

    fn on_record_sound(&mut self, note: Note) {
        match note {
            Note::Lost(Input::Microphone, failed) => {
                self.report(Fault::new(What::RecordMicLost, failed));
            }
            Note::Lost(Input::System, failed) => {
                self.report(Fault::new(What::RecordSound, failed));
            }
            Note::Back(Input::Microphone) => journal::info(tr!("journal.record_mic_back")),
            Note::Back(Input::System) => journal::info(tr!("journal.record_system_back")),
            Note::WithIsland(failed) => {
                journal::warn(tr!("journal.record_with_island", error = failed.detail));
            }
        }
    }

    pub(super) fn showing_clip(&self) -> bool {
        self.recording.clip.is_some()
            && self
                .stage
                .shown()
                .notice
                .is_some_and(|n| n.kind == NoticeKind::Clip)
    }

    pub(super) fn copy_clip(&mut self, only: bool) {
        let mut clipboard = SystemClipboard::new(self.island);
        let Some(clip) = &mut self.recording.clip else {
            return;
        };
        let done = if only {
            clip.take.cut(&mut clipboard, &mut TempShelf::new())
        } else {
            clip.take.copy(&mut clipboard)
        };
        match done {
            Ok(()) => {
                self.set_timer(timers::CLIP_CLOSE, QUICK_CLOSE_MS);
                self.render();
            }
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    pub(super) fn on_clip_close(&mut self) {
        self.kill_timer(timers::CLIP_CLOSE);
        let changed = self.stage.end(NoticeKind::Clip, self.now_ms());
        self.after_stage(changed);
    }

    pub(super) fn sync_clip(&mut self) {
        if self.recording.clip.is_some() && !self.stage.has(NoticeKind::Clip) {
            self.recording.clip = None;
            self.kill_timer(timers::CLIP_CLOSE);
            self.sync_card_keys();
        }
    }

    /// Quitting or logging off: the file is closed properly before the process ends.
    pub(super) fn finish_recording(&mut self) {
        self.end_countdown();
        // Dropping the engine stops the recording and waits for the file.
        self.recording.live = None;
        self.recording.marks = None;
        self.recording.leaving = None;
        self.recording.since.set(None);
    }

    pub(super) fn sweep_recordings(&self) {
        std::thread::spawn(|| TempShelf::new().sweep());
    }
}

/// When the last frame could not be read back: a dark placeholder of screen proportions.
fn blank_thumb() -> Image {
    let (width, height) = (48, 30);
    let mut pixels = vec![0x2C; (width * height * 4) as usize];
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    Image {
        width,
        height,
        pixels,
    }
}
