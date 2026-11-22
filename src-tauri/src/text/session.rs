//! Text on a frozen picture, selected as ordinary text: what a press, a drag and a release
//! do, with no Windows in it.
//!
//! - A press on a line starts a selection; a drag moves its end to the nearest place, even
//!   beyond the text. A press elsewhere clears it.
//! - A second press in place takes the word, a third the line, and a drag from them goes on
//!   by whole words or lines. Shift+press extends.
//! - Ctrl+press on a link opens it. A press on a QR code copies it, Ctrl+press opens a link
//!   in it. A code wins over the text under it.
//! - Alt+drag draws a rectangle; the release copies the words inside and the scanner ends.

use crate::text::{
    Page, Rect,
    compose::{self, Breaks},
    layout::{Layout, Pos},
    links::{self, Link},
    qr::Code,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    Text,
    Hand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// The selection or the rectangle changed: redraw.
    Changed,
    Open(String),
    /// Put in the clipboard; the scanner stays.
    Copy(String),
    /// Put in the clipboard, and the scanner ends.
    CopyAndClose(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    /// `from..to` is what the press took (a word on a double press); a drag adds whole
    /// `unit`s up to the pointer and always keeps it.
    Text {
        unit: Unit,
        from: Pos,
        to: Pos,
    },
    Area {
        x: f32,
        y: f32,
        to_x: f32,
        to_y: f32,
    },
}

#[derive(Clone, Copy, Debug)]
struct Click {
    x: f32,
    y: f32,
    time: u64,
    count: u32,
}

pub struct Session {
    layout: Layout,
    links: Vec<Link>,
    codes: Vec<Code>,
    anchor: Pos,
    caret: Pos,
    drag: Drag,
    last_click: Option<Click>,
    /// Windows' double-click time, ms.
    double_click: u64,
}

/// A second press farther than this from the first one, in picture pixels, is a new click.
const CLICK_SLOP: f32 = 4.0;
/// A smaller Alt rectangle was a press, not a drag.
const AREA_MIN: f32 = 4.0;

impl Session {
    pub fn new(page: &Page, codes: Vec<Code>, double_click: u64) -> Self {
        let layout = Layout::new(page);
        let links = links::find(&layout);
        let start = layout.start();
        Self {
            layout,
            links,
            codes,
            anchor: start,
            caret: start,
            drag: Drag::None,
            last_click: None,
            double_click,
        }
    }

    pub fn has_text(&self) -> bool {
        !self.layout.is_empty() || !self.codes.is_empty()
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    pub fn cursor(&self, x: f32, y: f32, keys: Keys) -> Cursor {
        if self.code_at(x, y).is_some() || keys.ctrl && self.link_at(x, y).is_some() {
            Cursor::Hand
        } else if matches!(self.drag, Drag::Text { .. }) || self.layout.line_under(x, y).is_some() {
            Cursor::Text
        } else {
            Cursor::Arrow
        }
    }

    pub fn press(&mut self, x: f32, y: f32, keys: Keys, time: u64) -> Outcome {
        if keys.alt {
            self.last_click = None;
            self.drag = Drag::Area {
                x,
                y,
                to_x: x,
                to_y: y,
            };
            return Outcome::Changed;
        }
        if let Some(code) = self.code_at(x, y) {
            let content = self.codes[code].content.clone();
            return match links_in(&content) {
                Some(target) if keys.ctrl => Outcome::Open(target),
                _ => Outcome::Copy(content),
            };
        }
        if keys.ctrl
            && let Some(link) = self.link_at(x, y)
        {
            return Outcome::Open(self.links[link].target.clone());
        }
        let count = self.count(x, y, time);
        let Some(line) = self.layout.line_under(x, y) else {
            let had = self.has_selection();
            self.caret = self.anchor;
            return if had {
                Outcome::Changed
            } else {
                Outcome::Nothing
            };
        };
        let Some(at) = self.layout.nearest(x, y) else {
            return Outcome::Nothing;
        };
        let at = if at.line == line {
            at
        } else {
            Pos { line, at: 0 }
        };
        let (unit, from, to) = match count {
            2 => {
                let (from, to) = self.extent(Unit::Word, at);
                (Unit::Word, from, to)
            }
            3 => {
                let (from, to) = self.extent(Unit::Line, at);
                (Unit::Line, from, to)
            }
            _ if keys.shift => {
                self.caret = at;
                (Unit::Char, self.anchor, self.anchor)
            }
            _ => {
                (self.anchor, self.caret) = (at, at);
                (Unit::Char, at, at)
            }
        };
        if unit != Unit::Char {
            (self.anchor, self.caret) = (from, to);
        }
        self.drag = Drag::Text { unit, from, to };
        Outcome::Changed
    }

    pub fn moved(&mut self, x: f32, y: f32) -> Outcome {
        match &mut self.drag {
            Drag::None => Outcome::Nothing,
            &mut Drag::Text { unit, from, to } => {
                let Some(at) = self.layout.nearest(x, y) else {
                    return Outcome::Nothing;
                };
                let (start, end) = self.extent(unit, at);
                let ends = if at < from { (to, start) } else { (from, end) };
                if ends == (self.anchor, self.caret) {
                    return Outcome::Nothing;
                }
                (self.anchor, self.caret) = ends;
                Outcome::Changed
            }
            Drag::Area { to_x, to_y, .. } => {
                (*to_x, *to_y) = (x, y);
                Outcome::Changed
            }
        }
    }

    pub fn release(&mut self, x: f32, y: f32, breaks: Breaks) -> Outcome {
        let moved = self.moved(x, y);
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        if let Drag::Area { .. } = drag {
            let area = self.area_of(drag);
            if area.width < AREA_MIN && area.height < AREA_MIN {
                return Outcome::Changed;
            }
            let text = compose::compose(&self.layout, &self.layout.inside(area), breaks);
            return if text.is_empty() {
                Outcome::Changed
            } else {
                Outcome::CopyAndClose(text)
            };
        }
        moved
    }

    /// Esc while dragging, or the window lost: the drag ends where it is.
    pub fn cancel_drag(&mut self) -> Outcome {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::Area { .. } => Outcome::Changed,
            _ => Outcome::Nothing,
        }
    }

    pub fn select_all(&mut self) -> Outcome {
        (self.anchor, self.caret) = (self.layout.start(), self.layout.end());
        Outcome::Changed
    }

    pub fn clear(&mut self) {
        self.caret = self.anchor;
    }

    pub fn selected(&self, breaks: Breaks) -> Option<String> {
        let text = compose::compose(&self.layout, &self.pieces(), breaks);
        (!text.is_empty()).then_some(text)
    }

    pub fn preview(&self) -> String {
        compose::preview(&self.layout, &self.pieces())
    }

    pub fn highlight(&self) -> Vec<Rect> {
        self.layout.spans(self.anchor, self.caret)
    }

    /// The Alt rectangle while it is drawn.
    pub fn area(&self) -> Option<Rect> {
        matches!(self.drag, Drag::Area { .. }).then(|| self.area_of(self.drag))
    }

    pub fn underlines(&self) -> Vec<Rect> {
        self.links
            .iter()
            .flat_map(|l| self.layout.spans(l.from, l.to))
            .collect()
    }

    /// The QR codes are searched for apart from the text and may come after it.
    pub fn set_codes(&mut self, codes: Vec<Code>) {
        self.codes = codes;
    }

    pub fn code_boxes(&self) -> Vec<Rect> {
        self.codes.iter().map(|c| c.rect).collect()
    }

    /// Every line's and code's box, for the glint that runs over what was found.
    pub fn found(&self) -> Vec<Rect> {
        self.layout
            .lines
            .iter()
            .map(|l| l.rect)
            .chain(self.codes.iter().map(|c| c.rect))
            .collect()
    }

    pub fn over_link(&self, x: f32, y: f32) -> bool {
        self.link_at(x, y).is_some()
    }

    fn pieces(&self) -> Vec<(usize, usize, usize)> {
        if !self.has_selection() {
            return Vec::new();
        }
        self.layout.pieces(self.anchor, self.caret)
    }

    fn area_of(&self, drag: Drag) -> Rect {
        let Drag::Area { x, y, to_x, to_y } = drag else {
            return Rect::default();
        };
        Rect {
            x: x.min(to_x),
            y: y.min(to_y),
            width: (to_x - x).abs(),
            height: (to_y - y).abs(),
        }
    }

    /// The word or the line around a place; a character place is itself.
    fn extent(&self, unit: Unit, at: Pos) -> (Pos, Pos) {
        let laid = &self.layout.lines[at.line];
        let (s, e) = match unit {
            Unit::Char => (at.at, at.at),
            Unit::Word => laid.word_at(at.at),
            Unit::Line => (0, laid.len()),
        };
        (
            Pos {
                line: at.line,
                at: s,
            },
            Pos {
                line: at.line,
                at: e,
            },
        )
    }

    fn count(&mut self, x: f32, y: f32, time: u64) -> u32 {
        let count = match self.last_click {
            Some(c)
                if time.saturating_sub(c.time) <= self.double_click
                    && (c.x - x).abs() <= CLICK_SLOP
                    && (c.y - y).abs() <= CLICK_SLOP =>
            {
                c.count % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some(Click { x, y, time, count });
        count
    }

    fn code_at(&self, x: f32, y: f32) -> Option<usize> {
        self.codes.iter().position(|c| {
            let r = c.rect;
            x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom()
        })
    }

    fn link_at(&self, x: f32, y: f32) -> Option<usize> {
        let line = self.layout.line_under(x, y)?;
        let at = self.layout.nearest(x, y)?;
        self.links.iter().position(|l| {
            l.from.line == line && at.line == line && (l.from.at..l.to.at).contains(&at.at)
        })
    }
}

/// The content's link, when the whole code is one.
fn links_in(content: &str) -> Option<String> {
    let layout = Layout::new(&Page {
        lines: vec![crate::text::Line {
            words: vec![crate::text::Word {
                text: content.trim().to_owned(),
                rect: Rect::default(),
                spaced: false,
            }],
        }],
    });
    let link = links::find(&layout).into_iter().next()?;
    (link.from.at == 0 && link.to.at == layout.lines[0].len()).then_some(link.target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Line, Word};

    fn line(text: &str, y: f32) -> Line {
        Line {
            words: text
                .split(' ')
                .enumerate()
                .scan(0.0, |at, (i, w)| {
                    let width = w.chars().count() as f32 * 10.0;
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
                    *at += width + 10.0;
                    Some(word)
                })
                .collect(),
        }
    }

    /// "один два три" at y 0, "четыре https://ex.com" at y 30.
    fn session(codes: Vec<Code>) -> Session {
        let page = Page {
            lines: vec![
                line("один два три", 0.0),
                line("четыре https://ex.com", 30.0),
            ],
        };
        Session::new(&page, codes, 500)
    }

    const NONE: Keys = Keys {
        shift: false,
        ctrl: false,
        alt: false,
    };

    #[test]
    fn a_drag_selects_from_the_press_to_the_pointer_across_lines() {
        let mut s = session(Vec::new());
        assert_eq!(s.press(50.0, 7.0, NONE, 0), Outcome::Changed);
        s.moved(60.0, 37.0);
        s.release(60.0, 37.0, Breaks::Keep);
        assert_eq!(s.selected(Breaks::Keep).as_deref(), Some("два три\nчетыре"));
        assert_eq!(s.highlight().len(), 2);
    }

    #[test]
    fn a_drag_past_the_text_ends_at_its_nearest_edge() {
        let mut s = session(Vec::new());
        s.press(0.0, 7.0, NONE, 0);
        s.release(900.0, 7.0, Breaks::Keep);
        assert_eq!(s.selected(Breaks::Keep).as_deref(), Some("один два три"));
    }

    #[test]
    fn second_and_third_presses_take_the_word_and_the_line() {
        let mut s = session(Vec::new());
        s.press(55.0, 7.0, NONE, 0);
        s.release(55.0, 7.0, Breaks::Keep);
        s.press(55.0, 7.0, NONE, 200);
        assert_eq!(s.selected(Breaks::Keep).as_deref(), Some("два"));
        s.press(56.0, 7.0, NONE, 400);
        assert_eq!(s.selected(Breaks::Keep).as_deref(), Some("один два три"));
        s.press(56.0, 7.0, NONE, 2000);
        assert_eq!(s.selected(Breaks::Keep), None, "too late: a new click");
    }

    #[test]
    fn a_drag_after_a_double_press_goes_by_words_and_after_a_triple_by_lines() {
        let mut s = session(Vec::new());
        s.press(55.0, 7.0, NONE, 0);
        s.release(55.0, 7.0, Breaks::Keep);
        s.press(55.0, 7.0, NONE, 200);
        s.moved(102.0, 7.0);
        assert_eq!(s.selected(Breaks::Keep).as_deref(), Some("два три"));
        s.moved(5.0, 7.0);
        assert_eq!(
            s.selected(Breaks::Keep).as_deref(),
            Some("один два"),
            "backwards keeps the pressed word"
        );
        s.release(5.0, 7.0, Breaks::Keep);

        s.press(5.0, 7.0, NONE, 1000);
        s.release(5.0, 7.0, Breaks::Keep);
        s.press(5.0, 7.0, NONE, 1100);
        s.release(5.0, 7.0, Breaks::Keep);
        s.press(5.0, 7.0, NONE, 1200);
        s.moved(10.0, 37.0);
        assert_eq!(
            s.selected(Breaks::Keep).as_deref(),
            Some("один два три\nчетыре https://ex.com")
        );
    }

    #[test]
    fn shift_extends_and_a_press_on_nothing_clears() {
        let mut s = session(Vec::new());
        s.press(0.0, 7.0, NONE, 0);
        s.release(0.0, 7.0, Breaks::Keep);
        let shift = Keys {
            shift: true,
            ..NONE
        };
        s.press(40.0, 7.0, shift, 1000);
        s.release(40.0, 7.0, Breaks::Keep);
        assert_eq!(s.selected(Breaks::Keep).as_deref(), Some("один"));
        assert_eq!(s.press(500.0, 500.0, NONE, 3000), Outcome::Changed);
        assert!(!s.has_selection());
        assert_eq!(s.press(500.0, 500.0, NONE, 6000), Outcome::Nothing);
    }

    #[test]
    fn ctrl_press_on_a_link_opens_it_and_a_plain_one_selects() {
        let mut s = session(Vec::new());
        let ctrl = Keys { ctrl: true, ..NONE };
        assert_eq!(s.cursor(100.0, 37.0, ctrl), Cursor::Hand);
        assert_eq!(s.cursor(100.0, 37.0, NONE), Cursor::Text);
        assert_eq!(
            s.press(100.0, 37.0, ctrl, 0),
            Outcome::Open("https://ex.com".into())
        );
        assert_eq!(s.press(100.0, 37.0, NONE, 5000), Outcome::Changed);
        assert_eq!(s.underlines().len(), 1);
    }

    #[test]
    fn alt_rectangle_copies_the_words_inside_and_closes() {
        let mut s = session(Vec::new());
        let alt = Keys { alt: true, ..NONE };
        s.press(45.0, -5.0, alt, 0);
        assert!(s.area().is_some());
        assert_eq!(
            s.release(150.0, 50.0, Breaks::Keep),
            Outcome::CopyAndClose("два три\nhttps://ex.com".into())
        );
        assert!(s.area().is_none());
        s.press(300.0, 300.0, alt, 1000);
        assert_eq!(
            s.release(301.0, 301.0, Breaks::Keep),
            Outcome::Changed,
            "a tap"
        );
    }

    #[test]
    fn a_code_is_copied_or_opened_and_wins_over_text() {
        let code = |content: &str| Code {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 30.0,
            },
            content: content.into(),
        };
        let mut s = session(vec![code("https://qr.example.com/x")]);
        assert_eq!(s.cursor(10.0, 7.0, NONE), Cursor::Hand);
        assert_eq!(
            s.press(10.0, 7.0, NONE, 0),
            Outcome::Copy("https://qr.example.com/x".into())
        );
        let ctrl = Keys { ctrl: true, ..NONE };
        assert_eq!(
            s.press(10.0, 7.0, ctrl, 1000),
            Outcome::Open("https://qr.example.com/x".into())
        );
        let mut s = session(vec![code("WIFI:S:home;T:WPA;P:secret;;")]);
        assert_eq!(
            s.press(10.0, 7.0, ctrl, 0),
            Outcome::Copy("WIFI:S:home;T:WPA;P:secret;;".into()),
            "not a link: copied"
        );
    }

    #[test]
    fn select_all_and_the_preview() {
        let mut s = session(Vec::new());
        assert_eq!(s.preview(), "");
        s.select_all();
        assert_eq!(s.preview(), "один два три четыре https://ex.com");
        assert!(s.has_text());
        assert_eq!(s.found().len(), 2, "one box per line");
        assert!(s.over_link(100.0, 37.0));
        assert!(!s.over_link(20.0, 7.0));
    }
}
