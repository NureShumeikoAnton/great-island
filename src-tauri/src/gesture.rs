//! Island gestures: mouse and touchpad swipe, wheel step, hold to rewind, pull down and up to open
//! and close the control center. Pure logic without windows: coordinates and time in, an action out.
//! Thresholds come from the design (design/now-playing.html, "Gesture thresholds").
//! The gesture axis is decided once by the first 6 px and doesn't change until release.

use std::collections::VecDeque;

pub const SWIPE_THRESHOLD: f32 = 6.0;
pub const HOLD_MS: u64 = 350;
/// At this offset the neighbouring activity sits fully in place.
pub const PAGE: f32 = 80.0;
const COMMIT_DISTANCE: f32 = 32.0;
const FLICK_SPEED: f32 = 0.4;
const FLICK_DISTANCE: f32 = 12.0;
const VELOCITY_WINDOW_MS: u64 = 80;
const RUBBER_LIMIT: f32 = 40.0;
const RUBBER_STIFFNESS: f32 = 0.55;
/// The touchpad doesn't report lifted fingers: the gesture ends when no events arrive for this long.
pub const TOUCHPAD_END_MS: u64 = 120;
/// After a touchpad gesture the inertial scroll tail is ignored while the gaps between its events are shorter than this.
const TOUCHPAD_INERTIA_MS: u64 = 300;
/// Fingers are lifted if the scroll decays for this many events in a row and falls below a share of the peak:
/// what follows is Windows inertia, not a finger.
const INERTIA_FALLING: u32 = 4;
const INERTIA_SHARE: f32 = 0.6;
/// Pixels of offset per unit of horizontal scroll delta (a wheel click = 120 units). Like Chrome:
/// 120 units = 100 px, same as the design prototype. To be tuned on a live touchpad.
const TOUCHPAD_PX_PER_UNIT: f32 = 100.0 / 120.0;
const WHEEL_STEP_MS: u64 = 300;
const WHEEL_DELTA: i32 = 120;
pub const PULL_TRAVEL: f32 = 60.0;
const PULL_COMMIT: f32 = 24.0;
const PULL_FLICK_SPEED: f32 = 0.3;
const PULL_FLICK_DISTANCE: f32 = 8.0;
const PULL_RUBBER_LIMIT: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Left in the ribbon: the content moves right.
    Prev,
    /// Right in the ribbon: the content moves left.
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Swipe,
    /// Timer ring or row: hold rewinds, movement swipes.
    Hold,
    /// Button or progress bar: press only, no swipe starts.
    Button,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pull {
    Open,
    Close,
}

