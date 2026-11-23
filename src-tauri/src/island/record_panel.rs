//! The recording's pre-start panel on the island.
//!
//! 1. A click on the control center's record button turns the control center into the
//!    panel: the button flies into the start circle. (While a recording runs or counts
//!    down, the same button stops it.)
//! 2. Each choice goes straight to the settings file: the record key, which records at
//!    once, uses it too. The panel is a page of the control center: it closes the way
//!    the menu does, when the pointer has left the island.
//! 3. The arrow on the microphone positions opens the list of microphones. The checked
//!    one is listened to while the list is open; its bars jump with its sound.
//! 4. The area circle closes the panel for the area selection over the live screen
//!    (`select.rs`). The release or a cancel brings the panel back: the title says the
//!    area's size and the circle is lit; a click on it returns to the whole screen. The
//!    pointer is away from the island then, so the panel waits for it: it closes the usual
//!    way only after the pointer has been there, and Esc closes it at once. While the panel
//!    holds an area, the recording's corner marks already show it. The area is for one
//!    recording: the next one, or a closed panel, is the whole screen again.
//! 5. The start circle closes the panel and starts the recording (`record.rs`).
//!
//! What a press changes is `crate::record::panel`; the picture is `view/panel.rs`.

use super::{
    App, log,
    select::Purpose,
    timers,
    view::{MAX_MICROPHONES, PanelModel, button_center},
};
use crate::{
    capture::{Area, Source},
    control::ControlId,
    journal,
    lang::tr,
    record::{
        Plan,
        adapters::{Hearing, Meter, microphones},
        choice::Choice,
        meter::{BARS, Bars},
        panel::{Microphone, Page, Panel, Press, heard, selected, slide},
    },
    stage::Sheet,
};

/// The bars are redrawn this often while the microphones page is open; never otherwise.
const METER_MS: u32 = 33;

#[derive(Default)]
pub(super) struct RecordPanel {
    open: Option<Open>,
    /// Back after the selection, the pointer not on the island yet.
    waiting: bool,
}

struct Open {
    panel: Panel,
    devices: Vec<Microphone>,
    meter: Option<Meter>,
    bars: Bars,
    /// The heights on screen: a quiet room redraws nothing.
    drawn: [f32; BARS],
    hearing: Hearing,
}

/// What the scene needs from the panel, copied: see `render::ViewInput`.
pub(super) struct PanelInput {
    page: Page,
    more: bool,
    area: Option<Area>,
    choice: Choice,
    devices: Vec<Microphone>,
    selected: usize,
    weight_mb: u32,
    hearing: bool,
}

impl PanelInput {
    pub(super) fn model(&self) -> PanelModel<'_> {
        PanelModel {
            page: self.page,
            more: self.more,
            area: self.area.map(|a| (a.width, a.height)),
            choice: &self.choice,
            devices: &self.devices,
            selected: self.selected,
            weight_mb: self.weight_mb,
            hearing: self.hearing,
        }
    }
}

impl Open {
    fn new(area: Option<Area>) -> Self {
        Self {
            panel: Panel {
                area,
                ..Panel::default()
            },
            devices: Vec::new(),
            meter: None,
            bars: Bars::default(),
            drawn: [0.0; BARS],
            hearing: Hearing::Opening,
        }
    }
}

impl RecordPanel {
    pub(super) fn page(&self) -> Option<Page> {
        self.open.as_ref().map(|o| o.panel.page)
    }
}

impl App {
    /// The record button was clicked: it flies into the start circle.
    pub(super) fn open_record_panel(&mut self) {
        self.record_panel.open = Some(Open::new(None));
        let from = button_center(&self.controls.items(), ControlId::Record);
        self.render();
        if let Some(from) = from {
            log(self.scene.fly_panel_in(from));
        }
    }

    /// After the area selection: `area` is what the panel records now.
    pub(super) fn reopen_record_panel(&mut self, area: Option<Area>) {
        let changed = self.stage.open_sheet(Sheet::Controls, self.now_ms());
        if self.stage.shown().sheet != Some(Sheet::Controls) {
            return;
        }
        self.record_panel.open = Some(Open::new(area));
        self.after_stage(changed);
        self.render();
        self.record_panel.waiting = !self.pointer.inside;
        self.sync_escape();
        self.sync_marks();
    }

