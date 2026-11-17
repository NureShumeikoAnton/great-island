//! Evens out the player's own volume for the level bars: the island scales each reading so the
//! player's typical loudness (the median of the last ~20 s of sound) lands on one reference. The
//! bars then look the same with the player's slider at 100% or at 20%.
//!
//! The median, not the average: single hits and short quiet moments don't move it, so a steady beat
//! keeps standing out for the whole track. Silence is left out, so a pause doesn't drag it down, and
//! it carries over between tracks: the player's volume stays the same.

/// Readings are pooled into slots: the loudest reading of each slot is one value for the median,
/// so the window doesn't depend on how often the bars sample.
const SLOT_MS: f32 = 100.0;
/// 20 s of sound.
const SLOTS: usize = 200;
/// Fewer slots than this (2 s of sound) say nothing yet: the gain stays 1.
const MIN_SLOTS: usize = 20;
/// Recounted every second of sound, not on every reading.
const RECOUNT_EVERY: usize = 10;
/// A slot quieter than this (−50 dB) is silence and isn't counted.
const SILENT: f32 = 0.0032;
/// Where the typical loudness is moved to (about −6 dB): roughly where a player at full volume
/// already is, so at full volume the bars look as before. Lower it to make the bars calmer.
const REFERENCE: f32 = 0.5;
/// Up to +24 dB, so a nearly muted player doesn't turn its hiss into music.
const MAX_GAIN: f32 = 16.0;
const MIN_GAIN: f32 = 0.5;

#[derive(Default)]
pub struct Leveler {
    source: String,
    slots: std::collections::VecDeque<f32>,
    slot_ms: f32,
    slot_peak: f32,
    since_recount: usize,
    gain: Option<f32>,
}

impl Leveler {
    /// The factor for this reading. `source` is the player: another player has its own volume, so
    /// the leveler starts over.
    pub fn gain(&mut self, source: &str, peak: f32, dt_ms: f32) -> f32 {
        if source != self.source {
            *self = Self {
                source: source.to_owned(),
                ..Self::default()
            };
        }
        self.slot_peak = self.slot_peak.max(peak);
        self.slot_ms += dt_ms;
        if self.slot_ms >= SLOT_MS {
            self.close_slot();
        }
        self.gain.unwrap_or(1.0)
    }

    fn close_slot(&mut self) {
        let peak = std::mem::take(&mut self.slot_peak);
        self.slot_ms -= SLOT_MS;
        if peak < SILENT {
            return;
        }
        if self.slots.len() == SLOTS {
            self.slots.pop_front();
        }
        self.slots.push_back(peak);
        self.since_recount += 1;
        if self.slots.len() >= MIN_SLOTS
            && (self.gain.is_none() || self.since_recount >= RECOUNT_EVERY)
        {
            self.since_recount = 0;
            let mut sorted: Vec<f32> = self.slots.iter().copied().collect();
            let mid = sorted.len() / 2;
            let (_, median, _) = sorted.select_nth_unstable_by(mid, f32::total_cmp);
            self.gain = Some((REFERENCE / *median).clamp(MIN_GAIN, MAX_GAIN));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = 33.0;

    /// Music: a hit every half second over a quieter bed, at `volume`. Returns the last gain.
    fn play(l: &mut Leveler, volume: f32, ms: f32) -> f32 {
        let mut gain = 1.0;
        let mut t = 0.0;
        while t < ms {
            let hit = t % 500.0 < 100.0;
            gain = l.gain("player", volume * if hit { 0.9 } else { 0.4 }, STEP);
            t += STEP;
        }
        gain
    }

    #[test]
    fn the_players_volume_cancels_out() {
        let loud = play(&mut Leveler::default(), 1.0, 20_000.0);
        let quiet = play(&mut Leveler::default(), 0.2, 20_000.0);
        assert!((loud * 1.0 - quiet * 0.2).abs() < 0.01, "{loud} vs {quiet}");
    }

    #[test]
    fn the_first_seconds_keep_the_level_as_is() {
        let mut l = Leveler::default();
        assert_eq!(play(&mut l, 0.2, 1500.0), 1.0);
        assert!(play(&mut l, 0.2, 2000.0) > 1.0);
    }

    #[test]
    fn hits_and_pauses_do_not_move_the_middle() {
        let mut l = Leveler::default();
        let before = play(&mut l, 0.5, 20_000.0);
        for _ in 0..(30_000.0 / STEP) as usize {
            l.gain("player", 0.0, STEP);
        }
        for _ in 0..(1000.0 / STEP) as usize {
            l.gain("player", 1.0, STEP);
        }
        let after = play(&mut l, 0.5, 1000.0);
        assert!((before - after).abs() / before < 0.1, "{before} → {after}");
    }

    #[test]
    fn another_player_starts_over() {
        let mut l = Leveler::default();
        play(&mut l, 0.2, 20_000.0);
        assert_eq!(l.gain("other", 0.2, STEP), 1.0);
    }

    #[test]
    fn a_nearly_muted_player_is_not_blown_up() {
        let gain = play(&mut Leveler::default(), 0.005, 20_000.0);
        assert_eq!(gain, MAX_GAIN);
    }
}
