//! The bars of the microphone the pre-start panel listens to: from the loudest sample of
//! each moment to bar heights that jump up at once and fall slowly after a short wait.

pub const BARS: usize = 4;
/// Below this the bar is empty: room noise stays flat.
const RANGE_DB: f32 = 50.0;
const HOLD_MS: u64 = 250;
/// A full bar falls to nothing in this long.
const FALL_MS: f32 = 600.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Bars {
    shown: [f32; BARS],
    held_until: [u64; BARS],
    last: u64,
}

impl Bars {
    /// `peaks` — the loudest sample of each bar since the last call, 0 to 1. Returns the
    /// heights, 0 to 1.
    pub fn show(&mut self, peaks: [f32; BARS], now: u64) -> [f32; BARS] {
        let fall = now.saturating_sub(self.last) as f32 / FALL_MS;
        self.last = now;
        for (i, peak) in peaks.into_iter().enumerate() {
            let height = height(peak);
            if height >= self.shown[i] {
                self.shown[i] = height;
                self.held_until[i] = now + HOLD_MS;
            } else if now >= self.held_until[i] {
                self.shown[i] = (self.shown[i] - fall).max(height);
            }
        }
        self.shown
    }
}

/// Loudness is heard in decibels: a linear bar would sit near the bottom for speech.
fn height(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_jump_up_and_fall_after_a_wait() {
        let mut bars = Bars::default();
        let up = bars.show([1.0, 0.1, 0.0, 0.0], 0);
        assert_eq!(up[0], 1.0);
        assert!(
            (up[1] - 0.6).abs() < 0.01,
            "-20 dB is 60% of 50 dB: {}",
            up[1]
        );
        assert_eq!(up[2], 0.0);
        let held = bars.show([0.0; BARS], 200);
        assert_eq!(held[0], 1.0, "held a moment");
        let falling = bars.show([0.0; BARS], 500);
        assert!(falling[0] < 0.6 && falling[0] > 0.4, "{}", falling[0]);
        assert_eq!(bars.show([0.0; BARS], 2000), [0.0; BARS]);
    }

    #[test]
    fn quiet_rooms_stay_flat() {
        assert_eq!(height(0.001), 0.0);
        assert_eq!(height(2.0), 1.0);
    }
}