    pub(super) fn panel_area(&self) -> Option<Area> {
        self.record_panel.open.as_ref()?.panel.area
    }

    pub(super) fn record_panel_waits(&self) -> bool {
        self.record_panel.waiting
    }

    /// The pointer came to the island: from now on the panel closes as the menu does.
    pub(super) fn end_panel_wait(&mut self) {
        if std::mem::take(&mut self.record_panel.waiting) {
            self.sync_escape();
        }
    }

    pub(super) fn panel_click(&mut self, press: Press) {
        let Some(open) = &mut self.record_panel.open else {
            return;
        };
        if press == Press::Microphones {
            open.devices = microphones().unwrap_or_else(|failed| {
                journal::warn(tr!("journal.microphones_failed", error = failed.detail));
                Vec::new()
            });
            open.devices.truncate(MAX_MICROPHONES);
        }
        let outcome = open
            .panel
            .press(&press, &mut self.settings.record, &open.devices);
        let area = open.panel.area;
        if outcome.save {
            self.save_settings();
        }
        if outcome.listen {
            self.listen();
        }
        if outcome.select {
            self.close_menu();
            self.select_area(Purpose::Record(area), true);
            return;
        }
        if outcome.start {
            let monitor = self.monitor;
            self.recording.area = area.map(|area| Source { monitor, area });
            self.close_menu();
            self.toggle_recording();
        }
        self.sync_marks();
        self.render();
    }

    /// The balance slider follows the pointer; the file is written once it is let go.
    pub(super) fn slide_balance(&mut self, x: f32, done: bool) {
        if let Some(fraction) = self.scene.balance_at(x)
            && slide(&mut self.settings.record, fraction)
        {
            self.render();
        }
        if done {
            self.save_settings();
        }
    }

    fn listen(&mut self) {
        let Some(open) = &mut self.record_panel.open else {
            return;
        };
        // The old one stops before the next opens the device.
        open.meter = None;
        open.bars = Bars::default();
        open.drawn = [0.0; BARS];
        if open.panel.page != Page::Microphones {
            self.kill_timer(timers::METER);
            return;
        }
        let device = heard(&self.settings.record, &open.devices);
        match Meter::start(device) {
            Ok(meter) => {
                open.meter = Some(meter);
                open.hearing = Hearing::Opening;
                self.set_timer(timers::METER, METER_MS);
            }
            Err(failed) => {
                open.hearing = Hearing::Failed;
                journal::warn(tr!("journal.meter_failed", error = failed.detail));
            }
        }
    }

    pub(super) fn on_meter_timer(&mut self) {
        let now = self.now_ms();
        let Some(open) = self.record_panel.open.as_mut() else {
            self.kill_timer(timers::METER);
            return;
        };
        let Some((peaks, hearing)) = open.meter.as_ref().map(Meter::take) else {
            self.kill_timer(timers::METER);
            return;
        };
        let heights = open.bars.show(peaks, now);
        let moved = std::mem::replace(&mut open.drawn, heights) != heights;
        let changed = hearing != open.hearing;
        open.hearing = hearing;
        if hearing == Hearing::Failed {
            open.meter = None;
            self.kill_timer(timers::METER);
        }
        if changed {
            self.render();
        }
        if moved {
            log(self.scene.set_bars(heights));
        }
    }

    pub(super) fn sync_record_panel(&mut self) {
        if self.record_panel.open.is_some() && self.stage.shown().sheet != Some(Sheet::Controls) {
            self.record_panel.open = None;
            self.kill_timer(timers::METER);
            self.end_panel_wait();
            self.sync_marks();
        }
    }

    pub(super) fn panel_input(&self) -> Option<PanelInput> {
        let open = self.record_panel.open.as_ref()?;
        let whole = self.capture_source().area;
        Some(PanelInput {
            page: open.panel.page,
            more: open.panel.more,
            area: open.panel.area,
            choice: self.settings.record.clone(),
            devices: open.devices.clone(),
            selected: selected(&self.settings.record, &open.devices),
            weight_mb: Plan::new(
                open.panel.area.unwrap_or(whole),
                (whole.width, whole.height),
                &self.settings.record,
            )
            .map_or(0, |plan| plan.megabytes_per_minute()),
            hearing: open.hearing != Hearing::Failed,
        })
    }
}
