//! The pointer on the island: hover, press, wheel and touchpad gestures. This file only decides
//! whom to ask: what a click or a swipe does is the feature's business.
//!
//! Telling a click from a swipe or a hold is `crate::gesture`.
//!
//! A press that ends past the island (a swipe that went too far) doesn't fold it: it stays open
//! until the cursor comes back and leaves again, or for `OUTSIDE_MS`.

use super::{
    App, log,
    render::ViewInput,
    timers,
    view::{HOLD_MS, Hit, Mode, PlayerHit},
};
use crate::control::ControlId;
use crate::gesture::{Dir, Gestures, Output, Pull, Settle, Zone};
use crate::platform;
use crate::record::panel::Press;
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::Input::KeyboardAndMouse::{
        GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    },
};

const HOVER_DELAY_MS: u32 = 150;
/// After a swipe the island doesn't expand on hover at once: not in the middle of a series of swipes.
const REOPEN_AFTER_SWIPE_MS: u32 = 600;
const OUTSIDE_MS: u32 = 5000;

pub(super) struct Pointer {
    pub(super) inside: bool,
    pub(super) pressed: Option<Hit>,
    /// The player's progress bar is being dragged.
    seeking: bool,
    /// The recording's cancel knob is being slid: cursor x where it was grabbed.
    pub(super) sliding: Option<f32>,
    /// The "to island" arrow is being held.
    holding: bool,
    /// The panel's balance slider is being dragged.
    balancing: bool,
    /// A control-center button is held: its ring runs.
    pub(super) held: Option<ControlId>,
    pub(super) gestures: Gestures,
    /// A press ended past the island and the leave waits: `timers::OUTSIDE` runs.
    outside: bool,
}

impl Pointer {
    pub(super) fn new() -> Self {
        Self {
            inside: false,
            pressed: None,
            seeking: false,
            sliding: None,
            holding: false,
            balancing: false,
            held: None,
            gestures: Gestures::new(),
            outside: false,
        }
    }
}

