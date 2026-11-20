//! Where each part of a dialog stands, in logical pixels of the card. The metrics are the update
//! window's CSS (`.win`, `.head`, `.notes`, `.actions`, `.bar`); text sizes come through [`Measure`].
//! Margins between the notes' lines collapse the way CSS's do: two neighbours stand as far apart
//! as the larger of their margins.

use super::{Block, Line, Sheet};

pub const WIDTH: f32 = 460.0;
pub const RADIUS: f32 = 22.0;
const PAD: f32 = 20.0;
const PAD_BOTTOM: f32 = 18.0;
pub const ICON: f32 = 46.0;
const ICON_GAP: f32 = 12.0;
/// The head keeps clear of the ×.
const HEAD_RIGHT: f32 = 18.0;
pub const CLOSE: f32 = 28.0;
const CLOSE_TOP: f32 = 10.0;
const CLOSE_RIGHT: f32 = 12.0;
const NOTES_GAP: f32 = 14.0;
const NOTES_MAX: f32 = 170.0;
pub const NOTES_RADIUS: f32 = 12.0;
const NOTES_PAD_X: f32 = 12.0;
const NOTES_PAD_Y: f32 = 10.0;
/// The list stands this far in from the box's title.
const INDENT: f32 = 16.0;
/// Taken from the right of the notes' text while they scroll.
pub const SCROLLBAR: f32 = 12.0;
const ROW_GAP_FIRST: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_PAD: f32 = 8.0;
const BAR_GAP: f32 = 16.0;
pub const BAR: f32 = 6.0;
const STAT_GAP: f32 = 8.0;
/// The window's bounds; the card in it may be lower than the least.
const MIN_HEIGHT: f32 = 160.0;
const MAX_HEIGHT: f32 = 640.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Title,
    Subtitle,
    /// The notes box's own title and its headings: small capitals.
    Caps,
    Note,
    Button,
    Stat,
    Close,
}

