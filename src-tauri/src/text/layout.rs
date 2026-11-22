//! Where every character of a page stands, and the pointer → a place between characters.
//!
//! A reader gives boxes of whole words only. A character's share of its word's box is
//! estimated from a width class (narrow `i`, wide `ш`), which is close enough for a caret
//! inside a word in any of the usual screen fonts. A space takes the gap between two words.

use crate::text::{Page, Rect};

/// A place between two characters: before character `at` of line `line` (`at` = the line's
/// length is after its last one). Ordered as the text reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: usize,
    pub at: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    pub chars: Vec<char>,
    /// `chars.len() + 1` x coordinates: the left edge of each character, then the right edge
    /// of the last one.
    pub edges: Vec<f32>,
    pub rect: Rect,
    /// Each word's first and past-the-end character.
    pub words: Vec<(usize, usize)>,
}

impl Laid {
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn text(&self, from: usize, to: usize) -> String {
        self.chars[from.min(to)..to.min(self.len())]
            .iter()
            .collect()
    }

    fn nearest_edge(&self, x: f32) -> usize {
        let mut best = 0;
        for (i, edge) in self.edges.iter().enumerate() {
            if (edge - x).abs() < (self.edges[best] - x).abs() {
                best = i;
            }
        }
        best
    }

    /// The word around character place `at`; a place on a space takes the word after it.
    pub fn word_at(&self, at: usize) -> (usize, usize) {
        self.words
            .iter()
            .copied()
            .find(|&(_, end)| at < end || at == end && end == self.len())
            .or(self.words.last().copied())
            .unwrap_or((0, 0))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub lines: Vec<Laid>,
}

impl Layout {
    pub fn new(page: &Page) -> Self {
        let lines = page
            .lines
            .iter()
            .map(|line| {
                // (character, left, right)
                let mut cells: Vec<(char, f32, f32)> = Vec::new();
                let mut words = Vec::new();
                let mut previous: Option<Rect> = None;
                for word in &line.words {
                    if word.spaced
                        && let Some(prev) = previous
                    {
                        cells.push((' ', prev.right(), word.rect.x.max(prev.right())));
                    }
                    let start = cells.len();
                    let chars: Vec<char> = word.text.chars().collect();
                    let total: f32 = chars.iter().map(|&c| weight(c)).sum::<f32>().max(0.01);
                    let mut x = word.rect.x;
                    for c in chars {
                        let w = word.rect.width * weight(c) / total;
                        cells.push((c, x, x + w));
                        x += w;
                    }
                    words.push((start, cells.len()));
                    previous = Some(word.rect);
                }
                let mut edges = Vec::with_capacity(cells.len() + 1);
                for (i, cell) in cells.iter().enumerate() {
                    edges.push(match i {
                        0 => cell.1,
                        _ => (cells[i - 1].2 + cell.1) / 2.0,
                    });
                }
                edges.push(cells.last().map_or(0.0, |c| c.2));
                Laid {
                    chars: cells.into_iter().map(|c| c.0).collect(),
                    edges,
                    rect: line.rect(),
                    words,
                }
            })
            .collect();
        Self { lines }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(Laid::is_empty)
    }

    pub fn start(&self) -> Pos {
        Pos { line: 0, at: 0 }
    }

    pub fn end(&self) -> Pos {
        let line = self.lines.len().saturating_sub(1);
        Pos {
            line,
            at: self.lines.get(line).map_or(0, Laid::len),
        }
    }

    /// The line the point is on, with a little room around its box: where a press starts a
    /// selection and the cursor is an I-beam.
    pub fn line_under(&self, x: f32, y: f32) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let r = l.rect;
                let (room_x, room_y) = (r.height * 0.6, r.height * 0.3);
                x >= r.x - room_x
                    && x <= r.right() + room_x
                    && y >= r.y - room_y
                    && y <= r.bottom() + room_y
            })
            .min_by(|a, b| closer(&a.1.rect, &b.1.rect, x, y))
            .map(|(i, _)| i)
    }

    /// The place nearest to the point, wherever it is: a drag beyond the text still moves
    /// the caret to the closest line.
    pub fn nearest(&self, x: f32, y: f32) -> Option<Pos> {
        let (line, laid) = self
            .lines
            .iter()
            .enumerate()
            .min_by(|a, b| closer(&a.1.rect, &b.1.rect, x, y))?;
        Some(Pos {
            line,
            at: laid.nearest_edge(x),
        })
    }

    /// One rectangle per line between two places, in reading order: the highlight and the
    /// underline of a link are drawn from these.
    pub fn spans(&self, from: Pos, to: Pos) -> Vec<Rect> {
        self.pieces(from, to)
            .into_iter()
            .filter(|&(_, s, e)| s < e)
            .map(|(line, s, e)| {
                let laid = &self.lines[line];
                Rect {
                    x: laid.edges[s],
                    y: laid.rect.y,
                    width: laid.edges[e] - laid.edges[s],
                    height: laid.rect.height,
                }
            })
            .collect()
    }

    /// `(line, from, to)` for each line between two places.
    pub fn pieces(&self, from: Pos, to: Pos) -> Vec<(usize, usize, usize)> {
        let (from, to) = (from.min(to), from.max(to));
        (from.line..=to.line.min(self.lines.len().saturating_sub(1)))
            .filter(|&line| line < self.lines.len())
            .map(|line| {
                let len = self.lines[line].len();
                let s = if line == from.line {
                    from.at.min(len)
                } else {
                    0
                };
                let e = if line == to.line { to.at.min(len) } else { len };
                (line, s, e)
            })
            .collect()
    }

    /// The words whose middle is inside `area`, as pieces of their lines: what an Alt-drag
    /// rectangle takes.
    pub fn inside(&self, area: Rect) -> Vec<(usize, usize, usize)> {
        let mut pieces = Vec::new();
        for (line, laid) in self.lines.iter().enumerate() {
            let cy = laid.rect.y + laid.rect.height / 2.0;
            if cy < area.y || cy > area.bottom() {
                continue;
            }
            let taken: Vec<_> = laid
                .words
                .iter()
                .filter(|&&(s, e)| {
                    let cx = (laid.edges[s] + laid.edges[e]) / 2.0;
                    cx >= area.x && cx <= area.right()
                })
                .collect();
            if let (Some(first), Some(last)) = (taken.first(), taken.last()) {
                pieces.push((line, first.0, last.1));
            }
        }
        pieces
    }
}

