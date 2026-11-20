//! What a dialog's card is made of: its visuals, built from the sheet and its layout, and what
//! changes in place (hover, the notes' scroll, the progress). The colors and timings are the update
//! window's CSS.

use super::super::comp::{Comp, Motion, color, snap, v2, v3};
use super::super::paragraph::{Para, Paragraph};
use super::icon;
use crate::dialog::{
    Block, Line, Sheet,
    layout::{self, Layout, Placed, Rect, SCROLLBAR, Style},
    scroll::Scroll,
};
use std::borrow::Cow;
use windows::{
    UI::Composition::{
        CompositionColorBrush, CompositionRoundedRectangleGeometry, ContainerVisual, ShapeVisual,
        SpriteVisual, Visual,
    },
    core::{Interface, Result},
};

const TEXT: &str = "#F5F5F7";
const MUTED: &str = "#8E8E93";
const NOTE: &str = "#D1D1D6";
const NOTES_BACK: &str = "#0B0B0C";
/// Fill at rest and under the pointer.
const BUTTON: (&str, &str) = ("#2C2C2E", "#3A3A3C");
const MAIN: (&str, &str) = ("#0A84FF", "#2F95FF");
const MAIN_LABEL: &str = "#FFFFFF";
const BAR_BACK: &str = "#2C2C2E";
const BAR_FILL: &str = "#0A84FF";
/// The thumb's white: at rest, with the pointer over the notes, held.
const THUMB: [f32; 3] = [0.14, 0.28, 0.42];
/// Drawn inside its 12 px slot on every side.
const THUMB_INSET: f32 = 4.0;
const BORDER: f32 = 0.08;
const FADE: f32 = 18.0;
/// The bar under way: 35% of it slides to the end and back.
const RUNNING: f32 = 0.35;
const SLIDE_MS: u32 = 1400;
/// CSS's `ease` and `ease-in-out`.
const EASE: Motion = Motion::curve(200, 0.25, 0.1, 0.25, 1.0);
const EASE_IN_OUT: Motion = Motion::curve(0, 0.42, 0.0, 0.58, 1.0);
const GROW: Motion = Motion::linear(320);
const SCROLL: Motion = Motion::curve(180, 0.0, 0.0, 0.58, 1.0);

pub(super) fn para(style: Style) -> Para {
    let p = |size, weight, hex, line| Para {
        size,
        weight,
        color: color(hex, 1.0),
        line,
        tracking: 0.0,
        tabular: false,
    };
    match style {
        Style::Title => p(17.0, 700, TEXT, Some(22.0)),
        Style::Subtitle => Para {
            tabular: true,
            ..p(12.5, 400, MUTED, None)
        },
        Style::Caps => Para {
            tracking: 0.04 * 11.5,
            ..p(11.5, 600, MUTED, Some(18.0))
        },
        Style::Note => p(12.5, 400, NOTE, Some(18.0)),
        Style::Button => p(13.0, 600, TEXT, None),
        Style::Stat => Para {
            tabular: true,
            ..p(12.0, 400, MUTED, None)
        },
        Style::Close => p(20.0, 400, MUTED, Some(20.0)),
    }
}

