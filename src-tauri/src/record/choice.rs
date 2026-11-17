//! What the next recording will be like: the choices of the pre-start panel. They are kept in
//! the settings file; only the island changes them.

use super::sound::Sound;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Medium,
    Low,
    /// Also what an unknown value becomes (serde wants that variant last).
    #[default]
    #[serde(other)]
    High,
}

impl Quality {
    /// Left to right in the panel: from the smallest file up.
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    /// How many times smaller the video is than the screen.
    pub fn shrink(self) -> f32 {
        match self {
            Self::High => 1.0,
            Self::Medium => 4.0 / 3.0,
            Self::Low => 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Codec {
    Hevc,
    #[default]
    #[serde(other)]
    H264,
}

impl Codec {
    pub const ALL: [Self; 2] = [Self::H264, Self::Hevc];
}

/// Frames per second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rate {
    #[serde(rename = "60")]
    Sixty,
    #[default]
    #[serde(rename = "30", other)]
    Thirty,
}

impl Rate {
    pub const ALL: [Self; 2] = [Self::Thirty, Self::Sixty];

    pub fn fps(self) -> u32 {
        match self {
            Self::Thirty => 30,
            Self::Sixty => 60,
        }
    }
}

/// Seconds before the recording starts; 0 = at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Countdown(u8);

impl Countdown {
    const STEPS: [u8; 3] = [0, 3, 5];

    pub fn seconds(self) -> u32 {
        u32::from(self.0)
    }

    pub fn next(self) -> Self {
        let at = Self::STEPS.iter().position(|&s| s == self.0).unwrap_or(0);
        Self(Self::STEPS[(at + 1) % Self::STEPS.len()])
    }
}

/// The first version kept only "countdown on": `true` was 3 seconds. A number that isn't a
/// step becomes the nearest one below it.
impl<'de> Deserialize<'de> for Countdown {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            On(bool),
            Seconds(u64),
        }
        let seconds = match Saved::deserialize(deserializer)? {
            Saved::On(on) => u64::from(on) * 3,
            Saved::Seconds(s) => s,
        };
        let step = Self::STEPS
            .iter()
            .rev()
            .find(|&&s| u64::from(s) <= seconds)
            .copied()
            .unwrap_or(0);
        Ok(Self(step))
    }
}

/// The names keep the flat keys of the settings file, from before the panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    #[serde(rename = "recordSound")]
    pub sound: Sound,
    #[serde(rename = "recordCountdown")]
    pub countdown: Countdown,
    /// Dissolve after a pause.
    #[serde(rename = "recordFade")]
    pub fade: bool,
    #[serde(rename = "recordQuality")]
    pub quality: Quality,
    #[serde(rename = "recordCodec")]
    pub codec: Codec,
    #[serde(rename = "recordRate")]
    pub rate: Rate,
    /// A Windows endpoint id; `None` — whichever microphone Windows has as default.
    #[serde(rename = "recordMicrophone")]
    pub microphone: Option<String>,
    #[serde(rename = "recordBalance")]
    pub balance: Balance,
}

/// The microphone against the computer's sound when both are recorded, in tenths:
/// -10 — only the voice, 0 — as they are, 10 — only the computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Balance(i8);

impl Balance {
    const STEPS: i8 = 10;

    /// From a slider position, 0 (the voice end) to 1 (the computer end).
    pub fn at(fraction: f32) -> Self {
        let steps = f32::from(Self::STEPS);
        Self(((fraction.clamp(0.0, 1.0) * 2.0 - 1.0) * steps).round() as i8)
    }

    pub fn fraction(self) -> f32 {
        (f32::from(self.0) / f32::from(Self::STEPS) + 1.0) / 2.0
    }

    /// How loud the computer and the microphone go into the mix: the side the slider
    /// moves to stays whole, the other one gets quieter.
    pub fn gains(self) -> (f32, f32) {
        let k = f32::from(self.0) / f32::from(Self::STEPS);
        if k < 0.0 {
            (1.0 + k, 1.0)
        } else {
            (1.0, 1.0 - k)
        }
    }
}

/// Anything out of range is pulled back to it; a value that isn't a number is the middle.
impl<'de> Deserialize<'de> for Balance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer).unwrap_or(0.0);
        let steps = f64::from(Self::STEPS);
        Ok(Self(value.round().clamp(-steps, steps) as i8))
    }
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            sound: Sound::Off,
            countdown: Countdown::default(),
            fade: true,
            quality: Quality::default(),
            codec: Codec::default(),
            rate: Rate::default(),
            microphone: None,
            balance: Balance::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &str) -> Choice {
        serde_json::from_str(json).expect("reads")
    }

    #[test]
    fn countdown_steps_round() {
        let mut c = Countdown::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(c.seconds());
            c = c.next();
        }
        assert_eq!(seen, [0, 3, 5, 0]);
    }

    #[test]
    fn the_old_countdown_switch_still_reads() {
        assert_eq!(read(r#"{"recordCountdown":true}"#).countdown.seconds(), 3);
        assert_eq!(read(r#"{"recordCountdown":false}"#).countdown.seconds(), 0);
        assert_eq!(read(r#"{"recordCountdown":5}"#).countdown.seconds(), 5);
        assert_eq!(read(r#"{"recordCountdown":4}"#).countdown.seconds(), 3);
        assert_eq!(read(r#"{"recordCountdown":99}"#).countdown.seconds(), 5);
    }

    #[test]
    fn choices_go_and_come_back_by_their_names() {
        let choice = Choice {
            sound: Sound::Both,
            countdown: Countdown::default().next().next(),
            fade: false,
            quality: Quality::Low,
            codec: Codec::Hevc,
            rate: Rate::Sixty,
            microphone: Some("{0.0.1.00000000}.{abc}".into()),
            balance: Balance::at(0.25),
        };
        let json = serde_json::to_string(&choice).expect("writes");
        assert!(json.contains(r#""recordCodec":"hevc""#), "{json}");
        assert!(json.contains(r#""recordRate":"60""#), "{json}");
        assert!(json.contains(r#""recordCountdown":5"#), "{json}");
        assert_eq!(read(&json), choice);
    }

    #[test]
    fn unknown_words_fall_back_one_by_one() {
        let choice = read(
            r#"{"recordQuality":"ultra","recordCodec":"av1","recordRate":"120","recordFade":false}"#,
        );
        assert_eq!(choice.quality, Quality::High);
        assert_eq!(choice.codec, Codec::H264);
        assert_eq!(choice.rate, Rate::Thirty);
        assert!(!choice.fade);
    }

    #[test]
    fn balance_quiets_the_other_side() {
        assert_eq!(Balance::default().gains(), (1.0, 1.0));
        assert_eq!(Balance::at(0.5), Balance::default());
        assert_eq!(Balance::at(0.0).gains(), (0.0, 1.0));
        assert_eq!(Balance::at(1.0).gains(), (1.0, 0.0));
        let voice = Balance::at(0.25);
        assert_eq!(voice.gains(), (0.5, 1.0));
        assert_eq!(voice.fraction(), 0.25);
        assert_eq!(read(r#"{"recordBalance":40}"#).balance, Balance::at(1.0));
        assert_eq!(read(r#"{"recordBalance":-3}"#).balance.gains().0, 0.7);
    }

    #[test]
    fn quality_divides_the_screen() {
        assert_eq!(Quality::High.shrink(), 1.0);
        assert_eq!((2560.0 / Quality::Medium.shrink()).round(), 1920.0);
        assert_eq!(Quality::Low.shrink(), 2.0);
        assert_eq!(Rate::Sixty.fps(), 60);
    }
}
