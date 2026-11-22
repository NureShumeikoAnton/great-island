//! Screenshot on the island: shutter, capture, flight, card, viewer window.
//!
//! 1. The control center's Screenshot button collapses the menu into the shutter. Meanwhile the island hides
//!    from screen capture: visible on the monitor, absent from the shot.
//! 2. Half a second later the monitor with the island is captured. A key and the held button
//!    capture at once: the shot flies while the shutter is still filling. For an area shot (the
//!    button held, the area key) the capture becomes the frozen screen of the area selection
//!    (`select.rs`); the cut-out area flies straight from its place to the island. A whole screen
//!    shrinks to the center with a flash, hangs there and flies to the island; the shutter stays
//!    filled, "holding" the shot. A click on the shot while it is at the center is a shortcut: it
//!    expands in place into the viewer window instead of flying.
//! 3. As the shot approaches, the shutter becomes a capsule sized for the thumbnail; the shot lands in it
//!    and fades out, and the island widens into a card.
//!    With the "copy to the clipboard" setting the shot is in the clipboard as soon as it is taken;
//!    the card looks the same as without it.
//! 4. While a shot exists (flash to card close), Ctrl+C and Ctrl+X are global hotkeys: they work in whatever
//!    window has focus and don't reach it. Ctrl+C or a click on the card text copies to the clipboard;
//!    Ctrl+X puts it only there, with no file. Either shows a tick and the card closes quickly. Other input
//!    goes where it was going: the island never takes focus.
//! 5. When the card closes the keys are released and the file is written in the background.
//! 6. A click on the thumbnail flies the shot into the viewer window (`viewer.rs`) and the file is written
//!    at once. Ctrl+X in the window leaves the shot only in the clipboard: the file is deleted, the window
//!    vanishes and the shot flies to the island, into the "In the clipboard" card.
//! 7. While the window is open the island shows its status: a thumbnail (a click raises the window) and
//!    an arrow that sends the shot back to the island to continue as an ordinary screenshot, file kept.
//!    The status holds until the shot lands.
//!
//! [`Shot`] (`crate::shot`) knows what was decided about the shot; this file is only the island and windows.
//! What it shares with the recording (the source, hiding the island, the card) is `capture.rs`,
//! its keys are in `hotkeys.rs`.

