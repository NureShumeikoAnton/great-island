//! Fast drain: the battery gives away at least twice its usual power for 5 minutes.
//!
//! The usual power is learnt on this computer ([`Baseline`]): the median of the minutes on
//! battery, the older ones slowly forgotten, kept between runs. Until it has two hours, a
//! battery that would last 5 hours stands in for it.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const BUCKET_MW: f32 = 250.0;
/// Up to 60 W: more lands in the last bucket.
const BUCKETS: usize = 240;
/// Each new minute fades the old ones a little: roughly the last 3000 minutes on battery
/// (a week or two of laptop use) make the usual.
const KEEP: f32 = 1.0 - 1.0 / 3_000.0;
const LEARNT_MINUTES: f32 = 120.0;
const STAND_IN_HOURS: f32 = 5.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// Minutes on battery by power, `BUCKET_MW` wide.
    minutes: Vec<f32>,
}

impl Default for Baseline {
    fn default() -> Self {
        Self {
            minutes: vec![0.0; BUCKETS],
        }
    }
}

impl Baseline {
    /// A file from another version with other buckets starts over rather than lie.
    pub fn sanitized(self) -> Self {
        if self.minutes.len() == BUCKETS && self.minutes.iter().all(|m| m.is_finite() && *m >= 0.0)
        {
            self
        } else {
            Self::default()
        }
    }

    pub fn add(&mut self, mw: f32) {
        for m in &mut self.minutes {
            *m *= KEEP;
        }
        let i = ((mw / BUCKET_MW) as usize).min(BUCKETS - 1);
        self.minutes[i] += 1.0;
    }

    /// The median power, in mW, once two hours are learnt.
    pub fn learnt_mw(&self) -> Option<f32> {
        let total: f32 = self.minutes.iter().sum();
        // What two hours add up to after fading: a little under 120.
        let learnt = (1.0 - KEEP.powf(LEARNT_MINUTES)) / (1.0 - KEEP);
        if total + 0.01 < learnt {
            return None;
        }
        let mut seen = 0.0;
        for (i, m) in self.minutes.iter().enumerate() {
            seen += m;
            if seen >= total / 2.0 {
                return Some((i as f32 + 0.5) * BUCKET_MW);
            }
        }
        None
    }

    pub fn usual_mw(&self, full_mwh: Option<u32>) -> Option<f32> {
        self.learnt_mw()
            .or_else(|| full_mwh.map(|full| full as f32 / STAND_IN_HOURS))
    }
}

const WINDOW: usize = 5;
const FAST: f32 = 2.0;
/// Said once until the drain falls back under this: no new notice for the same spell.
const CALM: f32 = 1.5;
const COOLDOWN_MS: u64 = 30 * 60_000;
/// Minutes further apart than this (the computer slept) don't make one spell.
const GAP_MS: u64 = 150_000;

#[derive(Debug, Default)]
pub struct Drain {
    recent: VecDeque<(u64, f32)>,
    spell: bool,
    quiet_until: u64,
}

impl Drain {
    /// A minute's power on battery. Returns the 5-minute average when it is too fast.
    pub fn sample(&mut self, now: u64, mw: f32, usual_mw: f32) -> Option<f32> {
        if self
            .recent
            .back()
            .is_some_and(|&(at, _)| now.saturating_sub(at) > GAP_MS)
        {
            self.recent.clear();
        }
        self.recent.push_back((now, mw));
        if self.recent.len() > WINDOW {
            self.recent.pop_front();
        }
        let average = self.recent.iter().map(|&(_, mw)| mw).sum::<f32>() / self.recent.len() as f32;
        if average < CALM * usual_mw {
            self.spell = false;
        }
        let fast = self.recent.len() == WINDOW && average >= FAST * usual_mw;
        if !fast || self.spell || now < self.quiet_until {
            return None;
        }
        self.spell = true;
        self.quiet_until = now + COOLDOWN_MS;
        Some(average)
    }

    /// On the charger: the next time on battery starts a new count.
    pub fn reset(&mut self) {
        self.recent.clear();
        self.spell = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn the_usual_is_the_median_after_two_hours() {
        let mut b = Baseline::default();
        assert_eq!(b.usual_mw(Some(50_000)), Some(10_000.0));
        for i in 0..200 {
            b.add(if i % 4 == 0 { 30_000.0 } else { 7_000.0 });
        }
        let usual = b.learnt_mw().expect("learnt");
        assert!((7_000.0..7_300.0).contains(&usual), "{usual}");
    }

    #[test]
    fn old_minutes_fade() {
        let mut b = Baseline::default();
        for _ in 0..3_000 {
            b.add(6_000.0);
        }
        for _ in 0..6_000 {
            b.add(12_000.0);
        }
        let usual = b.learnt_mw().expect("learnt");
        assert!((12_000.0..12_300.0).contains(&usual), "{usual}");
    }

    #[test]
    fn a_damaged_baseline_starts_over() {
        let b = Baseline {
            minutes: vec![1.0; 3],
        };
        assert_eq!(b.sanitized(), Baseline::default());
    }

    #[test]
    fn fast_after_five_minutes_at_twice_the_usual_once_per_spell() {
        let mut d = Drain::default();
        let fired: Vec<_> = (0..12)
            .filter_map(|m| d.sample(m * MIN, 20_000.0, 8_000.0))
            .collect();
        assert_eq!(fired, [20_000.0]);
    }

    #[test]
    fn a_spike_or_a_mild_rise_is_not_fast() {
        let mut d = Drain::default();
        let powers = [
            8_000.0, 40_000.0, 8_000.0, 8_000.0, 8_000.0, 14_000.0, 14_000.0,
        ];
        for (m, mw) in powers.into_iter().enumerate() {
            assert_eq!(d.sample(m as u64 * MIN, mw, 8_000.0), None);
        }
    }

    #[test]
    fn a_new_spell_waits_for_calm_and_half_an_hour() {
        let mut d = Drain::default();
        let mut fired = Vec::new();
        let mut at = 0;
        let mut run = |d: &mut Drain, mw: f32, minutes: u64| {
            for _ in 0..minutes {
                if d.sample(at, mw, 8_000.0).is_some() {
                    fired.push(at / MIN);
                }
                at += MIN;
            }
        };
        run(&mut d, 20_000.0, 6);
        run(&mut d, 8_000.0, 6);
        run(&mut d, 20_000.0, 10);
        run(&mut d, 8_000.0, 6);
        run(&mut d, 20_000.0, 10);
        // The second spell is fast from minute 32 but waits out the half hour.
        assert_eq!(fired, [4, 34]);
    }

    #[test]
    fn sleep_breaks_the_count() {
        let mut d = Drain::default();
        for m in 0..4 {
            assert_eq!(d.sample(m * MIN, 20_000.0, 8_000.0), None);
        }
        assert_eq!(d.sample(60 * MIN, 20_000.0, 8_000.0), None);
    }
}
