//! The island's side of the settings: it rereads the file when the settings window saves it
//! and hands each feature its part, opens that window, and writes what the island itself
//! changes (the tray's "show always", the recording panel's choices).
//!
//! A feature with a setting of its own gets its `sync_…` call in `check_settings`.

use super::App;
use crate::{
    fault::{Cause, Fault, What},
    lang, persist, platform,
    settings::{Recovery, Settings},
};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

pub(super) fn recovery_fault(recovery: Option<Recovery>) -> Option<Fault> {
    let what = match recovery? {
        Recovery::Restored => What::SettingsRestored,
        Recovery::Reset => What::SettingsReset,
    };
    Some(Fault::of(what, Cause::Unknown))
}

impl App {
    pub(super) fn check_settings(&mut self) {
        let modified = persist::modified(&self.settings_path);
        if modified == self.settings_mtime {
            return;
        }
        let (new, recovery) = Settings::recover(&self.settings_path);
        // Recovery rewrites the file: its new time is not a change to react to.
        self.settings_mtime = persist::modified(&self.settings_path);
        if new.autostart != self.settings.autostart {
            platform::set_autostart(new.autostart);
        }
        let events = self.scheduler.apply(&new.reminders, new.idle_min);
        let screen_changed = new.keep_screen_on != self.settings.keep_screen_on;
        self.keep_screen.set(new.keep_screen_on);
        self.settings = new;
        if screen_changed {
            self.on_controls();
        }
        self.sync_media();
        self.sync_viewer_settings();
        self.sync_watch_settings();
        self.sync_shot_key();
        self.sync_area_key();
        self.sync_record_key();
        self.sync_scan_key();
        self.sync_marks();
        self.sync_downloads();
        self.sync_devices();
        self.sync_explorer();
        self.relayout();
        self.on_reminder_events(&events);
        self.sync_visibility();
        if let Some(fault) = recovery_fault(recovery) {
            self.report(fault);
        }
    }

    /// "Show the island always" in the tray menu.
    pub(super) fn toggle_island(&mut self) {
        self.settings.show_island = !self.settings.show_island;
        self.save_settings();
        self.sync_visibility();
    }

    /// The island changed a setting itself. The file's new time is its own: not a change to
    /// reread.
    pub(super) fn save_settings(&mut self) {
        let saved = self.settings.save(&self.settings_path);
        self.settings_mtime = persist::modified(&self.settings_path);
        if let Err(e) = saved {
            self.report(Fault::new(What::SettingsSave, e.into()));
        }
    }

    pub(super) fn open_settings(&mut self) {
        // A new process can't bring itself to the front: allow it (the island has just received a click or a tray
        // command, so it holds that right).
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
        let language = format!("--lang={}", lang::current().code());
        if let Err(e) = platform::relaunch(&["--settings", &language]) {
            self.report(Fault::new(What::SettingsOpen, e.into()));
        }
    }

    /// The settings window is a process of its own: left open, it would outlive the island and
    /// hold the exe an update has to replace. It writes a change still waiting as it closes.
    pub(super) fn close_settings(&self) {
        for language in lang::Lang::ALL {
            platform::close_window_titled(&lang::tr_for!(language, "settings.window_title"));
        }
    }
}