/// Height first, as in a text editor: a pointer far right of a short line is still on it, not
/// on a longer line below. Between lines of the same height (two columns) the nearer one.
fn closer(a: &Rect, b: &Rect, x: f32, y: f32) -> std::cmp::Ordering {
    let gaps = |r: &Rect| {
        let dx = (r.x - x).max(x - r.right()).max(0.0);
        let dy = (r.y - y).max(y - r.bottom()).max(0.0);
        (dy, dx)
    };
    let ((ay, ax), (by, bx)) = (gaps(a), gaps(b));
    ay.total_cmp(&by).then(ax.total_cmp(&bx))
}

/// A character's width relative to an average letter.
fn weight(c: char) -> f32 {
    match c {
        'i' | 'l' | 'j' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' | 'і' | 'ї' => 0.45,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '1' | '-' | '"' | '«' | '»' => 0.65,
        'm' | 'w' | 'M' | 'W' | '@' | '%' | 'ш' | 'щ' | 'ж' | 'ю' | 'ы' | 'ф' | 'Ш' | 'Щ' | 'Ж'
        | 'Ю' | 'Ы' | 'Ф' | 'М' => 1.45,
        c if c.is_uppercase() => 1.2,
        c if c.is_ascii_digit() => 0.95,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Line, Word};

    pub fn word(text: &str, x: f32, y: f32, spaced: bool) -> Word {
        Word {
            text: text.into(),
            rect: Rect {
                x,
                y,
                width: text.chars().count() as f32 * 10.0,
                height: 14.0,
            },
            spaced,
        }
    }

    /// "один два" at y 0 and "три" at y 40.
    fn page() -> Page {
        Page {
            lines: vec![
                Line {
                    words: vec![word("один", 0.0, 0.0, false), word("два", 50.0, 0.0, true)],
                },
                Line {
                    words: vec![word("три", 0.0, 40.0, false)],
                },
            ],
        }
    }

    #[test]
    fn a_space_takes_the_gap_between_words() {
        let layout = Layout::new(&page());
        let line = &layout.lines[0];
        assert_eq!(line.chars.iter().collect::<String>(), "один два");
        assert_eq!(line.edges.len(), 9);
        assert_eq!(line.edges[4], 40.0, "after the first word");
        assert_eq!(line.edges[5], 50.0, "after the space");
        assert_eq!(line.edges[8], 80.0);
        assert_eq!(line.words, vec![(0, 4), (5, 8)]);
    }

    #[test]
    fn narrow_letters_take_less_of_the_word() {
        let page = Page {
            lines: vec![Line {
                words: vec![Word {
                    text: "iш".into(),
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 14.0,
                    },
                    spaced: false,
                }],
            }],
        };
        let layout = Layout::new(&page);
        assert!(layout.lines[0].edges[1] < 30.0);
    }

    #[test]
    fn a_press_near_a_line_is_on_it_and_far_away_is_not() {
        let layout = Layout::new(&page());
        assert_eq!(layout.line_under(20.0, 7.0), Some(0));
        assert_eq!(layout.line_under(20.0, 47.0), Some(1));
        assert_eq!(layout.line_under(-5.0, 7.0), Some(0), "just left of it");
        assert_eq!(layout.line_under(20.0, 27.0), None, "between the lines");
        assert_eq!(layout.line_under(300.0, 7.0), None);
    }

    #[test]
    fn the_nearest_place_is_the_closest_edge_of_the_closest_line() {
        let layout = Layout::new(&page());
        assert_eq!(layout.nearest(21.0, 7.0), Some(Pos { line: 0, at: 2 }));
        assert_eq!(layout.nearest(500.0, 5.0), Some(Pos { line: 0, at: 8 }));
        assert_eq!(layout.nearest(5.0, 200.0), Some(Pos { line: 1, at: 0 }));
    }

    #[test]
    fn spans_cover_each_line_between_two_places() {
        let layout = Layout::new(&page());
        let spans = layout.spans(Pos { line: 1, at: 2 }, Pos { line: 0, at: 5 });
        assert_eq!(spans.len(), 2, "the order of the ends doesn't matter");
        assert_eq!((spans[0].x, spans[0].width), (50.0, 30.0));
        assert_eq!((spans[1].x, spans[1].y, spans[1].width), (0.0, 40.0, 20.0));
    }

    #[test]
    fn a_place_on_a_space_takes_the_next_word() {
        let layout = Layout::new(&page());
        assert_eq!(layout.lines[0].word_at(2), (0, 4));
        assert_eq!(layout.lines[0].word_at(4), (5, 8));
        assert_eq!(layout.lines[0].word_at(8), (5, 8), "after the last one");
    }

    #[test]
    fn a_rectangle_takes_the_words_whose_middle_is_inside() {
        let layout = Layout::new(&page());
        let area = Rect {
            x: 45.0,
            y: -5.0,
            width: 100.0,
            height: 70.0,
        };
        assert_eq!(layout.inside(area), vec![(0, 5, 8)]);
    }
}
