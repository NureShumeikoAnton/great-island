//! The pre-start panel: what a press changes in the [`Choice`], which page is open and
//! whether an area is picked.

use crate::capture::Area;

use super::{
    choice::{Balance, Choice, Codec, Quality, Rate},
    sound::Sound,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Microphones,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press {
    Start,
    /// The area circle: picks an area, or drops the picked one.
    Area,
    Countdown,
    More,
    Sound(Sound),
    Microphones,
    Back,
    /// A row of the microphones page: 0 is "as in Windows", then the devices.
    Microphone(usize),
    Quality(Quality),
    Codec(Codec),
    Rate(Rate),
    Fade,
    /// The balance slider: it is dragged, see [`slide`].
    Balance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub start: bool,
    /// Open the area selection.
    pub select: bool,
    pub save: bool,
    /// The microphone to listen to changed, or listening should start or stop.
    pub listen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panel {
    pub page: Page,
    /// The rarely changed choices are unfolded.
    pub more: bool,
    /// For this recording only, so not in the [`Choice`]; `None` — the whole screen.
    pub area: Option<Area>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            page: Page::Main,
            more: false,
            area: None,
        }
    }
}

impl Panel {
    pub fn press(&mut self, press: &Press, choice: &mut Choice, devices: &[Microphone]) -> Outcome {
        let before = choice.clone();
        let mut outcome = Outcome::default();
        match *press {
            Press::Start => outcome.start = true,
            Press::Area => outcome.select = self.area.take().is_none(),
            Press::Countdown => choice.countdown = choice.countdown.next(),
            Press::More => self.more = !self.more,
            Press::Sound(sound) => choice.sound = sound,
            Press::Microphones => {
                self.page = Page::Microphones;
                outcome.listen = true;
            }
            Press::Back => {
                self.page = Page::Main;
                outcome.listen = true;
            }
            Press::Microphone(row) => {
                if let Some(id) = row_device(row, devices) {
                    choice.microphone = id;
                }
                outcome.listen = true;
            }
            Press::Quality(quality) => choice.quality = quality,
            Press::Codec(codec) => choice.codec = codec,
            Press::Rate(rate) => choice.rate = rate,
            Press::Fade => choice.fade = !choice.fade,
            Press::Balance => {}
        }
        outcome.save = *choice != before;
        outcome
    }
}

/// The balance slider moved to `fraction` (0 — the voice end). `true` — the choice changed.
pub fn slide(choice: &mut Choice, fraction: f32) -> bool {
    let balance = Balance::at(fraction);
    std::mem::replace(&mut choice.balance, balance) != balance
}

/// `None` — no such row. `Some(None)` — the Windows default.
fn row_device(row: usize, devices: &[Microphone]) -> Option<Option<String>> {
    match row {
        0 => Some(None),
        n => devices.get(n - 1).map(|d| Some(d.id.clone())),
    }
}

/// The checked row. A chosen microphone that isn't plugged in is replaced by the Windows
/// default when recording, so the default is what is checked.
pub fn selected(choice: &Choice, devices: &[Microphone]) -> usize {
    choice
        .microphone
        .as_ref()
        .and_then(|id| devices.iter().position(|d| &d.id == id))
        .map_or(0, |i| i + 1)
}

/// The microphone the recording would open: the chosen one if it is there, otherwise the
/// Windows default (`None`).
pub fn heard(choice: &Choice, devices: &[Microphone]) -> Option<String> {
    match selected(choice, devices) {
        0 => None,
        n => Some(devices[n - 1].id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> Vec<Microphone> {
        ["a", "b"]
            .into_iter()
            .map(|id| Microphone {
                id: id.into(),
                name: id.to_uppercase(),
            })
            .collect()
    }

    #[test]
    fn a_press_changes_the_choice_and_says_to_save() {
        let mut panel = Panel::default();
        let mut choice = Choice::default();
        let out = panel.press(&Press::Sound(Sound::Both), &mut choice, &[]);
        assert_eq!(choice.sound, Sound::Both);
        assert!(out.save && !out.start);
        let again = panel.press(&Press::Sound(Sound::Both), &mut choice, &[]);
        assert!(!again.save, "the same choice is not saved again");
        panel.press(&Press::Countdown, &mut choice, &[]);
        assert_eq!(choice.countdown.seconds(), 3);
        panel.press(&Press::Fade, &mut choice, &[]);
        assert!(!choice.fade);
        panel.press(&Press::Rate(Rate::Sixty), &mut choice, &[]);
        assert_eq!(choice.rate, Rate::Sixty);
    }

    #[test]
    fn unfolding_and_pages_change_only_the_panel() {
        let mut panel = Panel::default();
        let mut choice = Choice::default();
        let out = panel.press(&Press::More, &mut choice, &[]);
        assert!(panel.more && !out.save);
        let out = panel.press(&Press::Microphones, &mut choice, &[]);
        assert_eq!(panel.page, Page::Microphones);
        assert!(out.listen && !out.save);
        panel.press(&Press::Back, &mut choice, &[]);
        assert_eq!(panel.page, Page::Main);
        assert!(panel.press(&Press::Start, &mut choice, &[]).start);
    }

    #[test]
    fn the_area_circle_picks_or_drops_the_area() {
        let mut panel = Panel::default();
        let mut choice = Choice::default();
        let out = panel.press(&Press::Area, &mut choice, &[]);
        assert!(out.select && !out.save);
        panel.area = Some(Area {
            x: 10,
            y: 20,
            width: 300,
            height: 200,
        });
        let out = panel.press(&Press::Area, &mut choice, &[]);
        assert!(!out.select, "a picked area is dropped, not picked again");
        assert_eq!(panel.area, None);
    }

    #[test]
    fn the_slider_moves_the_balance_by_steps() {
        let mut choice = Choice::default();
        assert!(!slide(&mut choice, 0.51), "the middle step");
        assert!(slide(&mut choice, 0.0));
        assert_eq!(choice.balance.gains(), (0.0, 1.0));
        assert!(!slide(&mut choice, 0.01));
    }

    #[test]
    fn a_microphone_is_chosen_by_its_row() {
        let mut panel = Panel::default();
        let mut choice = Choice::default();
        let list = devices();
        let out = panel.press(&Press::Microphone(2), &mut choice, &list);
        assert_eq!(choice.microphone.as_deref(), Some("b"));
        assert!(out.save && out.listen);
        assert_eq!(selected(&choice, &list), 2);
        assert_eq!(heard(&choice, &list).as_deref(), Some("b"));
        panel.press(&Press::Microphone(9), &mut choice, &list);
        assert_eq!(choice.microphone.as_deref(), Some("b"), "no such row");
        panel.press(&Press::Microphone(0), &mut choice, &list);
        assert_eq!(choice.microphone, None);
    }

    #[test]
    fn an_unplugged_choice_listens_to_the_default() {
        let choice = Choice {
            microphone: Some("gone".into()),
            ..Choice::default()
        };
        assert_eq!(selected(&choice, &devices()), 0);
        assert_eq!(heard(&choice, &devices()), None);
    }
}
