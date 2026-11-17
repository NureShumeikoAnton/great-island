//! Video editing: everything that does not need Windows.
//!
//! A [`Montage`] is the edit of one recording: its fragments in their order, each a piece of the
//! source with its own speed, sound and zoom, and the transitions on the joins between them. Its
//! edits are values, so the editor keeps whole states for undo. What a moment of the result
//! shows ([`Montage::shot_at`]) is one answer for watching the edit and for saving it.
//!
//! - [`curve`] — the zoom and transition curves;
//! - [`splice`] — the saved sound joined fragment by fragment, crossed at transitions;
//! - [`squeeze`] — the compression modes: the output's size and bitrate, the file size estimate;
//! - [`stretch`] — sound sped up with its pitch kept;
//! - [`strip`] — the timeline's thumbnail moments and the sound's wave;
//! - [`watch`] — watching an edit on two engines in turns.
//!
//! Times are seconds: `source` of the recording, `result` of the edited video.

pub mod adapters;
pub mod curve;
pub mod splice;
pub mod squeeze;
pub mod stretch;
pub mod strip;
pub mod watch;

use curve::{Frame, Transition, Zoom};

pub const SPEEDS: [f64; 6] = [1.0, 1.5, 2.0, 3.0, 4.0, 8.0];
/// A transition overlaps the two fragments by this much, less when one of them is short.
pub const TRANSITION: f64 = 0.5;
/// A cut never leaves a piece shorter than this (result seconds).
pub const MIN_PIECE: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fragment {
    pub from: f64,
    pub to: f64,
    pub speed: f64,
    pub muted: bool,
    pub zoom: Option<Zoom>,
}

impl Fragment {
    fn whole(source: f64) -> Self {
        Self {
            from: 0.0,
            to: source,
            speed: 1.0,
            muted: false,
            zoom: None,
        }
    }

