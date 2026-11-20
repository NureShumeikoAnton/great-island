//! Global keys: they reach the island whichever window has focus.
//!
//! - the screenshot, area screenshot and record keys come from the settings; a key another
//!   program holds is reported once ([`Slot`]);
//! - Ctrl+C and Ctrl+X belong to the island while a capture card exists;
//! - Esc is the island's while there is something to cancel: the area selection, the
//!   recording panel waiting after it, the countdown before a recording;
//! - while Explorer is in front, copies of the keys for Space (raw input: nothing is taken).
//!
//! A new key is an id here, a line in `on_hotkey` and a method that takes and releases it.
//! What the key does is its feature's business.

use super::{App, screenshot::Trigger};
use crate::{
    fault::{Cause, Fault, What},
    files::adapters::explorer,
    hotkey::{Hotkey, Slot},
    journal,
    lang::tr,
    platform,
};
use windows::Win32::Foundation::HWND;

const COPY: i32 = 1;
const CUT: i32 = 2;
const SHOT: i32 = 3;
const RECORD: i32 = 4;
const ESCAPE: i32 = 5;
const AREA: i32 = 6;

/// By letter, not by place: Ctrl+C is the key that types C in the current layout.
const CARD_KEYS: [(i32, u8); 2] = [(COPY, b'C'), (CUT, b'X')];
const VK_ESCAPE: u16 = 0x1B;

#[derive(Default)]
pub(super) struct Hotkeys {
    shot: Slot,
    area: Slot,
    record: Slot,
    card_keys_taken: bool,
    escape_taken: bool,
    explorer_keys: bool,
}

fn sync_slot(slot: &mut Slot, island: HWND, id: i32, wanted: Option<Hotkey>) -> Option<Hotkey> {
    slot.sync(
        wanted,
        |key| platform::register_hotkey(Some(island), id, key),
        || platform::unregister_hotkey(Some(island), id),
    )
}

impl App {
    pub(super) fn on_hotkey(&mut self, id: i32) {
        // The capture takes the island's monitor, and the island trails the cursor by up to a tick.
        if matches!(id, SHOT | AREA | RECORD) {
            self.follow_monitor();
        }
        match id {
            SHOT => self.start_shutter(Trigger::Key),
            AREA => self.start_shutter(Trigger::AreaKey),
            RECORD => self.toggle_recording(),
            COPY => self.copy_card(false),
            CUT => self.copy_card(true),
            ESCAPE if self.selecting() => self.cancel_selection(),
            ESCAPE if self.record_panel_waits() => self.close_menu(),
            ESCAPE => self.cancel_countdown(),
            _ => {}
        }
    }

    /// Plain PrtSc is not taken while Windows Snipping Tool holds it: otherwise both would react.
    pub(super) fn sync_shot_key(&mut self) {
        let wanted = self
            .settings
            .screenshot_key
            .clone()
            .filter(|key| !(key.is_print_screen() && platform::snipping_owns_print_screen()));
        if let Some(key) = sync_slot(&mut self.hotkeys.shot, self.island, SHOT, wanted) {
            self.report(Fault::of(What::ShotKey, Cause::Taken).about(key.label()));
        }
    }

    /// Not taken when the screenshot key is the same.
    pub(super) fn sync_area_key(&mut self) {
        let wanted = self
            .settings
            .area_key
            .clone()
            .filter(|key| Some(key) != self.settings.screenshot_key.as_ref());
        if let Some(key) = sync_slot(&mut self.hotkeys.area, self.island, AREA, wanted) {
            self.report(Fault::of(What::AreaKey, Cause::Taken).about(key.label()));
        }
    }

    /// The record key is not taken when a screenshot key already has the same one.
    pub(super) fn sync_record_key(&mut self) {
        let wanted = self.settings.record_key.clone().filter(|key| {
            Some(key) != self.settings.screenshot_key.as_ref()
                && Some(key) != self.settings.area_key.as_ref()
        });
        if let Some(key) = sync_slot(&mut self.hotkeys.record, self.island, RECORD, wanted) {
            self.report(Fault::of(What::RecordKey, Cause::Taken).about(key.label()));
        }
    }

    /// If another program already holds one of the keys, it simply stays with it.
    pub(super) fn sync_card_keys(&mut self) {
        let wanted = self.has_card();
        if wanted == self.hotkeys.card_keys_taken {
            return;
        }
        self.hotkeys.card_keys_taken = wanted;
        for (id, key) in CARD_KEYS {
            if !wanted {
                platform::unregister_hotkey(Some(self.island), id);
            } else if let Err(e) =
                platform::register_fixed_hotkey(self.island, id, true, u16::from(key))
            {
                journal::warn(tr!(
                    "journal.shot_chord_taken",
                    key = key as char,
                    error = e
                ));
            }
        }
    }

    /// Copies of the keys while Explorer is in front, for Space (`explorer.rs`). They are not
    /// taken: Explorer gets every key as usual.
    pub(super) fn sync_explorer_keys(&mut self) {
        let wanted = self.settings.explorer_space && self.explorer.front.is_some();
        if wanted == self.hotkeys.explorer_keys {
            return;
        }
        self.hotkeys.explorer_keys = wanted;
        if let Err(failed) = explorer::listen_keys(wanted.then_some(self.service)) {
            journal::warn(tr!("journal.explorer_keys_failed", error = failed.detail));
        }
    }

    pub(super) fn sync_escape(&mut self) {
        let wanted =
            self.selecting() || self.record_panel_waits() || self.recording.countdown.is_some();
        if wanted == self.hotkeys.escape_taken {
            return;
        }
        self.hotkeys.escape_taken = wanted;
        if wanted {
            let _ = platform::register_fixed_hotkey(self.island, ESCAPE, false, VK_ESCAPE);
        } else {
            platform::unregister_hotkey(Some(self.island), ESCAPE);
        }
    }
}
