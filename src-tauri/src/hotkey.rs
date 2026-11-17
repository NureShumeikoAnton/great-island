//! Global keys from the settings (screenshot, recording). A key is stored by physical place (the
//! `KeyboardEvent.code` from the settings window: "KeyS", "PrintScreen"), not by letter, so it doesn't
//! depend on the layout. [`Slot`] keeps one such key taken from Windows in step with the settings.

use crate::lang::tr;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotkey {
    pub code: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub win: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Letters, digits and symbols: scan code of the physical key position. Windows derives the
    /// virtual-key code from it via the layout (on French, A sits where Q is).
    Place(u16),
    /// Keys outside the layout (F1, PrtSc, arrows): the Windows virtual-key code directly.
    Virtual(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    Unknown,
    /// A letter or F-key with only Ctrl, Alt or Shift: such combinations already exist in programs
    /// (Ctrl+S, Alt+F4), and a global hotkey would take them away everywhere.
    TooPlain,
}

const QWERTY: [(&str, u16); 3] = [("QWERTYUIOP", 0x10), ("ASDFGHJKL", 0x1E), ("ZXCVBNM", 0x2C)];

/// (code, label, scan code)
const MARKS: [(&str, &str, u16); 11] = [
    ("Minus", "-", 0x0C),
    ("Equal", "=", 0x0D),
    ("BracketLeft", "[", 0x1A),
    ("BracketRight", "]", 0x1B),
    ("Semicolon", ";", 0x27),
    ("Quote", "'", 0x28),
    ("Backquote", "`", 0x29),
    ("Backslash", "\\", 0x2B),
    ("Comma", ",", 0x33),
    ("Period", ".", 0x34),
    ("Slash", "/", 0x35),
];

/// (code, label, Windows virtual-key code)
const SPECIAL: [(&str, &str, u16); 15] = [
    ("PrintScreen", "PrtSc", 0x2C),
    ("Pause", "Pause", 0x13),
    ("ScrollLock", "ScrollLock", 0x91),
    ("Insert", "Insert", 0x2D),
    ("Delete", "Delete", 0x2E),
    ("Home", "Home", 0x24),
    ("End", "End", 0x23),
    ("PageUp", "PgUp", 0x21),
    ("PageDown", "PgDn", 0x22),
    ("ArrowLeft", "←", 0x25),
    ("ArrowUp", "↑", 0x26),
    ("ArrowRight", "→", 0x27),
    ("ArrowDown", "↓", 0x28),
    ("Space", "Space", 0x20),
    ("Enter", "Enter", 0x0D),
];

const STANDALONE: [&str; 3] = ["PrintScreen", "Pause", "ScrollLock"];

impl Hotkey {
    pub fn print_screen() -> Self {
        Self {
            code: "PrintScreen".into(),
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        }
    }

    /// Plain PrtSc, no Ctrl etc.: by default Windows Snipping Tool owns it.
    pub fn is_print_screen(&self) -> bool {
        *self == Self::print_screen()
    }

    pub fn key(&self) -> Option<Key> {
        let code = self.code.as_str();
        if let Some(letter) = code.strip_prefix("Key").filter(|l| l.len() == 1) {
            let c = letter.chars().next()?;
            return QWERTY
                .iter()
                .find_map(|(row, first)| row.find(c).map(|i| Key::Place(first + i as u16)));
        }
        if let Some(digit) = code.strip_prefix("Digit") {
            let n: u16 = digit.parse().ok().filter(|n| *n <= 9)?;
            // '1' = 0x02 … '9' = 0x0A, '0' comes after them.
            return Some(Key::Place(if n == 0 { 0x0B } else { n + 1 }));
        }
        if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
            return (1..=24).contains(&n).then_some(Key::Virtual(0x70 + n - 1));
        }
        MARKS
            .iter()
            .find(|m| m.0 == code)
            .map(|m| Key::Place(m.2))
            .or_else(|| {
                SPECIAL
                    .iter()
                    .find(|s| s.0 == code)
                    .map(|s| Key::Virtual(s.2))
            })
    }

    fn key_label(&self) -> String {
        let code = self.code.as_str();
        if let Some(rest) = code
            .strip_prefix("Key")
            .or_else(|| code.strip_prefix("Digit"))
        {
            return rest.to_string();
        }
        if code == "Space" {
            return tr!("key.space").into_owned();
        }
        MARKS
            .iter()
            .map(|m| (m.0, m.1))
            .chain(SPECIAL.iter().map(|s| (s.0, s.1)))
            .find(|(c, _)| *c == code)
            .map_or_else(|| code.to_string(), |(_, label)| label.to_string())
    }

    pub fn label(&self) -> String {
        let mut parts: Vec<String> = [
            (self.win, "Win"),
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name.to_string())
        .collect();
        parts.push(self.key_label());
        parts.join(" + ")
    }

    /// Whether the combination is valid. Whether another program holds it is a separate
    /// question: only Windows knows.
    pub fn check(&self) -> Option<Problem> {
        if self.key().is_none() {
            return Some(Problem::Unknown);
        }
        if STANDALONE.contains(&self.code.as_str()) || self.win {
            return None;
        }
        let held = [self.ctrl, self.alt, self.shift]
            .iter()
            .filter(|&&on| on)
            .count();
        (held < 2).then_some(Problem::TooPlain)
    }
}

/// One configurable global key: which one is held now, and which one Windows refused.
#[derive(Default)]
pub struct Slot {
    held: Option<Hotkey>,
    refused: Option<Hotkey>,
}

