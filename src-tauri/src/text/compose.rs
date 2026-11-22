//! Pieces of lines → the text that is copied.
//!
//! Lines are joined as they stand on the screen: a line beside the previous one (a reader
//! splits a row at an arrow or a wide gap) follows it after a space; a line below starts a
//! new line, or with [`Breaks::Join`] continues its paragraph after a space. A word hyphenated
//! across the break is joined back.

use crate::text::{Rect, layout::Layout};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Breaks {
    /// The lines of a paragraph in one line.
    Join,
    /// Every line on a line of its own. Also what an unknown value becomes: one unknown word
    /// must not reset all the settings.
    #[default]
    #[serde(other)]
    Keep,
}

impl Breaks {
    pub fn toggled(self) -> Self {
        match self {
            Self::Keep => Self::Join,
            Self::Join => Self::Keep,
        }
    }
}

pub fn compose(layout: &Layout, pieces: &[(usize, usize, usize)], breaks: Breaks) -> String {
    let mut text = String::new();
    let mut previous: Option<Rect> = None;
    for &(line, from, to) in pieces {
        let laid = &layout.lines[line];
        let part = laid.text(from, to);
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(prev) = previous {
            let rect = laid.rect;
            if beside(&prev, &rect) {
                text.push(' ');
            } else if breaks == Breaks::Join && same_paragraph(&prev, &rect) {
                if hyphenated(&text, part) {
                    text.pop();
                } else {
                    text.push(' ');
                }
            } else {
                text.push('\n');
            }
        }
        text.push_str(part);
        previous = Some(laid.rect);
    }
    text
}

/// One line for the island's preview.
pub fn preview(layout: &Layout, pieces: &[(usize, usize, usize)]) -> String {
    compose(layout, pieces, Breaks::Keep).replace('\n', " ")
}

/// On the same row, to the right and not far.
fn beside(prev: &Rect, next: &Rect) -> bool {
    let h = prev.height.min(next.height).max(1.0);
    let overlap = prev.bottom().min(next.bottom()) - prev.y.max(next.y);
    let gap = next.x - prev.right();
    overlap >= h * 0.5 && gap > -h * 0.5 && gap < h * 6.0
}

/// Right below, starting at the same left edge, of a similar size.
fn same_paragraph(prev: &Rect, next: &Rect) -> bool {
    let h = prev.height.max(next.height).max(1.0);
    let gap = next.y - prev.bottom();
    let ratio = prev.height.max(next.height) / prev.height.min(next.height).max(1.0);
    gap >= -h * 0.3 && gap <= h * 0.9 && (next.x - prev.x).abs() <= h && ratio <= 1.5
}

/// The text so far ends with a hyphen after a letter, and the next line starts in lowercase.
fn hyphenated(text: &str, next: &str) -> bool {
    let mut end = text.chars().rev();
    matches!(end.next(), Some('-' | '\u{2010}' | '\u{00AD}'))
        && end.next().is_some_and(char::is_alphabetic)
        && next.chars().next().is_some_and(char::is_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Line, Page, Word};

    fn line(text: &str, x: f32, y: f32) -> Line {
        Line {
            words: text
                .split(' ')
                .enumerate()
                .scan(x, |at, (i, w)| {
                    let width = w.chars().count() as f32 * 8.0;
                    let word = Word {
                        text: w.into(),
                        rect: Rect {
                            x: *at,
                            y,
                            width,
                            height: 14.0,
                        },
                        spaced: i > 0,
                    };
                    *at += width + 4.0;
                    Some(word)
                })
                .collect(),
        }
    }

    fn all(layout: &Layout) -> Vec<(usize, usize, usize)> {
        layout.pieces(layout.start(), layout.end())
    }

    fn text(lines: Vec<Line>, breaks: Breaks) -> String {
        let layout = Layout::new(&Page { lines });
        compose(&layout, &all(&layout), breaks)
    }

    #[test]
    fn lines_below_each_other_keep_their_breaks() {
        let lines = vec![
            line("Идёт зарядка.", 40.0, 45.0),
            line("Батарея", 40.0, 65.0),
        ];
        assert_eq!(text(lines, Breaks::Keep), "Идёт зарядка.\nБатарея");
    }

    #[test]
    fn a_row_split_by_the_reader_is_one_line() {
        let lines = vec![
            line("Откройте Settings", 43.0, 285.0),
            line("System", 188.0, 285.0),
            line("Display", 257.0, 285.0),
        ];
        assert_eq!(
            text(lines, Breaks::Keep),
            "Откройте Settings System Display"
        );
    }

    #[test]
    fn join_makes_a_paragraph_one_line_and_keeps_paragraphs_apart() {
        let lines = vec![
            line("Чтобы продлить срок", 40.0, 45.0),
            line("службы батареи.", 40.0, 65.0),
            line("Новый абзац", 40.0, 165.0),
        ];
        assert_eq!(
            text(lines, Breaks::Join),
            "Чтобы продлить срок службы батареи.\nНовый абзац"
        );
    }

    #[test]
    fn join_doesnt_take_a_line_of_another_column() {
        let lines = vec![line("Левая", 40.0, 45.0), line("Правая", 1000.0, 65.0)];
        assert_eq!(text(lines, Breaks::Join), "Левая\nПравая");
    }

    #[test]
    fn a_hyphenated_word_is_joined_back() {
        let lines = vec![
            line("Аккуму-", 40.0, 45.0),
            line("лятор заряжен", 40.0, 65.0),
        ];
        assert_eq!(text(lines.clone(), Breaks::Join), "Аккумулятор заряжен");
        assert_eq!(
            text(lines, Breaks::Keep),
            "Аккуму-\nлятор заряжен",
            "kept as on screen"
        );
        let dash = vec![line("Москва -", 40.0, 45.0), line("Киев", 40.0, 65.0)];
        assert_eq!(
            text(dash, Breaks::Join),
            "Москва - Киев",
            "a dash isn't a hyphen"
        );
    }

    #[test]
    fn a_part_of_a_line_is_taken_without_its_edge_spaces() {
        let layout = Layout::new(&Page {
            lines: vec![line("один два три", 0.0, 0.0)],
        });
        assert_eq!(compose(&layout, &[(0, 4, 9)], Breaks::Keep), "два");
    }

    #[test]
    fn the_preview_is_one_line() {
        let layout = Layout::new(&Page {
            lines: vec![line("один", 0.0, 0.0), line("два", 0.0, 40.0)],
        });
        assert_eq!(preview(&layout, &all(&layout)), "один два");
        assert_eq!(Breaks::Keep.toggled(), Breaks::Join);
    }
}
