//! The saved sound, fragment by fragment: each fragment's samples (already sped up, silence when
//! muted) are cut or filled to its exact length in the result, and where a transition overlaps
//! two fragments the end of the first is held back and crossed into the start of the next at
//! equal power. It streams: a fragment never has to be in memory whole.
//!
//! Counts are frames (one sample of every channel); samples are interleaved.

use super::curve::sound_gains;

pub struct Splice {
    channels: usize,
    length: usize,
    before: usize,
    after: usize,
    done: usize,
    /// The previous fragment's last frames, to cross into this one's first.
    tail: Vec<f32>,
    /// This fragment's last frames, kept for the next one.
    held: Vec<f32>,
}

impl Splice {
    pub fn new(channels: usize) -> Self {
        Self {
            channels: channels.max(1),
            length: 0,
            before: 0,
            after: 0,
            done: 0,
            tail: Vec::new(),
            held: Vec::new(),
        }
    }

    /// A fragment starts: `length` frames in the result, overlapping the previous one by
    /// `before` frames and the next one by `after`.
    pub fn begin(&mut self, length: usize, before: usize, after: usize) {
        self.length = length;
        self.before = before.min(self.tail.len() / self.channels);
        self.after = after.min(length.saturating_sub(self.before));
        self.done = 0;
        self.held.clear();
    }

    /// The fragment's samples as they come; what is final goes to `out`.
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        let c = self.channels;
        for frame in samples.chunks_exact(c) {
            let k = self.done;
            if k >= self.length {
                return;
            }
            self.done += 1;
            if k < self.before {
                let (fade_out, fade_in) = sound_gains((k as f64 + 0.5) / self.before as f64);
                for (ch, &s) in frame.iter().enumerate() {
                    let old = self.tail[k * c + ch];
                    out.push(old * fade_out as f32 + s * fade_in as f32);
                }
            } else if k >= self.length - self.after {
                self.held.extend_from_slice(frame);
            } else {
                out.extend_from_slice(frame);
            }
        }
    }

    /// The fragment's samples are over: short of its length it gets silence; its held end waits
    /// for the next fragment, or goes to `out` when this was the last.
    pub fn end(&mut self, out: &mut Vec<f32>, last: bool) {
        if self.done < self.length {
            let silence = vec![0.0; (self.length - self.done) * self.channels];
            self.push(&silence, out);
        }
        self.tail = std::mem::take(&mut self.held);
        if last {
            out.append(&mut self.tail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(fragments: &[(Vec<f32>, usize, usize, usize)]) -> Vec<f32> {
        let mut splice = Splice::new(1);
        let mut out = Vec::new();
        for (i, (samples, length, before, after)) in fragments.iter().enumerate() {
            splice.begin(*length, *before, *after);
            splice.push(samples, &mut out);
            splice.end(&mut out, i + 1 == fragments.len());
        }
        out
    }

    #[test]
    fn fragments_without_transitions_follow_each_other() {
        let out = run(&[(vec![1.0; 3], 3, 0, 0), (vec![2.0; 2], 2, 0, 0)]);
        assert_eq!(out, [1.0, 1.0, 1.0, 2.0, 2.0]);
    }

    #[test]
    fn a_fragment_is_cut_or_filled_to_its_length() {
        let out = run(&[(vec![1.0; 5], 3, 0, 0), (vec![2.0; 1], 3, 0, 0)]);
        assert_eq!(out, [1.0, 1.0, 1.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn a_transition_crosses_the_two_sounds_over_the_overlap() {
        let out = run(&[(vec![1.0; 4], 4, 0, 2), (vec![1.0; 4], 4, 2, 0)]);
        // 4 + 4 frames overlapping by 2: 6 in the result.
        assert_eq!(out.len(), 6);
        // Equal power: a steady sound gets louder in the middle of the cross, never quieter.
        assert!(out[2] >= 1.0 && out[3] >= 1.0);
        let (a, b) = sound_gains(0.25);
        assert!((out[2] - (a + b) as f32).abs() < 1e-6);
    }

    #[test]
    fn channels_stay_apart_in_a_cross() {
        let mut splice = Splice::new(2);
        let mut out = Vec::new();
        splice.begin(2, 0, 1);
        splice.push(&[1.0, 0.0, 1.0, 0.0], &mut out);
        splice.end(&mut out, false);
        splice.begin(2, 1, 0);
        splice.push(&[0.0, 1.0, 0.0, 1.0], &mut out);
        splice.end(&mut out, true);
        assert_eq!(out.len(), 6);
        let (a, b) = sound_gains(0.5);
        assert!((out[2] - a as f32).abs() < 1e-6 && (out[3] - b as f32).abs() < 1e-6);
    }
}
