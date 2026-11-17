//! Short sounds: sine waves with overtones and decay, synthesized in memory and played by the
//! system PlaySound. No files or audio libraries.

use std::{f32::consts::TAU, sync::Mutex};
use windows::{
    Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT},
    core::PCWSTR,
};

const SAMPLE_RATE: u32 = 44_100;
const MASTER_GAIN: f32 = 0.32;
const END_GAIN: f32 = 0.6;
/// "Silence" for exponential envelopes: an exponential never reaches zero.
const SILENCE: f32 = 0.0001;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Start,
    End,
}

struct Voice {
    /// Notes: frequency in Hz and offset from the start in seconds.
    start: &'static [(f32, f32)],
    end: &'static [(f32, f32)],
    decay: f32,
    attack: f32,
    /// Overtones: frequency multiplier and relative volume.
    partials: &'static [(f32, f32)],
    /// Short downward pitch slide: the "drop".
    glide: Option<f32>,
}

const BELL: Voice = Voice {
    start: &[(659.25, 0.0), (987.77, 0.13)],
    end: &[(987.77, 0.0), (659.25, 0.15)],
    decay: 1.4,
    attack: 0.006,
    partials: &[(1.0, 1.0), (2.0, 0.22), (3.01, 0.08)],
    glide: None,
};

const SOFT: Voice = Voice {
    start: &[(523.25, 0.0), (659.25, 0.09), (783.99, 0.18)],
    end: &[(783.99, 0.0), (523.25, 0.16)],
    decay: 1.1,
    attack: 0.03,
    partials: &[(1.0, 1.0), (2.0, 0.1)],
    glide: None,
};

const DROP: Voice = Voice {
    start: &[(1046.5, 0.0), (1318.5, 0.12)],
    end: &[(1318.5, 0.0), (880.0, 0.14)],
    decay: 0.35,
    attack: 0.004,
    partials: &[(1.0, 1.0)],
    glide: Some(1.6),
};

fn voice(tone: &str) -> &'static Voice {
    match tone {
        "soft" => &SOFT,
        "drop" => &DROP,
        _ => &BELL,
    }
}

/// The buffer that is playing now: PlaySound reads it asynchronously.
static PLAYING: Mutex<Option<Vec<u8>>> = Mutex::new(None);

pub fn play(tone: &str, kind: Kind, volume: f32) {
    if volume <= 0.0 {
        return;
    }
    let gain = volume.min(1.0) * MASTER_GAIN * if kind == Kind::End { END_GAIN } else { 1.0 };
    let wav = to_wav(&render(voice(tone), kind, gain));

    let mut playing = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // A new call stops the previous sound, so the old buffer is freed only after it.
        let _ = PlaySoundW(
            PCWSTR(wav.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
    *playing = Some(wav);
}

fn render(voice: &Voice, kind: Kind, gain: f32) -> Vec<f32> {
    let notes = if kind == Kind::Start {
        voice.start
    } else {
        voice.end
    };
    let length = notes
        .iter()
        .map(|&(_, at)| at + voice.decay)
        .fold(0.0, f32::max)
        + 0.05;
    let mut out = vec![0.0f32; (length * SAMPLE_RATE as f32) as usize];

    for &(freq, at) in notes {
        let first = (at * SAMPLE_RATE as f32) as usize;
        for &(ratio, partial_gain) in voice.partials {
            let mut phase = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate().skip(first) {
                let t = (i - first) as f32 / SAMPLE_RATE as f32;
                if t > voice.decay {
                    break;
                }
                let f = freq * ratio * glide_factor(voice.glide, t);
                phase = (phase + TAU * f / SAMPLE_RATE as f32) % TAU;
                *sample +=
                    phase.sin() * envelope(t, voice.attack, voice.decay, partial_gain) * gain;
            }
        }
    }
    out
}

/// Pitch slides from `glide`×f to f over 60 ms, exponentially.
fn glide_factor(glide: Option<f32>, t: f32) -> f32 {
    const GLIDE_TIME: f32 = 0.06;
    match glide {
        Some(g) if t < GLIDE_TIME => g.powf(1.0 - t / GLIDE_TIME),
        _ => 1.0,
    }
}

/// Fast attack and exponential decay, like exponentialRampToValueAtTime in Web Audio.
fn envelope(t: f32, attack: f32, decay: f32, peak: f32) -> f32 {
    let ramp = |from: f32, to: f32, k: f32| from * (to / from).powf(k);
    if t < attack {
        ramp(SILENCE, peak, t / attack)
    } else {
        ramp(peak, SILENCE, ((t - attack) / (decay - attack)).min(1.0))
    }
}

fn to_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    let mut put = |bytes: &[u8]| wav.extend_from_slice(bytes);
    put(b"RIFF");
    put(&(36 + data_len).to_le_bytes());
    put(b"WAVEfmt ");
    put(&16u32.to_le_bytes());
    put(&1u16.to_le_bytes()); // PCM
    put(&1u16.to_le_bytes()); // mono
    put(&SAMPLE_RATE.to_le_bytes());
    put(&(SAMPLE_RATE * 2).to_le_bytes());
    put(&2u16.to_le_bytes());
    put(&16u16.to_le_bytes());
    put(b"data");
    put(&data_len.to_le_bytes());
    for s in samples {
        put(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    wav
}