/// The text as it is drawn and measured: small capitals are capitals.
pub(super) fn shown(text: &str, style: Style) -> Cow<'_, str> {
    match style {
        Style::Caps => Cow::Owned(text.to_uppercase()),
        _ => Cow::Borrowed(text),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spot {
    Close,
    /// A block and the button in it.
    Button(usize, usize),
    /// A notes block: its thumb, its track off the thumb, the rest of it.
    Thumb(usize),
    Track(usize),
    Notes(usize),
}

struct NotesParts {
    content: ContainerVisual,
    edges: (SpriteVisual, SpriteVisual),
    /// Whether each edge is faded now: a fade is started only when that changes.
    faded: (bool, bool),
    thumb: Option<(CompositionRoundedRectangleGeometry, CompositionColorBrush)>,
    scroll: Scroll,
}

struct ProgressParts {
    fill: CompositionRoundedRectangleGeometry,
    width: f32,
    running: bool,
    stat: Option<(Rect, ContainerVisual)>,
}

enum Part {
    Notes(NotesParts),
    Progress(ProgressParts),
    Buttons(Vec<(CompositionColorBrush, bool)>),
}

pub(super) struct Scene {
    pub card: ContainerVisual,
    back: [CompositionRoundedRectangleGeometry; 2],
    close: Option<(Visual, Visual)>,
    parts: Vec<Part>,
    scale: f32,
}

fn place(
    parent: &ContainerVisual,
    visual: &impl Interface,
    (x, y): (f32, f32),
    scale: f32,
) -> Result<()> {
    let visual: Visual = visual.cast()?;
    visual.SetOffset(v3(snap(x, scale), snap(y, scale)))?;
    parent.Children()?.InsertAtTop(&visual)
}

/// A rounded rectangle on a shape visual of `size`; filled when `fill` is given, else outlined.
fn rounded(
    comp: &Comp,
    size: (f32, f32),
    shape: (f32, f32, f32, f32, f32),
    fill: Option<&CompositionColorBrush>,
    stroke: Option<(&CompositionColorBrush, f32)>,
) -> Result<(ShapeVisual, CompositionRoundedRectangleGeometry)> {
    let c = &comp.compositor;
    let visual = c.CreateShapeVisual()?;
    visual.SetSize(v2(size.0, size.1))?;
    let (x, y, w, h, r) = shape;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetOffset(v2(x, y))?;
    geometry.SetSize(v2(w, h))?;
    geometry.SetCornerRadius(v2(r, r))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&geometry)?;
    if let Some(fill) = fill {
        sprite.SetFillBrush(fill)?;
    }
    if let Some((brush, width)) = stroke {
        sprite.SetStrokeBrush(brush)?;
        sprite.SetStrokeThickness(width)?;
    }
    visual.Shapes()?.Append(&sprite)?;
    Ok((visual, geometry))
}

impl Scene {
    pub fn build(
        comp: &Comp,
        sheet: &Sheet,
        l: &Layout,
        card: f32,
        scale: f32,
        scrolls: &[Option<f32>],
    ) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;
        root.SetSize(v2(layout::WIDTH, card))?;

        let black = comp.brush(color("#000000", 1.0))?;
        let (back, fill) = rounded(
            comp,
            (layout::WIDTH, card),
            (0.0, 0.0, layout::WIDTH, card, layout::RADIUS),
            Some(&black),
            None,
        )?;
        root.Children()?.InsertAtTop(&back)?;
        let line = comp.brush(color("#FFFFFF", BORDER))?;
        // CSS's inset 1 px shadow: a line just inside the edge.
        let (edge, outline) = rounded(
            comp,
            (layout::WIDTH, card),
            (
                0.5,
                0.5,
                layout::WIDTH - 1.0,
                card - 1.0,
                layout::RADIUS - 0.5,
            ),
            None,
            Some((&line, 1.0)),
        )?;
        root.Children()?.InsertAtTop(&edge)?;

        let art = icon::app(comp, layout::ICON)?;
        place(&root, &art, (l.icon.x, l.icon.y), scale)?;
        let text = |content: &str, style: Style, width: Option<f32>, hex: Option<&str>| {
            let mut p = para(style);
            if let Some(hex) = hex {
                p.color = color(hex, 1.0);
            }
            comp.paragraph(&shown(content, style), &p, width, scale)
        };
        let title = text(&sheet.title, Style::Title, Some(l.title.w), None)?;
        place(&root, &title.visual, (l.title.x, l.title.y), scale)?;
        if let Some(r) = l.subtitle {
            let sub = text(&sheet.subtitle, Style::Subtitle, Some(r.w), None)?;
            place(&root, &sub.visual, (r.x, r.y), scale)?;
        }
        let close = match l.close {
            Some(r) => {
                let mut pair = Vec::new();
                for hex in [MUTED, TEXT] {
                    let x = text("×", Style::Close, None, Some(hex))?;
                    let at = (r.x + (r.w - x.width) / 2.0, r.y + (r.h - x.height) / 2.0);
                    place(&root, &x.visual, at, scale)?;
                    pair.push(x.visual.cast::<Visual>()?);
                }
                pair[1].SetOpacity(0.0)?;
                Some((pair[0].clone(), pair[1].clone()))
            }
            None => None,
        };