pub(super) fn point(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

pub(super) fn wheel_delta(wparam: WPARAM) -> i32 {
    ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32
}

impl App {
    pub(super) fn on_mouse_move(&mut self, p: (i32, i32)) {
        if !self.pointer.inside {
            self.pointer.inside = true;
            if std::mem::take(&mut self.pointer.outside) {
                self.kill_timer(timers::OUTSIDE);
            }
            self.track_leave();
            self.set_timer(timers::HOVER, HOVER_DELAY_MS);
            self.keep_menu();
            let held = self.stage.hold(true, self.now_ms());
            self.after_stage(held);
            self.render();
            self.on_island_touched();
            self.end_panel_wait();
        }
        let (x, y) = self.logical(p);
        if self.pointer.seeking {
            if let Some(fraction) = self.scene.seek_fraction(x) {
                log(self.scene.preview_seek(fraction));
            }
            return;
        }
        if let Some(start) = self.pointer.sliding {
            log(self.scene.slide_cancel(x - start));
            return;
        }
        if self.pointer.balancing {
            self.slide_balance(x, false);
            return;
        }
        let hit = self
            .scene
            .hit_test(x, y)
            .filter(|h| !matches!(h, Hit::Timer(_)));
        if hit != self.pointer.held.map(Hit::Control) {
            self.release_control();
        }
        log(self.scene.set_hovered(hit));
        if self.pointer.gestures.is_active()
            && let Some(output) = self.pointer.gestures.move_to(self.now_ms(), x, y)
        {
            self.on_gesture(output);
        }
    }

    fn track_leave(&self) {
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.island,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut track);
        }
    }

    /// WM_MOUSELEAVE. While a press holds the mouse, and after it ended past the island, the
    /// leave waits: `end_press` decides.
    pub(super) fn on_mouse_leave(&mut self) {
        let pressing = unsafe { GetCapture() } == self.island;
        if pressing || self.pointer.outside {
            return;
        }
        self.leave();
    }

    fn leave(&mut self) {
        self.pointer.inside = false;
        self.kill_timer(timers::HOVER);
        log(self.scene.set_hovered(None));
        // The knob is still in hand: the track must not fold away under it.
        if self.reminders.skip.is_none() && self.pointer.sliding.is_none() {
            self.expanded = false;
        }
        self.close_menu_soon();
        let released = self.stage.hold(false, self.now_ms());
        self.after_stage(released);
        self.render();
    }

    pub(super) fn on_mouse_down(&mut self, p: (i32, i32)) {
        let (x, y) = self.logical(p);
        let hit = self.scene.hit_test(x, y);
        if hit == Some(Hit::Player(PlayerHit::Seek)) {
            self.pointer.seeking = true;
            unsafe {
                SetCapture(self.island);
            }
            if let Some(fraction) = self.scene.seek_fraction(x) {
                log(self.scene.preview_seek(fraction));
            }
            return;
        }
        if hit == Some(Hit::Panel(Press::Balance)) {
            self.pointer.balancing = true;
            unsafe {
                SetCapture(self.island);
            }
            self.slide_balance(x, false);
            return;
        }
        log(self.scene.set_pressed(hit.clone()));
        // In the editor the "to island" arrow must be held: a stray click can't interrupt an edit.
        if hit == Some(Hit::ToIsland) && self.viewer_editing() {
            self.pointer.holding = true;
            self.set_timer(timers::HOLD, HOLD_MS);
            log(self.scene.hold_arrow(true, false));
        }
        // A recording is cancelled only by sliding the knob to the end: a stray click
        // must not lose it.
        if hit == Some(Hit::RecordCancel) {
            self.pointer.sliding = Some(x);
        }
        if let Some(Hit::Control(id)) = hit {
            self.hold_control(id);
        }
        let zone = match &hit {
            Some(Hit::Timer(_)) => Zone::Hold,
            // The text is wide and sits where a swipe starts: a click on it copies, a move swipes.
            Some(Hit::Player(PlayerHit::Copy)) if self.stage.can_swipe() => Zone::Swipe,
            // The downloads card is all picture and text: a move over them takes the notice away.
            Some(Hit::DownloadOpen | Hit::DownloadFolder) => Zone::Swipe,
            Some(_) => Zone::Button,
            None if self.stage.can_swipe() => Zone::Swipe,
            None => Zone::Button,
        };
        self.pointer.pressed = hit;
        self.pointer
            .gestures
            .press(self.now_ms(), x, y, zone, self.stage.edges());
        unsafe {
            SetCapture(self.island);
        }
        self.schedule_gesture();
    }

    pub(super) fn on_mouse_up(&mut self, p: (i32, i32)) {
        self.release(p);
        self.end_press();
    }

    /// After a press, once the mouse is free: the leave it held back. Past the island the island
    /// stays as it is a while; over it, the leave is watched for again (the held one is spent).
    fn end_press(&mut self) {
        if !self.pointer.inside || self.pointer.outside {
            return;
        }
        if platform::cursor_over(self.island) {
            self.track_leave();
            return;
        }
        self.pointer.inside = false;
        self.pointer.outside = true;
        self.kill_timer(timers::HOVER);
        log(self.scene.set_hovered(None));
        self.set_timer(timers::OUTSIDE, OUTSIDE_MS);
    }

    pub(super) fn on_outside_timer(&mut self) {
        self.kill_timer(timers::OUTSIDE);
        if std::mem::take(&mut self.pointer.outside) && !self.pointer.inside {
            self.leave();
        }
    }

    fn release(&mut self, p: (i32, i32)) {
        let (x, y) = self.logical(p);
        if self.pointer.seeking {
            self.pointer.seeking = false;
            unsafe {
                let _ = ReleaseCapture();
            }
            if let Some(fraction) = self.scene.seek_fraction(x) {
                self.seek_to(fraction);
            }
            return;
        }
        if std::mem::take(&mut self.pointer.balancing) {
            unsafe {
                let _ = ReleaseCapture();
            }
            self.slide_balance(x, true);
            return;
        }
        if let Some(start) = self.pointer.sliding.take() {
            self.pointer.pressed = None;
            if x - start >= self.scene.cancel_travel() {
                self.cancel_recording();
            } else {
                log(self.scene.slide_back());
            }
            if !platform::cursor_over(self.island) {
                self.expanded = false;
                self.render();
            }
        }
        log(self.scene.set_pressed(None));
        self.cancel_hold();
        self.release_control();
        let output = self.pointer.gestures.release();
        let pressed = self.pointer.pressed.take();
        self.kill_timer(timers::GESTURE);
        unsafe {
            let _ = ReleaseCapture();
        }
        match output {
            Some(Output::Click) => match pressed {
                Some(hit) if self.scene.hit_test(x, y).as_ref() == Some(&hit) => self.click(hit),
                Some(_) => {}
                None if !self.expanded => {
                    self.expanded = true;
                    self.render();
                }
                None => {}
            },
            Some(output) => self.on_gesture(output),
            None => {}
        }
    }

    pub(super) fn on_wheel(&mut self, delta: i32, horizontal: bool) {
        if !self.stage.can_swipe() {
            return;
        }
        let (now, edges) = (self.now_ms(), self.stage.edges());
        let output = if horizontal {
            self.pointer.gestures.hscroll(now, delta, edges)
        } else {
            self.pointer.gestures.vscroll(now, delta, edges)
        };
        if let Some(output) = output {
            self.on_gesture(output);
        }
        self.schedule_gesture();
    }

    fn cancel_hold(&mut self) {
        if std::mem::take(&mut self.pointer.holding) {
            self.kill_timer(timers::HOLD);
            log(self.scene.hold_arrow(false, false));
            self.pointer.pressed = None;
        }
    }

    pub(super) fn on_capture_lost(&mut self) {
        self.cancel_hold();
        self.release_control();
        if std::mem::take(&mut self.pointer.balancing) {
            self.save_settings();
        }
        if self.pointer.sliding.take().is_some() {
            log(self.scene.slide_back());
        }
        self.pointer.pressed = None;
        self.pointer.seeking = false;
        log(self.scene.set_pressed(None));
        if let Some(output) = self.pointer.gestures.cancel() {
            self.on_gesture(output);
        }
        self.end_press();
    }

    pub(super) fn on_hover_timer(&mut self) {
        self.kill_timer(timers::HOVER);
        if self.pointer.inside && !self.pointer.gestures.is_active() && !self.expanded {
            self.expanded = true;
            self.render();
        }
    }

    pub(super) fn on_gesture_timer(&mut self) {
        self.kill_timer(timers::GESTURE);
        if let Some(output) = self.pointer.gestures.poll(self.now_ms()) {
            self.on_gesture(output);
        }
        self.schedule_gesture();
    }

    pub(super) fn on_hold_timer(&mut self) {
        self.kill_timer(timers::HOLD);
        self.pointer.holding = false;
        log(self.scene.hold_arrow(false, true));
        self.pointer.pressed = None;
        self.return_front();
    }

    fn on_gesture(&mut self, output: Output) {
        match output {
            Output::Drag(offset) => {
                let dir = if offset < 0.0 { Dir::Next } else { Dir::Prev };
                let neighbor = self.neighbor_input(dir);
                let model = neighbor.as_ref().map(ViewInput::model);
                log(self.scene.drag(offset, model.as_ref().map(|m| (dir, m))));
                if let Some(size) = self.scene.gesture_size() {
                    self.set_shape(size);
                }
            }
            Output::Pull(pull, offset) => {
                let target = match pull {
                    Pull::Open => self.view_input(Mode::Menu),
                    Pull::Close => self.view_input(self.activity_mode()),
                };
                log(self.scene.pull(pull, offset, &target.model()));
                if let Some(size) = self.scene.gesture_size() {
                    self.set_shape(size);
                }
            }
            Output::PullEnd(pull, commit) => {
                log(self.scene.settle_pull(pull, commit));
                self.purge_later();
                let now = self.now_ms();
                let changed = match (pull, commit) {
                    (Pull::Open, true) => self.stage.open_menu(now),
                    (Pull::Close, true) => self.stage.close_menu(now),
                    (_, false) => false,
                };
                self.after_stage(changed);
                self.render();
            }
            Output::Release(settle) => {
                log(self.scene.settle(settle));
                self.purge_later();
                if let Settle::Commit(dir) = settle {
                    self.step_to(dir);
                }
                self.render();
            }
            Output::Step(dir) => {
                if let Some(neighbor) = self.neighbor_input(dir) {
                    log(self.scene.step(dir, &neighbor.model()));
                    self.purge_later();
                }
                self.step_to(dir);
            }
            Output::Bump(dir) => log(self.scene.bump(dir)),
            Output::Dismiss => {
                let changed = self.stage.dismiss(self.now_ms());
                self.after_stage(changed);
            }
            Output::HoldStart => {
                if let Some(Hit::Timer(id)) = self.pointer.pressed.clone() {
                    self.start_skip(id);
                }
            }
            Output::HoldEnd => self.stop_skip(),
            Output::Click => {}
        }
    }

    fn step_to(&mut self, dir: Dir) {
        let changed = self.stage.step(dir, self.now_ms());
        self.set_timer(timers::HOVER, REOPEN_AFTER_SWIPE_MS);
        self.after_stage(changed);
    }

    fn schedule_gesture(&self) {
        match self.pointer.gestures.next_poll() {
            Some(at) => self.set_timer(
                timers::GESTURE,
                at.saturating_sub(self.now_ms()).max(1) as u32,
            ),
            None => self.kill_timer(timers::GESTURE),
        }
    }

    fn click(&mut self, hit: Hit) {
        match hit {
            Hit::Pause => self.toggle_reminders(),
            Hit::Done => self.finish_alert(),
            Hit::Timer(_) => {}
            Hit::Player(hit) => self.player_click(hit),
            Hit::Card => self.open_card(),
            Hit::CardCopy => self.copy_card(false),
            Hit::RecordMic => self.toggle_record_mic(),
            Hit::RecordPause => self.toggle_pause(),
            Hit::RecordStop => self.stop_recording(),
            Hit::RecordNow => self.begin_recording(),
            Hit::RecordCancel => {}
            Hit::Problem => self.dismiss_problem(),
            Hit::Viewer => self.raise_front(),
            Hit::ViewerPlay => self.toggle_watched(),
            Hit::ToIsland => self.return_front(),
            Hit::Control(id) => self.press_control(id),
            Hit::Panel(press) => self.panel_click(press),
            Hit::DownloadOpen => self.open_download(),
            Hit::DownloadFolder => self.show_downloads(),
        }
    }
}
