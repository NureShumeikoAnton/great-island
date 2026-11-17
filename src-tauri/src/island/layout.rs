//! Where the island's window sits and how much of it takes clicks.
//!
//! The window is a fixed frame at the top center of a monitor, large enough for the biggest
//! view; the capsule moves inside it. Only the area under the capsule takes clicks.

use super::{
    App, log, timers,
    view::{Mode, PANEL_TALLEST, PlayerForm},
};
use crate::{platform, stage::RIBBON};
use windows::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};

pub(super) const FRAME_WIDTH: f32 = 400.0;
const TOP_MARGIN: f32 = 6.0;
/// Margin of the window and click area for the animation's spring overshoot.
const OVERSHOOT: (f32, f32) = (10.0, 6.0);
const SHRINK_DELAY_MS: u32 = 460;

impl App {
    pub(super) fn relayout(&mut self) {
        let mut timers = self.view_input(Mode::Expanded);
        timers.dots = Some((RIBBON.len(), 0));
        let card = PlayerForm::Expanded { timeline: true }.size();
        let menu = self.scene.measure(&self.view_input(Mode::Menu).model());
        let height = self
            .scene
            .measure(&timers.model())
            .1
            .max(card.1)
            .max(menu.1)
            .max(PANEL_TALLEST);
        self.frame = (FRAME_WIDTH, height + OVERSHOOT.1);

        // The second pass is needed if the window moved to a monitor with a different scale.
        for _ in 0..2 {
            self.monitor = platform::place_top_center(
                self.island,
                self.monitor,
                self.frame.0,
                self.frame.1,
                TOP_MARGIN,
            );
        }
        self.scale = platform::monitor_scale(self.monitor);
        log(self.scene.set_scale(self.scale));
        log(self.scene.set_frame_width(self.frame.0));

        self.shape = (0.0, 0.0);
        self.target_shape = (0.0, 0.0);
        self.render();
        self.sync_rings();
    }

    /// The monitor the setting asks for: the cursor's, or the pinned one (the primary one once it is unplugged).
    pub(super) fn wanted_monitor(&self) -> platform::MonitorId {
        if self.settings.island_monitor.is_empty() {
            return platform::cursor_monitor();
        }
        platform::monitor_named(&self.settings.island_monitor)
            .unwrap_or_else(platform::primary_monitor)
    }

    /// Moves the island to the wanted monitor, but only while it sits at rest: a card, a recording or an open window
    /// stay where they started.
    pub(super) fn follow_monitor(&mut self) {
        let wanted = self.wanted_monitor();
        let at_rest = matches!(
            self.activity_mode(),
            Mode::Collapsed | Mode::Player(PlayerForm::Collapsed)
        ) && !self.expanded
            && !self.selecting();
        if wanted != self.monitor && at_rest {
            self.monitor = wanted;
            self.relayout();
        }
    }

    /// Click area: grows at once on expansion, shrinks after the animation.
    pub(super) fn set_shape(&mut self, (w, h): (f32, f32)) {
        let needed = (w + OVERSHOOT.0, h + OVERSHOOT.1);
        if needed == self.target_shape {
            return;
        }
        self.target_shape = needed;
        let grown = (self.shape.0.max(needed.0), self.shape.1.max(needed.1));
        if grown != self.shape {
            self.apply_shape(grown);
        }
        if grown != needed {
            self.set_timer(timers::SHRINK, SHRINK_DELAY_MS);
        }
    }

    fn apply_shape(&mut self, (w, h): (f32, f32)) {
        self.shape = (w, h);
        let mut rect = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.island, &mut rect);
        }
        platform::set_click_area(
            self.island,
            rect,
            (w * self.scale).ceil() as i32,
            (h * self.scale).ceil() as i32,
        );
    }

    pub(super) fn on_shrink_timer(&mut self) {
        self.kill_timer(timers::SHRINK);
        self.apply_shape(self.target_shape);
    }

    /// The middle of the capsule on its monitor, logical pixels: where something leaves the island from.
    pub(super) fn island_point(&self) -> (f32, f32) {
        let monitor = platform::monitor_rect(self.monitor);
        let mut window = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.island, &mut window);
        }
        let k = self.scale;
        (
            (window.left - monitor.left) as f32 / k + self.frame.0 / 2.0,
            (window.top - monitor.top) as f32 / k + self.shape.1 / 2.0,
        )
    }

    /// Window pixels to the scene's logical ones.
    pub(super) fn logical(&self, (x, y): (i32, i32)) -> (f32, f32) {
        (x as f32 / self.scale, y as f32 / self.scale)
    }
}
