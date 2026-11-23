//! The text scanner's status: one row with what the scanner says (the selection's start, a
//! hint), the recognition language, the line-break toggle and Copy. The circles keep their size;
//! the text gives way. A click on the language opens the list of installed languages under the
//! row, the island growing down; the row's buttons stay.
//!
//! This is a state, not a notice: the island doesn't dim in it (`Mode::dims`).

use super::super::comp::{Motion, Text, color, v2, v3};
use super::super::glyph::Glyph;
use super::{Button, Content, Hit, MUTED, Model, Rect, Scene, TEXT, style};
use crate::lang::tr;
use windows::{
    UI::{Color, Composition::Visual},
    core::{Interface, Result},
};

const WIDTH: f32 = 372.0;
const HEIGHT: f32 = 36.0;
const ICON: f32 = 16.0;
const ICON_LEFT: f32 = 14.0;
const TEXT_LEFT: f32 = 38.0;
const TEXT_SIZE: f32 = 13.0;
const CIRCLE: f32 = 24.0;
const INSET: f32 = (HEIGHT - CIRCLE) / 2.0;
const GAP: f32 = 6.0;
const CIRCLE_ICON: f32 = 14.0;
const LANGUAGE_W: f32 = 46.0;
const CHEVRON: f32 = 10.0;
/// From the text to the language capsule.
const TEXT_ROOM: f32 = 8.0;
const ROW: f32 = 30.0;
const ROW_GAP: f32 = 2.0;
const LIST_PAD: f32 = 8.0;
const ROW_RADIUS: f32 = 10.0;