impl Slot {
    /// Brings the slot to `wanted`: `release` gives the held key back, `take` asks Windows for the
    /// new one (`false` = another program holds it). A refused key is asked for again on every call
    /// but returned only the first time: Windows re-sends its settings broadcast often, and the
    /// user is told once.
    pub fn sync(
        &mut self,
        wanted: Option<Hotkey>,
        take: impl FnOnce(&Hotkey) -> bool,
        release: impl FnOnce(),
    ) -> Option<Hotkey> {
        if wanted == self.held {
            return None;
        }
        if self.held.take().is_some() {
            release();
        }
        let Some(key) = wanted else {
            self.refused = None;
            return None;
        };
        if take(&key) {
            self.held = Some(key);
            self.refused = None;
            return None;
        }
        if self.refused.as_ref() == Some(&key) {
            return None;
        }
        self.refused = Some(key.clone());
        Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str, ctrl: bool, alt: bool, shift: bool) -> Hotkey {
        Hotkey {
            code: code.into(),
            ctrl,
            alt,
            shift,
            win: false,
        }
    }

    /// In the Russian layout the S key carries a different letter but is the same physical place, so the combination is the same.
    #[test]
    fn letters_are_places_not_characters() {
        assert_eq!(key("KeyS", true, false, true).key(), Some(Key::Place(0x1F)));
        assert_eq!(key("KeyQ", true, false, true).key(), Some(Key::Place(0x10)));
        assert_eq!(key("KeyM", true, false, true).key(), Some(Key::Place(0x32)));
        assert_eq!(
            key("Digit1", true, false, true).key(),
            Some(Key::Place(0x02))
        );
        assert_eq!(
            key("Digit0", true, false, true).key(),
            Some(Key::Place(0x0B))
        );
        assert_eq!(
            key("F13", false, false, false).key(),
            Some(Key::Virtual(0x7C))
        );
        assert_eq!(Hotkey::print_screen().key(), Some(Key::Virtual(0x2C)));
    }

    #[test]
    fn labels_read_like_a_keyboard() {
        assert_eq!(key("KeyS", true, false, true).label(), "Ctrl + Shift + S");
        assert_eq!(Hotkey::print_screen().label(), "PrtSc");
        assert_eq!(key("Slash", false, true, true).label(), "Alt + Shift + /");
    }

    /// What Windows saw: keys taken and released, in order.
    #[derive(Default)]
    struct System {
        taken_by_others: Vec<Hotkey>,
        log: Vec<String>,
    }

    impl System {
        fn sync(&mut self, slot: &mut Slot, wanted: Option<&Hotkey>) -> Option<Hotkey> {
            let log = std::cell::RefCell::new(Vec::new());
            let refused = slot.sync(
                wanted.cloned(),
                |key| {
                    let free = !self.taken_by_others.contains(key);
                    log.borrow_mut()
                        .push(format!("take {} {free}", key.label()));
                    free
                },
                || log.borrow_mut().push("release".to_string()),
            );
            self.log.append(&mut log.into_inner());
            refused
        }
    }

    #[test]
    fn slot_takes_the_key_once_and_swaps_it_on_change() {
        let (mut system, mut slot) = (System::default(), Slot::default());
        let (a, b) = (
            key("KeyA", true, true, false),
            key("KeyB", true, true, false),
        );
        assert_eq!(system.sync(&mut slot, Some(&a)), None);
        assert_eq!(system.sync(&mut slot, Some(&a)), None);
        assert_eq!(system.sync(&mut slot, Some(&b)), None);
        assert_eq!(system.sync(&mut slot, None), None);
        assert_eq!(system.sync(&mut slot, None), None);
        assert_eq!(
            system.log,
            [
                "take Ctrl + Alt + A true",
                "release",
                "take Ctrl + Alt + B true",
                "release"
            ]
        );
    }

    #[test]
    fn refused_key_is_told_once_and_asked_again() {
        let (mut system, mut slot) = (System::default(), Slot::default());
        let a = key("KeyA", true, true, false);
        system.taken_by_others.push(a.clone());
        assert_eq!(system.sync(&mut slot, Some(&a)), Some(a.clone()));
        assert_eq!(system.sync(&mut slot, Some(&a)), None);
        system.taken_by_others.clear();
        assert_eq!(system.sync(&mut slot, Some(&a)), None);
        assert_eq!(system.log.len(), 3, "asked every time");
        assert_eq!(system.log[2], "take Ctrl + Alt + A true");
    }

    #[test]
    fn refusal_is_told_again_after_another_key_was_held() {
        let (mut system, mut slot) = (System::default(), Slot::default());
        let (a, b) = (
            key("KeyA", true, true, false),
            key("KeyB", true, true, false),
        );
        system.taken_by_others.push(a.clone());
        assert_eq!(system.sync(&mut slot, Some(&a)), Some(a.clone()));
        assert_eq!(system.sync(&mut slot, Some(&b)), None);
        assert_eq!(system.sync(&mut slot, Some(&a)), Some(a.clone()));
    }

    #[test]
    fn plain_shortcuts_are_refused() {
        assert_eq!(
            key("KeyS", true, false, false).check(),
            Some(Problem::TooPlain)
        );
        assert_eq!(
            key("F4", false, true, false).check(),
            Some(Problem::TooPlain)
        );
        assert_eq!(key("KeyS", true, false, true).check(), None);
        assert_eq!(Hotkey::print_screen().check(), None);
        assert_eq!(key("PrintScreen", true, false, false).check(), None);
        assert_eq!(
            key("Numpad1", true, false, true).check(),
            Some(Problem::Unknown)
        );
    }
}
