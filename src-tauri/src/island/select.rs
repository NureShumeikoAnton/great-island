//! Area selection on the island: one selection for the screenshot and the recording.
//!
//! 1. Screenshot: holding the control center's Screenshot button, or the area key, runs the
//!    shutter as usual (`screenshot.rs`). The monitor is captured, but the shot doesn't fly:
//!    it stays on the screen as a frozen picture, and the selection opens over it.
//!    The release cuts the area out, and the shot goes on as an ordinary one, flying from
//!    where the area was.
//! 2. Recording: the pre-start panel's area circle closes the panel and opens the selection
//!    over the live screen. The release brings the panel back with the area.
//! 3. Esc, a right click or leaving for another window cancels: the shot is dropped, the
//!    panel comes back as it was.
//!
//! What the pointer picks is `crate::select`; the window and its picture are `selector.rs`.

use super::{
    App, log,
    selector::{Input, Selector, alt_down},
    timers,
};
use crate::{
    capture::{Area, Image, Stamp},
    fault::{Fault, What},
    journal,
    lang::tr,
    platform,
    select::{Selection, Tuning, adapters::windows_on},
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

const ALT_WATCH_MS: u32 = 30;

pub(super) enum Purpose {
    Shot {
        image: Image,
        stamp: Stamp,
    },
    /// The panel's area before the selection: a cancel keeps it.
    Record(Option<Area>),
}

struct Open {
    selection: Selection,
    window: Selector,
    purpose: Purpose,
}

#[derive(Default)]
pub(super) struct Select {
    open: Option<Open>,
}

enum Next {
    Stay,
    Done(Option<Area>),
}

impl App {
    pub(super) fn selecting(&self) -> bool {
        self.select.open.is_some()
    }

    /// `from_island` — asked for on the island: a wave runs out of it as the selection opens,
    /// unless the settings turn animations off.
    pub(super) fn select_area(&mut self, purpose: Purpose, from_island: bool) {
        let monitor = platform::monitor_rect(self.monitor);
        // Without the list the selection still works, only nothing lights up.
        let windows = windows_on(monitor).unwrap_or_else(|failed| {
            journal::warn(tr!("journal.windows_failed", error = failed.detail));
            Vec::new()
        });
        let frozen = match &purpose {
            Purpose::Shot { image, .. } => Some(image),
            Purpose::Record(_) => None,
        };
        let wave = (from_island && self.settings.animations).then(|| self.island_point());
        match Selector::open(&self.scene.comp, monitor, self.scale, frozen, wave) {
            Ok(window) => {
                let screen = (
                    (monitor.right - monitor.left) as u32,
                    (monitor.bottom - monitor.top) as u32,
                );
                let mut selection = Selection::new(screen, windows, Tuning::at(self.scale));
                if let Some((x, y)) = window.cursor() {
                    selection.point(x, y);
                }
                self.select.open = Some(Open {
                    selection,
                    window,
                    purpose,
                });
                self.sync_escape();
                self.show_selection();
            }
            Err(e) => {
                self.report(Fault::new(What::AreaOpen, e.into()));
                self.finish(purpose, None);
            }
        }
    }

    pub(super) fn on_select_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let open = self
            .select
            .open
            .as_mut()
            .filter(|o| o.window.hwnd() == hwnd)?;
        let (result, input) = open.window.handle(msg, wparam, lparam);
        let selection = &mut open.selection;
        let next = match input {
            None => return result,
            Some(Input::Move { x, y, free }) => {
                selection.set_free(free);
                selection.point(x, y);
                Next::Stay
            }
            Some(Input::Press(x, y)) => {
                selection.press(x, y);
                Next::Stay
            }
            Some(Input::Release(x, y)) => match selection.release(x, y) {
                Some(area) => Next::Done(Some(area)),
                None => Next::Stay,
            },
            Some(Input::Free(free)) => {
                selection.set_free(free);
                Next::Stay
            }
            Some(Input::Cancel) => Next::Done(None),
        };
        match next {
            Next::Stay => self.show_selection(),
            Next::Done(area) => self.end_selection(area),
        }
        result
    }

    /// While Alt is held the key is watched: letting it go brings the window back without a move.
    pub(super) fn on_select_timer(&mut self) {
        self.kill_timer(timers::SELECT);
        if let Some(open) = &mut self.select.open {
            open.selection.set_free(alt_down());
        }
        self.show_selection();
    }

    pub(super) fn cancel_selection(&mut self) {
        self.end_selection(None);
    }

    fn show_selection(&mut self) {
        let Some(open) = &mut self.select.open else {
            return;
        };
        let look = open.selection.look();
        log(open.window.show(&self.scene.comp, &look));
        if look.free {
            self.set_timer(timers::SELECT, ALT_WATCH_MS);
        }
    }

    /// `None` — cancelled.
    fn end_selection(&mut self, area: Option<Area>) {
        self.kill_timer(timers::SELECT);
        let Some(Open {
            window, purpose, ..
        }) = self.select.open.take()
        else {
            return;
        };
        self.sync_escape();
        self.finish(purpose, area);
        // After the shot's flight has started over it: the frozen picture goes only now.
        drop(window);
    }

    fn finish(&mut self, purpose: Purpose, area: Option<Area>) {
        match purpose {
            Purpose::Shot { image, stamp } => match area {
                Some(area) => self.land_area_shot(image, stamp, area),
                None => self.drop_shutter(),
            },
            Purpose::Record(before) => self.reopen_record_panel(area.or(before)),
        }
    }
}
