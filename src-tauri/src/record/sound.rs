//! Sound in a recording: what is recorded, and how the packets of each input become one
//! track in recorded time.
//!
//! Inputs hand over packets stamped with the moment their first frame was heard. A [`Lane`]
//! lays one input on the timeline: a gap becomes silence, a late overlap is dropped. The
//! [`Track`] adds the lanes up and cuts the pauses out, so it runs in the same time as the
//! video. Everything is 48 kHz, stereo, 16 bit, interleaved.

use super::choice::Balance;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sound {
    System,
    Microphone,
    Both,
    /// Also what an unknown value becomes: one unknown word must not reset all the settings.
    #[default]
    #[serde(other)]
    Off,
}

impl Sound {
    pub fn system(self) -> bool {
        matches!(self, Self::System | Self::Both)
    }

    pub fn microphone(self) -> bool {
        matches!(self, Self::Microphone | Self::Both)
    }

    pub fn without_microphone(self) -> Self {
        match self {
            Self::Both => Self::System,
            Self::Microphone => Self::Off,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    System,
    Microphone,
}

impl Input {
    fn lane(self) -> usize {
        match self {
            Self::System => 0,
            Self::Microphone => 1,
        }
    }
}

/// Frames in `ticks` of 100 ns.
pub fn frames(ticks: u64) -> u64 {
    ticks * u64::from(RATE) / 10_000_000
}

/// Stamps jitter by a few milliseconds. Past this the input really jumped (lost data, a
/// drifting clock caught up) and the packet goes where its stamp says.
const SLACK: u64 = RATE as u64 / 50;

/// One input on the timeline, from the first frame not yet taken.
#[derive(Default)]
pub struct Lane {
    start: u64,
    samples: VecDeque<i16>,
}

impl Lane {
    fn end(&self) -> u64 {
        self.start + (self.samples.len() / CHANNELS) as u64
    }

    /// A packet whose first frame was heard at frame `at` (negative: before the start).
    pub fn push(&mut self, at: i64, samples: &[i16]) {
        let end = self.end() as i64;
        let at = if at.abs_diff(end) <= SLACK { end } else { at };
        if at > end {
            let gap = (at - end) as usize * CHANNELS;
            self.samples.extend(std::iter::repeat_n(0, gap));
        }
        let late = (end - at).max(0) as usize * CHANNELS;
        self.samples.extend(samples.iter().skip(late));
    }

    /// Adds the next `out.len() / CHANNELS` frames into `out`, at `gain` of the frame's
    /// index; what is not there is silence.
    fn take_into(&mut self, out: &mut [i32], gain: impl Fn(usize) -> f32) {
        for (i, sample) in out.iter_mut().enumerate() {
            let s = self.samples.pop_front().unwrap_or(0);
            let gain = gain(i / CHANNELS);
            *sample += if gain < 1.0 {
                (f32::from(s) * gain) as i32
            } else {
                i32::from(s)
            };
        }
        self.start += (out.len() / CHANNELS) as u64;
    }
}

/// A short fade on both sides of a pause or a muted stretch, so the splice doesn't click.
const RAMP: u64 = RATE as u64 / 200;

/// A pause or a muted stretch, in frames of the timeline: `to` is unknown while it lasts.
#[derive(Clone, Copy, Debug)]
struct Cut {
    from: u64,
    to: Option<u64>,
}

fn open(cuts: &mut VecDeque<Cut>, at: u64) {
    if cuts.back().is_none_or(|c| c.to.is_some()) {
        cuts.push_back(Cut { from: at, to: None });
    }
}

fn close(cuts: &mut VecDeque<Cut>, at: u64) {
    if let Some(cut) = cuts.back_mut().filter(|c| c.to.is_none()) {
        cut.to = Some(at.max(cut.from));
    }
}

/// `None`: the frame is inside a cut; near one it fades.
fn fade(cuts: &VecDeque<Cut>, at: u64) -> Option<f32> {
    let mut gain = 1.0f32;
    for cut in cuts {
        if at >= cut.from && cut.to.is_none_or(|to| at < to) {
            return None;
        }
        if at < cut.from {
            gain = gain.min((cut.from - at) as f32 / RAMP as f32);
        }
        if let Some(to) = cut.to.filter(|&to| at >= to) {
            gain = gain.min((at - to + 1) as f32 / RAMP as f32);
        }
    }
    Some(gain.min(1.0))
}

/// Cuts whose fade-in is wholly before `cursor` no longer change anything.
fn forget(cuts: &mut VecDeque<Cut>, cursor: u64) {
    while cuts
        .front()
        .and_then(|c| c.to)
        .is_some_and(|to| to + RAMP <= cursor)
    {
        cuts.pop_front();
    }
}

pub struct Track {
    lanes: [Lane; 2],
    mixing: bool,
    /// Of each lane, by `Input::lane`; the balance counts only when both are recorded.
    gains: [f32; 2],
    /// Frames of the timeline before this are already handed out.
    cursor: u64,
    /// Pauses: cut out of the track.
    cuts: VecDeque<Cut>,
    /// The microphone muted: silence in its place, the track's time runs on.
    muted: VecDeque<Cut>,
}

impl Track {
    pub fn new(sound: Sound, balance: Balance) -> Self {
        let mixing = sound == Sound::Both;
        let (system, microphone) = if mixing { balance.gains() } else { (1.0, 1.0) };
        Self {
            lanes: Default::default(),
            mixing,
            gains: [system, microphone],
            cursor: 0,
            cuts: VecDeque::new(),
            muted: VecDeque::new(),
        }
    }

    pub fn push(&mut self, input: Input, at: i64, samples: &[i16]) {
        self.lanes[input.lane()].push(at, samples);
    }

    pub fn pause(&mut self, at: u64) {
        open(&mut self.cuts, at);
    }

    pub fn resume(&mut self, at: u64) {
        close(&mut self.cuts, at);
    }

    pub fn mute(&mut self, at: u64) {
        open(&mut self.muted, at);
    }

    pub fn unmute(&mut self, at: u64) {
        close(&mut self.muted, at);
    }

    /// The track from where it stopped up to frame `until` of the timeline, pauses left out.
    pub fn take(&mut self, until: u64) -> Vec<i16> {
        let frames = until.saturating_sub(self.cursor) as usize;
        let mut mixed = vec![0i32; frames * CHANNELS];
        let cursor = self.cursor;
        let [system, microphone] = &mut self.lanes;
        let [system_gain, microphone_gain] = self.gains;
        system.take_into(&mut mixed, |_| system_gain);
        let muted = &self.muted;
        microphone.take_into(&mut mixed, |i| {
            fade(muted, cursor + i as u64).map_or(0.0, |f| f * microphone_gain)
        });
        let mut out = Vec::with_capacity(mixed.len());
        for (i, frame) in mixed.as_chunks::<CHANNELS>().0.iter().enumerate() {
            let at = self.cursor + i as u64;
            let Some(gain) = fade(&self.cuts, at) else {
                continue;
            };
            for &sample in frame {
                let sample = if self.mixing {
                    limit(sample)
                } else {
                    sample.clamp(i16::MIN.into(), i16::MAX.into()) as i16
                };
                out.push(if gain < 1.0 {
                    (f32::from(sample) * gain) as i16
                } else {
                    sample
                });
            }
        }
        self.cursor = self.cursor.max(until);
        forget(&mut self.cuts, self.cursor);
        forget(&mut self.muted, self.cursor);
        out
    }
}

/// Two loud inputs together go past full scale: the peaks are bent down smoothly instead of
/// being cut flat (which crackles). Below the knee nothing changes.
fn limit(sample: i32) -> i16 {
    const KNEE: f32 = 0.8;
    let x = sample as f32 / 32768.0;
    let size = x.abs();
    if size <= KNEE {
        return sample as i16;
    }
    let bent = KNEE + (1.0 - KNEE) * ((size - KNEE) / (1.0 - KNEE)).tanh();
    (bent.copysign(x) * 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, value: i16) -> Vec<i16> {
        vec![value; frames * CHANNELS]
    }

    #[test]
    fn sound_choices_name_their_inputs() {
        assert!(!Sound::Off.system() && !Sound::Off.microphone());
        assert!(Sound::System.system() && !Sound::System.microphone());
        assert!(!Sound::Microphone.system() && Sound::Microphone.microphone());
        assert!(Sound::Both.system() && Sound::Both.microphone());
        assert_eq!(Sound::Both.without_microphone(), Sound::System);
        assert_eq!(Sound::Microphone.without_microphone(), Sound::Off);
        assert_eq!(Sound::System.without_microphone(), Sound::System);
    }

    #[test]
    fn sound_loads_from_settings_words() {
        for (text, sound) in [
            ("off", Sound::Off),
            ("system", Sound::System),
            ("microphone", Sound::Microphone),
            ("both", Sound::Both),
            ("loud", Sound::Off),
        ] {
            let read: Sound = serde_json::from_str(&format!("\"{text}\"")).expect("reads");
            assert_eq!(read, sound, "{text}");
        }
    }

    #[test]
    fn ticks_become_frames() {
        assert_eq!(frames(10_000_000), 48_000);
        assert_eq!(frames(10_000), 48);
    }

    #[test]
    fn silence_where_nothing_was_heard() {
        let mut track = Track::new(Sound::System, Balance::default());
        assert_eq!(track.take(100), tone(100, 0));
        track.push(Input::System, 2100, &tone(50, 1000));
        let out = track.take(2200);
        assert_eq!(&out[..2000 * CHANNELS], &tone(2000, 0)[..]);
        assert_eq!(&out[2000 * CHANNELS..2050 * CHANNELS], &tone(50, 1000)[..]);
        assert_eq!(&out[2050 * CHANNELS..], &tone(50, 0)[..]);
    }

    #[test]
    fn jitter_keeps_packets_back_to_back() {
        let mut lane = Lane::default();
        lane.push(0, &tone(480, 1));
        lane.push(480 + 100, &tone(480, 2));
        lane.push(960 - 100, &tone(480, 3));
        assert_eq!(lane.end(), 1440);
    }

    #[test]
    fn a_real_jump_leaves_a_gap_and_a_late_packet_is_cut() {
        let mut lane = Lane::default();
        lane.push(0, &tone(100, 1));
        lane.push(100 + 2 * SLACK as i64, &tone(100, 2));
        assert_eq!(lane.end(), 200 + 2 * SLACK);
        let mut late = Lane::default();
        late.push(5000, &tone(100, 1));
        late.push(5100 - 2 * SLACK as i64, &tone(3 * SLACK as usize, 2));
        assert_eq!(late.end(), 5100 + SLACK);
    }

    #[test]
    fn what_was_heard_before_the_start_is_dropped() {
        let mut track = Track::new(Sound::Microphone, Balance::default());
        track.push(Input::Microphone, -1000, &tone(1100, 7));
        let out = track.take(100);
        assert_eq!(out, tone(100, 7));
    }

    #[test]
    fn a_pause_is_cut_out_with_short_fades() {
        let mut track = Track::new(Sound::System, Balance::default());
        let total = 4 * RAMP as usize;
        track.push(Input::System, 0, &tone(total * 2, 10_000));
        track.pause(total as u64);
        track.resume(total as u64 + 500);
        let out = track.take((total * 2) as u64);
        assert_eq!(out.len(), (total * 2 - 500) * CHANNELS);
        assert_eq!(out[0], 10_000);
        let before = (total - 1) * CHANNELS;
        assert!(out[before] < 200, "fades out to the pause: {}", out[before]);
        let after = total * CHANNELS;
        assert!(out[after] < 200, "fades in after it: {}", out[after]);
        assert_eq!(*out.last().expect("samples"), 10_000);
    }

    #[test]
    fn nothing_comes_out_while_paused() {
        let mut track = Track::new(Sound::System, Balance::default());
        track.pause(0);
        track.push(Input::System, 0, &tone(1000, 5));
        assert!(track.take(1000).is_empty());
        track.resume(1000);
        track.push(Input::System, 1000, &tone(1000, 5));
        assert_eq!(track.take(2000).len(), 1000 * CHANNELS);
    }

    #[test]
    fn two_inputs_add_up_and_loud_peaks_bend() {
        let mut track = Track::new(Sound::Both, Balance::default());
        track.push(Input::System, 0, &tone(10, 1000));
        track.push(Input::Microphone, 0, &tone(10, 2000));
        assert_eq!(track.take(10), tone(10, 3000));
        track.push(Input::System, 10, &tone(10, 30_000));
        track.push(Input::Microphone, 10, &tone(10, 30_000));
        let loud = track.take(20)[0];
        assert!(loud > 30_000, "{loud}");
    }

    #[test]
    fn the_balance_quiets_one_input_only_in_a_mix() {
        let voice = Balance::at(0.25);
        let mut both = Track::new(Sound::Both, voice);
        both.push(Input::System, 0, &tone(10, 1000));
        both.push(Input::Microphone, 0, &tone(10, 2000));
        assert_eq!(both.take(10), tone(10, 2500));
        let mut alone = Track::new(Sound::System, voice);
        alone.push(Input::System, 0, &tone(10, 1000));
        assert_eq!(alone.take(10), tone(10, 1000));
    }

    #[test]
    fn a_muted_microphone_is_silence_and_the_computer_plays_on() {
        let mut track = Track::new(Sound::Both, Balance::default());
        let total = 8 * RAMP as usize;
        track.push(Input::System, 0, &tone(total, 1000));
        track.push(Input::Microphone, 0, &tone(total, 2000));
        let (from, to) = (2 * RAMP, 5 * RAMP);
        track.mute(from);
        track.unmute(to);
        let out = track.take(total as u64);
        assert_eq!(out.len(), total * CHANNELS, "muting cuts nothing out");
        assert_eq!(out[0], 3000);
        let inside = ((from + to) / 2) as usize * CHANNELS;
        assert_eq!(out[inside], 1000, "only the computer while muted");
        let before = (from - 1) as usize * CHANNELS;
        assert!(out[before] < 1100, "fades out to the mute: {}", out[before]);
        assert_eq!(*out.last().expect("samples"), 3000);
    }

    #[test]
    fn muted_to_the_end_stays_silent() {
        let mut track = Track::new(Sound::Microphone, Balance::default());
        track.push(Input::Microphone, 0, &tone(1000, 5000));
        track.mute(0);
        assert_eq!(track.take(1000), tone(1000, 0));
    }

    #[test]
    fn one_input_passes_untouched() {
        let mut track = Track::new(Sound::System, Balance::default());
        track.push(Input::System, 0, &tone(10, 32_000));
        assert_eq!(track.take(10), tone(10, 32_000));
    }

    #[test]
    fn the_knee_is_smooth_and_never_overflows() {
        assert_eq!(limit(1000), 1000);
        assert_eq!(limit(-20_000), -20_000);
        let mut last = limit(26_000);
        for x in (26_000..200_000).step_by(1000) {
            let y = limit(x);
            assert!(y >= last);
            last = y;
        }
        assert!(limit(-200_000) < -30_000);
    }
}