impl Pull {
    fn sign(self) -> f32 {
        match self {
            Self::Open => 1.0,
            Self::Close => -1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub prev: bool,
    pub next: bool,
    /// Whether a vertical pull is possible and why. `Close` = the menu is open: paging the ribbon
    /// and wheel steps are not allowed from it.
    pub pull: Option<Pull>,
}

impl Edges {
    fn has(self, dir: Dir) -> bool {
        match dir {
            Dir::Prev => self.prev,
            Dir::Next => self.next,
        }
    }

    fn in_menu(self) -> bool {
        self.pull == Some(Pull::Close)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Settle {
    Commit(Dir),
    Cancel,
    Bounce,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Output {
    /// Offset in logical pixels, already with rubber.
    Drag(f32),
    Release(Settle),
    HoldStart,
    HoldEnd,
    Click,
    Step(Dir),
    Pull(Pull, f32),
    /// true = settle it (open or close the menu), false = bring it back.
    PullEnd(Pull, bool),
    Bump(Dir),
}

#[derive(Debug)]
enum State {
    Idle,
    Pressed {
        zone: Zone,
        x: f32,
        y: f32,
        at: u64,
        /// Went past the threshold, but not horizontally: no longer a click or a hold.
        wandered: bool,
    },
    Holding,
    Dragging(Drag),
}

#[derive(Debug)]
struct Drag {
    /// `None` = ribbon swipe (X axis), otherwise a menu pull (Y axis).
    pull: Option<Pull>,
    /// Where the offset counts from: the point where the threshold was crossed.
    origin: f32,
    /// Offset along the gesture axis: right or down is positive.
    raw: f32,
    samples: VecDeque<(u64, f32)>,
    touchpad: bool,
    last_event: u64,
    /// Touchpad: previous delta, the largest by magnitude, and for how many events in a row it has been decaying.
    last_delta: i32,
    peak: i32,
    falling: u32,
}

impl Drag {
    fn new(pull: Option<Pull>, origin: f32, at: u64, touchpad: bool) -> Self {
        Self {
            pull,
            origin,
            raw: 0.0,
            samples: VecDeque::from([(at, 0.0)]),
            touchpad,
            last_event: at,
            last_delta: 0,
            peak: 0,
            falling: 0,
        }
    }

    /// Touchpad scroll delta. True = looks like inertia: the fingers are already lifted.
    fn scroll(&mut self, delta: i32) -> bool {
        let same_way = delta.signum() == self.last_delta.signum();
        if same_way && delta.abs() <= self.last_delta.abs() {
            self.falling += 1;
        } else {
            self.falling = 0;
        }
        self.last_delta = delta;
        self.peak = self.peak.max(delta.abs());
        self.falling >= INERTIA_FALLING && (delta.abs() as f32) <= self.peak as f32 * INERTIA_SHARE
    }

    fn push(&mut self, at: u64, raw: f32) {
        self.raw = raw;
        self.last_event = at;
        self.samples.push_back((at, raw));
        while self
            .samples
            .front()
            .is_some_and(|&(t, _)| at.saturating_sub(t) > VELOCITY_WINDOW_MS)
        {
            self.samples.pop_front();
        }
    }

    /// Pixels per millisecond.
    fn velocity(&self) -> f32 {
        match (self.samples.front(), self.samples.back()) {
            (Some(&(t0, x0)), Some(&(t1, x1))) if t1 > t0 => (x1 - x0) / (t1 - t0) as f32,
            _ => 0.0,
        }
    }
}

pub struct Gestures {
    state: State,
    edges: Edges,
    touchpad_quiet_until: u64,
    /// Axis (true = menu pull) and direction of the last touchpad gesture, to tell its inertia from a new swipe.
    touchpad_tail: Option<(bool, i32)>,
    wheel_quiet_until: u64,
}

impl Default for Gestures {
    fn default() -> Self {
        Self::new()
    }
}

impl Gestures {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            edges: Edges::default(),
            touchpad_quiet_until: 0,
            touchpad_tail: None,
            wheel_quiet_until: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.state, State::Dragging(_))
    }

    pub fn press(&mut self, now: u64, x: f32, y: f32, zone: Zone, edges: Edges) {
        self.edges = edges;
        self.state = State::Pressed {
            zone,
            x,
            y,
            at: now,
            wandered: false,
        };
    }

    pub fn move_to(&mut self, now: u64, x: f32, y: f32) -> Option<Output> {
        match &mut self.state {
            State::Pressed {
                zone,
                x: x0,
                y: y0,
                wandered,
                ..
            } => {
                let (dx, dy) = (x - *x0, y - *y0);
                if *zone == Zone::Button || dx.abs().max(dy.abs()) < SWIPE_THRESHOLD {
                    return None;
                }
                let horizontal = dx.abs() >= dy.abs();
                // The offset counts from the threshold point, so the content doesn't jump by 6 px.
                if horizontal && !*wandered && !self.edges.in_menu() {
                    let origin = *x0 + SWIPE_THRESHOLD * dx.signum();
                    let mut drag = Drag::new(None, origin, now, false);
                    drag.push(now, x - origin);
                    let output = self.follow(&drag);
                    self.state = State::Dragging(drag);
                    return Some(output);
                }
                if let Some(pull) = self.edges.pull
                    && !horizontal
                    && !*wandered
                    && dy.signum() == pull.sign()
                {
                    let origin = *y0 + SWIPE_THRESHOLD * dy.signum();
                    let mut drag = Drag::new(Some(pull), origin, now, false);
                    drag.push(now, y - origin);
                    let output = self.follow(&drag);
                    self.state = State::Dragging(drag);
                    return Some(output);
                }
                *wandered = true;
                None
            }
            State::Dragging(drag) if !drag.touchpad => {
                let at = if drag.pull.is_some() { y } else { x };
                drag.push(now, at - drag.origin);
                let State::Dragging(drag) = &self.state else {
                    unreachable!()
                };
                Some(self.follow(drag))
            }
            _ => None,
        }
    }

    pub fn release(&mut self) -> Option<Output> {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Pressed { wandered, .. } => (!wandered).then_some(Output::Click),
            State::Holding => Some(Output::HoldEnd),
            State::Dragging(drag) if !drag.touchpad => Some(self.finish(&drag)),
            other => {
                self.state = other;
                None
            }
        }
    }

    pub fn cancel(&mut self) -> Option<Output> {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Holding => Some(Output::HoldEnd),
            State::Dragging(Drag {
                pull: Some(pull), ..
            }) => Some(Output::PullEnd(pull, false)),
            State::Dragging(_) => Some(Output::Release(Settle::Cancel)),
            _ => None,
        }
    }

    pub fn poll(&mut self, now: u64) -> Option<Output> {
        match &self.state {
            State::Pressed {
                zone: Zone::Hold,
                at,
                wandered: false,
                ..
            } if now.saturating_sub(*at) >= HOLD_MS => {
                self.state = State::Holding;
                Some(Output::HoldStart)
            }
            State::Dragging(drag)
                if drag.touchpad && now.saturating_sub(drag.last_event) >= TOUCHPAD_END_MS =>
            {
                Some(self.end_touchpad(now))
            }
            _ => None,
        }
    }

    /// Horizontal two-finger scroll (WM_MOUSEHWHEEL): positive delta = right.
    pub fn hscroll(&mut self, now: u64, delta: i32, edges: Edges) -> Option<Output> {
        if edges.in_menu() && matches!(self.state, State::Idle) {
            return None;
        }
        // Scrolling right moves the content left, like in a browser.
        self.scroll(now, None, -delta, edges)
    }

    /// Vertical scroll (WM_MOUSEWHEEL). Mouse wheel clicks (multiples of 120) step the ribbon; small deltas
    /// are a two-finger touchpad pull of the menu. A positive delta is scroll up: with the default
    /// touchpad setting that is fingers moving down, so the content moves down too.
    pub fn vscroll(&mut self, now: u64, delta: i32, edges: Edges) -> Option<Output> {
        let pulling = match &self.state {
            State::Dragging(drag) if drag.touchpad => drag.pull,
            _ => None,
        };
        if let Some(pull) = pulling {
            return self.scroll(now, Some(pull), delta, edges);
        }
        // The inertia tail after a touchpad gesture can accidentally be a multiple of 120.
        if now < self.touchpad_quiet_until {
            self.touchpad_quiet_until = now + TOUCHPAD_INERTIA_MS;
            return None;
        }
        if delta % WHEEL_DELTA == 0 {
            return self.wheel(now, delta, edges);
        }
        let pull = edges.pull?;
        if !matches!(self.state, State::Idle) || (delta as f32).signum() != pull.sign() {
            return None;
        }
        self.scroll(now, Some(pull), delta, edges)
    }

    /// Two-finger scroll along the axis of the `pull` gesture (`None` = ribbon).
    /// `delta` already has the sign of the content offset.
    fn scroll(&mut self, now: u64, pull: Option<Pull>, delta: i32, edges: Edges) -> Option<Output> {
        match &self.state {
            State::Idle => {
                if now < self.touchpad_quiet_until {
                    // Inertia tail: swallow it until it dies down. Inertia never turns around,
                    // so the opposite direction is a new swipe.
                    let turned = self.touchpad_tail == Some((pull.is_some(), -delta.signum()));
                    if !turned {
                        self.touchpad_quiet_until = now + TOUCHPAD_INERTIA_MS;
                        return None;
                    }
                }
                self.edges = edges;
                self.state = State::Dragging(Drag::new(pull, 0.0, now, true));
            }
            // The axis doesn't change: scrolling along the other axis doesn't continue the gesture.
            State::Dragging(drag) if drag.touchpad && drag.pull.is_some() == pull.is_some() => {}
            _ => return None,
        }
        let State::Dragging(drag) = &mut self.state else {
            unreachable!()
        };
        let raw = drag.raw + delta as f32 * TOUCHPAD_PX_PER_UNIT;
        drag.push(now, raw);
        let inertia = drag.scroll(delta);
        let State::Dragging(drag) = &self.state else {
            unreachable!()
        };
        // The neighbour slid in fully or the menu opened all the way: commit at once, without waiting for the gesture to end.
        let complete = match drag.pull {
            Some(pull) => raw * pull.sign() >= PULL_TRAVEL,
            None => {
                let dir = if raw < 0.0 { Dir::Next } else { Dir::Prev };
                self.edges.has(dir) && raw.abs() >= PAGE
            }
        };
        if inertia || complete {
            return Some(self.end_touchpad(now));
        }
        Some(self.follow(drag))
    }

    /// Touchpad gesture ended: settle, then stay quiet until the inertia dies down.
    fn end_touchpad(&mut self, now: u64) -> Output {
        let output = match &self.state {
            State::Dragging(drag) => {
                self.touchpad_tail = Some((drag.pull.is_some(), drag.last_delta.signum()));
                self.finish(drag)
            }
            _ => Output::Release(Settle::Cancel),
        };
        self.state = State::Idle;
        self.touchpad_quiet_until = now + TOUCHPAD_INERTIA_MS;
        output
    }

    /// Mouse wheel: a click down = next activity, up = previous. Small deltas are touchpad vertical scroll
    /// and are left alone. The wheel doesn't page while the menu is open.
    pub fn wheel(&mut self, now: u64, delta: i32, edges: Edges) -> Option<Output> {
        if delta == 0
            || delta % WHEEL_DELTA != 0
            || now < self.wheel_quiet_until
            || self.is_active()
            || edges.in_menu()
        {
            return None;
        }
        self.wheel_quiet_until = now + WHEEL_STEP_MS;
        let dir = if delta < 0 { Dir::Next } else { Dir::Prev };
        Some(if edges.has(dir) {
            Output::Step(dir)
        } else {
            Output::Bump(dir)
        })
    }

    pub fn next_poll(&self) -> Option<u64> {
        match &self.state {
            State::Pressed {
                zone: Zone::Hold,
                at,
                wandered: false,
                ..
            } => Some(at + HOLD_MS),
            State::Dragging(drag) if drag.touchpad => Some(drag.last_event + TOUCHPAD_END_MS),
            _ => None,
        }
    }

    fn follow(&self, drag: &Drag) -> Output {
        match drag.pull {
            Some(pull) => Output::Pull(pull, shape_pull(drag.raw * pull.sign())),
            None => Output::Drag(self.shape(drag.raw)),
        }
    }

    fn finish(&self, drag: &Drag) -> Output {
        match drag.pull {
            Some(pull) => {
                let progress = drag.raw * pull.sign();
                let speed = drag.velocity() * pull.sign();
                let flick = speed >= PULL_FLICK_SPEED && progress >= PULL_FLICK_DISTANCE;
                Output::PullEnd(pull, progress >= PULL_COMMIT || flick)
            }
            None => Output::Release(self.settle(drag)),
        }
    }

    fn settle(&self, drag: &Drag) -> Settle {
        let raw = drag.raw;
        if raw == 0.0 {
            return Settle::Cancel;
        }
        let dir = if raw < 0.0 { Dir::Next } else { Dir::Prev };
        if !self.edges.has(dir) {
            return Settle::Bounce;
        }
        let v = drag.velocity();
        let flick =
            v.abs() >= FLICK_SPEED && v.signum() == raw.signum() && raw.abs() >= FLICK_DISTANCE;
        if raw.abs() >= COMMIT_DISTANCE || flick {
            Settle::Commit(dir)
        } else {
            Settle::Cancel
        }
    }

    /// Content offset: 1:1 up to a page, then resistance; with no neighbour, all rubber.
    fn shape(&self, raw: f32) -> f32 {
        let dir = if raw < 0.0 { Dir::Next } else { Dir::Prev };
        let a = raw.abs();
        let shaped = if !self.edges.has(dir) {
            rubber(a)
        } else if a <= PAGE {
            a
        } else {
            PAGE + rubber(a - PAGE)
        };
        shaped * raw.signum()
    }
}

pub fn rubber(x: f32) -> f32 {
    rubber_to(RUBBER_LIMIT, x)
}

fn rubber_to(limit: f32, x: f32) -> f32 {
    limit * (1.0 - 1.0 / (1.0 + RUBBER_STIFFNESS * x / limit))
}

/// Menu travel: 1:1 up to full, then rubber; never goes back past the start point.
fn shape_pull(progress: f32) -> f32 {
    if progress <= PULL_TRAVEL {
        progress.max(0.0)
    } else {
        PULL_TRAVEL + rubber_to(PULL_RUBBER_LIMIT, progress - PULL_TRAVEL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Edges = Edges {
        prev: true,
        next: true,
        pull: None,
    };
    const NONE: Edges = Edges {
        prev: false,
        next: false,
        pull: None,
    };
    const CAN_OPEN: Edges = Edges {
        prev: true,
        next: true,
        pull: Some(Pull::Open),
    };
    const MENU: Edges = Edges {
        prev: false,
        next: false,
        pull: Some(Pull::Close),
    };

    #[test]
    fn pulling_down_opens_the_menu() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 10.0, Zone::Swipe, CAN_OPEN);
        assert_eq!(
            g.move_to(10, 101.0, 17.0),
            Some(Output::Pull(Pull::Open, 1.0))
        );
        assert_eq!(
            g.move_to(200, 101.0, 46.0),
            Some(Output::Pull(Pull::Open, 30.0))
        );
        assert_eq!(g.release(), Some(Output::PullEnd(Pull::Open, true)));
    }

    #[test]
    fn short_pull_returns() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 10.0, Zone::Swipe, CAN_OPEN);
        g.move_to(10, 100.0, 17.0);
        g.move_to(1000, 100.0, 30.0);
        assert_eq!(g.release(), Some(Output::PullEnd(Pull::Open, false)));
    }

    #[test]
    fn pull_past_travel_is_rubber() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 0.0, Zone::Swipe, CAN_OPEN);
        g.move_to(10, 100.0, 7.0);
        let Some(Output::Pull(_, shown)) = g.move_to(20, 100.0, 206.0) else {
            panic!("тянут меню");
        };
        assert!(shown > PULL_TRAVEL && shown < PULL_TRAVEL + PULL_RUBBER_LIMIT);
    }

    #[test]
    fn pulling_up_does_nothing_until_the_menu_is_open() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 40.0, Zone::Swipe, CAN_OPEN);
        assert_eq!(g.move_to(10, 100.0, 20.0), None);
        assert_eq!(g.release(), None);

        g.press(0, 100.0, 40.0, Zone::Swipe, MENU);
        assert_eq!(
            g.move_to(10, 100.0, 20.0),
            Some(Output::Pull(Pull::Close, 14.0))
        );
    }

    #[test]
    fn open_menu_does_not_swipe_or_step() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 10.0, Zone::Swipe, MENU);
        assert_eq!(g.move_to(10, 60.0, 10.0), None);
        assert_eq!(g.release(), None);
        assert_eq!(g.vscroll(1000, -120, MENU), None);
        assert_eq!(g.hscroll(2000, 30, MENU), None);
    }

    #[test]
    fn two_fingers_down_open_the_menu_at_full_travel() {
        let mut g = Gestures::new();
        assert_eq!(
            g.vscroll(0, 24, CAN_OPEN),
            Some(Output::Pull(Pull::Open, 20.0))
        );
        assert_eq!(
            g.vscroll(10, 24, CAN_OPEN),
            Some(Output::Pull(Pull::Open, 40.0))
        );
        assert_eq!(
            g.vscroll(20, 24, CAN_OPEN),
            Some(Output::PullEnd(Pull::Open, true))
        );
        assert_eq!(g.vscroll(40, 10, CAN_OPEN), None);
    }

    #[test]
    fn wheel_clicks_still_step_the_ribbon() {
        let mut g = Gestures::new();
        assert_eq!(g.vscroll(0, -120, CAN_OPEN), Some(Output::Step(Dir::Next)));
    }

    fn drag(g: &mut Gestures, t0: u64, x0: f32, x1: f32, ms: u64) -> Option<Output> {
        let steps = 10;
        let mut last = None;
        for i in 1..=steps {
            let t = t0 + ms * i / steps;
            let x = x0 + (x1 - x0) * i as f32 / steps as f32;
            last = g.move_to(t, x, 0.0).or(last);
        }
        last
    }

    #[test]
    fn small_movement_is_still_a_click() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 10.0, Zone::Swipe, BOTH);
        assert_eq!(g.move_to(10, 104.0, 12.0), None);
        assert_eq!(g.release(), Some(Output::Click));
    }

    #[test]
    fn swipe_starts_from_threshold_without_jump() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 10.0, Zone::Swipe, BOTH);
        assert_eq!(g.move_to(10, 93.0, 10.0), Some(Output::Drag(-1.0)));
        assert_eq!(g.move_to(20, 80.0, 10.0), Some(Output::Drag(-14.0)));
    }

    #[test]
    fn long_swipe_commits_short_one_returns() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 0.0, Zone::Swipe, BOTH);
        drag(&mut g, 0, 100.0, 50.0, 800);
        assert_eq!(
            g.release(),
            Some(Output::Release(Settle::Commit(Dir::Next)))
        );

        g.press(1000, 100.0, 0.0, Zone::Swipe, BOTH);
        drag(&mut g, 1000, 100.0, 120.0, 800);
        assert_eq!(g.release(), Some(Output::Release(Settle::Cancel)));
    }

    #[test]
    fn quick_flick_commits_even_if_short() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 0.0, Zone::Swipe, BOTH);
        drag(&mut g, 0, 100.0, 125.0, 40);
        assert_eq!(
            g.release(),
            Some(Output::Release(Settle::Commit(Dir::Prev)))
        );
    }

    #[test]
    fn edge_uses_rubber_and_bounces() {
        let mut g = Gestures::new();
        g.press(0, 100.0, 0.0, Zone::Swipe, NONE);
        let Some(Output::Drag(offset)) = drag(&mut g, 0, 100.0, -300.0, 500) else {
            panic!("ожидалось смещение");
        };
        assert!(offset < 0.0 && offset > -RUBBER_LIMIT);
        assert_eq!(g.release(), Some(Output::Release(Settle::Bounce)));
    }

    #[test]
    fn beyond_page_resists() {
        let mut g = Gestures::new();
        g.press(0, 0.0, 0.0, Zone::Swipe, BOTH);
        let Some(Output::Drag(offset)) = drag(&mut g, 0, 0.0, -206.0, 500) else {
            panic!("ожидалось смещение");
        };
        assert!(offset < -PAGE && offset > -(PAGE + RUBBER_LIMIT));
    }

    #[test]
    fn hold_on_timer_starts_skip_and_locks_swipe() {
        let mut g = Gestures::new();
        g.press(0, 50.0, 10.0, Zone::Hold, BOTH);
        assert_eq!(g.next_poll(), Some(HOLD_MS));
        assert_eq!(g.poll(HOLD_MS - 1), None);
        assert_eq!(g.poll(HOLD_MS), Some(Output::HoldStart));
        assert_eq!(g.move_to(400, 0.0, 10.0), None);
        assert_eq!(g.release(), Some(Output::HoldEnd));
    }

    #[test]
    fn moving_before_hold_turns_into_swipe() {
        let mut g = Gestures::new();
        g.press(0, 50.0, 10.0, Zone::Hold, BOTH);
        assert!(matches!(g.move_to(100, 40.0, 10.0), Some(Output::Drag(_))));
        assert_eq!(g.poll(HOLD_MS), None);
    }

    #[test]
    fn vertical_wander_cancels_click_and_hold() {
        let mut g = Gestures::new();
        g.press(0, 50.0, 10.0, Zone::Hold, BOTH);
        assert_eq!(g.move_to(50, 51.0, 30.0), None);
        assert_eq!(g.poll(HOLD_MS), None);
        assert_eq!(g.release(), None);
    }

    #[test]
    fn buttons_never_swipe() {
        let mut g = Gestures::new();
        g.press(0, 50.0, 10.0, Zone::Button, BOTH);
        assert_eq!(g.move_to(50, 0.0, 10.0), None);
        assert_eq!(g.release(), Some(Output::Click));
    }

    #[test]
    fn touchpad_swipe_ends_after_silence() {
        let mut g = Gestures::new();
        g.hscroll(0, 24, BOTH);
        g.hscroll(10, 24, BOTH);
        assert_eq!(g.poll(100), None);
        assert_eq!(
            g.poll(10 + TOUCHPAD_END_MS),
            Some(Output::Release(Settle::Commit(Dir::Next)))
        );
    }

    #[test]
    fn touchpad_commits_once_neighbor_is_in_place_and_swallows_the_tail() {
        let mut g = Gestures::new();
        for i in 0..3 {
            assert!(matches!(g.hscroll(i * 10, 24, BOTH), Some(Output::Drag(_))));
        }
        // 4 × 20 px = 80 px: the neighbour is already in place, switch at once.
        assert_eq!(
            g.hscroll(30, 24, BOTH),
            Some(Output::Release(Settle::Commit(Dir::Next)))
        );
        for t in (50..2000).step_by(16) {
            assert_eq!(g.hscroll(t, 12, BOTH), None);
        }
        assert!(g.hscroll(2000 + TOUCHPAD_INERTIA_MS, 24, BOTH).is_some());
    }

    #[test]
    fn touchpad_swipe_back_starts_at_once_but_the_same_way_stays_swallowed() {
        let mut g = Gestures::new();
        for i in 0..4 {
            g.hscroll(i * 10, 24, BOTH);
        }
        assert_eq!(g.hscroll(50, 12, BOTH), None);
        assert!(matches!(g.hscroll(66, -24, BOTH), Some(Output::Drag(_))));
    }

    #[test]
    fn touchpad_inertia_ends_the_gesture_without_waiting() {
        let mut g = Gestures::new();
        let deltas = [30, 30, 25, 20];
        for (i, d) in deltas.into_iter().enumerate() {
            assert!(matches!(
                g.hscroll(i as u64 * 10, d, NONE),
                Some(Output::Drag(_))
            ));
        }
        // Fifth decaying event in a row: fingers lifted, bounce at the edge.
        assert_eq!(
            g.hscroll(40, 15, NONE),
            Some(Output::Release(Settle::Bounce))
        );
        assert_eq!(g.hscroll(56, 10, NONE), None);
    }

    #[test]
    fn wheel_steps_once_per_pause_and_ignores_touchpad_deltas() {
        let mut g = Gestures::new();
        assert_eq!(g.wheel(0, -120, BOTH), Some(Output::Step(Dir::Next)));
        assert_eq!(g.wheel(100, -120, BOTH), None);
        assert_eq!(g.wheel(400, 120, NONE), Some(Output::Bump(Dir::Prev)));
        assert_eq!(g.wheel(800, -7, BOTH), None);
    }

    #[test]
    fn rubber_is_bounded() {
        assert_eq!(rubber(0.0), 0.0);
        assert!(rubber(10_000.0) < RUBBER_LIMIT);
        assert!(rubber(20.0) > rubber(10.0));
    }
}