const IDLE: &str = "#1C1C1E";
const IDLE_HOVER: &str = "#2C2C2E";
const ON_HOVER: &str = "#E5E5EA";
const DIM_ICON: &str = "#636366";
const SWITCH: Motion = Motion::curve(140, 0.25, 0.1, 0.25, 1.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanHit {
    Language,
    Breaks,
    Copy,
    /// A language of the open list, by its place in it.
    Pick(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CopyLook {
    /// Nothing selected: the circle is dimmed and does nothing.
    Off,
    Ready,
    /// Just copied: a check for a moment.
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanLine<'a> {
    Reading,
    Hint,
    Empty,
    Link,
    Copied,
    Languages,
    Selected(&'a str),
}

pub struct ScanModel<'a> {
    pub line: ScanLine<'a>,
    /// The language capsule's label; empty while not known yet.
    pub language: &'a str,
    pub join: bool,
    pub copy: CopyLook,
    /// The open list: each language's name and whether it is the one in use. Empty = closed.
    pub list: &'a [(String, bool)],
}

pub(super) struct Parts {
    line: Text,
    /// Off, on.
    breaks: Vec<Visual>,
    /// Dimmed copy, ready copy, check.
    copy: Vec<Visual>,
}

fn line_text(line: ScanLine) -> (String, &'static str) {
    match line {
        ScanLine::Reading => (tr!("scan.reading").into_owned(), MUTED),
        ScanLine::Hint => (tr!("scan.hint").into_owned(), MUTED),
        ScanLine::Empty => (tr!("scan.empty").into_owned(), MUTED),
        ScanLine::Link => (tr!("scan.link").into_owned(), MUTED),
        ScanLine::Languages => (tr!("scan.languages").into_owned(), MUTED),
        ScanLine::Copied => (tr!("scan.copied").into_owned(), TEXT),
        ScanLine::Selected(text) => (text.to_owned(), TEXT),
    }
}

/// The fill and hover of a circle that is on (white) or off (dark).
fn fills(on: bool) -> (Color, Color) {
    if on {
        (color(TEXT, 1.0), color(ON_HOVER, 1.0))
    } else {
        (color(IDLE, 1.0), color(IDLE_HOVER, 1.0))
    }
}

fn copy_x() -> f32 {
    WIDTH - INSET - CIRCLE
}

fn breaks_x() -> f32 {
    copy_x() - GAP - CIRCLE
}

fn language_x() -> f32 {
    breaks_x() - GAP - LANGUAGE_W
}

fn text_room() -> f32 {
    language_x() - TEXT_ROOM - TEXT_LEFT
}

impl Scene {
    pub(super) fn scan_size(&self, model: &Model) -> (f32, f32) {
        let rows = model.scan.as_ref().map_or(0, |s| s.list.len()) as f32;
        if rows == 0.0 {
            return (WIDTH, HEIGHT);
        }
        (WIDTH, HEIGHT + rows * (ROW + ROW_GAP) - ROW_GAP + LIST_PAD)
    }

    pub(super) fn build_scan(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.scan_size(model);
        content.width = width;
        content.height = height;
        let Some(scan) = &model.scan else {
            return Ok(());
        };
        let root = content.root.clone();
        let s = self.scale;

        let icon = self
            .comp
            .glyph(Glyph::ScanText, ICON, color(MUTED, 1.0), s)?;
        self.place(&root, &icon, ICON_LEFT, (HEIGHT - ICON) / 2.0)?;

        let (text, hex) = line_text(scan.line);
        let st = style(TEXT_SIZE, false, hex);
        let fitted = self.comp.fit(&text, st, text_room());
        let line = self.comp.text(&fitted, st, s)?;
        let line_y = self.comp.center_top(&line, HEIGHT);
        self.place(&root, &line.visual, TEXT_LEFT, line_y)?;

        let open = !scan.list.is_empty();
        let rect = Rect {
            x: language_x(),
            y: INSET,
            w: LANGUAGE_W,
            h: CIRCLE,
        };
        let (base, hover) = if open {
            (color(IDLE_HOVER, 1.0), color(IDLE_HOVER, 1.0))
        } else {
            fills(false)
        };
        let fill = self.capsule(&root, rect, base)?;
        let label_st = style(12.0, true, TEXT);
        let label_w = self.comp.measure(scan.language, label_st);
        let label = self.comp.text(scan.language, label_st, s)?;
        let pair = label_w + 3.0 + CHEVRON;
        let left = rect.x + (rect.w - pair) / 2.0;
        self.place(
            &root,
            &label.visual,
            left,
            rect.y + self.comp.center_top(&label, rect.h),
        )?;
        let chevron = self
            .comp
            .glyph(Glyph::ChevronDown, CHEVRON, color(MUTED, 1.0), s)?;
        if open {
            chevron.SetCenterPoint(v3(CHEVRON / 2.0, CHEVRON / 2.0))?;
            chevron.SetRotationAngleInDegrees(180.0)?;
        }
        self.place(
            &root,
            &chevron,
            left + label_w + 3.0,
            rect.y + (rect.h - CHEVRON) / 2.0,
        )?;
        self.hit_button(
            content,
            rect,
            Hit::Scan(ScanHit::Language),
            fill,
            base,
            hover,
        );

        let breaks = self.circle(
            content,
            breaks_x(),
            Hit::Scan(ScanHit::Breaks),
            scan.join,
            &[(Glyph::Breaks, TEXT), (Glyph::Breaks, "#000000")],
        )?;
        let copy = self.circle(
            content,
            copy_x(),
            Hit::Scan(ScanHit::Copy),
            scan.copy != CopyLook::Off,
            &[
                (Glyph::Copy, DIM_ICON),
                (Glyph::Copy, "#000000"),
                (Glyph::Check, "#000000"),
            ],
        )?;

        for (i, (name, chosen)) in scan.list.iter().enumerate() {
            let rect = Rect {
                x: LIST_PAD,
                y: HEIGHT + i as f32 * (ROW + ROW_GAP),
                w: width - 2.0 * LIST_PAD,
                h: ROW,
            };
            let base = if *chosen {
                color(IDLE, 1.0)
            } else {
                color(IDLE, 0.0)
            };
            let fill = self.rounded(&root, rect, ROW_RADIUS, base)?;
            let name_st = style(TEXT_SIZE, false, TEXT);
            let shown = self.comp.fit(name, name_st, rect.w - 48.0);
            let text = self.comp.text(&shown, name_st, s)?;
            self.place(
                &root,
                &text.visual,
                rect.x + 12.0,
                rect.y + self.comp.center_top(&text, ROW),
            )?;
            if *chosen {
                let check = self
                    .comp
                    .glyph(Glyph::Check, CIRCLE_ICON, color(TEXT, 1.0), s)?;
                self.place(
                    &root,
                    &check,
                    rect.x + rect.w - 12.0 - CIRCLE_ICON,
                    rect.y + (ROW - CIRCLE_ICON) / 2.0,
                )?;
            }
            let hover = color(IDLE_HOVER, 1.0);
            self.hit_button(
                content,
                rect,
                Hit::Scan(ScanHit::Pick(i)),
                fill,
                base,
                hover,
            );
        }

        content.scan = Some(Parts { line, breaks, copy });
        Ok(())
    }

    /// The text, the toggle and Copy change in place: no fade on every move of a drag.
    pub(super) fn update_scan(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (Some(scan), Some(parts)) = (&model.scan, content.scan.as_mut()) else {
            return Ok(());
        };
        let s = self.scale;
        let (text, hex) = line_text(scan.line);
        let st = style(TEXT_SIZE, false, hex);
        let fitted = self.comp.fit(&text, st, text_room());
        self.comp
            .set_text_color(&mut parts.line, color(hex, 1.0), s)?;
        self.comp.set_text(&mut parts.line, &fitted, s)?;

        let shown = |v: &Visual, on: bool| v.SetOpacity(if on { 1.0 } else { 0.0 });
        shown(&parts.breaks[0], !scan.join)?;
        shown(&parts.breaks[1], scan.join)?;
        shown(&parts.copy[0], scan.copy == CopyLook::Off)?;
        shown(&parts.copy[1], scan.copy == CopyLook::Ready)?;
        shown(&parts.copy[2], scan.copy == CopyLook::Done)?;

        for (hit, on) in [
            (Hit::Scan(ScanHit::Breaks), scan.join),
            (Hit::Scan(ScanHit::Copy), scan.copy != CopyLook::Off),
        ] {
            let (base, hover) = fills(on);
            if let Some(button) = content.buttons.iter_mut().find(|b| b.hit == hit) {
                if button.base == base {
                    continue;
                }
                button.base = base;
                button.hover = hover;
                let target = if self.hovered.as_ref() == Some(&hit) {
                    hover
                } else {
                    base
                };
                self.comp.animate_color(&button.fill, target, SWITCH)?;
            }
        }
        Ok(())
    }

    /// A round button with its icons stacked, one shown at a time by `update_scan`.
    fn circle(
        &self,
        content: &mut Content,
        x: f32,
        hit: Hit,
        on: bool,
        icons: &[(Glyph, &str)],
    ) -> Result<Vec<Visual>> {
        let root = content.root.clone();
        let rect = Rect {
            x,
            y: INSET,
            w: CIRCLE,
            h: CIRCLE,
        };
        let (base, hover) = fills(on);
        let fill = self.capsule(&root, rect, base)?;
        let mut visuals = Vec::with_capacity(icons.len());
        let inset = (CIRCLE - CIRCLE_ICON) / 2.0;
        for &(glyph, hex) in icons {
            let icon = self
                .comp
                .glyph(glyph, CIRCLE_ICON, color(hex, 1.0), self.scale)?;
            icon.SetOpacity(0.0)?;
            self.place(&root, &icon, x + inset, INSET + inset)?;
            visuals.push(icon.cast()?);
        }
        // A little more than the circle: a press beside it still counts.
        let target = Rect {
            x: x - GAP / 2.0,
            y: 0.0,
            w: CIRCLE + GAP,
            h: HEIGHT,
        };
        self.hit_button(content, target, hit, fill, base, hover);
        Ok(visuals)
    }

    fn capsule(
        &self,
        root: &windows::UI::Composition::ContainerVisual,
        rect: Rect,
        base: Color,
    ) -> Result<windows::UI::Composition::CompositionColorBrush> {
        self.rounded(root, rect, rect.h / 2.0, base)
    }

    fn rounded(
        &self,
        root: &windows::UI::Composition::ContainerVisual,
        rect: Rect,
        radius: f32,
        base: Color,
    ) -> Result<windows::UI::Composition::CompositionColorBrush> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(rect.w, rect.h))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(rect.w, rect.h))?;
        geometry.SetCornerRadius(v2(radius, radius))?;
        let fill = self.comp.brush(base)?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&fill)?;
        visual.Shapes()?.Append(&shape)?;
        self.place(root, &visual, rect.x, rect.y)?;
        Ok(fill)
    }

    fn hit_button(
        &self,
        content: &mut Content,
        rect: Rect,
        hit: Hit,
        fill: windows::UI::Composition::CompositionColorBrush,
        base: Color,
        hover: Color,
    ) {
        content.hits.push((rect, hit.clone()));
        content.buttons.push(Button {
            fill,
            rect,
            hit,
            base,
            hover,
        });
    }
}