        let mut parts = Vec::new();
        for (i, (block, placed)) in sheet.blocks.iter().zip(&l.blocks).enumerate() {
            let part = match (block, placed) {
                (Block::Notes { title, lines }, Placed::Notes(n)) => {
                    let f = n.frame;
                    let frame = comp.container()?;
                    frame.SetSize(v2(f.w, f.h))?;
                    let clip_shape = c.CreateRoundedRectangleGeometry()?;
                    clip_shape.SetSize(v2(f.w, f.h))?;
                    clip_shape.SetCornerRadius(v2(layout::NOTES_RADIUS, layout::NOTES_RADIUS))?;
                    frame.SetClip(&c.CreateGeometricClipWithGeometry(&clip_shape)?)?;
                    place(&root, &frame, (f.x, f.y), scale)?;
                    let back = c.CreateSpriteVisual()?;
                    back.SetSize(v2(f.w, f.h))?;
                    back.SetBrush(&comp.brush(color(NOTES_BACK, 1.0))?)?;
                    frame.Children()?.InsertAtTop(&back)?;
                    let content = comp.container()?;
                    content.SetSize(v2(f.w, n.content))?;
                    frame.Children()?.InsertAtTop(&content)?;
                    let head = text(title, Style::Caps, Some(n.title.w), None)?;
                    place(&content, &head.visual, (n.title.x, n.title.y), scale)?;
                    for (line, r) in lines.iter().zip(&n.lines) {
                        let (content_text, style) = match line {
                            Line::Heading(t) => (t, Style::Caps),
                            Line::Item(t) | Line::Text(t) => (t, Style::Note),
                        };
                        let shown_line = text(content_text, style, Some(r.w), None)?;
                        place(&content, &shown_line.visual, (r.x, r.y), scale)?;
                        if let Line::Item(_) = line {
                            // An outside list marker: the bullet and a space end where the text begins.
                            let bullet = text("• ", Style::Note, None, None)?;
                            place(&content, &bullet.visual, (r.x - bullet.width, r.y), scale)?;
                        }
                    }
                    let fade = |down: bool| -> Result<SpriteVisual> {
                        let brush = c.CreateLinearGradientBrush()?;
                        brush.SetStartPoint(v2(0.5, 0.0))?;
                        brush.SetEndPoint(v2(0.5, 1.0))?;
                        let (from, to) = if down { (1.0, 0.0) } else { (0.0, 1.0) };
                        let stops = brush.ColorStops()?;
                        for (offset, a) in [(0.0, from), (1.0, to)] {
                            stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(
                                offset,
                                color(NOTES_BACK, a),
                            )?)?;
                        }
                        let v = c.CreateSpriteVisual()?;
                        v.SetSize(v2(f.w - SCROLLBAR, FADE))?;
                        v.SetBrush(&brush)?;
                        v.SetOpacity(0.0)?;
                        Ok(v)
                    };
                    let top = fade(true)?;
                    place(&frame, &top, (0.0, 0.0), scale)?;
                    let bottom = fade(false)?;
                    place(&frame, &bottom, (0.0, f.h - FADE), scale)?;
                    let thumb = if n.scrolls() {
                        let brush = comp.brush(color("#FFFFFF", THUMB[0]))?;
                        let (bar, shape) = rounded(
                            comp,
                            (SCROLLBAR, f.h),
                            (
                                THUMB_INSET,
                                THUMB_INSET,
                                SCROLLBAR - 2.0 * THUMB_INSET,
                                1.0,
                                2.0,
                            ),
                            Some(&brush),
                            None,
                        )?;
                        place(&frame, &bar, (f.w - SCROLLBAR, 0.0), scale)?;
                        Some((shape, brush))
                    } else {
                        None
                    };
                    let mut scroll = Scroll::new(f.h, n.content);
                    scroll.set(scrolls.get(i).copied().flatten().unwrap_or(0.0));
                    let mut notes = NotesParts {
                        content,
                        edges: (top, bottom),
                        faded: (false, false),
                        thumb,
                        scroll,
                    };
                    show_scroll(comp, &mut notes, None)?;
                    Part::Notes(notes)
                }
                (Block::Progress { share, left, right }, Placed::Progress { bar, stat }) => {
                    let track = comp.container()?;
                    track.SetSize(v2(bar.w, bar.h))?;
                    let clip_shape = c.CreateRoundedRectangleGeometry()?;
                    clip_shape.SetSize(v2(bar.w, bar.h))?;
                    clip_shape.SetCornerRadius(v2(bar.h / 2.0, bar.h / 2.0))?;
                    track.SetClip(&c.CreateGeometricClipWithGeometry(&clip_shape)?)?;
                    place(&root, &track, (bar.x, bar.y), scale)?;
                    let back = c.CreateSpriteVisual()?;
                    back.SetSize(v2(bar.w, bar.h))?;
                    back.SetBrush(&comp.brush(color(BAR_BACK, 1.0))?)?;
                    track.Children()?.InsertAtTop(&back)?;
                    let blue = comp.brush(color(BAR_FILL, 1.0))?;
                    let width = share.map_or(bar.w * RUNNING, |s| bar.w * s);
                    let (fill_visual, fill) = rounded(
                        comp,
                        (bar.w, bar.h),
                        (0.0, 0.0, width, bar.h, bar.h / 2.0),
                        Some(&blue),
                        None,
                    )?;
                    track.Children()?.InsertAtTop(&fill_visual)?;
                    if share.is_none() {
                        let end = bar.w * (1.0 - RUNNING);
                        comp.loop_scalar(
                            &fill_visual,
                            "Offset.X",
                            &[
                                (0.0, 0.0, EASE_IN_OUT),
                                (0.5, end, EASE_IN_OUT),
                                (1.0, 0.0, EASE_IN_OUT),
                            ],
                            2 * SLIDE_MS,
                        )?;
                    }
                    let mut progress = ProgressParts {
                        fill,
                        width: bar.w,
                        running: share.is_none(),
                        stat: None,
                    };
                    if let Some(r) = stat {
                        let holder = comp.container()?;
                        root.Children()?.InsertAtTop(&holder)?;
                        progress.stat = Some((*r, holder));
                        stat_texts(comp, &progress, left, right, scale)?;
                    }
                    Part::Progress(progress)
                }
                (Block::Buttons(buttons), Placed::Buttons(rects)) => {
                    let mut fills = Vec::new();
                    for (b, r) in buttons.iter().zip(rects) {
                        let hex = if b.primary { MAIN.0 } else { BUTTON.0 };
                        let brush = comp.brush(color(hex, 1.0))?;
                        let (pill, _) = rounded(
                            comp,
                            (r.w, r.h),
                            (0.0, 0.0, r.w, r.h, r.h / 2.0),
                            Some(&brush),
                            None,
                        )?;
                        place(&root, &pill, (r.x, r.y), scale)?;
                        let label_hex = if b.primary { MAIN_LABEL } else { TEXT };
                        let label = text(&b.label, Style::Button, None, Some(label_hex))?;
                        let at = (
                            r.x + (r.w - label.width) / 2.0,
                            r.y + (r.h - label.height) / 2.0,
                        );
                        place(&root, &label.visual, at, scale)?;
                        fills.push((brush, b.primary));
                    }
                    Part::Buttons(fills)
                }
                _ => continue,
            };
            parts.push(part);
        }
        Ok(Self {
            card: root,
            back: [fill, outline],
            close,
            parts,
            scale,
        })
    }

    pub fn set_card(&self, card: f32) -> Result<()> {
        self.card.SetSize(v2(layout::WIDTH, card))?;
        self.back[0].SetSize(v2(layout::WIDTH, card))?;
        self.back[1].SetSize(v2(layout::WIDTH - 1.0, card - 1.0))
    }

    /// The notes' scroll per block, kept over a rebuild.
    pub fn scrolls(&self) -> Vec<Option<f32>> {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Notes(n) => Some(n.scroll.offset),
                _ => None,
            })
            .collect()
    }

    pub fn scroll(&self, block: usize) -> Option<Scroll> {
        match self.parts.get(block) {
            Some(Part::Notes(n)) => Some(n.scroll),
            _ => None,
        }
    }

    /// `animate`: a wheel notch or a page glides; a dragged thumb follows at once.
    pub fn set_scroll(
        &mut self,
        comp: &Comp,
        block: usize,
        scroll: Scroll,
        animate: bool,
    ) -> Result<()> {
        if let Some(Part::Notes(n)) = self.parts.get_mut(block) {
            n.scroll = scroll;
            show_scroll(comp, n, animate.then_some(SCROLL))?;
        }
        Ok(())
    }

    /// The pointer is over `spot`; `notes`: over that notes block; `held`: its thumb is held.
    pub fn hover(
        &self,
        spot: Option<Spot>,
        notes: Option<usize>,
        held: Option<usize>,
    ) -> Result<()> {
        if let Some((rest, lit)) = &self.close {
            let on = spot == Some(Spot::Close);
            rest.SetOpacity(if on { 0.0 } else { 1.0 })?;
            lit.SetOpacity(if on { 1.0 } else { 0.0 })?;
        }
        for (block, part) in self.parts.iter().enumerate() {
            match part {
                Part::Buttons(fills) => {
                    for (i, (brush, primary)) in fills.iter().enumerate() {
                        let (base, lit) = if *primary { MAIN } else { BUTTON };
                        let on = spot == Some(Spot::Button(block, i));
                        brush.SetColor(color(if on { lit } else { base }, 1.0))?;
                    }
                }
                Part::Notes(n) => {
                    if let Some((_, brush)) = &n.thumb {
                        let level = if held == Some(block) {
                            THUMB[2]
                        } else if notes == Some(block) {
                            THUMB[1]
                        } else {
                            THUMB[0]
                        };
                        brush.SetColor(color("#FFFFFF", level))?;
                    }
                }
                Part::Progress(_) => {}
            }
        }
        Ok(())
    }

    /// Only the progress changed: the bar grows to the new share and the line under it is redrawn.
    pub fn set_progress(
        &self,
        comp: &Comp,
        block: usize,
        share: Option<f32>,
        left: &str,
        right: &str,
    ) -> Result<()> {
        let Some(Part::Progress(p)) = self.parts.get(block) else {
            return Ok(());
        };
        if let (Some(share), false) = (share, p.running) {
            comp.animate_vector2(&p.fill, "Size", v2(p.width * share, layout::BAR), GROW)?;
        }
        stat_texts(comp, p, left, right, self.scale)
    }

    pub fn same_progress_kind(&self, block: usize, share: Option<f32>) -> bool {
        matches!(self.parts.get(block), Some(Part::Progress(p)) if p.running == share.is_none())
    }
}