use super::{
    App,
    capture::{FLIGHT_TAIL_MS, Front, PREVIEW_WIDTH, QUICK_CLOSE_MS, THUMB_WIDTH},
    flight::{
        CATCH_UNTIL_MS, EXPAND_FLIGHT_MS, EXPAND_MS, FLIGHT_MS, Flight, GLIDE_FLIGHT_MS,
        GLIDE_LANDING_MS, GLIDE_START_MS, LANDING_MS, Place, SLOT_RADIUS,
    },
    select::Purpose,
    timers,
    view::{CardPhase, SHUTTER_MS},
    viewer::{CORNER_RADIUS, Request, Spot, Viewer},
};
use crate::{
    capture::{
        Area, Image, Stamp, Status,
        adapters::{self as system, SystemClipboard},
    },
    fault::{Cause, Failed, Fault, What},
    journal,
    lang::tr,
    platform,
    shot::{Saved, Shot, Target, adapters::Folder},
    stage::NoticeKind,
};
use std::{path::Path, sync::Arc, thread::JoinHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

/// The shutter starts becoming the thumbnail capsule this long before the shot arrives: that is how long
/// the morph takes, so it is ready on arrival.
const CATCH_LEAD_MS: u32 = 420;
const OPEN_AFTER_MS: u32 = 220;

pub(super) struct Taken {
    shot: Shot,
    pub(super) thumb: Arc<Image>,
    open_at: u64,
    /// How the card meets the shot; `None` = the shot is still flying, no card yet.
    pub(super) phase: Option<CardPhase>,
}

impl Taken {
    pub(super) fn status(&self) -> Status {
        self.shot.status()
    }
}

enum Arrival {
    Appear,
    Glide(Place),
    Expand,
}

pub(super) struct Viewing {
    window: Viewer,
    shot: Shot,
    pub(super) thumb: Arc<Image>,
    /// Island ms: the island's status belongs to the window opened last.
    pub(super) opened_at: u64,
}

#[derive(Default)]
pub(super) struct Shots {
    pub(super) taken: Option<Taken>,
    pub(super) flight: Option<Flight>,
    pub(super) viewing: Option<Viewing>,
    /// The viewer window on its way out, after its shot left for the island.
    vanishing: Option<Viewer>,
    /// Files being written in the background: waited for at quit.
    saving: Vec<JoinHandle<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Trigger {
    /// A click on the button: the menu folds into the shutter, and the whole screen is taken
    /// when it has filled.
    Click,
    /// The button held until its ring closed: captured at once, and the area selection opens
    /// with a wave from the island.
    Hold,
    /// The screenshot key: the whole screen as the user saw it at the press.
    Key,
    /// The area key: captured at the press, then the area selection.
    AreaKey,
}

impl Trigger {
    fn area(self) -> bool {
        matches!(self, Self::Hold | Self::AreaKey)
    }
}

/// What to tell about a save: it failed, or the file went to the default folder because
/// the one from the settings could not take it.
fn save_fault(failed: Option<Failed>, folder: &mut Folder) -> Option<Fault> {
    match failed {
        Some(failed) => Some(Fault::new(What::ShotSave, failed)),
        None => folder
            .take_fallback()
            .map(|failed| Fault::new(What::ShotFolder, failed)),
    }
}

impl App {
    pub(super) fn start_shutter(&mut self, trigger: Trigger) {
        if self.stage.has(NoticeKind::Shutter) {
            return;
        }
        // The previous shot is still waiting for its card (a reminder covered it): save it.
        self.stage.end(NoticeKind::Shot, self.now_ms());
        self.finish_shot();
        // The shutter queues while the menu is open: the menu collapses straight into it.
        self.stage.post(NoticeKind::Shutter, self.now_ms());
        self.sync_capture_hiding();
        self.close_menu();
        if trigger == Trigger::Click {
            self.set_timer(timers::SHUTTER, SHUTTER_MS);
            return;
        }
        system::wait_for_hiding();
        self.take_shot(trigger);
    }

    pub(super) fn on_shutter(&mut self) {
        self.kill_timer(timers::SHUTTER);
        self.take_shot(Trigger::Click);
    }

    fn take_shot(&mut self, trigger: Trigger) {
        let Some((image, stamp)) = self.capture_screen() else {
            return;
        };
        if trigger.area() {
            let from_island = trigger == Trigger::Hold;
            self.select_area(Purpose::Shot { image, stamp }, from_island);
        } else {
            self.land_shot(image, stamp, None);
        }
    }

    fn capture_screen(&mut self) -> Option<(Image, Stamp)> {
        let captured = system::capture(self.capture_source());
        if captured.is_none() {
            self.drop_shutter();
            self.report(Fault::of(What::ShotTake, Cause::Unknown));
        }
        captured
    }

    /// The selection ended without an area, or the capture failed: the island is itself again.
    pub(super) fn drop_shutter(&mut self) {
        let changed = self.stage.end(NoticeKind::Shutter, self.now_ms());
        self.sync_capture_hiding();
        self.after_stage(changed);
    }

    pub(super) fn land_area_shot(&mut self, image: Image, stamp: Stamp, area: Area) {
        self.land_shot(image.crop(area), stamp, Some(area));
    }

    /// The shot is taken: it flies to the island from `from` (`None` — the whole monitor).
    fn land_shot(&mut self, image: Image, stamp: Stamp, from: Option<Area>) {
        let now = self.now_ms();
        let preview = image.preview(PREVIEW_WIDTH);
        let shot = Shot::new(image, stamp);
        if self.settings.shot_clipboard
            && let Err(failed) = shot.lend(&mut SystemClipboard::new(self.island))
        {
            self.report(Fault::new(What::Copy, failed));
        }
        self.shots.taken = Some(Taken {
            thumb: Arc::new(preview.preview(THUMB_WIDTH)),
            shot,
            open_at: now,
            phase: None,
        });
        self.sync_card_keys();
        // The shutter is on screen (a reminder didn't cover it): the shot flies to the island.
        let shutter_shown = self
            .stage
            .shown()
            .notice
            .is_some_and(|n| n.kind == NoticeKind::Shutter);
        let landing = (self.visible && shutter_shown)
            .then(|| self.start_flight(&preview, from))
            .flatten();
        if let Some(landing) = landing {
            if let Some(taken) = &mut self.shots.taken {
                taken.open_at = now + u64::from(landing + OPEN_AFTER_MS);
            }
            self.set_timer(timers::LANDING, landing - CATCH_LEAD_MS);
            return;
        }
        self.show_card(CardPhase::Open);
    }

    /// When the shot lands; `None` — it doesn't fly.
    fn start_flight(&mut self, preview: &Image, from: Option<Area>) -> Option<u32> {
        let slot = self.card_place()?;
        let monitor = platform::monitor_rect(self.monitor);
        let k = self.scale;
        let from = from.map(|area| area.logical(k));
        let comp = &self.scene.comp;
        let started = match from {
            None => Flight::shot(comp, monitor, k, preview, slot).map(|flight| {
                self.set_timer(timers::CATCH, CATCH_UNTIL_MS);
                (flight, LANDING_MS, FLIGHT_MS)
            }),
            Some(area) => Flight::area(comp, monitor, k, preview, area, slot)
                .map(|flight| (flight, GLIDE_LANDING_MS, GLIDE_FLIGHT_MS)),
        };
        match started {
            Ok((flight, landing, total)) => {
                self.shots.flight = Some(flight);
                self.set_timer(timers::FLIGHT, total + FLIGHT_TAIL_MS);
                Some(landing)
            }
            Err(e) => {
                journal::warn(tr!("journal.shot_flight_failed", error = e));
                None
            }
        }
    }

    pub(super) fn on_flight_end(&mut self) {
        self.kill_timer(timers::FLIGHT);
        self.shots.flight = None;
    }

    pub(super) fn on_catch_timer(&mut self) {
        self.kill_timer(timers::CATCH);
        if let Some(flight) = &mut self.shots.flight {
            flight.stop_catching();
        }
    }

    pub(super) fn on_landing_step(&mut self) {
        self.kill_timer(timers::LANDING);
        let next = match self.shots.taken.as_ref().and_then(|t| t.phase) {
            None => (CardPhase::Catching, Some(CATCH_LEAD_MS)),
            Some(CardPhase::Catching) => (CardPhase::Caught, Some(OPEN_AFTER_MS)),
            Some(_) => (CardPhase::Open, None),
        };
        self.show_card(next.0);
        if let Some(delay) = next.1 {
            self.set_timer(timers::LANDING, delay);
        }
    }

    fn show_card(&mut self, phase: CardPhase) {
        let now = self.now_ms();
        let Some(taken) = &mut self.shots.taken else {
            return;
        };
        let first = taken.phase.is_none();
        taken.phase = Some(phase);
        if first {
            let mut changed = self.stage.end(NoticeKind::Shutter, now);
            // A shot from the viewer window landed in the island: the window status is replaced by the card.
            changed |= self.end_viewing();
            changed |= self.stage.post(NoticeKind::Shot, now);
            self.after_stage(changed);
        } else {
            self.render();
        }
    }

    /// Copy to the clipboard (`only` = without a file). The tick is visible for a second after the card has
    /// fully opened, then the card closes.
    pub(super) fn copy_shot(&mut self, only: bool) {
        let mut clipboard = SystemClipboard::new(self.island);
        let mut folder = Folder::new(&self.settings.screenshot_dir);
        let Some(taken) = &mut self.shots.taken else {
            return;
        };
        let done = if only {
            taken.shot.cut(&mut clipboard, &mut folder)
        } else {
            taken.shot.copy(&mut clipboard)
        };
        match done {
            Ok(()) => {
                self.close_card_soon();
                self.render();
            }
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    fn close_card_soon(&self) {
        let Some(taken) = &self.shots.taken else {
            return;
        };
        let delay = taken.open_at.saturating_sub(self.now_ms()) + u64::from(QUICK_CLOSE_MS);
        self.set_timer(timers::SHOT_CLOSE, delay as u32);
    }

    pub(super) fn on_shot_close(&mut self) {
        self.kill_timer(timers::SHOT_CLOSE);
        let changed = self.stage.end(NoticeKind::Shot, self.now_ms());
        self.after_stage(changed);
    }

    pub(super) fn sync_shot(&mut self) {
        if self.shots.taken.as_ref().is_some_and(|t| t.phase.is_some())
            && !self.stage.has(NoticeKind::Shot)
        {
            self.finish_shot();
        }
    }

    fn release_card(&mut self) -> Option<Taken> {
        let taken = self.shots.taken.take()?;
        self.kill_timer(timers::SHOT_CLOSE);
        self.kill_timer(timers::LANDING);
        // The island is visible in screen captures again. Not right after the shot but here:
        // the compositor's window rebuild must not land on the start of the flight.
        self.sync_capture_hiding();
        self.sync_card_keys();
        Some(taken)
    }

    fn finish_shot(&mut self) {
        let Some(taken) = self.release_card() else {
            return;
        };
        self.save_later(taken.shot);
    }

    /// A picture from elsewhere (a video's frame) becomes a screenshot: into the clipboard at
    /// once, the file in the background. True = it is in the clipboard.
    pub(super) fn shot_from(&mut self, image: Image) -> bool {
        let mut clipboard = SystemClipboard::new(self.island);
        let mut shot = Shot::new(image, system::now());
        let copied = shot.copy(&mut clipboard);
        self.save_later(shot);
        match copied {
            Ok(()) => true,
            Err(failed) => {
                self.report(Fault::new(What::Copy, failed));
                false
            }
        }
    }

    fn save_later(&mut self, shot: Shot) {
        let mut folder = Folder::new(&self.settings.screenshot_dir);
        let reporter = self.reporter();
        self.shots.saving.retain(|h| !h.is_finished());
        self.shots.saving.push(std::thread::spawn(move || {
            let saved = shot.finish(&mut folder);
            if let Some(fault) = save_fault(saved.err(), &mut folder) {
                reporter.send(fault);
            }
        }));
    }

    pub(super) fn flush_shots(&mut self) {
        self.finish_shot();
        for handle in self.shots.saving.drain(..) {
            let _ = handle.join();
        }
        // Nobody will see a notice now; the journal still gets what went wrong.
        self.on_faults();
    }

    pub(super) fn open_shot(&mut self) {
        let from = self.card_place();
        let Some(taken) = self.release_card() else {
            return;
        };
        let changed = self.stage.end(NoticeKind::Shot, self.now_ms());
        self.after_stage(changed);
        let arrival = from.map_or(Arrival::Appear, Arrival::Glide);
        self.open_viewer(taken.shot, taken.thumb, arrival);
    }

    /// A click on the shot while it is at the center: it doesn't fly to the island and there is no card:
    /// the shot expands in place into the viewer window.
    pub(super) fn on_catch(&mut self) {
        let in_flight = self.shots.taken.as_ref().is_some_and(|t| t.phase.is_none());
        if !in_flight || self.shots.flight.is_none() {
            return;
        }
        self.kill_timer(timers::CATCH);
        // The shutter goes first: while it is there the island stays hidden from captures.
        let changed = self.stage.end(NoticeKind::Shutter, self.now_ms());
        let Some(taken) = self.release_card() else {
            return;
        };
        self.after_stage(changed);
        self.open_viewer(taken.shot, taken.thumb, Arrival::Expand);
    }

    /// The shot at the center expands into window `to`. Returns when it lands; on failure the flight is closed
    /// so the shot doesn't fly off to the island.
    fn expand_flight(&mut self, to: Place) -> Option<u32> {
        let flight = self.shots.flight.as_mut()?;
        match flight.expand(&self.scene.comp, to, CORNER_RADIUS) {
            Ok(()) => {
                self.set_timer(timers::FLIGHT, EXPAND_FLIGHT_MS + FLIGHT_TAIL_MS);
                Some(EXPAND_MS)
            }
            Err(e) => {
                journal::warn(tr!("journal.shot_window_failed", error = e));
                self.on_flight_end();
                None
            }
        }
    }

    /// The shot flies into the viewer window (`arrival`) and the window shows under it;
    /// with nowhere to fly from, the window just appears.
    fn open_viewer(&mut self, shot: Shot, thumb: Arc<Image>, arrival: Arrival) {
        // The previous window closes silently: the island status stays, now belonging to the new one.
        self.kill_timer(timers::VIEWER_SHOW);
        self.shots.viewing = None;
        let work = platform::work_rect(self.monitor);
        let has_file = shot.status() != Status::ClipboardOnly;
        let bar = self.settings.viewer_bar;
        let window = match Viewer::new(
            &self.scene.comp,
            Spot::middle(work),
            self.scale,
            shot.image(),
            has_file,
            bar,
            &tr!("window.screenshot"),
        ) {
            Ok(window) => window,
            Err(e) => {
                let mut folder = Folder::new(&self.settings.screenshot_dir);
                let saved = shot.finish(&mut folder);
                let changed = self.end_viewing();
                self.after_stage(changed);
                // A lost file matters more than a window that did not open.
                let fault = match saved {
                    Err(failed) => Fault::new(What::ShotSave, failed),
                    Ok(_) => Fault::new(What::ShotOpen, e.into()),
                };
                self.report(fault);
                return;
            }
        };
        let monitor = platform::monitor_rect(self.monitor);
        let to = window.photo_place(monitor);
        let landing = match arrival {
            Arrival::Appear => None,
            Arrival::Glide(from) => {
                let preview = shot.image().preview(PREVIEW_WIDTH);
                self.start_glide(&preview, (from, to), (SLOT_RADIUS, CORNER_RADIUS))
                    .then_some(GLIDE_LANDING_MS)
            }
            Arrival::Expand => self.expand_flight(to),
        };
        self.shots.viewing = Some(Viewing {
            window,
            shot,
            thumb,
            opened_at: self.now_ms(),
        });
        // While the window is open the island shows its status: the island flows into it at once,
        // while the shot is still flying.
        let changed = self.stage.post(NoticeKind::Viewing, self.now_ms());
        self.after_stage(changed);
        match landing {
            Some(ms) => self.set_timer(timers::VIEWER_SHOW, ms),
            None => self.on_viewer_show(),
        }
    }

    /// The shot landed: the window takes its place under it, and only now is the file written:
    /// it takes a fraction of a second and doesn't slow the flight.
    pub(super) fn on_viewer_show(&mut self) {
        self.kill_timer(timers::VIEWER_SHOW);
        let mut folder = Folder::new(&self.settings.screenshot_dir);
        let Some(viewing) = &mut self.shots.viewing else {
            return;
        };
        viewing.window.show(self.shots.flight.is_none());
        let saved = viewing.shot.save(&mut folder);
        let name = saved
            .as_ref()
            .ok()
            .and_then(|path| path.and_then(Path::file_name))
            .map(|name| name.to_string_lossy().into_owned());
        let fault = save_fault(saved.err(), &mut folder);
        viewing.window.set_file(&self.scene.comp, name);
        if let Some(fault) = fault {
            self.report(fault);
        }
    }

    pub(super) fn close_viewer(&mut self) {
        self.kill_timer(timers::VIEWER_SHOW);
        if self.shots.viewing.take().is_some() {
            let changed = self.end_viewing();
            self.after_stage(changed);
        }
    }

    pub(super) fn raise_viewer(&self) {
        if let Some(viewing) = &self.shots.viewing {
            viewing.window.raise();
        }
    }

    pub(super) fn sync_viewer_settings(&mut self) {
        if let Some(viewing) = &mut self.shots.viewing {
            viewing
                .window
                .set_bar(&self.scene.comp, self.settings.viewer_bar);
        }
    }

    pub(super) fn on_viewer_hide(&mut self) {
        self.kill_timer(timers::VIEWER_HIDE);
        self.shots.vanishing = None;
    }

    pub(super) fn on_viewer_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let viewing = self
            .shots
            .viewing
            .as_mut()
            .filter(|v| v.window.hwnd() == hwnd)?;
        let (result, request) =
            viewing
                .window
                .handle(&self.scene.comp, viewing.shot.image(), msg, wparam, lparam);
        match request {
            Some(Request::Close) => self.close_viewer(),
            Some(Request::Save(target)) => {
                self.save_viewed(target);
            }
            Some(Request::Copy) => self.copy_viewed(),
            Some(Request::Cut) => self.cut_viewed(),
            Some(Request::Delete) => self.delete_viewed(),
            Some(Request::Folder) => {
                let shown = self
                    .shots
                    .viewing
                    .as_ref()
                    .and_then(|v| v.shot.path())
                    .map(system::show_in_folder);
                if let Some(Err(failed)) = shown {
                    self.report(Fault::new(What::FolderOpen, failed));
                }
            }
            Some(Request::CopyPath) => self.copy_viewed_path(),
            None => {}
        }
        result
    }

    fn copy_viewed_path(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let Some(viewing) = &mut self.shots.viewing else {
            return;
        };
        let Some(path) = viewing.shot.path() else {
            return;
        };
        match clipboard.put_text(&path.to_string_lossy()) {
            Ok(()) => viewing.window.show_path_copied(&self.scene.comp),
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    /// In the editor: the shot with the edits, the file isn't touched.
    fn copy_viewed(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let Some(viewing) = &mut self.shots.viewing else {
            return;
        };
        let copied = if viewing.window.is_editing() {
            match viewing.window.flatten(viewing.shot.image()) {
                Some(edited) => clipboard.put_image(&edited),
                None => Err(Failed::new(Cause::Unknown)),
            }
        } else {
            viewing
                .shot
                .copy(&mut clipboard)
                .map(|()| viewing.window.show_copied(&self.scene.comp))
        };
        if let Err(failed) = copied {
            self.report(Fault::new(What::Copy, failed));
        }
    }

    /// The status's arrow belongs to a screenshot in the editor: it must be held.
    pub(super) fn viewer_editing(&self) -> bool {
        self.front() == Some(Front::Shot)
            && self
                .shots
                .viewing
                .as_ref()
                .is_some_and(|v| v.window.is_editing())
    }

    /// Saves the editor's edits: over the file or as a copy. The window shows the new shot, the island status
    /// its thumbnail. On failure the editor stays and "Save" pulses.
    fn save_viewed(&mut self, target: Target) -> bool {
        let mut folder = Folder::new(&self.settings.screenshot_dir);
        let comp = &self.scene.comp;
        let Some(viewing) = &mut self.shots.viewing else {
            return false;
        };
        let saved = match viewing.window.flatten(viewing.shot.image()) {
            Some(edited) => viewing.shot.save_edited(edited, &mut folder, target),
            None => Saved::Failed(Failed::new(Cause::Unknown)),
        };
        match saved {
            Saved::Written => {}
            Saved::Unchanged => {
                viewing.window.close_editor(comp);
                return true;
            }
            Saved::Failed(failed) => {
                viewing.window.save_failed(comp);
                self.report(Fault::new(What::ShotEdit, failed));
                return false;
            }
        }
        viewing.thumb = Arc::new(viewing.shot.image().preview(THUMB_WIDTH));
        let name = viewing
            .shot
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        viewing.window.finish_edit(comp, viewing.shot.image(), name);
        self.render();
        if let Some(fault) = save_fault(None, &mut folder) {
            self.report(fault);
        }
        true
    }

    fn delete_viewed(&mut self) {
        let Some(Viewing {
            window,
            shot,
            thumb,
            opened_at,
        }) = self.shots.viewing.take()
        else {
            return;
        };
        match shot.delete(&mut Folder::new(&self.settings.screenshot_dir)) {
            Ok(()) => {
                drop(window);
                let changed = self.end_viewing();
                self.after_stage(changed);
            }
            Err((shot, failed)) => {
                self.shots.viewing = Some(Viewing {
                    window,
                    shot,
                    thumb,
                    opened_at,
                });
                self.report(Fault::new(What::ShotDelete, failed));
            }
        }
    }

    fn cut_viewed(&mut self) {
        let mut clipboard = SystemClipboard::new(self.island);
        let mut folder = Folder::new(&self.settings.screenshot_dir);
        let Some(viewing) = &mut self.shots.viewing else {
            return;
        };
        match viewing.shot.cut(&mut clipboard, &mut folder) {
            Ok(()) => {
                self.fly_home();
                self.close_card_soon();
            }
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    pub(super) fn return_viewed(&mut self) {
        let Some(viewing) = &self.shots.viewing else {
            return;
        };
        if viewing.window.has_edits() && !self.save_viewed(Target::Same) {
            if let Some(viewing) = &self.shots.viewing {
                viewing.window.raise();
            }
            return;
        }
        self.fly_home();
        self.sync_card_keys();
    }

    fn fly_home(&mut self) {
        let Some(Viewing {
            window,
            shot,
            thumb,
            ..
        }) = self.shots.viewing.take()
        else {
            return;
        };
        self.kill_timer(timers::VIEWER_SHOW);
        let now = self.now_ms();
        // The card of another shot (a new one was taken while this was open) is saved, and an open menu
        // closes: the card needs the island.
        let changed = self.stage.end(NoticeKind::Shot, now);
        self.finish_shot();
        self.after_stage(changed);
        self.close_menu();

        let monitor = platform::monitor_rect(self.monitor);
        let from = window.photo_place(monitor);
        let preview = shot.image().preview(PREVIEW_WIDTH);
        self.shots.taken = Some(Taken {
            shot,
            thumb,
            open_at: now,
            phase: None,
        });
        let to = self.card_place();
        let flown = self.island_takes_cards()
            && to.is_some_and(|to| {
                self.start_glide(&preview, (from, to), (CORNER_RADIUS, SLOT_RADIUS))
            });
        if flown {
            window.vanish();
            self.shots.vanishing = Some(window);
            self.set_timer(timers::VIEWER_HIDE, GLIDE_START_MS);
            if let Some(taken) = &mut self.shots.taken {
                taken.open_at = now + u64::from(GLIDE_LANDING_MS + OPEN_AFTER_MS);
            }
            self.set_timer(timers::LANDING, GLIDE_LANDING_MS - CATCH_LEAD_MS);
        } else {
            drop(window);
            self.show_card(CardPhase::Open);
        }
    }
}