    /// Result seconds.
    pub fn length(&self) -> f64 {
        (self.to - self.from) / self.speed
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub index: usize,
    pub source: f64,
    pub frame: Frame,
}

/// What a moment of the result shows: `main`, and during a transition the next fragment coming
/// in over it, `progress` 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub main: Piece,
    pub incoming: Option<Incoming>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Incoming {
    pub piece: Piece,
    pub transition: Transition,
    pub progress: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Montage {
    source: f64,
    fragments: Vec<Fragment>,
    /// `joins[i]` is between fragments `i` and `i + 1`.
    joins: Vec<Option<Transition>>,
}

impl Montage {
    pub fn new(source: f64) -> Self {
        Self {
            source,
            fragments: vec![Fragment::whole(source)],
            joins: Vec::new(),
        }
    }

    pub fn fragments(&self) -> &[Fragment] {
        &self.fragments
    }

    pub fn joins(&self) -> &[Option<Transition>] {
        &self.joins
    }

    /// Saving it would write the same video again.
    pub fn is_untouched(&self) -> bool {
        self.fragments == [Fragment::whole(self.source)]
    }

    pub fn overlap(&self, join: usize) -> f64 {
        match self.joins.get(join) {
            Some(Some(_)) => TRANSITION
                .min(self.fragments[join].length() / 2.0)
                .min(self.fragments[join + 1].length() / 2.0),
            _ => 0.0,
        }
    }

    pub fn starts(&self) -> Vec<f64> {
        let mut starts = Vec::with_capacity(self.fragments.len());
        let mut at = 0.0;
        for (i, fragment) in self.fragments.iter().enumerate() {
            starts.push(at);
            at += fragment.length() - self.overlap(i);
        }
        starts
    }

    pub fn duration(&self) -> f64 {
        let last = self.fragments.len() - 1;
        self.starts()[last] + self.fragments[last].length()
    }

    /// The fragment that began last at or before `result`, and the source moment in it. During
    /// a transition that is the incoming one.
    pub fn at(&self, result: f64) -> (usize, f64) {
        let starts = self.starts();
        let index = starts.iter().rposition(|&s| s <= result).unwrap_or(0);
        (index, self.source_in(index, result - starts[index]))
    }

    fn source_in(&self, index: usize, local: f64) -> f64 {
        let f = &self.fragments[index];
        (f.from + local.clamp(0.0, f.length()) * f.speed).min(f.to)
    }

    fn piece(&self, index: usize, local: f64) -> Piece {
        let f = &self.fragments[index];
        Piece {
            index,
            source: self.source_in(index, local),
            frame: curve::zoom_frame(f.zoom, local, f.length()),
        }
    }

    pub fn shot_at(&self, result: f64) -> Shot {
        let starts = self.starts();
        let index = starts.iter().rposition(|&s| s <= result).unwrap_or(0);
        let local = result - starts[index];
        if index > 0 && local < self.overlap(index - 1) {
            let out = index - 1;
            let progress = local / self.overlap(out);
            return Shot {
                main: self.piece(out, result - starts[out]),
                incoming: self.joins[out].map(|transition| Incoming {
                    piece: self.piece(index, local),
                    transition,
                    progress,
                }),
            };
        }
        Shot {
            main: self.piece(index, local),
            incoming: None,
        }
    }

    pub fn result_of(&self, index: usize, source: f64) -> f64 {
        let f = &self.fragments[index];
        self.starts()[index] + (source.clamp(f.from, f.to) - f.from) / f.speed
    }

    /// Each fragment's sound in the result at `rate` frames a second: (length, overlap with the
    /// previous fragment, overlap with the next). Counted from rounded positions, so the parts add
    /// up to the whole exactly.
    pub fn sound_parts(&self, rate: u32) -> Vec<(usize, usize, usize)> {
        let at = |t: f64| (t * f64::from(rate)).round() as usize;
        let starts = self.starts();
        let spans: Vec<(usize, usize)> = self
            .fragments
            .iter()
            .zip(&starts)
            .map(|(f, &s)| (at(s), at(s + f.length())))
            .collect();
        (0..spans.len())
            .map(|i| {
                let (start, end) = spans[i];
                let before = i
                    .checked_sub(1)
                    .map_or(0, |p| spans[p].1.saturating_sub(start));
                let after = spans.get(i + 1).map_or(0, |n| end.saturating_sub(n.0));
                (end - start, before, after)
            })
            .collect()
    }

    /// Cuts the fragment under `result` in two. Refused too close to its ends.
    pub fn split(&mut self, result: f64) -> bool {
        let (index, source) = self.at(result);
        let f = self.fragments[index];
        let local = (source - f.from) / f.speed;
        if local < MIN_PIECE || f.length() - local < MIN_PIECE {
            return false;
        }
        self.fragments[index].to = source;
        self.fragments
            .insert(index + 1, Fragment { from: source, ..f });
        self.joins.insert(index, None);
        true
    }

    /// The last fragment can't go.
    pub fn remove(&mut self, index: usize) -> bool {
        if self.fragments.len() < 2 || index >= self.fragments.len() {
            return false;
        }
        self.take(index);
        true
    }

    /// Moves fragment `index` to `slot`: 0 is the start, `len()` the end, any other the join
    /// before that fragment. The joins it leaves and the one it lands in lose their transitions.
    pub fn move_to(&mut self, index: usize, slot: usize) -> bool {
        let n = self.fragments.len();
        if index >= n || slot > n || slot == index || slot == index + 1 {
            return false;
        }
        let fragment = self.take(index);
        let slot = if slot > index { slot - 1 } else { slot };
        if slot == 0 {
            self.joins.insert(0, None);
        } else if slot == self.fragments.len() {
            self.joins.push(None);
        } else {
            self.joins[slot - 1] = None;
            self.joins.insert(slot, None);
        }
        self.fragments.insert(slot, fragment);
        true
    }

    fn take(&mut self, index: usize) -> Fragment {
        let last = self.fragments.len() - 1;
        if index == 0 {
            self.joins.remove(0);
        } else if index == last {
            self.joins.remove(last - 1);
        } else {
            self.joins.remove(index);
            self.joins[index - 1] = None;
        }
        self.fragments.remove(index)
    }

    pub fn set_speed(&mut self, index: usize, speed: f64) -> bool {
        self.change(index, |f| f.speed = speed)
    }

    pub fn set_muted(&mut self, index: usize, muted: bool) -> bool {
        self.change(index, |f| f.muted = muted)
    }

    pub fn set_zoom(&mut self, index: usize, zoom: Option<Zoom>) -> bool {
        self.change(index, |f| f.zoom = zoom)
    }

    pub fn set_transition(&mut self, join: usize, transition: Option<Transition>) -> bool {
        match self.joins.get_mut(join) {
            Some(slot) if *slot != transition => {
                *slot = transition;
                true
            }
            _ => false,
        }
    }

    /// `true` when something changed.
    fn change(&mut self, index: usize, edit: impl FnOnce(&mut Fragment)) -> bool {
        let Some(fragment) = self.fragments.get_mut(index) else {
            return false;
        };
        let was = *fragment;
        edit(fragment);
        *fragment != was
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn cut(times: &[f64]) -> Montage {
        let mut m = Montage::new(10.0);
        for &t in times {
            assert!(m.split(t));
        }
        m
    }

    #[test]
    fn a_new_montage_is_the_whole_recording() {
        let m = Montage::new(10.0);
        assert!(m.is_untouched());
        assert!(near(m.duration(), 10.0));
        assert_eq!(m.at(4.0), (0, 4.0));
    }

    #[test]
    fn a_cut_makes_two_fragments_and_keeps_the_length() {
        let m = cut(&[4.0]);
        assert_eq!(m.fragments().len(), 2);
        assert!(near(m.fragments()[0].to, 4.0));
        assert!(near(m.fragments()[1].from, 4.0));
        assert!(near(m.duration(), 10.0));
        assert!(!m.is_untouched());
        assert_eq!(m.joins(), &[None]);
    }

    #[test]
    fn a_cut_too_close_to_an_end_is_refused() {
        let mut m = Montage::new(10.0);
        assert!(!m.split(0.1));
        assert!(!m.split(9.9));
        assert!(m.is_untouched());
    }

    #[test]
    fn removing_shortens_the_result_but_never_takes_the_last() {
        let mut m = cut(&[4.0]);
        assert!(m.remove(0));
        assert!(near(m.duration(), 6.0));
        assert_eq!(m.at(0.0), (0, 4.0));
        assert!(!m.remove(0));
    }

    #[test]
    fn a_fragment_moves_to_the_start_the_end_or_a_join() {
        let mut m = cut(&[2.0, 5.0]);
        let order = |m: &Montage| m.fragments().iter().map(|f| f.from).collect::<Vec<_>>();
        assert!(m.move_to(2, 0));
        assert_eq!(order(&m), [5.0, 0.0, 2.0]);
        assert!(m.move_to(0, 3));
        assert_eq!(order(&m), [0.0, 2.0, 5.0]);
        assert!(m.move_to(0, 2));
        assert_eq!(order(&m), [2.0, 0.0, 5.0]);
        assert!(near(m.duration(), 10.0));
        assert_eq!(m.joins().len(), 2);
    }

    #[test]
    fn a_move_next_to_itself_changes_nothing() {
        let mut m = cut(&[2.0, 5.0]);
        assert!(!m.move_to(1, 1));
        assert!(!m.move_to(1, 2));
    }

    #[test]
    fn the_joins_a_fragment_leaves_or_lands_in_lose_their_transitions() {
        let mut m = cut(&[2.0, 5.0, 8.0]);
        for join in 0..3 {
            m.set_transition(join, Some(Transition::Dissolve));
        }
        let dissolve = Some(Transition::Dissolve);
        assert!(m.move_to(0, 4));
        assert_eq!(m.joins(), &[dissolve, dissolve, None]);
        assert!(m.move_to(1, 3));
        assert_eq!(m.joins(), &[None, None, None]);
    }

    #[test]
    fn a_cut_keeps_the_neighbours_transitions_and_a_removal_merges_joins() {
        let mut m = cut(&[5.0]);
        m.set_transition(0, Some(Transition::Black));
        assert!(m.split(2.0));
        assert_eq!(m.joins(), &[None, Some(Transition::Black)]);
        assert!(m.remove(1));
        assert_eq!(m.joins(), &[None]);
    }

    #[test]
    fn speed_shortens_its_fragment() {
        let mut m = cut(&[4.0]);
        assert!(m.set_speed(1, 2.0));
        assert!(!m.set_speed(1, 2.0));
        assert!(near(m.duration(), 7.0));
        let (index, source) = m.at(5.0);
        assert_eq!(index, 1);
        assert!(near(source, 6.0));
        assert!(near(m.result_of(1, 6.0), 5.0));
    }

    #[test]
    fn a_transition_overlaps_the_fragments() {
        let mut m = cut(&[4.0]);
        assert!(m.set_transition(0, Some(Transition::Dissolve)));
        assert!(near(m.duration(), 10.0 - TRANSITION));
        let shot = m.shot_at(3.75);
        assert_eq!(shot.main.index, 0);
        assert!(near(shot.main.source, 3.75));
        let incoming = shot.incoming.unwrap();
        assert_eq!(incoming.piece.index, 1);
        assert!(near(incoming.piece.source, 4.25));
        assert!(near(incoming.progress, 0.5));
        assert_eq!(m.shot_at(5.0).incoming, None);
    }

    #[test]
    fn the_sound_parts_add_up_to_the_result() {
        let mut m = cut(&[3.333, 6.71]);
        m.set_transition(1, Some(Transition::Dissolve));
        m.set_speed(0, 1.5);
        let parts = m.sound_parts(48_000);
        let total: usize =
            parts.iter().map(|p| p.0).sum::<usize>() - parts.iter().map(|p| p.1).sum::<usize>();
        assert_eq!(total, (m.duration() * 48_000.0).round() as usize);
        assert_eq!(parts[2].1, 24_000);
        assert_eq!(parts[1].2, 24_000);
        assert_eq!(parts[0].1, 0);
    }

    #[test]
    fn a_short_fragment_shortens_its_transition() {
        let mut m = cut(&[4.0, 4.6]);
        m.set_transition(0, Some(Transition::Slide));
        assert!(near(m.overlap(0), 0.3));
    }

    #[test]
    fn a_muted_fragment_and_a_zoom_are_changes() {
        let mut m = Montage::new(10.0);
        assert!(m.set_muted(0, true));
        assert!(!m.is_untouched());
        assert!(m.set_muted(0, false));
        assert!(m.is_untouched());
        let zoom = Zoom {
            frame: Frame::around((0.5, 0.5), 0.5),
            smooth: false,
        };
        assert!(m.set_zoom(0, Some(zoom)));
        assert_eq!(m.shot_at(5.0).main.frame, zoom.frame);
    }
}
