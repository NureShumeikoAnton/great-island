//! Problems on the island: the one place where every failure ends up.
//!
//! 1. Whoever meets a failure calls [`App::report`]. It goes to the journal and onto the
//!    island: a capsule for a few seconds, the reason under the cursor, a click closes it.
//! 2. A notice the cursor never came to — or a problem found while the island was
//!    starting, when nobody may be looking — is kept as missed: a small mark in the
//!    collapsed island. Two seconds after the cursor next comes to the island it is shown.
//! 3. Other threads report through a [`Reporter`].

use super::{App, log, timers, window::WM_FAULT};
use crate::{
    fault::{Cause, Fault, Level, What},
    journal,
    lang::tr,
    stage::NoticeKind,
};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, Sender},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

/// Missed problems kept for showing; older ones stay in the journal only.
const MISSED_MAX: usize = 3;
/// After the cursor comes to the island: first let the user do what they came for.
const REPLAY_AFTER_MS: u32 = 2000;
const REPLAY_NEXT_MS: u32 = 400;

pub(super) struct Problems {
    pub(super) shown: Option<Shown>,
    /// Not seen yet, oldest first.
    pub(super) missed: VecDeque<Fault>,
    /// A missed problem is on the island; the next one follows it.
    replaying: bool,
    /// The island is starting: nobody may be looking, problems go straight to `missed`.
    pub(super) booting: bool,
    faults: Receiver<Fault>,
    tx: Sender<Fault>,
}

impl Problems {
    pub(super) fn new() -> Self {
        let (tx, faults) = mpsc::channel();
        Self {
            shown: None,
            missed: VecDeque::new(),
            replaying: false,
            booting: true,
            faults,
            tx,
        }
    }

    /// A problem met before the island exists.
    pub(super) fn early(&mut self, fault: Fault) {
        journal::fault(&fault);
        self.missed.push_back(fault);
    }
}

pub(super) struct Shown {
    pub(super) fault: Fault,
    /// The cursor was on it, or it is a replay: it won't be kept as missed.
    pub(super) seen: bool,
}

#[derive(Clone)]
pub(super) struct Reporter {
    tx: Sender<Fault>,
    /// The service window; HWND can't cross threads, a number can.
    target: isize,
}

impl Reporter {
    pub(super) fn new(tx: Sender<Fault>, service: HWND) -> Self {
        Self {
            tx,
            target: service.0 as isize,
        }
    }

    pub(super) fn send(&self, fault: Fault) {
        let _ = self.tx.send(fault);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.target as *mut _)),
                WM_FAULT,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

fn kind(level: Level) -> NoticeKind {
    match level {
        Level::Warning => NoticeKind::Warning,
        Level::Critical => NoticeKind::Failure,
    }
}

impl App {
    pub(super) fn reporter(&self) -> Reporter {
        Reporter::new(self.problems.tx.clone(), self.service)
    }

    pub(super) fn report(&mut self, fault: Fault) {
        journal::fault(&fault);
        if self.problems.booting {
            self.miss(fault);
        } else {
            self.show_problem(fault, false);
        }
    }

    pub(super) fn on_faults(&mut self) {
        let faults: Vec<Fault> = self.problems.faults.try_iter().collect();
        for fault in faults {
            self.report(fault);
        }
    }

    fn show_problem(&mut self, fault: Fault, replay: bool) {
        let now = self.now_ms();
        let level = fault.level();
        // The answer to a press in the menu must not wait for the menu to close.
        self.close_menu();
        // One problem at a time: the new one takes the place of the old.
        if let Some(old) = self.problems.shown.take()
            && !old.seen
        {
            self.miss(old.fault);
        }
        let mut changed = self.stage.end(NoticeKind::Warning, now);
        changed |= self.stage.end(NoticeKind::Failure, now);
        self.problems.shown = Some(Shown {
            fault,
            seen: replay,
        });
        changed |= self.stage.post(kind(level), now);
        self.after_stage(changed);
        self.render();
        let on_screen = self
            .stage
            .shown()
            .notice
            .is_some_and(|n| n.kind.is_problem());
        if level == Level::Critical && on_screen && self.visible {
            log(self.scene.shake());
        }
    }

    pub(super) fn sync_problem(&mut self) {
        let shown = self.stage.has(NoticeKind::Warning) || self.stage.has(NoticeKind::Failure);
        if shown {
            return;
        }
        let Some(gone) = self.problems.shown.take() else {
            return;
        };
        if !gone.seen {
            self.miss(gone.fault);
        }
        if std::mem::take(&mut self.problems.replaying) && !self.problems.missed.is_empty() {
            self.set_timer(timers::REPLAY, REPLAY_NEXT_MS);
        }
    }

    fn miss(&mut self, fault: Fault) {
        self.problems.missed.retain(|f| f.what != fault.what);
        self.problems.missed.push_back(fault);
        while self.problems.missed.len() > MISSED_MAX {
            self.problems.missed.pop_front();
        }
        if self.visible {
            self.render();
        }
    }

    pub(super) fn on_island_touched(&mut self) {
        if !self.problems.missed.is_empty() && self.problems.shown.is_none() {
            self.set_timer(timers::REPLAY, REPLAY_AFTER_MS);
        }
    }

    pub(super) fn on_replay(&mut self) {
        self.kill_timer(timers::REPLAY);
        // A fresh problem is on the island: the missed ones wait for the next visit.
        if self.problems.shown.is_some() {
            return;
        }
        if let Some(fault) = self.problems.missed.pop_front() {
            self.show_problem(fault, true);
            self.problems.replaying = true;
        }
    }

    pub(super) fn demo_problem(&mut self, critical: bool) {
        let sample = if critical {
            Fault::of(What::ShotSave, Cause::DiskFull)
        } else {
            Fault::of(What::Copy, Cause::Busy)
        };
        self.report(Fault {
            detail: tr!("journal.demo").into(),
            ..sample
        });
    }

    pub(super) fn mark_problem_seen(&mut self) {
        if let Some(problem) = &mut self.problems.shown {
            problem.seen = true;
        }
    }

    pub(super) fn dismiss_problem(&mut self) {
        self.mark_problem_seen();
        let now = self.now_ms();
        let mut changed = self.stage.end(NoticeKind::Warning, now);
        changed |= self.stage.end(NoticeKind::Failure, now);
        self.after_stage(changed);
    }
}