fn stat_texts(comp: &Comp, p: &ProgressParts, left: &str, right: &str, scale: f32) -> Result<()> {
    let Some((r, holder)) = &p.stat else {
        return Ok(());
    };
    holder.Children()?.RemoveAll()?;
    let style = para(Style::Stat);
    if !left.is_empty() {
        let l: Paragraph = comp.paragraph(left, &style, None, scale)?;
        place(holder, &l.visual, (r.x, r.y), scale)?;
    }
    if !right.is_empty() {
        let rt = comp.paragraph(right, &style, None, scale)?;
        place(holder, &rt.visual, (r.x + r.w - rt.width, r.y), scale)?;
    }
    Ok(())
}

fn show_scroll(comp: &Comp, n: &mut NotesParts, motion: Option<Motion>) -> Result<()> {
    let s = n.scroll;
    let at = v3(0.0, -s.offset);
    match motion {
        Some(m) => comp.animate_vector3(&n.content, "Offset", at, m)?,
        None => {
            comp.stop(&n.content, "Offset")?;
            n.content.SetOffset(at)?;
        }
    }
    if let Some((shape, _)) = &n.thumb {
        let (top, len) = s.thumb();
        shape.SetSize(v2(
            SCROLLBAR - 2.0 * THUMB_INSET,
            (len - 2.0 * THUMB_INSET).max(1.0),
        ))?;
        let to = v2(THUMB_INSET, top + THUMB_INSET);
        match motion {
            Some(m) => comp.animate_vector2(shape, "Offset", to, m)?,
            None => {
                comp.stop(shape, "Offset")?;
                shape.SetOffset(to)?;
            }
        }
    }
    let fades = s.fades();
    let was = std::mem::replace(&mut n.faded, fades);
    for (visual, on, before) in [(&n.edges.0, fades.0, was.0), (&n.edges.1, fades.1, was.1)] {
        if on != before {
            comp.animate_scalar(visual, "Opacity", if on { 1.0 } else { 0.0 }, EASE)?;
        }
    }
    Ok(())
}
