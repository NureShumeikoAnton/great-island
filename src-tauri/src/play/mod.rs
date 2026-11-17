//! Video playback: everything that does not need Windows.
//!
//! [`Player`] is what the viewer window knows about the video: where it is, whether it runs, its
//! speed and sound, a drag of the progress bar. The playing itself is the [`Engine`] port (Media
//! Engine in `adapters/`); what the engine says comes back as [`Event`]s.
//!
//! A press changes the state at once and tells the engine; the engine's events then correct
//! the position. Between events the position is extrapolated from the last one, so the window
//! needs no per-frame work: [`Player::bar`] tells the compositor where the bar is and how long
//! it runs to the end, [`Player::next_second`] when the time text changes.
//!
//! Positions and durations are seconds of the video; `now` is island milliseconds.

pub mod adapters;

use crate::fault::Failed;

/// The speed steps; [`NORMAL`] is 1×.
pub const SPEEDS: [f64; 7] = [0.5, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
pub const NORMAL: usize = 1;
pub const SKIP: f64 = 5.0;
/// The speed while a press on the picture is held, whatever speed is chosen.
pub const BOOST: f64 = 2.0;
/// An event that moves the position less than this keeps the bar's animation going.
const DRIFT: f64 = 0.15;

pub trait Engine {
    fn play(&mut self) -> Result<(), Failed>;
    fn pause(&mut self) -> Result<(), Failed>;
    /// `exact: false` while the bar is dragged: the nearest frame that decodes fast.
    fn seek(&mut self, to: f64, exact: bool) -> Result<(), Failed>;
    fn set_rate(&mut self, rate: f64) -> Result<(), Failed>;
    fn set_volume(&mut self, volume: f64, muted: bool) -> Result<(), Failed>;
    /// One frame forward or back; only while paused.
    fn step(&mut self, forward: bool) -> Result<(), Failed>;
}

/// An engine not open yet, or let go meanwhile: a press reaches nothing.
impl<E: Engine> Engine for Option<E> {
    fn play(&mut self) -> Result<(), Failed> {
        self.as_mut().map_or(Ok(()), E::play)
    }
    fn pause(&mut self) -> Result<(), Failed> {
        self.as_mut().map_or(Ok(()), E::pause)
    }
    fn seek(&mut self, to: f64, exact: bool) -> Result<(), Failed> {
        self.as_mut().map_or(Ok(()), |e| e.seek(to, exact))
    }
    fn set_rate(&mut self, rate: f64) -> Result<(), Failed> {
        self.as_mut().map_or(Ok(()), |e| e.set_rate(rate))
    }
    fn set_volume(&mut self, volume: f64, muted: bool) -> Result<(), Failed> {
        self.as_mut()
            .map_or(Ok(()), |e| e.set_volume(volume, muted))
    }
    fn step(&mut self, forward: bool) -> Result<(), Failed> {
        self.as_mut().map_or(Ok(()), |e| e.step(forward))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub duration: f64,
    pub size: (u32, u32),
    /// Windows could decode the picture.
    pub picture: bool,
    /// Windows could decode the sound.
    pub sound: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Loaded(Info),
    /// The engine's position when it said so.
    Playing(f64),
    Paused(f64),
    Time(f64),
    Seeked(f64),
    Ended,
    Failed(Failed),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Playing,
    Paused,
    /// Stopped on the last frame: play starts over.
    Ended,
    Broken,
}

/// What an event means for the user; the island reports it.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// Nothing to show: the video can't be watched.
    NoPicture,
    NoSound,
    Failed(Failed),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    /// The bar's animation must be set again from [`Player::bar`].
    pub bar: bool,
    pub problem: Option<Problem>,
}

/// Where the progress bar stands and, while playing, how long until it reaches the end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// 0–1.
    pub fraction: f64,
    /// `None` = it stands.
    pub run_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Scrub {
    target: f64,
    /// It was playing when the bar was grabbed.
    resume: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Seek {
    to: f64,
    exact: bool,
}

pub struct Player<E> {
    engine: E,
    phase: Phase,
    info: Option<Info>,
    /// The position at `since` (island ms); while playing it moves on at the speed.
    at: f64,
    since: u64,
    speed: usize,
    volume: f64,
    muted: bool,
    /// Start as soon as the video is loaded.
    start: bool,
    /// The engine was replaced: once loaded it goes back to `at`, and the sound was already told.
    resume: bool,
    scrub: Option<Scrub>,
    /// A seek the engine is busy with, and the one that waits for it.
    seeking: bool,
    queued: Option<Seek>,
    /// Paused by hiding the window, to continue when it shows.
    hidden: bool,
    /// Held at [`BOOST`]; whether the video ran before the hold.
    boost: Option<bool>,
}

impl<E: Engine> Player<E> {
    pub fn new(engine: E, volume: f64, muted: bool) -> Self {
        Self {
            engine,
            phase: Phase::Loading,
            info: None,
            at: 0.0,
            since: 0,
            speed: NORMAL,
            volume: volume.clamp(0.0, 1.0),
            muted,
            start: false,
            resume: false,
            scrub: None,
            seeking: false,
            queued: None,
            hidden: false,
            boost: None,
        }
    }

    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// Another engine of the same video takes over: once it loads, the video stands paused at
    /// `at` with its speed and sound. Returns the engine it replaces.
    pub fn replace(&mut self, engine: E, at: f64, now: u64) -> E {
        self.anchor(at, now);
        self.phase = Phase::Loading;
        self.resume = true;
        self.start = false;
        self.scrub = None;
        self.seeking = false;
        self.queued = None;
        self.hidden = false;
        self.boost = None;
        std::mem::replace(&mut self.engine, engine)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn info(&self) -> Option<Info> {
        self.info
    }

    pub fn duration(&self) -> f64 {
        self.info.map_or(0.0, |i| i.duration)
    }

    pub fn speed(&self) -> usize {
        self.speed
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn scrubbing(&self) -> bool {
        self.scrub.is_some()
    }

    /// ▶/❚❚ shows "pause" while the video runs or is held under a dragged bar.
    pub fn running(&self) -> bool {
        match self.scrub {
            Some(scrub) => scrub.resume,
            None => self.phase == Phase::Playing,
        }
    }

    fn rate(&self) -> f64 {
        if self.boost.is_some() {
            BOOST
        } else {
            SPEEDS[self.speed]
        }
    }

    fn ready(&self) -> bool {
        matches!(self.phase, Phase::Playing | Phase::Paused | Phase::Ended)
    }

    fn can_seek(&self) -> bool {
        self.ready() && self.duration() > 0.0
    }

    pub fn position(&self, now: u64) -> f64 {
        if let Some(scrub) = self.scrub {
            return scrub.target;
        }
        let at = match self.phase {
            Phase::Playing => {
                self.at + now.saturating_sub(self.since) as f64 / 1000.0 * self.rate()
            }
            Phase::Ended => self.duration(),
            _ => self.at,
        };
        at.clamp(0.0, self.duration())
    }

    fn anchor(&mut self, at: f64, now: u64) {
        self.at = at.clamp(0.0, self.duration());
        self.since = now;
    }

    pub fn bar(&self, now: u64) -> Bar {
        let duration = self.duration();
        if duration <= 0.0 {
            return Bar {
                fraction: 0.0,
                run_ms: None,
            };
        }
        let position = self.position(now);
        let run_ms = (self.phase == Phase::Playing && self.scrub.is_none())
            .then(|| ((duration - position) / self.rate() * 1000.0).round() as u64);
        Bar {
            fraction: position / duration,
            run_ms,
        }
    }

    /// Island ms when the shown second changes; `None` while it stands.
    pub fn next_second(&self, now: u64) -> Option<u64> {
        if self.phase != Phase::Playing || self.scrub.is_some() {
            return None;
        }
        let position = self.position(now);
        if position >= self.duration() {
            return None;
        }
        let left = (position.floor() + 1.0 - position) / self.rate();
        Some(now + (left * 1000.0).ceil().max(1.0) as u64)
    }

    /// The time under a point of the bar, 0–1.
    pub fn time_at(&self, fraction: f64) -> f64 {
        fraction.clamp(0.0, 1.0) * self.duration()
    }

    /// Starts once the video is loaded: the window calls it when its opening animation ends.
    pub fn start(&mut self, now: u64) -> Result<(), Failed> {
        match self.phase {
            Phase::Loading => {
                self.start = true;
                Ok(())
            }
            Phase::Paused | Phase::Ended => self.play(now),
            Phase::Playing | Phase::Broken => Ok(()),
        }
    }

    pub fn toggle(&mut self, now: u64) -> Result<(), Failed> {
        self.hidden = false;
        match self.phase {
            Phase::Loading => {
                self.start = !self.start;
                Ok(())
            }
            Phase::Playing => self.pause(now),
            Phase::Paused | Phase::Ended => self.play(now),
            Phase::Broken => Ok(()),
        }
    }

    fn play(&mut self, now: u64) -> Result<(), Failed> {
        if self.phase == Phase::Ended {
            self.anchor(0.0, now);
            self.engine.seek(0.0, true)?;
        }
        self.phase = Phase::Playing;
        self.since = now;
        self.engine.play()
    }

    pub fn pause(&mut self, now: u64) -> Result<(), Failed> {
        self.start = false;
        if self.phase != Phase::Playing {
            return Ok(());
        }
        let at = self.position(now);
        self.anchor(at, now);
        self.phase = Phase::Paused;
        self.engine.pause()
    }

    /// The window was minimized: pause, and continue when it shows again.
    pub fn hide(&mut self, now: u64) -> Result<(), Failed> {
        if self.phase != Phase::Playing {
            return Ok(());
        }
        self.hidden = true;
        self.pause(now)
    }

    pub fn show(&mut self, now: u64) -> Result<(), Failed> {
        if !std::mem::take(&mut self.hidden) || self.phase != Phase::Paused {
            return Ok(());
        }
        self.play(now)
    }

    /// ←/→: `SKIP` seconds back or forward, exact. Off the end it stops on the last frame.
    pub fn skip(&mut self, forward: bool, now: u64) -> Result<(), Failed> {
        if !self.can_seek() || self.scrub.is_some() {
            return Ok(());
        }
        let delta = if forward { SKIP } else { -SKIP };
        let to = (self.position(now) + delta).clamp(0.0, self.duration());
        self.seek_to(to, now)
    }

    fn seek_to(&mut self, to: f64, now: u64) -> Result<(), Failed> {
        if self.phase == Phase::Ended {
            self.phase = Phase::Paused;
        }
        self.anchor(to, now);
        self.seek(Seek { to, exact: true })
    }

    /// One seek at a time: while the engine is busy, only the latest target waits.
    fn seek(&mut self, seek: Seek) -> Result<(), Failed> {
        if self.seeking {
            self.queued = Some(seek);
            return Ok(());
        }
        self.seeking = true;
        self.engine.seek(seek.to, seek.exact)
    }

    /// `,`/`.` while paused.
    pub fn step(&mut self, forward: bool) -> Result<(), Failed> {
        let allowed = match self.phase {
            Phase::Paused => true,
            Phase::Ended => !forward,
            _ => false,
        };
        if !allowed || self.scrub.is_some() {
            return Ok(());
        }
        self.phase = Phase::Paused;
        self.engine.step(forward)
    }

    /// The bar is pressed at `fraction`: the video holds and follows the cursor.
    pub fn grab(&mut self, fraction: f64, now: u64) -> Result<(), Failed> {
        if !self.can_seek() {
            return Ok(());
        }
        let resume = self.phase == Phase::Playing;
        if resume {
            self.pause(now)?;
        }
        if self.phase == Phase::Ended {
            self.phase = Phase::Paused;
        }
        let target = self.time_at(fraction);
        self.scrub = Some(Scrub { target, resume });
        self.seek(Seek {
            to: target,
            exact: false,
        })
    }

    pub fn drag(&mut self, fraction: f64) -> Result<(), Failed> {
        let target = self.time_at(fraction);
        let Some(scrub) = self.scrub.as_mut() else {
            return Ok(());
        };
        if scrub.target == target {
            return Ok(());
        }
        scrub.target = target;
        self.seek(Seek {
            to: target,
            exact: false,
        })
    }

    /// Released: the exact frame under the cursor, and the video goes on if it was running.
    pub fn release(&mut self, fraction: f64, now: u64) -> Result<(), Failed> {
        let Some(scrub) = self.scrub.take() else {
            return Ok(());
        };
        let to = self.time_at(fraction);
        self.anchor(to, now);
        self.seek(Seek { to, exact: true })?;
        if scrub.resume {
            self.play(now)?;
        }
        Ok(())
    }

    pub fn set_speed(&mut self, speed: usize, now: u64) -> Result<(), Failed> {
        let speed = speed.min(SPEEDS.len() - 1);
        if speed == self.speed {
            return Ok(());
        }
        let at = self.position(now);
        self.anchor(at, now);
        self.speed = speed;
        self.engine.set_rate(self.rate())
    }

    /// The hold starts: the video runs at [`BOOST`], also from a pause. Not at the end.
    pub fn boost(&mut self, now: u64) -> Result<(), Failed> {
        let held = matches!(self.phase, Phase::Playing | Phase::Paused);
        if !held || self.scrub.is_some() || self.boost.is_some() {
            return Ok(());
        }
        let resume = self.phase == Phase::Playing;
        // The position is anchored at the old rate before the rate changes.
        let at = self.position(now);
        self.anchor(at, now);
        self.boost = Some(resume);
        self.engine.set_rate(BOOST)?;
        if !resume {
            self.play(now)?;
        }
        Ok(())
    }

    /// The hold ends: back to the chosen speed, and to the pause if there was one.
    pub fn unboost(&mut self, now: u64) -> Result<(), Failed> {
        let Some(resume) = self.boost else {
            return Ok(());
        };
        let at = self.position(now);
        self.anchor(at, now);
        self.boost = None;
        self.engine.set_rate(self.rate())?;
        if !resume {
            self.pause(now)?;
        }
        Ok(())
    }

    pub fn boosted(&self) -> bool {
        self.boost.is_some()
    }

    /// A volume above zero also turns the sound back on.
    pub fn set_volume(&mut self, volume: f64) -> Result<(), Failed> {
        self.volume = volume.clamp(0.0, 1.0);
        if self.volume > 0.0 {
            self.muted = false;
        }
        self.engine.set_volume(self.volume, self.muted)
    }

    pub fn toggle_mute(&mut self) -> Result<(), Failed> {
        self.muted = !self.muted;
        self.engine.set_volume(self.volume, self.muted)
    }

    pub fn on_event(&mut self, event: Event, now: u64) -> Update {
        match event {
            Event::Loaded(info) => self.loaded(info, now),
            Event::Playing(at) => {
                if self.phase == Phase::Playing && self.scrub.is_none() {
                    return self.correct(at, now);
                }
                Update::default()
            }
            Event::Paused(at) => {
                if self.phase == Phase::Paused && !self.seeking && self.scrub.is_none() {
                    return self.correct(at, now);
                }
                Update::default()
            }
            Event::Time(at) => {
                if self.seeking || self.scrub.is_some() || self.phase == Phase::Ended {
                    return Update::default();
                }
                self.correct(at, now)
            }
            Event::Seeked(at) => {
                self.seeking = false;
                if let Some(next) = self.queued.take() {
                    return match self.seek(next) {
                        Ok(()) => Update::default(),
                        Err(failed) => problem(Problem::Failed(failed)),
                    };
                }
                if self.scrub.is_some() {
                    return Update::default();
                }
                self.anchor(at, now);
                Update {
                    bar: true,
                    problem: None,
                }
            }
            Event::Ended => {
                if self.scrub.is_some() || self.seeking {
                    return Update::default();
                }
                self.phase = Phase::Ended;
                self.anchor(self.duration(), now);
                Update {
                    bar: true,
                    problem: None,
                }
            }
            Event::Failed(failed) => {
                self.phase = Phase::Broken;
                self.scrub = None;
                Update {
                    bar: true,
                    problem: Some(Problem::Failed(failed)),
                }
            }
        }
    }

    fn loaded(&mut self, info: Info, now: u64) -> Update {
        if self.phase != Phase::Loading {
            return Update::default();
        }
        let duration = if info.duration.is_finite() {
            info.duration.max(0.0)
        } else {
            0.0
        };
        self.info = Some(Info { duration, ..info });
        if !info.picture {
            self.phase = Phase::Broken;
            return problem(Problem::NoPicture);
        }
        self.phase = Phase::Paused;
        let resume = std::mem::take(&mut self.resume);
        self.anchor(if resume { self.at } else { 0.0 }, now);
        let mut update = Update {
            bar: true,
            problem: None,
        };
        let mut done = self.engine.set_volume(self.volume, self.muted);
        if resume {
            if self.rate() != 1.0 {
                done = done.and(self.engine.set_rate(self.rate()));
            }
            let to = self.at;
            done = done.and(self.seek(Seek { to, exact: true }));
        }
        if std::mem::take(&mut self.start) {
            done = done.and(self.play(now));
        }
        if let Err(failed) = done {
            update.problem = Some(Problem::Failed(failed));
        } else if !info.sound && !resume {
            update.problem = Some(Problem::NoSound);
        }
        update
    }

    /// The engine's word on the position; a small difference doesn't restart the bar.
    fn correct(&mut self, at: f64, now: u64) -> Update {
        let drift = (self.position(now) - at).abs();
        self.anchor(at, now);
        Update {
            bar: drift > DRIFT,
            problem: None,
        }
    }
}

fn problem(problem: Problem) -> Update {
    Update {
        bar: true,
        problem: Some(problem),
    }
}

/// `m:ss`, or `h:mm:ss` when the video is an hour or longer. Whole seconds, rounded down.
pub fn clock(seconds: f64, duration: f64) -> String {
    let s = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    if duration >= 3600.0 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// `m:ss.d`: the editor's time, to a tenth, rounded down.
pub fn clock_tenths(seconds: f64, duration: f64) -> String {
    let tenths = if seconds.is_finite() {
        (seconds.max(0.0) * 10.0 + 1e-6).floor() as u64
    } else {
        0
    };
    format!("{}.{}", clock(tenths as f64 / 10.0, duration), tenths % 10)
}

/// `1×`, `1.25×`: the speed button's label.
pub fn speed_label(speed: usize) -> String {
    rate_label(SPEEDS[speed.min(SPEEDS.len() - 1)])
}

pub fn rate_label(rate: f64) -> String {
    format!("{rate}×")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::Cause;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Play,
        Pause,
        Seek(f64, bool),
        Rate(f64),
        Volume(f64, bool),
        Step(bool),
    }

    #[derive(Clone, Default)]
    struct Fake {
        calls: Rc<RefCell<Vec<Call>>>,
    }

    impl Fake {
        fn take(&self) -> Vec<Call> {
            std::mem::take(&mut self.calls.borrow_mut())
        }

        fn push(&self, call: Call) -> Result<(), Failed> {
            self.calls.borrow_mut().push(call);
            Ok(())
        }
    }

    impl Engine for Fake {
        fn play(&mut self) -> Result<(), Failed> {
            self.push(Call::Play)
        }
        fn pause(&mut self) -> Result<(), Failed> {
            self.push(Call::Pause)
        }
        fn seek(&mut self, to: f64, exact: bool) -> Result<(), Failed> {
            self.push(Call::Seek(to, exact))
        }
        fn set_rate(&mut self, rate: f64) -> Result<(), Failed> {
            self.push(Call::Rate(rate))
        }
        fn set_volume(&mut self, volume: f64, muted: bool) -> Result<(), Failed> {
            self.push(Call::Volume(volume, muted))
        }
        fn step(&mut self, forward: bool) -> Result<(), Failed> {
            self.push(Call::Step(forward))
        }
    }

    fn info(duration: f64) -> Info {
        Info {
            duration,
            size: (1920, 1080),
            picture: true,
            sound: true,
        }
    }

    /// A 60 s video, loaded and paused at 0; the engine's calls so far are cleared.
    fn loaded() -> (Player<Fake>, Fake) {
        let fake = Fake::default();
        let mut player = Player::new(fake.clone(), 0.8, false);
        player.on_event(Event::Loaded(info(60.0)), 0);
        fake.take();
        (player, fake)
    }

    #[test]
    fn a_start_asked_before_loading_waits_for_it() {
        let fake = Fake::default();
        let mut player = Player::new(fake.clone(), 0.5, true);
        player.start(0).unwrap();
        assert!(fake.take().is_empty());
        let update = player.on_event(Event::Loaded(info(60.0)), 100);
        assert_eq!(update.problem, None);
        assert!(update.bar);
        assert_eq!(fake.take(), vec![Call::Volume(0.5, true), Call::Play]);
        assert_eq!(player.phase(), Phase::Playing);
    }

    #[test]
    fn without_a_start_it_waits_paused_on_the_first_frame() {
        let (player, _) = loaded();
        assert_eq!(player.phase(), Phase::Paused);
        assert_eq!(player.position(5000), 0.0);
    }

    #[test]
    fn a_replaced_engine_stands_where_the_old_one_was_once_it_loads() {
        let (mut player, old) = loaded();
        player.set_speed(3, 0).unwrap();
        player.start(0).unwrap();
        old.take();
        let fresh = Fake::default();
        let back = player.replace(fresh.clone(), 12.5, 4000);
        assert!(Rc::ptr_eq(&back.calls, &old.calls));
        assert_eq!(player.phase(), Phase::Loading);
        assert_eq!(player.position(9000), 12.5);
        assert_eq!(player.bar(9000).fraction, 12.5 / 60.0);
        let silent = Info {
            sound: false,
            ..info(60.0)
        };
        let update = player.on_event(Event::Loaded(silent), 9000);
        // The missing sound was said when the video first opened.
        assert_eq!(update.problem, None);
        assert_eq!(
            fresh.take(),
            vec![
                Call::Volume(0.8, false),
                Call::Rate(1.5),
                Call::Seek(12.5, true)
            ]
        );
        assert_eq!(player.phase(), Phase::Paused);
        assert_eq!(player.position(9000), 12.5);
        assert!(old.take().is_empty());
    }

    #[test]
    fn a_play_pressed_while_the_engine_reloads_plays_from_its_place() {
        let (mut player, _) = loaded();
        let fresh = Fake::default();
        player.replace(fresh.clone(), 7.0, 0);
        player.toggle(100).unwrap();
        assert!(fresh.take().is_empty());
        player.on_event(Event::Loaded(info(60.0)), 500);
        assert_eq!(
            fresh.take(),
            vec![Call::Volume(0.8, false), Call::Seek(7.0, true), Call::Play]
        );
        assert_eq!(player.phase(), Phase::Playing);
        assert_eq!(player.position(1500), 8.0);
    }

    #[test]
    fn a_video_let_go_keeps_its_place_and_takes_presses_without_an_engine() {
        let fake = Fake::default();
        let mut player = Player::new(Some(fake.clone()), 0.8, false);
        player.on_event(Event::Loaded(info(60.0)), 0);
        player.start(0).unwrap();
        fake.take();
        let old = player.replace(None, 20.0, 1000);
        assert!(old.is_some());
        player.toggle(2000).unwrap();
        player.toggle(3000).unwrap();
        player.skip(true, 3000).unwrap();
        assert!(fake.take().is_empty());
        assert_eq!(player.phase(), Phase::Loading);
        assert_eq!(player.position(5000), 20.0);
        assert_eq!(player.duration(), 60.0);
    }

    #[test]
    fn the_position_runs_at_the_speed_and_stops_at_the_end() {
        let (mut player, _) = loaded();
        player.start(1000).unwrap();
        assert_eq!(player.position(3000), 2.0);
        player.set_speed(4, 3000).unwrap();
        assert_eq!(player.position(4000), 4.0);
        assert_eq!(player.position(1_000_000), 60.0);
    }

    #[test]
    fn a_hold_runs_at_twice_and_goes_back_to_the_chosen_speed() {
        let (mut player, fake) = loaded();
        player.start(0).unwrap();
        player.set_speed(2, 0).unwrap();
        fake.take();
        player.boost(4000).unwrap();
        assert_eq!(fake.take(), vec![Call::Rate(BOOST)]);
        assert_eq!(player.position(4000), 5.0);
        assert_eq!(player.position(6000), 9.0);
        player.unboost(6000).unwrap();
        assert_eq!(fake.take(), vec![Call::Rate(1.25)]);
        assert_eq!(player.speed(), 2);
        assert_eq!(player.phase(), Phase::Playing);
        assert_eq!(player.position(10_000), 14.0);
    }

    #[test]
    fn a_hold_from_a_pause_plays_and_pauses_again_where_it_got() {
        let (mut player, fake) = loaded();
        player.boost(1000).unwrap();
        assert_eq!(fake.take(), vec![Call::Rate(BOOST), Call::Play]);
        assert!(player.running());
        player.unboost(2000).unwrap();
        assert_eq!(fake.take(), vec![Call::Rate(1.0), Call::Pause]);
        assert_eq!(player.phase(), Phase::Paused);
        assert_eq!(player.position(9000), 2.0);
    }

    #[test]
    fn the_bar_runs_to_the_end_in_the_time_left_at_the_speed() {
        let (mut player, _) = loaded();
        player.start(0).unwrap();
        let bar = player.bar(30_000);
        assert_eq!(bar.fraction, 0.5);
        assert_eq!(bar.run_ms, Some(30_000));
        player.set_speed(4, 30_000).unwrap();
        assert_eq!(player.bar(30_000).run_ms, Some(15_000));
        player.pause(30_000).unwrap();
        assert_eq!(player.bar(40_000).run_ms, None);
        assert_eq!(player.bar(40_000).fraction, 0.5);
    }

    #[test]
    fn small_corrections_keep_the_bar_running_and_big_ones_restart_it() {
        let (mut player, _) = loaded();
        player.start(0).unwrap();
        assert!(!player.on_event(Event::Time(1.05), 1000).bar);
        assert!(player.on_event(Event::Time(1.5), 1100).bar);
        assert_eq!(player.position(1100), 1.5);
    }

    #[test]
    fn the_time_text_changes_on_the_next_whole_second() {
        let (mut player, _) = loaded();
        player.start(0).unwrap();
        assert_eq!(player.next_second(400), Some(1000));
        player.set_speed(4, 400).unwrap();
        // 0.4 s at 1×, then 0.6 s of video at 2× takes 300 ms.
        assert_eq!(player.next_second(400), Some(700));
        player.pause(700).unwrap();
        assert_eq!(player.next_second(700), None);
    }

    #[test]
    fn toggle_pauses_and_plays() {
        let (mut player, fake) = loaded();
        player.toggle(0).unwrap();
        assert!(player.running());
        player.toggle(2000).unwrap();
        assert!(!player.running());
        assert_eq!(player.position(9000), 2.0);
        assert_eq!(fake.take(), vec![Call::Play, Call::Pause]);
    }

    #[test]
    fn at_the_end_it_stands_on_the_last_frame_and_play_starts_over() {
        let (mut player, fake) = loaded();
        player.start(0).unwrap();
        fake.take();
        player.on_event(Event::Ended, 60_000);
        assert_eq!(player.phase(), Phase::Ended);
        assert_eq!(player.bar(70_000).fraction, 1.0);
        assert_eq!(player.bar(70_000).run_ms, None);
        player.toggle(70_000).unwrap();
        assert_eq!(fake.take(), vec![Call::Seek(0.0, true), Call::Play]);
        assert_eq!(player.position(70_000), 0.0);
        assert_eq!(player.phase(), Phase::Playing);
    }

    #[test]
    fn skipping_is_exact_and_stays_inside_the_video() {
        let (mut player, fake) = loaded();
        player.skip(false, 0).unwrap();
        assert_eq!(fake.take(), vec![Call::Seek(0.0, true)]);
        player.on_event(Event::Seeked(0.0), 10);
        player.skip(true, 20).unwrap();
        assert_eq!(fake.take(), vec![Call::Seek(5.0, true)]);
        assert_eq!(player.position(20), 5.0);
    }

    #[test]
    fn a_skip_back_from_the_end_leaves_it_paused() {
        let (mut player, fake) = loaded();
        player.on_event(Event::Ended, 0);
        player.skip(false, 100).unwrap();
        assert_eq!(player.phase(), Phase::Paused);
        assert_eq!(fake.take(), vec![Call::Seek(55.0, true)]);
    }

    #[test]
    fn while_the_engine_seeks_only_the_latest_target_waits() {
        let (mut player, fake) = loaded();
        player.grab(0.1, 0).unwrap();
        player.drag(0.2).unwrap();
        player.drag(0.3).unwrap();
        assert_eq!(fake.take(), vec![Call::Seek(6.0, false)]);
        assert_eq!(player.position(0), 18.0);
        player.on_event(Event::Seeked(6.0), 50);
        assert_eq!(fake.take(), vec![Call::Seek(18.0, false)]);
        player.on_event(Event::Seeked(18.0), 90);
        assert!(fake.take().is_empty());
    }

    #[test]
    fn a_grabbed_bar_holds_the_video_and_the_release_lets_it_go_on() {
        let (mut player, fake) = loaded();
        player.start(0).unwrap();
        fake.take();
        player.grab(0.5, 1000).unwrap();
        assert!(player.running(), "the button keeps showing pause");
        assert_eq!(player.bar(5000).run_ms, None);
        assert_eq!(fake.take(), vec![Call::Pause, Call::Seek(30.0, false)]);
        player.on_event(Event::Seeked(30.0), 1100);
        player.release(0.75, 2000).unwrap();
        assert_eq!(fake.take(), vec![Call::Seek(45.0, true), Call::Play]);
        assert_eq!(player.phase(), Phase::Playing);
        assert_eq!(player.position(3000), 46.0);
    }

    #[test]
    fn a_paused_video_stays_paused_after_a_drag() {
        let (mut player, fake) = loaded();
        player.grab(0.5, 0).unwrap();
        player.on_event(Event::Seeked(30.0), 10);
        player.release(0.5, 20).unwrap();
        assert_eq!(
            fake.take(),
            vec![Call::Seek(30.0, false), Call::Seek(30.0, true)]
        );
        assert_eq!(player.phase(), Phase::Paused);
    }

    #[test]
    fn the_engine_time_does_not_pull_a_dragged_bar() {
        let (mut player, _) = loaded();
        player.grab(0.5, 0).unwrap();
        player.on_event(Event::Time(3.0), 10);
        player.on_event(Event::Ended, 20);
        assert_eq!(player.position(30), 30.0);
        assert!(player.scrubbing());
    }

    #[test]
    fn frames_step_only_while_paused() {
        let (mut player, fake) = loaded();
        player.start(0).unwrap();
        fake.take();
        player.step(true).unwrap();
        assert!(fake.take().is_empty());
        player.pause(100).unwrap();
        fake.take();
        player.step(true).unwrap();
        player.step(false).unwrap();
        assert_eq!(fake.take(), vec![Call::Step(true), Call::Step(false)]);
        player.on_event(Event::Ended, 200);
        player.step(true).unwrap();
        player.step(false).unwrap();
        assert_eq!(fake.take(), vec![Call::Step(false)]);
    }

    #[test]
    fn hiding_the_window_pauses_and_showing_it_goes_on() {
        let (mut player, fake) = loaded();
        player.hide(0).unwrap();
        player.show(10).unwrap();
        assert!(fake.take().is_empty(), "a paused video stays paused");
        player.start(100).unwrap();
        player.hide(1100).unwrap();
        assert_eq!(player.phase(), Phase::Paused);
        player.show(5000).unwrap();
        assert_eq!(player.phase(), Phase::Playing);
        assert_eq!(player.position(5000), 1.0);
        assert_eq!(fake.take(), vec![Call::Play, Call::Pause, Call::Play]);
    }

    #[test]
    fn a_pause_while_hidden_is_not_undone_by_showing() {
        let (mut player, _) = loaded();
        player.start(0).unwrap();
        player.hide(100).unwrap();
        player.toggle(200).unwrap();
        player.toggle(300).unwrap();
        player.show(400).unwrap();
        assert_eq!(player.phase(), Phase::Paused);
    }

    #[test]
    fn volume_above_zero_turns_the_sound_back_on() {
        let (mut player, fake) = loaded();
        player.toggle_mute().unwrap();
        assert!(player.muted());
        player.set_volume(0.0).unwrap();
        assert!(player.muted());
        player.set_volume(0.4).unwrap();
        assert!(!player.muted());
        player.set_volume(7.0).unwrap();
        assert_eq!(
            fake.take(),
            vec![
                Call::Volume(0.8, true),
                Call::Volume(0.0, true),
                Call::Volume(0.4, false),
                Call::Volume(1.0, false),
            ]
        );
    }

    #[test]
    fn a_video_without_a_picture_cant_be_watched() {
        let fake = Fake::default();
        let mut player = Player::new(fake.clone(), 1.0, false);
        player.start(0).unwrap();
        let update = player.on_event(
            Event::Loaded(Info {
                picture: false,
                ..info(10.0)
            }),
            0,
        );
        assert_eq!(update.problem, Some(Problem::NoPicture));
        assert_eq!(player.phase(), Phase::Broken);
        player.toggle(10).unwrap();
        assert!(fake.take().is_empty());
    }

    #[test]
    fn a_video_without_sound_plays_and_says_so() {
        let fake = Fake::default();
        let mut player = Player::new(fake.clone(), 1.0, false);
        let update = player.on_event(
            Event::Loaded(Info {
                sound: false,
                ..info(10.0)
            }),
            0,
        );
        assert_eq!(update.problem, Some(Problem::NoSound));
        assert_eq!(player.phase(), Phase::Paused);
    }

    #[test]
    fn an_engine_failure_breaks_the_player() {
        let (mut player, fake) = loaded();
        let failed = Failed::new(Cause::Graphics);
        let update = player.on_event(Event::Failed(failed.clone()), 0);
        assert_eq!(update.problem, Some(Problem::Failed(failed)));
        player.toggle(0).unwrap();
        player.skip(true, 0).unwrap();
        assert!(fake.take().is_empty());
    }

    #[test]
    fn an_unknown_duration_does_not_seek() {
        let fake = Fake::default();
        let mut player = Player::new(fake.clone(), 1.0, false);
        player.on_event(Event::Loaded(info(f64::INFINITY)), 0);
        fake.take();
        assert_eq!(player.duration(), 0.0);
        player.grab(0.5, 0).unwrap();
        player.skip(true, 0).unwrap();
        assert!(fake.take().is_empty());
        assert_eq!(player.bar(0).fraction, 0.0);
    }

    #[test]
    fn the_hovered_time_follows_the_bar() {
        let (player, _) = loaded();
        assert_eq!(player.time_at(0.25), 15.0);
        assert_eq!(player.time_at(-1.0), 0.0);
        assert_eq!(player.time_at(2.0), 60.0);
    }

    #[test]
    fn times_are_minutes_and_hours_only_for_long_videos() {
        assert_eq!(clock(0.0, 65.0), "0:00");
        assert_eq!(clock(12.9, 65.0), "0:12");
        assert_eq!(clock(65.0, 65.0), "1:05");
        assert_eq!(clock(605.0, 3600.0), "0:10:05");
        assert_eq!(clock(3725.0, 4000.0), "1:02:05");
        assert_eq!(clock(f64::NAN, 10.0), "0:00");
    }

    #[test]
    fn the_editors_time_has_tenths() {
        assert_eq!(clock_tenths(24.36, 65.0), "0:24.3");
        assert_eq!(clock_tenths(0.3, 65.0), "0:00.3");
        assert_eq!(clock_tenths(59.99, 65.0), "0:59.9");
    }

    #[test]
    fn speeds_go_from_half_to_four_and_read_short() {
        assert_eq!(SPEEDS[NORMAL], 1.0);
        assert_eq!(SPEEDS[0], 0.5);
        assert_eq!(SPEEDS[SPEEDS.len() - 1], 4.0);
        assert_eq!(speed_label(NORMAL), "1×");
        assert_eq!(speed_label(2), "1.25×");
        assert_eq!(speed_label(0), "0.5×");
        assert_eq!(speed_label(99), "4×");
    }
}
