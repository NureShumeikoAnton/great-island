//! Text on a picture, with no Windows in it: what a reader found ([`Page`]) and the port a
//! reader stands behind ([`Reader`]).
//!
//! A page is in the picture's own pixels. The scanner reads a frozen monitor; the gallery,
//! the search or another app can read any [`Image`] the same way. Which engine reads is the
//! adapter's business (`adapters/`), so a second engine plugs in without touching a caller.

pub mod adapters;
pub mod compose;
pub mod layout;
pub mod links;
pub mod qr;
pub mod script;
pub mod session;

use crate::{capture::Image, fault::Failed};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: Rect,
    /// Whether a space separates it from the previous word of its line (none in Chinese or
    /// Japanese, and none before the first word).
    pub spaced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub words: Vec<Word>,
}

impl Line {
    pub fn rect(&self) -> Rect {
        let mut words = self.words.iter().map(|w| w.rect);
        let first = words.next().unwrap_or_default();
        words.fold(first, |all, r| all.union(&r))
    }

    pub fn text(&self) -> String {
        let mut text = String::new();
        for word in &self.words {
            if word.spaced {
                text.push(' ');
            }
            text.push_str(&word.text);
        }
        text
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    /// In the reader's order: top to bottom, a column at a time where it sees columns.
    pub lines: Vec<Line>,
}

impl Page {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// A recognition language. `tag` is BCP-47 as Windows names it ("ru", "en-US").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    pub tag: String,
    /// In the language itself ("русский", "English"), for the list.
    pub name: String,
}

impl Language {
    /// The capsule's short label: the primary subtag in capitals ("RU", "EN").
    pub fn short(&self) -> String {
        self.tag
            .split('-')
            .next()
            .unwrap_or(&self.tag)
            .to_uppercase()
    }
}

pub trait Reader {
    /// The languages it can read now; the first is the first of Windows' languages it reads.
    fn languages(&self) -> Vec<Language>;
    fn read(&mut self, image: &Image, language: &str) -> Result<Page, Failed>;
}

/// The remembered language while it is still installed, otherwise the reader's first.
/// An empty `remembered` means none was chosen yet.
pub fn choose<'a>(installed: &'a [Language], remembered: &str) -> Option<&'a Language> {
    installed
        .iter()
        .find(|l| !remembered.is_empty() && l.tag.eq_ignore_ascii_case(remembered))
        .or(installed.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: f32, spaced: bool) -> Word {
        Word {
            text: text.into(),
            rect: Rect {
                x,
                y: 10.0,
                width: 30.0,
                height: 12.0,
            },
            spaced,
        }
    }

    #[test]
    fn a_line_joins_its_words_by_their_spaces() {
        let line = Line {
            words: vec![
                word("Идёт", 0.0, false),
                word("зарядка", 40.0, true),
                word(".", 70.0, false),
            ],
        };
        assert_eq!(line.text(), "Идёт зарядка.");
        assert_eq!(
            line.rect(),
            Rect {
                x: 0.0,
                y: 10.0,
                width: 100.0,
                height: 12.0
            }
        );
    }

    #[test]
    fn the_short_label_is_the_primary_subtag() {
        let en = Language {
            tag: "en-US".into(),
            name: "English".into(),
        };
        assert_eq!(en.short(), "EN");
    }

    #[test]
    fn the_remembered_language_wins_while_it_is_installed() {
        let installed = [
            Language {
                tag: "ru".into(),
                name: "Русский".into(),
            },
            Language {
                tag: "en-US".into(),
                name: "English".into(),
            },
        ];
        assert_eq!(choose(&installed, "en-us").unwrap().tag, "en-US");
        assert_eq!(choose(&installed, "").unwrap().tag, "ru");
        assert_eq!(
            choose(&installed, "de-DE").unwrap().tag,
            "ru",
            "removed since"
        );
        assert!(choose(&[], "ru").is_none());
    }
}
