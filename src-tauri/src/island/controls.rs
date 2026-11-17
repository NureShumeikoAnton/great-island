//! The control center on the island (the menu pulled down from it): its system parts, what
//! a press or a hold leads to, and when the menu closes.
//!
//! The buttons and their states are `crate::control`.

use super::{App, log, screenshot::Trigger, timers, view::HOLD_MS, window::WM_CONTROLS};
use crate::{
    control::{
        ControlId, Flag, Outcome, Ports, Switch, Unavailable,
        adapters::{DeviceMute, Flow, KeepAwake, Notify},
    },
    fault::{Cause, Fault, What},
    journal,
    lang::tr,
    stage::NoticeKind,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

const MENU_CLOSE_MS: u32 = 300;

/// System parts of the control-center buttons. Sound and microphone report an outside toggle with a message
/// to the service window; if connecting failed the button is unavailable.
pub(super) fn ports(
    service: HWND,
    quiet: bool,
    keep_screen: Rc<Cell<bool>>,
    recording: Rc<Cell<Option<u64>>>,
) -> Ports {
    // A HWND can't be passed to another thread, a number can.
    let target = service.0 as isize;
    let notify: Notify = Arc::new(move || unsafe {
        let _ = PostMessageW(
            Some(HWND(target as *mut _)),
            WM_CONTROLS,
            WPARAM(0),
            LPARAM(0),
        );
    });
    let device = |flow| -> Box<dyn Switch> {
        match DeviceMute::new(flow, notify.clone()) {
            Ok(mute) => Box::new(mute),
            Err(e) => {
                journal::warn(tr!(
                    "journal.device_unavailable",
                    device = format!("{flow:?}"),
                    error = e
                ));
                Box::new(Unavailable)
            }
        }
    };
    Ports {
        sound: device(Flow::Output),
        microphone: device(Flow::Input),
        quiet: Box::new(Flag::new(quiet)),
        awake: Box::new(KeepAwake::new(keep_screen)),
        recording,
    }
}

impl App {
    pub(super) fn press_control(&mut self, id: ControlId) {
        match self.controls.press(id, self.now_ms()) {
            Outcome::Stay => {}
            Outcome::Refused => match id {
                ControlId::Sound | ControlId::Microphone => {
                    let what = if id == ControlId::Sound {
                        What::Sound
                    } else {
                        What::Microphone
                    };
                    let fault = match self.controls.refusal(id) {
                        Some(failed) => Fault::new(what, failed),
                        None => Fault::of(what, Cause::Missing),
                    };
                    self.report(fault);
                }
                _ => journal::warn(tr!("journal.button_failed", id = format!("{id:?}"))),
            },
            Outcome::Screenshot => self.start_shutter(Trigger::Click),
            Outcome::Record if self.recording_on() => self.toggle_recording(),
            Outcome::Record => self.open_record_panel(),
            Outcome::OpenSettings => {
                self.close_menu();
                self.open_settings();
            }
        }
        if id == ControlId::DoNotDisturb {
            self.sync_quiet();
            self.save();
        }
        self.render();
    }

    fn sync_quiet(&mut self) {
        let quiet = self.controls.is_on(ControlId::DoNotDisturb);
        let changed = self.stage.set_quiet(quiet, self.now_ms());
        self.after_stage(changed);
    }

    /// The system reported changes: the buttons are reread. Sound or microphone toggled from outside is shown
    /// by a short indicator; the button already accounted for its own press, so that one isn't visible here.
    pub(super) fn on_controls(&mut self) {
        let now = self.now_ms();
        let mut changed = false;
        for id in self.controls.refresh() {
            let kind = match id {
                ControlId::Sound => NoticeKind::Sound,
                ControlId::Microphone => NoticeKind::Microphone,
                _ => continue,
            };
            changed |= self.stage.post(kind, now);
        }
        self.after_stage(changed);
        if self.visible {
            self.render();
        }
    }

    pub(super) fn hold_control(&mut self, id: ControlId) {
        if !id.holds() || self.stage.has(NoticeKind::Shutter) {
            return;
        }
        self.pointer.held = Some(id);
        self.set_timer(timers::BUTTON_HOLD, HOLD_MS);
        log(self.scene.hold_button(id, true));
    }

    /// Let go, or the pointer went elsewhere: the ring runs back.
    pub(super) fn release_control(&mut self) {
        if let Some(id) = self.pointer.held.take() {
            self.kill_timer(timers::BUTTON_HOLD);
            log(self.scene.hold_button(id, false));
        }
    }

    pub(super) fn on_button_hold_timer(&mut self) {
        self.kill_timer(timers::BUTTON_HOLD);
        let Some(id) = self.pointer.held.take() else {
            return;
        };
        if !self.stage.shown().menu {
            return;
        }
        // The press became a hold: letting go now must not also click.
        self.pointer.pressed = None;
        let _ = self.pointer.gestures.cancel();
        if id == ControlId::Screenshot {
            self.start_shutter(Trigger::Hold);
        }
    }

    pub(super) fn close_menu(&mut self) {
        self.kill_timer(timers::MENU);
        let changed = self.stage.close_menu(self.now_ms());
        self.after_stage(changed);
    }

    /// The cursor is back on the island: an open menu stays.
    pub(super) fn keep_menu(&self) {
        self.kill_timer(timers::MENU);
    }

    /// The cursor left: an open menu closes after a moment.
    pub(super) fn close_menu_soon(&self) {
        if self.stage.shown().menu {
            self.set_timer(timers::MENU, MENU_CLOSE_MS);
        }
    }

    pub(super) fn on_menu_timer(&mut self) {
        self.kill_timer(timers::MENU);
        // While a gesture runs (pulling up at the edge) the menu isn't closed from under the finger.
        if !self.pointer.inside && !self.pointer.gestures.is_active() {
            self.close_menu();
        }
    }
}
