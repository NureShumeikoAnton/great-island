//! Watching an edit before it is saved: the montage plays on two engines ("decks") in turns.
//!
//! Fragment `i` plays on deck `i % 2`. While one deck plays, the other already waits on the first
//! frame of the next fragment, at its speed and with its sound, so a join is a switch, not a seek.
//! A transition plays both decks at once, the incoming picture over the outgoing one.
//!
//! The clock is ours, as in `play`: a press anchors the result position at an island moment, and
//! the window asks [`Program::next_change`] when the next join comes. Between joins nothing runs:
//! [`Program::looks`] gives each deck's zoom and transition as keys the compositor animates.
//!
//! Positions are result seconds; `now` is island milliseconds.

use super::{
    Montage,
    curve::{self, Frame, Layer},
};
use crate::fault::Failed;
use crate::play::{Engine, Event};

/// A join this close ahead counts as reached: timers fire a little early or late.
const SLACK: f64 = 0.002;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Paused,
    Playing,
    Ended,
}

/// What a deck shows from now until the next change. Keys are (seconds from now, value); one key
/// is a value that holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub fragment: usize,
    /// The incoming picture of a transition lies over the outgoing one.
    pub top: bool,
    pub frame: Vec<(f64, Frame)>,
    pub layer: Vec<(f64, Layer)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mark {
    /// The fragment starts: its deck, waiting on its first frame, plays.
    Enter(usize),
    /// The fragment is over: its deck stops and gets the fragment after next.
    Leave(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Seek {
    to: f64,
    exact: bool,
}

pub struct Program<E> {
    decks: [E; 2],
    loaded: [bool; 2],
    /// The fragment each deck is set to.
    holds: [Option<usize>; 2],
    seeking: [bool; 2],
    queued: [Option<Seek>; 2],
    montage: Montage,
    marks: Vec<(f64, Mark)>,
    /// The next mark to reach while playing.
    next: usize,
    phase: Phase,
    at: f64,
    since: u64,
    volume: f64,
    muted: bool,
}

fn deck(fragment: usize) -> usize {
    fragment % 2
}

impl<E: Engine> Program<E> {
    /// The decks load; the program then stands at `at`.
    pub fn new(decks: [E; 2], montage: Montage, at: f64, volume: f64, muted: bool) -> Self {
        let marks = marks_of(&montage);
        let at = at.clamp(0.0, montage.duration());
        Self {
            decks,
            loaded: [false; 2],
            holds: [None; 2],
            seeking: [false; 2],
            queued: [None; 2],
            montage,
            marks,
            next: 0,
            phase: Phase::Loading,
            at,
            since: 0,
            volume: volume.clamp(0.0, 1.0),
            muted,
        }
    }

    pub fn decks(&self) -> &[E; 2] {
        &self.decks
    }

    /// The engines, opened after the program was made: it is still loading.
    pub fn decks_mut(&mut self) -> &mut [E; 2] {
        &mut self.decks
    }

    pub fn montage(&self) -> &Montage {
        &self.montage
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn running(&self) -> bool {
        self.phase == Phase::Playing
    }

    pub fn duration(&self) -> f64 {
        self.montage.duration()
    }

    pub fn position(&self, now: u64) -> f64 {
        let at = match self.phase {
            Phase::Playing => self.at + now.saturating_sub(self.since) as f64 / 1000.0,
            _ => self.at,
        };
        at.clamp(0.0, self.duration())
    }

    /// The fragment under the position: the one the tools act on.
    pub fn current(&self, now: u64) -> usize {
        self.montage.at(self.position(now)).0
    }

    /// Island ms of the next join while playing.
    pub fn next_change(&self, now: u64) -> Option<u64> {
        if self.phase != Phase::Playing {
            return None;
        }
        let (time, _) = self.marks.get(self.next)?;
        let left = (time - self.position(now)).max(0.0);
        Some(now + (left * 1000.0).ceil() as u64)
    }

    /// A deck said something; `Err` = a failure to report.
    pub fn on_event(&mut self, index: usize, event: Event) -> Result<(), Failed> {
        match event {
            Event::Loaded(_) => {
                self.loaded[index] = true;
                if self.phase == Phase::Loading && self.loaded == [true; 2] {
                    self.phase = Phase::Paused;
                    self.arrange()?;
                }
                Ok(())
            }
            Event::Seeked(_) => {
                self.seeking[index] = false;
                match self.queued[index].take() {
                    Some(seek) => self.seek_deck(index, seek),
                    None => Ok(()),
                }
            }
            Event::Failed(failed) => Err(failed),
            _ => Ok(()),
        }
    }

    pub fn toggle(&mut self, now: u64) -> Result<(), Failed> {
        match self.phase {
            Phase::Playing => self.pause(now),
            Phase::Paused | Phase::Ended => self.play(now),
            Phase::Loading => Ok(()),
        }
    }

    pub fn play(&mut self, now: u64) -> Result<(), Failed> {
        match self.phase {
            Phase::Paused => {}
            Phase::Ended => {
                self.at = 0.0;
                self.phase = Phase::Paused;
                self.arrange()?;
            }
            _ => return Ok(()),
        }
        self.since = now;
        self.phase = Phase::Playing;
        self.next = self.marks.partition_point(|(t, _)| *t <= self.at + SLACK);
        let shot = self.montage.shot_at(self.at);
        self.decks[deck(shot.main.index)].play()?;
        if let Some(incoming) = shot.incoming {
            self.decks[deck(incoming.piece.index)].play()?;
        }
        Ok(())
    }

    pub fn pause(&mut self, now: u64) -> Result<(), Failed> {
        if self.phase != Phase::Playing {
            return Ok(());
        }
        self.at = self.position(now);
        self.since = now;
        self.phase = Phase::Paused;
        for d in &mut self.decks {
            d.pause()?;
        }
        Ok(())
    }

    /// `exact: false` while the caret is dragged: the nearest frame that decodes fast.
    pub fn seek(&mut self, to: f64, exact: bool, now: u64) -> Result<(), Failed> {
        if self.phase == Phase::Loading {
            return Ok(());
        }
        let playing = self.phase == Phase::Playing;
        if playing {
            self.pause(now)?;
        }
        self.at = to.clamp(0.0, self.duration());
        self.since = now;
        if self.phase == Phase::Ended {
            self.phase = Phase::Paused;
        }
        self.arrange_with(exact)?;
        if playing { self.play(now) } else { Ok(()) }
    }

    /// An edit: the position stays at `at` (clamped), the decks are set again.
    pub fn set_montage(&mut self, montage: Montage, at: f64, now: u64) -> Result<(), Failed> {
        let playing = self.phase == Phase::Playing;
        if playing {
            self.pause(now)?;
        }
        self.marks = marks_of(&montage);
        self.montage = montage;
        self.holds = [None; 2];
        if self.phase == Phase::Loading {
            return Ok(());
        }
        self.at = at.clamp(0.0, self.duration());
        if self.phase == Phase::Ended && self.at < self.duration() {
            self.phase = Phase::Paused;
        }
        self.arrange()?;
        if playing { self.play(now) } else { Ok(()) }
    }

    /// The program's sound: each deck at this volume, muted with it or by its fragment.
    pub fn set_volume(&mut self, volume: f64, muted: bool) -> Result<(), Failed> {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = muted;
        for d in 0..2 {
            if let Some(fragment) = self.holds[d] {
                self.sound(d, fragment)?;
            }
        }
        Ok(())
    }

    /// The timer of [`Program::next_change`] fired: the joins reached by now are played.
    pub fn tick(&mut self, now: u64) -> Result<(), Failed> {
        if self.phase != Phase::Playing {
            return Ok(());
        }
        let position = self.position(now);
        while let Some(&(time, mark)) = self.marks.get(self.next) {
            if time > position + SLACK {
                break;
            }
            self.next += 1;
            match mark {
                Mark::Enter(i) => {
                    let d = deck(i);
                    if self.holds[d] != Some(i) {
                        self.prepare(i, self.montage.fragments()[i].from, true)?;
                    }
                    self.decks[d].play()?;
                }
                Mark::Leave(i) if i + 1 == self.montage.fragments().len() => {
                    self.at = self.duration();
                    self.since = now;
                    self.phase = Phase::Ended;
                    for d in &mut self.decks {
                        d.pause()?;
                    }
                    return Ok(());
                }
                Mark::Leave(i) => {
                    self.decks[deck(i)].pause()?;
                    self.holds[deck(i)] = None;
                    if let Some(after) = self.montage.fragments().get(i + 2) {
                        self.prepare(i + 2, after.from, true)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn arrange(&mut self) -> Result<(), Failed> {
        self.arrange_with(true)
    }

    /// Both decks for the position, paused: the main fragment's deck on its moment, the other on
    /// the incoming moment of a transition, or waiting on the next fragment's first frame.
    fn arrange_with(&mut self, exact: bool) -> Result<(), Failed> {
        let shot = self.montage.shot_at(self.at);
        let main = shot.main.index;
        self.prepare(main, shot.main.source, exact)?;
        let next = main + 1;
        match shot.incoming {
            Some(incoming) => self.prepare(next, incoming.piece.source, exact)?,
            None => match self.montage.fragments().get(next) {
                Some(f) => self.prepare(next, f.from, true)?,
                None => {
                    self.holds[deck(next)] = None;
                    self.decks[deck(next)].pause()?;
                }
            },
        }
        self.decks[deck(main)].pause()?;
        Ok(())
    }

    fn prepare(&mut self, fragment: usize, source: f64, exact: bool) -> Result<(), Failed> {
        let d = deck(fragment);
        self.decks[d].pause()?;
        self.decks[d].set_rate(self.montage.fragments()[fragment].speed)?;
        self.holds[d] = Some(fragment);
        self.sound(d, fragment)?;
        self.seek_deck(d, Seek { to: source, exact })
    }

    fn sound(&mut self, d: usize, fragment: usize) -> Result<(), Failed> {
        let muted = self.muted || self.montage.fragments()[fragment].muted;
        self.decks[d].set_volume(self.volume, muted)
    }

    /// One seek at a time per deck: while it is busy, only the latest target waits.
    fn seek_deck(&mut self, d: usize, seek: Seek) -> Result<(), Failed> {
        if self.seeking[d] {
            self.queued[d] = Some(seek);
            return Ok(());
        }
        self.seeking[d] = true;
        self.decks[d].seek(seek.to, seek.exact)
    }

    /// Each deck's picture from now to the next change; `None` = the deck is not shown.
    pub fn looks(&self, now: u64) -> [Option<Look>; 2] {
        let mut looks = [None, None];
        if self.phase == Phase::Loading {
            return looks;
        }
        let at = self.position(now);
        let horizon = match self.next_change(now) {
            Some(when) => (when - now) as f64 / 1000.0,
            None => 0.0,
        };
        let starts = self.montage.starts();
        let fragments = self.montage.fragments();
        let frames = |i: usize| {
            let local = at - starts[i];
            let f = &fragments[i];
            curve::zoom_keys(f.zoom, f.length(), local, (local + horizon).min(f.length()))
        };
        let shot = self.montage.shot_at(at);
        let main = shot.main.index;
        match shot.incoming {
            Some(incoming) => {
                let overlap = self.montage.overlap(main);
                let p = incoming.progress;
                let to = (p + horizon / overlap.max(f64::EPSILON)).min(1.0);
                looks[deck(main)] = Some(Look {
                    fragment: main,
                    top: false,
                    frame: frames(main),
                    layer: curve::layer_keys(incoming.transition, overlap, (p, to), false),
                });
                let i = incoming.piece.index;
                looks[deck(i)] = Some(Look {
                    fragment: i,
                    top: true,
                    frame: frames(i),
                    layer: curve::layer_keys(incoming.transition, overlap, (p, to), true),
                });
            }
            None => {
                looks[deck(main)] = Some(Look {
                    fragment: main,
                    top: false,
                    frame: frames(main),
                    layer: vec![(0.0, curve::PLAIN)],
                });
            }
        }
        looks
    }
}

/// The joins in result time, in order; at one moment an entry comes before a leave.
fn marks_of(montage: &Montage) -> Vec<(f64, Mark)> {
    let starts = montage.starts();
    let mut marks = Vec::with_capacity(2 * starts.len());
    for (i, f) in montage.fragments().iter().enumerate() {
        if i > 0 {
            marks.push((starts[i], Mark::Enter(i)));
        }
        marks.push((starts[i] + f.length(), Mark::Leave(i)));
    }
    marks.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| matches!(a.1, Mark::Leave(_)).cmp(&matches!(b.1, Mark::Leave(_))))
    });
    marks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::montage::curve::Transition;
    use crate::play::Info;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Play(usize),
        Pause(usize),
        Seek(usize, f64),
        Rate(usize, f64),
        Volume(usize, bool),
    }

    struct Fake {
        id: usize,
        log: Rc<RefCell<Vec<Call>>>,
    }

    impl Engine for Fake {
        fn play(&mut self) -> Result<(), Failed> {
            self.log.borrow_mut().push(Call::Play(self.id));
            Ok(())
        }
        fn pause(&mut self) -> Result<(), Failed> {
            self.log.borrow_mut().push(Call::Pause(self.id));
            Ok(())
        }
        fn seek(&mut self, to: f64, _exact: bool) -> Result<(), Failed> {
            self.log.borrow_mut().push(Call::Seek(self.id, to));
            Ok(())
        }
        fn set_rate(&mut self, rate: f64) -> Result<(), Failed> {
            self.log.borrow_mut().push(Call::Rate(self.id, rate));
            Ok(())
        }
        fn set_volume(&mut self, _volume: f64, muted: bool) -> Result<(), Failed> {
            self.log.borrow_mut().push(Call::Volume(self.id, muted));
            Ok(())
        }
        fn step(&mut self, _forward: bool) -> Result<(), Failed> {
            Ok(())
        }
    }

    const INFO: Info = Info {
        duration: 10.0,
        size: (100, 100),
        picture: true,
        sound: true,
    };

    fn program(montage: Montage) -> (Program<Fake>, Rc<RefCell<Vec<Call>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let decks = [0, 1].map(|id| Fake {
            id,
            log: log.clone(),
        });
        let mut p = Program::new(decks, montage, 0.0, 1.0, false);
        for d in 0..2 {
            p.on_event(d, Event::Loaded(INFO)).unwrap();
        }
        for d in 0..2 {
            p.on_event(d, Event::Seeked(0.0)).unwrap();
        }
        (p, log)
    }

    fn cut(times: &[f64]) -> Montage {
        let mut m = Montage::new(10.0);
        for &t in times {
            assert!(m.split(t));
        }
        m
    }

    fn seeks(log: &Rc<RefCell<Vec<Call>>>) -> Vec<Call> {
        log.borrow()
            .iter()
            .filter(|c| matches!(c, Call::Seek(..)))
            .cloned()
            .collect()
    }

    #[test]
    fn nothing_moves_until_both_decks_are_loaded() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let decks = [0, 1].map(|id| Fake {
            id,
            log: log.clone(),
        });
        let mut p = Program::new(decks, cut(&[4.0]), 0.0, 1.0, false);
        p.on_event(0, Event::Loaded(INFO)).unwrap();
        assert_eq!(p.phase(), Phase::Loading);
        assert!(log.borrow().is_empty());
        p.on_event(1, Event::Loaded(INFO)).unwrap();
        assert_eq!(p.phase(), Phase::Paused);
        assert_eq!(seeks(&log), [Call::Seek(0, 0.0), Call::Seek(1, 4.0)]);
    }

    #[test]
    fn the_next_fragment_waits_on_the_other_deck_and_a_join_is_a_switch() {
        let mut m = cut(&[4.0]);
        m.move_to(1, 0);
        let (mut p, log) = program(m);
        log.borrow_mut().clear();
        p.play(1000).unwrap();
        assert_eq!(*log.borrow(), [Call::Play(0)]);
        assert_eq!(p.next_change(1000), Some(7000));
        log.borrow_mut().clear();
        p.tick(7000).unwrap();
        assert!(log.borrow().contains(&Call::Play(1)));
        assert!(log.borrow().contains(&Call::Pause(0)));
        assert!(seeks(&log).is_empty(), "the join seeks: {:?}", log.borrow());
        assert_eq!(p.current(7500), 1);
    }

    #[test]
    fn the_deck_that_is_done_gets_the_fragment_after_next() {
        let (mut p, log) = program(cut(&[3.0, 6.0]));
        p.play(0).unwrap();
        log.borrow_mut().clear();
        p.tick(3000).unwrap();
        assert!(
            log.borrow().contains(&Call::Seek(0, 6.0)),
            "{:?}",
            log.borrow()
        );
    }

    #[test]
    fn the_end_stops_both_decks_and_play_starts_over() {
        let (mut p, _) = program(cut(&[4.0]));
        p.play(0).unwrap();
        p.tick(4000).unwrap();
        p.tick(10_000).unwrap();
        assert_eq!(p.phase(), Phase::Ended);
        assert_eq!(p.next_change(10_000), None);
        p.play(11_000).unwrap();
        assert_eq!(p.phase(), Phase::Playing);
        assert!(p.position(11_000) < 0.001);
    }

    #[test]
    fn a_seek_into_a_transition_shows_both_pictures() {
        let mut m = cut(&[4.0]);
        m.set_transition(0, Some(Transition::Dissolve));
        let (mut p, log) = program(m);
        log.borrow_mut().clear();
        p.seek(3.75, true, 0).unwrap();
        assert!(seeks(&log).contains(&Call::Seek(0, 3.75)));
        assert!(seeks(&log).contains(&Call::Seek(1, 4.25)));
        let [a, b] = p.looks(0);
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(!a.top && b.top);
        assert_eq!(b.layer.len(), 1);
        assert!((b.layer[0].1.opacity - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_transition_plays_both_decks_and_the_outgoing_one_stops_at_its_end() {
        let mut m = cut(&[4.0]);
        m.set_transition(0, Some(Transition::Dissolve));
        let (mut p, log) = program(m);
        p.play(0).unwrap();
        assert_eq!(p.next_change(0), Some(3500));
        log.borrow_mut().clear();
        p.tick(3500).unwrap();
        assert_eq!(*log.borrow(), [Call::Play(1)]);
        let [a, b] = p.looks(3500);
        assert!(a.unwrap().layer.len() > 1 && b.unwrap().layer.len() > 1);
        assert_eq!(p.next_change(3500), Some(4000));
        log.borrow_mut().clear();
        p.tick(4000).unwrap();
        assert_eq!(*log.borrow(), [Call::Pause(0)]);
        assert!(p.looks(4000)[0].is_none());
    }

    #[test]
    fn it_stands_where_it_was_opened_once_loaded() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let decks = [0, 1].map(|id| Fake {
            id,
            log: log.clone(),
        });
        let mut p = Program::new(decks, cut(&[4.0]), 6.5, 1.0, false);
        for d in 0..2 {
            p.on_event(d, Event::Loaded(INFO)).unwrap();
        }
        assert!((p.position(0) - 6.5).abs() < 1e-9);
        assert!(
            seeks(&log).contains(&Call::Seek(1, 6.5)),
            "{:?}",
            log.borrow()
        );
    }

    #[test]
    fn decks_take_their_fragments_speed_and_sound() {
        let mut m = cut(&[4.0]);
        m.set_speed(1, 2.0);
        m.set_muted(1, true);
        let (_, log) = program(m);
        assert!(log.borrow().contains(&Call::Rate(1, 2.0)));
        assert!(log.borrow().contains(&Call::Volume(1, true)));
        assert!(log.borrow().contains(&Call::Volume(0, false)));
    }

    #[test]
    fn a_busy_deck_keeps_only_the_latest_seek() {
        let (mut p, log) = program(Montage::new(10.0));
        log.borrow_mut().clear();
        p.seek(1.0, false, 0).unwrap();
        p.seek(2.0, false, 0).unwrap();
        p.seek(3.0, false, 0).unwrap();
        assert_eq!(seeks(&log), [Call::Seek(0, 1.0)]);
        p.on_event(0, Event::Seeked(1.0)).unwrap();
        assert_eq!(seeks(&log), [Call::Seek(0, 1.0), Call::Seek(0, 3.0)]);
    }

    #[test]
    fn an_edit_keeps_playing_from_the_same_moment() {
        let (mut p, _) = program(Montage::new(10.0));
        p.play(0).unwrap();
        let mut m = Montage::new(10.0);
        m.split(5.0);
        p.set_montage(m, p.position(2000), 2000).unwrap();
        assert!(p.running());
        assert!((p.position(2000) - 2.0).abs() < 1e-9);
        assert_eq!(p.next_change(2000), Some(5000));
    }

    #[test]
    fn a_smooth_zoom_moves_while_playing_and_holds_while_paused() {
        let mut m = Montage::new(10.0);
        m.set_zoom(
            0,
            Some(curve::Zoom {
                frame: Frame::around((0.5, 0.5), 0.5),
                smooth: true,
            }),
        );
        let (mut p, _) = program(m);
        assert_eq!(p.looks(0)[0].as_ref().unwrap().frame.len(), 1);
        p.play(0).unwrap();
        assert!(p.looks(0)[0].as_ref().unwrap().frame.len() > 2);
    }
}
