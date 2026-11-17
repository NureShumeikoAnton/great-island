//! Sound sped up with its pitch kept (WSOLA). The input is cut into overlapping windows taken
//! `speed` times further apart than they are laid out; each window is shifted a little to where
//! it continues the previous one best, so voices don't warble. Streaming: a fragment's sound
//! goes through in pieces, memory stays at a fraction of a second.
//!
//! Samples are interleaved `f32`, any number of channels; the windows are chosen on their mix
//! and applied to every channel, so channels stay in step.

/// 40 ms windows suit speech; music would prefer longer.
const FRAME_SECONDS: f64 = 0.04;
const SEEK_SECONDS: f64 = 0.012;
/// The search looks at every this-many shifts first, then around the best one.
const COARSE: usize = 4;

pub struct Stretch {
    channels: usize,
    speed: f64,
    frame: usize,
    hop: usize,
    seek: usize,
    window: Vec<f32>,
    /// Interleaved, beginning at the absolute frame `base`.
    input: Vec<f32>,
    base: usize,
    /// The next window's place before the search (absolute frames).
    nominal: f64,
    /// Where the previous window continues by itself.
    natural: Option<usize>,
    /// The previous window's second half, waiting for the next one to overlap it.
    tail: Vec<f32>,
}

impl Stretch {
    pub fn new(sample_rate: u32, channels: usize, speed: f64) -> Self {
        let frame = ((f64::from(sample_rate) * FRAME_SECONDS) as usize).max(64) & !1;
        let hop = frame / 2;
        // A periodic Hann window: halves overlapping by `hop` add up to exactly one.
        let window = (0..frame)
            .map(|n| (0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / frame as f64).cos()) as f32)
            .collect();
        Self {
            channels: channels.max(1),
            speed,
            frame,
            hop,
            seek: (f64::from(sample_rate) * SEEK_SECONDS) as usize,
            window,
            input: Vec::new(),
            base: 0,
            nominal: 0.0,
            natural: None,
            tail: vec![0.0; hop * channels.max(1)],
        }
    }

    fn frames(&self) -> usize {
        self.input.len() / self.channels
    }

    pub fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        if self.speed == 1.0 {
            out.extend_from_slice(samples);
            return;
        }
        self.input.extend_from_slice(samples);
        loop {
            let nominal = self.nominal.round() as usize;
            if nominal + self.seek + self.frame > self.base + self.frames() {
                break;
            }
            let start = match self.natural {
                Some(natural) => self.best(nominal, natural),
                None => nominal,
            };
            self.add(start, out);
            self.natural = Some(start + self.hop);
            self.nominal += self.hop as f64 * self.speed;
            let keep = (self.nominal.round() as usize)
                .saturating_sub(self.seek)
                .min(start + self.hop);
            if keep > self.base {
                self.input.drain(..(keep - self.base) * self.channels);
                self.base = keep;
            }
        }
    }

    /// The last window's fading half. The input's last fraction of a window is dropped.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if self.speed != 1.0 {
            out.append(&mut self.tail);
        }
    }

    fn mono(&self, frame: usize) -> f32 {
        let at = (frame - self.base) * self.channels;
        self.input[at..at + self.channels].iter().sum::<f32>()
    }

    fn fit(&self, candidate: usize, natural: usize) -> f32 {
        let (mut dot, mut energy) = (0.0f32, 1e-9f32);
        for k in 0..self.hop {
            let c = self.mono(candidate + k);
            dot += c * self.mono(natural + k);
            energy += c * c;
        }
        dot / energy.sqrt()
    }

    fn best(&self, nominal: usize, natural: usize) -> usize {
        let low = nominal.saturating_sub(self.seek).max(self.base);
        let high = nominal + self.seek;
        let pick = |range: &mut dyn Iterator<Item = usize>, best: usize| {
            range.fold((best, self.fit(best, natural)), |(b, score), c| {
                let s = self.fit(c, natural);
                if s > score { (c, s) } else { (b, score) }
            })
        };
        let (coarse, _) = pick(&mut (low..=high).step_by(COARSE), low);
        let from = coarse.saturating_sub(COARSE - 1).max(low);
        let to = (coarse + COARSE - 1).min(high);
        pick(&mut (from..=to), coarse).0
    }

    fn add(&mut self, start: usize, out: &mut Vec<f32>) {
        let ch = self.channels;
        let at = (start - self.base) * ch;
        for k in 0..self.frame {
            for c in 0..ch {
                let s = self.input[at + k * ch + c] * self.window[k];
                if k < self.hop {
                    out.push(self.tail[k * ch + c] + s);
                } else {
                    self.tail[(k - self.hop) * ch + c] = s;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(seconds: f64, hz: f64) -> Vec<f32> {
        (0..(seconds * f64::from(RATE)) as usize)
            .map(|n| (std::f64::consts::TAU * hz * n as f64 / f64::from(RATE)).sin() as f32 * 0.5)
            .collect()
    }

    fn run(input: &[f32], channels: usize, speed: f64) -> Vec<f32> {
        let mut stretch = Stretch::new(RATE, channels, speed);
        let mut out = Vec::new();
        for piece in input.chunks(4096 * channels) {
            stretch.push(piece, &mut out);
        }
        stretch.finish(&mut out);
        out
    }

    /// Crossings of zero per second in the middle half, where the edges' fades don't reach.
    fn crossings_per_second(samples: &[f32]) -> f64 {
        let middle = &samples[samples.len() / 4..samples.len() * 3 / 4];
        let crossings = middle
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        crossings as f64 / (middle.len() as f64 / f64::from(RATE))
    }

    #[test]
    fn normal_speed_passes_the_sound_untouched() {
        let input = sine(0.5, 440.0);
        assert_eq!(run(&input, 1, 1.0), input);
    }

    #[test]
    fn twice_the_speed_halves_the_length_and_keeps_the_pitch() {
        let input = sine(2.0, 440.0);
        let out = run(&input, 1, 2.0);
        let seconds = out.len() as f64 / f64::from(RATE);
        assert!((seconds - 1.0).abs() < 0.05, "{seconds}");
        let hz = crossings_per_second(&out) / 2.0;
        assert!((hz - 440.0).abs() < 440.0 * 0.03, "{hz}");
    }

    #[test]
    fn eight_times_still_keeps_the_pitch() {
        let input = sine(4.0, 300.0);
        let out = run(&input, 1, 8.0);
        let seconds = out.len() as f64 / f64::from(RATE);
        assert!((seconds - 0.5).abs() < 0.05, "{seconds}");
        let hz = crossings_per_second(&out) / 2.0;
        assert!((hz - 300.0).abs() < 300.0 * 0.05, "{hz}");
    }

    #[test]
    fn channels_stay_apart() {
        let left = sine(1.0, 440.0);
        let stereo: Vec<f32> = left.iter().flat_map(|&l| [l, 0.0]).collect();
        let out = run(&stereo, 2, 1.5);
        assert!(out.chunks(2).all(|frame| frame[1] == 0.0));
        assert!(out.chunks(2).any(|frame| frame[0].abs() > 0.1));
    }
}