pub trait Measure {
    /// Width and height of `text` wrapped at `width`; `None` — on one line.
    fn size(&self, text: &str, style: Style, width: Option<f32>) -> (f32, f32);
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    #[cfg(test)]
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notes {
    pub frame: Rect,
    /// The height of all that is in it, the padding included; more than the frame's — it scrolls.
    pub content: f32,
    /// In the box's own coordinates, before scrolling.
    pub title: Rect,
    pub lines: Vec<Rect>,
}

impl Notes {
    pub fn scrolls(&self) -> bool {
        self.content > self.frame.h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Placed {
    Notes(Notes),
    /// `stat`: the line under the bar, left and right ends of it.
    Progress {
        bar: Rect,
        stat: Option<Rect>,
    },
    Buttons(Vec<Rect>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub height: f32,
    pub icon: Rect,
    pub title: Rect,
    pub subtitle: Option<Rect>,
    pub close: Option<Rect>,
    /// One per block of the sheet, in its order.
    pub blocks: Vec<Placed>,
}

const INNER: f32 = WIDTH - 2.0 * PAD;

pub fn layout(sheet: &Sheet, m: &dyn Measure) -> Layout {
    let text_x = PAD + ICON + ICON_GAP;
    let text_w = INNER - HEAD_RIGHT - ICON - ICON_GAP;
    let (_, title_h) = m.size(&sheet.title, Style::Title, Some(text_w));
    let subtitle_h = match sheet.subtitle.is_empty() {
        true => 0.0,
        false => m.size(&sheet.subtitle, Style::Subtitle, Some(text_w)).1,
    };
    let text_h = title_h + subtitle_h;
    let head = ICON.max(text_h);
    let text_y = PAD + (head - text_h) / 2.0;
    let title = Rect {
        x: text_x,
        y: text_y,
        w: text_w,
        h: title_h,
    };
    let subtitle = (subtitle_h > 0.0).then_some(Rect {
        x: text_x,
        y: text_y + title_h,
        w: text_w,
        h: subtitle_h,
    });
    let close = sheet.dismiss.map(|_| Rect {
        x: WIDTH - CLOSE_RIGHT - CLOSE,
        y: CLOSE_TOP,
        w: CLOSE,
        h: CLOSE,
    });

    let mut y = PAD + head;
    let mut after_buttons = false;
    let mut blocks = Vec::new();
    for block in &sheet.blocks {
        let placed = match block {
            Block::Notes { title, lines } => {
                y += NOTES_GAP;
                let notes = notes(title, lines, y, m);
                y += notes.frame.h;
                Placed::Notes(notes)
            }
            Block::Progress { left, right, .. } => {
                y += BAR_GAP;
                let bar = Rect {
                    x: PAD,
                    y,
                    w: INNER,
                    h: BAR,
                };
                y += BAR;
                let stat = (!left.is_empty() || !right.is_empty()).then(|| {
                    y += STAT_GAP;
                    let h = m.size(&format!("{left}{right}"), Style::Stat, None).1;
                    let stat = Rect {
                        x: PAD,
                        y,
                        w: INNER,
                        h,
                    };
                    y += h;
                    stat
                });
                Placed::Progress { bar, stat }
            }
            Block::Buttons(buttons) => {
                y += if after_buttons {
                    ROW_GAP
                } else {
                    ROW_GAP_FIRST
                };
                let n = buttons.len().max(1) as f32;
                let label = buttons
                    .iter()
                    .map(|b| m.size(&b.label, Style::Button, None).1)
                    .fold(0.0, f32::max);
                let h = 2.0 * BUTTON_PAD + label;
                let w = (INNER - BUTTON_GAP * (n - 1.0)) / n;
                let rects = (0..buttons.len())
                    .map(|i| Rect {
                        x: PAD + i as f32 * (w + BUTTON_GAP),
                        y,
                        w,
                        h,
                    })
                    .collect();
                y += h;
                Placed::Buttons(rects)
            }
        };
        after_buttons = matches!(block, Block::Buttons(_));
        blocks.push(placed);
    }
    Layout {
        height: y + PAD_BOTTOM,
        icon: Rect {
            x: PAD,
            y: PAD + (head - ICON) / 2.0,
            w: ICON,
            h: ICON,
        },
        title,
        subtitle,
        close,
        blocks,
    }
}

fn margins(line: &Line) -> (f32, f32) {
    match line {
        Line::Heading(_) => (8.0, 3.0),
        Line::Item(_) | Line::Text(_) => (0.0, 4.0),
    }
}

pub fn line_text(line: &Line) -> &str {
    match line {
        Line::Heading(t) | Line::Item(t) | Line::Text(t) => t,
    }
}

fn line_style(line: &Line) -> Style {
    match line {
        Line::Heading(_) => Style::Caps,
        Line::Item(_) | Line::Text(_) => Style::Note,
    }
}

fn notes(title: &str, lines: &[Line], top: f32, m: &dyn Measure) -> Notes {
    let full = INNER - 2.0 * NOTES_PAD_X;
    let placed = place_lines(title, lines, full, m);
    // A box that scrolls gives its scrollbar room and lays the text out again, narrower.
    let (title, lines, content) = if placed.2 > NOTES_MAX {
        place_lines(title, lines, full - SCROLLBAR, m)
    } else {
        placed
    };
    Notes {
        frame: Rect {
            x: PAD,
            y: top,
            w: INNER,
            h: content.min(NOTES_MAX),
        },
        content,
        title,
        lines,
    }
}

fn place_lines(title: &str, lines: &[Line], room: f32, m: &dyn Measure) -> (Rect, Vec<Rect>, f32) {
    let mut y = NOTES_PAD_Y;
    let (_, h) = m.size(title, Style::Caps, Some(room));
    let title = Rect {
        x: NOTES_PAD_X,
        y,
        w: room,
        h,
    };
    y += h;
    let mut after = 3.0f32;
    let mut rects = Vec::with_capacity(lines.len());
    for line in lines {
        let (top, bottom) = margins(line);
        y += after.max(top);
        let w = room - INDENT;
        let (_, h) = m.size(line_text(line), line_style(line), Some(w));
        rects.push(Rect {
            x: NOTES_PAD_X + INDENT,
            y,
            w,
            h,
        });
        y += h;
        after = bottom;
    }
    (title, rects, y + after + NOTES_PAD_Y)
}

/// The card never gets lower while the dialog is open: what was under the pointer stays there.
pub fn card_height(layout: f32, before: f32) -> f32 {
    layout.max(before)
}

pub fn window_height(card: f32) -> f32 {
    card.clamp(MIN_HEIGHT, MAX_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::{Action, Button};

    /// Every character 6 wide; lines wrap at the width.
    struct Fixed;

    impl Measure for Fixed {
        fn size(&self, text: &str, style: Style, width: Option<f32>) -> (f32, f32) {
            let line = match style {
                Style::Title => 22.0,
                Style::Caps | Style::Note => 18.0,
                Style::Close => 20.0,
                Style::Subtitle | Style::Button => 17.0,
                Style::Stat => 16.0,
            };
            let w = text.chars().count() as f32 * 6.0;
            let lines = width.map_or(1.0, |room| (w / room).ceil().max(1.0));
            (w.min(width.unwrap_or(w)), lines * line)
        }
    }

    fn sheet(blocks: Vec<Block>) -> Sheet {
        Sheet {
            title: "Update available".into(),
            subtitle: String::new(),
            blocks,
            dismiss: None,
        }
    }

    fn button(label: &str) -> Button {
        Button {
            label: label.into(),
            action: Action(1),
            primary: false,
        }
    }

    fn notes_of(lines: Vec<Line>) -> Notes {
        let l = layout(
            &sheet(vec![Block::Notes {
                title: "What's new".into(),
                lines,
            }]),
            &Fixed,
        );
        match &l.blocks[0] {
            Placed::Notes(n) => n.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_title_stands_in_the_middle_of_the_icon() {
        let l = layout(&sheet(Vec::new()), &Fixed);
        assert_eq!(
            l.icon,
            Rect {
                x: 20.0,
                y: 20.0,
                w: 46.0,
                h: 46.0
            }
        );
        assert_eq!((l.title.x, l.title.y), (78.0, 32.0));
        assert_eq!(l.title.w, 460.0 - 40.0 - 18.0 - 46.0 - 12.0);
        assert_eq!(l.height, 20.0 + 46.0 + 18.0);
        assert_eq!(l.subtitle, None);
    }

    #[test]
    fn a_long_subtitle_makes_the_head_taller_than_the_icon() {
        let mut s = sheet(Vec::new());
        s.subtitle = "x".repeat(200);
        let l = layout(&s, &Fixed);
        let sub = l.subtitle.expect("a subtitle");
        assert_eq!(sub.h, 4.0 * 17.0);
        assert_eq!(l.title.y, 20.0);
        assert_eq!(l.icon.y, 20.0 + (22.0 + 68.0 - 46.0) / 2.0);
    }

    #[test]
    fn the_close_button_comes_with_a_dismiss_action() {
        let mut s = sheet(Vec::new());
        assert_eq!(layout(&s, &Fixed).close, None);
        s.dismiss = Some(Action(2));
        assert_eq!(
            layout(&s, &Fixed).close,
            Some(Rect {
                x: 420.0,
                y: 10.0,
                w: 28.0,
                h: 28.0
            })
        );
    }

    #[test]
    fn buttons_share_a_row_and_rows_stand_closer_to_each_other() {
        let l = layout(
            &sheet(vec![
                Block::Buttons(vec![button("Install")]),
                Block::Buttons(vec![button("Postpone"), button("Skip")]),
            ]),
            &Fixed,
        );
        let (Placed::Buttons(first), Placed::Buttons(second)) = (&l.blocks[0], &l.blocks[1]) else {
            panic!("two rows");
        };
        assert_eq!(
            first[0],
            Rect {
                x: 20.0,
                y: 82.0,
                w: 420.0,
                h: 33.0
            }
        );
        assert_eq!(second[0].y, 82.0 + 33.0 + 8.0);
        assert_eq!(second[0].w, 206.0);
        assert_eq!(second[1].x, 20.0 + 206.0 + 8.0);
        assert_eq!(l.height, second[0].bottom() + 18.0);
    }

    #[test]
    fn neighbouring_margins_collapse_to_the_larger() {
        let n = notes_of(vec![
            Line::Item("a".into()),
            Line::Heading("b".into()),
            Line::Item("c".into()),
        ]);
        assert_eq!(n.title.y, 10.0);
        assert_eq!(n.lines[0].y, 10.0 + 18.0 + 3.0);
        assert_eq!(n.lines[1].y, n.lines[0].bottom() + 8.0);
        assert_eq!(n.lines[2].y, n.lines[1].bottom() + 3.0);
        assert_eq!(n.content, n.lines[2].bottom() + 4.0 + 10.0);
        assert_eq!(n.lines[0].x, 12.0 + 16.0);
        assert!(!n.scrolls());
    }

    #[test]
    fn long_notes_scroll_in_a_box_of_170_and_make_room_for_the_bar() {
        let short = notes_of(vec![Line::Item("a".into())]);
        assert_eq!(short.lines[0].w, 420.0 - 24.0 - 16.0);
        let long = notes_of((0..20).map(|i| Line::Item(format!("item {i}"))).collect());
        assert!(long.scrolls());
        assert_eq!(long.frame.h, 170.0);
        assert_eq!(long.lines[0].w, 420.0 - 24.0 - 12.0 - 16.0);
    }

    #[test]
    fn the_bar_has_its_line_only_when_there_is_something_to_say() {
        let progress = |left: &str| Block::Progress {
            share: None,
            left: left.into(),
            right: String::new(),
        };
        let bare = layout(&sheet(vec![progress("")]), &Fixed);
        assert!(
            matches!(&bare.blocks[0], Placed::Progress { stat: None, bar } if bar.y == 82.0 && bar.h == 6.0)
        );
        let told = layout(&sheet(vec![progress("1 of 2 MB")]), &Fixed);
        let Placed::Progress {
            stat: Some(stat), ..
        } = &told.blocks[0]
        else {
            panic!("a line under the bar");
        };
        assert_eq!(stat.y, 82.0 + 6.0 + 8.0);
        assert_eq!(told.height, stat.bottom() + 18.0);
    }

    #[test]
    fn the_card_keeps_its_height_and_the_window_its_bounds() {
        assert_eq!(card_height(200.0, 300.0), 300.0);
        assert_eq!(card_height(320.0, 300.0), 320.0);
        assert_eq!(window_height(130.0), 160.0);
        assert_eq!(window_height(700.0), 640.0);
    }
}
