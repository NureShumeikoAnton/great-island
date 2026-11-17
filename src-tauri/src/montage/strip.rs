//! What the timeline is drawn from: the moments its thumbnails are taken at, and the sound's
//! peaks, gathered once when the editor opens; a fragment's bars are picked from them.

/// Recordings have a keyframe every 2 s: a thumbnail there decodes without the frames before it.
pub const KEY_EVERY: f64 = 2.0;
pub const MAX_THUMBS: usize = 120;
pub const PEAK_SECONDS: f64 = 0.02;

/// Every keyframe, or every few of them in a long recording.
pub fn thumb_times(duration: f64) -> Vec<f64> {
    let keys = (duration / KEY_EVERY).ceil().max(1.0);
    let step = KEY_EVERY * (keys / MAX_THUMBS as f64).ceil().max(1.0);
    (0..)
        .map(|i| i as f64 * step)
        .take_while(|&t| t < duration.max(f64::EPSILON))
        .collect()
}

/// The loudest sample (of any channel) per [`PEAK_SECONDS`], 0..1.
pub struct Peaks {
    per_peak: usize,
    count: usize,
    current: f32,
    peaks: Vec<f32>,
}

impl Peaks {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        Self {
            per_peak: ((f64::from(sample_rate) * PEAK_SECONDS) as usize).max(1) * channels.max(1),
            count: 0,
            current: 0.0,
            peaks: Vec::new(),
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.current = self.current.max(s.abs().min(1.0));
            self.count += 1;
            if self.count == self.per_peak {
                self.peaks.push(self.current);
                self.count = 0;
                self.current = 0.0;
            }
        }
    }

    pub fn finish(mut self) -> Vec<f32> {
        if self.count > 0 {
            self.peaks.push(self.current);
        }
        self.peaks
    }
}

/// `count` bars for the source span `from..to`: each the loudest peak under it.
pub fn bars(peaks: &[f32], from: f64, to: f64, count: usize) -> Vec<f32> {
    let index = |t: f64| ((t / PEAK_SECONDS + 1e-9) as usize).min(peaks.len());
    let span = (to - from) / count.max(1) as f64;
    (0..count)
        .map(|i| {
            let a = index(from + span * i as f64);
            let b = index(from + span * (i + 1) as f64)
                .max(a + 1)
                .min(peaks.len());
            peaks[a.min(b)..b].iter().copied().fold(0.0, f32::max)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_sit_on_keyframes() {
        assert_eq!(thumb_times(9.0), [0.0, 2.0, 4.0, 6.0, 8.0]);
        assert_eq!(thumb_times(1.0), [0.0]);
    }

    #[test]
    fn a_long_recording_takes_every_few_keyframes() {
        let times = thumb_times(3600.0);
        assert!(times.len() <= MAX_THUMBS);
        let step = times[1] - times[0];
        assert!(step >= KEY_EVERY && step % KEY_EVERY == 0.0);
    }

    #[test]
    fn peaks_keep_the_loudest_sample_of_any_channel() {
        let mut peaks = Peaks::new(100, 2);
        peaks.push(&[0.1, -0.5, 0.2, 0.0]);
        peaks.push(&[0.3, 0.0]);
        assert_eq!(peaks.finish(), [0.5, 0.3]);
    }

    #[test]
    fn bars_pick_the_loudest_peak_under_them() {
        let peaks: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let bars = bars(&peaks, 0.0, 2.0, 4);
        assert_eq!(bars, [0.24, 0.49, 0.74, 0.99]);
    }

    #[test]
    fn bars_past_the_sound_are_silent() {
        let bars = bars(&[0.5; 10], 0.0, 1.0, 2);
        assert_eq!(bars, [0.5, 0.0]);
    }
}
