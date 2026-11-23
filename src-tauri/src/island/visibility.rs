//! When the island is on screen, and when it dims: a fullscreen app, "hide" from the tray,
//! the "show always" setting, and rest with no pointer over it.

use super::{App, log, timers, view::Mode};
use crate::{persist::Hidden, platform, settings::Fullscreen};

const HIDE_DELAY_MS: u32 = 450;
const DIM_DELAY_MS: u32 = 10_000;

impl App {
    /// Rules in order: a critical charge shows always; fullscreen "Hide" hides everything; an island
    /// hidden from the tray lets only priority notices through; fullscreen "Alerts" and "Show always"
    /// turned off let any notice through.
    fn wants_visible(&self) -> bool {
        let notice = self.stage.shown().notice;
        if notice.is_some_and(|n| n.kind.breaks_fullscreen()) {
            return true;
        }
        // An app's notice may ask to show over a fullscreen app (an air raid alert).
        let fullscreen = (self.fullscreen && !self.app_notice_over_fullscreen())
            .then_some(self.settings.fullscreen);
        if fullscreen == Some(Fullscreen::Hide) {
            return false;
        }
        if self.hidden.is_some() {
            return notice.is_some_and(|n| n.kind.is_priority());
        }
        if fullscreen == Some(Fullscreen::Alerts) {
            return notice.is_some();
        }
        self.settings.show_island || notice.is_some()
    }

    pub(super) fn set_hidden(&mut self, hidden: Option<Hidden>) {
        self.hidden = hidden;
        self.save();
        self.sync_visibility();
    }

    pub(super) fn sync_visibility(&mut self) {
        if self.wants_visible() {
            self.kill_timer(timers::HIDE);
            if !self.visible {
                self.visible = true;
                platform::show_without_focus(self.island);
                self.render();
                self.sync_rings();
            }
        } else if self.visible {
            self.set_timer(timers::HIDE, HIDE_DELAY_MS);
        }
    }

    pub(super) fn on_hide_timer(&mut self) {
        self.kill_timer(timers::HIDE);
        if !self.wants_visible() {
            self.visible = false;
            platform::hide(self.island);
            self.sync_bars();
        }
    }

    /// Once a second: a fullscreen app came or left, the time to stay hidden ran out.
    pub(super) fn tick_visibility(&mut self) {
        let fullscreen = platform::is_busy();
        if fullscreen != self.fullscreen {
            self.fullscreen = fullscreen;
            self.sync_visibility();
        }
        if self.hidden.is_some_and(Hidden::expired) {
            self.set_hidden(None);
        }
    }

    pub(super) fn frozen(&self, mode: Mode) -> bool {
        mode.dims() && self.dimmed && !self.settings.live_when_dimmed
    }

    pub(super) fn update_rest(&mut self, mode: Mode) {
        let resting = mode.dims() && !self.pointer.inside;
        if resting == self.resting {
            return;
        }
        self.resting = resting;
        if resting {
            self.set_timer(timers::DIM, DIM_DELAY_MS);
        } else {
            self.kill_timer(timers::DIM);
            self.dimmed = false;
            log(self.scene.set_dimmed(false));
        }
    }

    pub(super) fn on_dim_timer(&mut self) {
        self.kill_timer(timers::DIM);
        self.dimmed = true;
        log(self.scene.set_dimmed(true));
        self.render();
    }
}
