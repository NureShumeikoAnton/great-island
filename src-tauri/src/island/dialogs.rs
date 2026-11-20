//! Dialog windows on the island: the window is `dialog/`, what it says `crate::dialog`.
//!
//! 1. A feature shows its dialog with a sheet: a new window in the middle of the island's monitor,
//!    or, when that feature's dialog is open, the same window with the new sheet.
//! 2. One dialog is on screen at a time: another feature's waits and opens when that one has gone.
//! 3. A press comes back to the feature that owns the dialog ([`Owner`]) as its button's action;
//!    a dialog closed by Windows' means (Alt+F4) goes, and its feature is told. Otherwise the
//!    feature closes its dialog itself, when it has nothing more to show.
//! 4. A closed dialog fades out and is dropped after the fade.

use super::{
    App,
    dialog::{Dialog, LEAVE_MS, Request},
    timers,
};
use crate::{
    dialog::{Action, Sheet},
    fault::Failed,
    lang::tr,
    platform,
};
use std::collections::VecDeque;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    Update,
}

impl Owner {
    fn title(self) -> String {
        match self {
            Self::Update => tr!("update.window_title").into_owned(),
        }
    }
}

#[derive(Default)]
pub(super) struct Dialogs {
    open: Option<(Owner, Dialog)>,
    /// Fading out; dropped by `timers::DIALOG_GONE`.
    leaving: Vec<Dialog>,
    waiting: VecDeque<(Owner, Sheet)>,
}

impl App {
    /// Open, or waiting for its turn.
    pub(super) fn has_dialog(&self, owner: Owner) -> bool {
        self.dialogs.open.as_ref().is_some_and(|(o, _)| *o == owner)
            || self.dialogs.waiting.iter().any(|(o, _)| *o == owner)
    }

    /// Err: the window could not be made, nothing is shown.
    pub(super) fn show_dialog(&mut self, owner: Owner, sheet: Sheet) -> Result<(), Failed> {
        match &mut self.dialogs.open {
            Some((o, dialog)) if *o == owner => {
                dialog.set(&self.scene.comp, sheet);
                return Ok(());
            }
            Some(_) => {
                let waiting = &mut self.dialogs.waiting;
                match waiting.iter_mut().find(|(o, _)| *o == owner) {
                    Some(slot) => slot.1 = sheet,
                    None => waiting.push_back((owner, sheet)),
                }
                return Ok(());
            }
            None => {}
        }
        // A fading dialog still stands where the new one opens: the new one waits for it.
        if !self.dialogs.leaving.is_empty() {
            self.dialogs.waiting.push_back((owner, sheet));
            return Ok(());
        }
        self.open_dialog(owner, sheet)
    }

    fn open_dialog(&mut self, owner: Owner, sheet: Sheet) -> Result<(), Failed> {
        let work = platform::work_rect(self.monitor);
        let dialog = Dialog::open(
            &self.scene.comp,
            work,
            self.scale,
            &owner.title(),
            sheet,
            self.settings.animations,
        )?;
        self.dialogs.open = Some((owner, dialog));
        Ok(())
    }

    pub(super) fn close_dialog(&mut self, owner: Owner) {
        self.dialogs.waiting.retain(|(o, _)| *o != owner);
        if !self.dialogs.open.as_ref().is_some_and(|(o, _)| *o == owner) {
            return;
        }
        let Some((_, dialog)) = self.dialogs.open.take() else {
            return;
        };
        if dialog.decor() {
            dialog.leave(&self.scene.comp);
            self.dialogs.leaving.push(dialog);
            self.set_timer(timers::DIALOG_GONE, LEAVE_MS);
        } else {
            drop(dialog);
            self.open_next_dialog();
        }
    }

    pub(super) fn on_dialog_gone(&mut self) {
        self.kill_timer(timers::DIALOG_GONE);
        self.dialogs.leaving.clear();
        self.open_next_dialog();
    }

    fn open_next_dialog(&mut self) {
        if self.dialogs.open.is_some() {
            return;
        }
        while let Some((owner, sheet)) = self.dialogs.waiting.pop_front() {
            match self.open_dialog(owner, sheet) {
                Ok(()) => return,
                Err(failed) => self.dialog_failed(owner, failed),
            }
        }
    }

    /// A message to one of the dialog windows; `None` — Windows' default handling.
    pub(super) fn on_dialog_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let (owner, dialog) = self
            .dialogs
            .open
            .as_mut()
            .filter(|(_, d)| d.hwnd() == hwnd)?;
        let owner = *owner;
        let (result, request) = dialog.handle(&self.scene.comp, msg, wparam, lparam);
        match request {
            Some(Request::Action(action)) => self.dialog_action(owner, action),
            Some(Request::Closed) => {
                self.close_dialog(owner);
                self.dialog_closed(owner);
            }
            None => {}
        }
        result
    }

    /// Quitting: every dialog goes at once.
    pub(super) fn drop_dialogs(&mut self) {
        self.kill_timer(timers::DIALOG_GONE);
        self.dialogs = Dialogs::default();
    }

    fn dialog_action(&mut self, owner: Owner, action: Action) {
        match owner {
            Owner::Update => self.on_update_answer(action),
        }
    }

    fn dialog_closed(&mut self, owner: Owner) {
        match owner {
            Owner::Update => self.on_update_dialog_closed(),
        }
    }

    fn dialog_failed(&mut self, owner: Owner, failed: Failed) {
        match owner {
            Owner::Update => self.update_dialog_failed(failed),
        }
    }
}
