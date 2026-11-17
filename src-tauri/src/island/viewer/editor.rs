//! Screenshot editor: a mode of the viewer window. The viewer panel slides up and the editor panel arrives
//! in its place, in three groups:
//! - on the left Back, Undo and Redo;
//! - in the middle the tools (brush, marker, arrow, frame, pixels, crop), color and thickness each with
//!   its own popover;
//! - on the right Save copy and Save.
//!
//! Without the strip (a setting) the capsules lie on the shot and don't get in the way of drawing: if the
//! cursor lingers near them for 3 s they hide, and return when it leaves.
//!
//! Unsaved edits raise no dialogs: the first Esc (or Back, or closing the window) makes white glints run over
//! Save, the second discards the edits.
//!
//! Marks are drawn on one sheet the size of the whole shot: on each change only the affected piece is
//! redrawn, with the same code that writes the file (`paint.rs`).

use super::super::comp::{Comp, Motion, TextStyle, color, v2, v3};
use super::super::glyph::Glyph;
use super::super::log;
use super::capsule::{Capsule, Face, Panel};
use super::{Metrics, TEXT, paint};
use crate::capture::Image;
use crate::lang::tr;
use crate::shot::{
    Target,
    edit::{self, Area, History, Mark, Point},
};
use windows::{
    UI::Composition::{
        CompositionColorBrush, CompositionDrawingSurface, CompositionRoundedRectangleGeometry,
        CompositionSpriteShape, ContainerVisual, ShapeVisual, SpriteVisual,
    },
    Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{KillTimer, SetTimer},
    },
    core::Result,
};

/// Editor timers on the viewer window (the window's own are 1 and 2).
pub const TIMER_HIDE: usize = 3;
pub const TIMER_DISARM: usize = 4;
const HIDE_AFTER_MS: u32 = 3000;
/// "Save" pulses this long; a second Esc within it discards the edits.
const DISARM_MS: u32 = 4000;
const ZONE: f32 = 20.0;
const FADE: Motion = Motion::curve(200, 0.25, 0.1, 0.25, 1.0);

const PALETTE: [&str; 6] = [
    "#FF3B30", "#FFD60A", "#30D158", "#0A84FF", "#FFFFFF", "#000000",
];
/// Brush, arrow and frame thickness are in logical screen pixels; in the file the same number of monitor
/// pixels. The marker is wider.
const SIZES: [f32; 3] = [3.0, 6.0, 12.0];
const MARKER_SIZES: [f32; 3] = [12.0, 20.0, 32.0];
const SIZE_BARS: [f32; 3] = [2.0, 4.0, 7.0];
const COLOR_DOT: f32 = 12.0;
const PIXEL_BLOCK: f32 = 12.0;
/// Shorter than this (in shot pixels) an arrow or frame doesn't count: it was just a click.
const MIN_SHAPE: f32 = 4.0;
/// A new line point is no closer than this to the previous one, in logical screen pixels.
const STEP: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Brush,
    Marker,
    Arrow,
    Frame,
    Pixels,
    Crop,
}

const TOOLS: [(Tool, Glyph); 6] = [
    (Tool::Brush, Glyph::Brush),
    (Tool::Marker, Glyph::Marker),
    (Tool::Arrow, Glyph::Pointer),
    (Tool::Frame, Glyph::Frame),
    (Tool::Pixels, Glyph::Pixels),
    (Tool::Crop, Glyph::Crop),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Back,
    Undo,
    Redo,
    Tool(Tool),
    Color,
    Size,
    SaveCopy,
    Save,
}

impl Key {
    fn group(self) -> usize {
        match self {
            Self::Back | Self::Undo | Self::Redo => 0,
            Self::Tool(_) | Self::Color | Self::Size => 1,
            Self::SaveCopy | Self::Save => 2,
        }
    }
}

const LEFT: [Key; 3] = [Key::Back, Key::Undo, Key::Redo];
const CENTER: [Key; 8] = [
    Key::Tool(Tool::Brush),
    Key::Tool(Tool::Marker),
    Key::Tool(Tool::Arrow),
    Key::Tool(Tool::Frame),
    Key::Tool(Tool::Pixels),
    Key::Tool(Tool::Crop),
    Key::Color,
    Key::Size,
];
const RIGHT: [Key; 2] = [Key::SaveCopy, Key::Save];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Color(usize),
    Size(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The visible part of the shot changed (crop or its undo): fit again.
    Refit,
    Exit,
    Save(Target),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Hand,
    Cross,
}

enum Draft {
    Stroke(Mark),
    Shape { tool: Tool, from: Point, to: Point },
}

/// Selection for pixels and crop: a frame, and around a crop a dimming. For pixels the frame is dark yellow:
/// white isn't visible on light.
struct Selection {
    root: ContainerVisual,
    dims: [SpriteVisual; 4],
    fill: SpriteVisual,
    outline: CompositionRoundedRectangleGeometry,
    stroke: CompositionSpriteShape,
    white: CompositionColorBrush,
    amber: CompositionColorBrush,
}

const SELECTION_AMBER: &str = "#D99A00";
const SELECTION_TINT: &str = "#FFD60A";

impl Selection {
    fn new(comp: &Comp, full: (u32, u32)) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;
        root.SetOpacity(0.0)?;
        let sprite = |hex: &str, alpha: f32| -> Result<SpriteVisual> {
            let s = c.CreateSpriteVisual()?;
            s.SetBrush(&comp.brush(color(hex, alpha))?)?;
            root.Children()?.InsertAtTop(&s)?;
            Ok(s)
        };
        let dims = [
            sprite("#000000", 0.55)?,
            sprite("#000000", 0.55)?,
            sprite("#000000", 0.55)?,
            sprite("#000000", 0.55)?,
        ];
        let fill = sprite(SELECTION_TINT, 0.14)?;
        let shapes: ShapeVisual = c.CreateShapeVisual()?;
        shapes.SetSize(v2(full.0 as f32, full.1 as f32))?;
        let outline = c.CreateRoundedRectangleGeometry()?;
        let stroke = c.CreateSpriteShapeWithGeometry(&outline)?;
        shapes.Shapes()?.Append(&stroke)?;
        root.Children()?.InsertAtTop(&shapes)?;
        Ok(Self {
            root,
            dims,
            fill,
            outline,
            stroke,
            white: comp.brush(color("#FFFFFF", 0.95))?,
            amber: comp.brush(color(SELECTION_AMBER, 1.0))?,
        })
    }

    fn show(&self, a: Area, v: Area, crop: bool, zoom: f32) -> Result<()> {
        let place = |s: &SpriteVisual, x: f32, y: f32, w: f32, h: f32| -> Result<()> {
            s.SetOffset(v3(x, y))?;
            s.SetSize(v2(w.max(0.0), h.max(0.0)))
        };
        let [top, bottom, left, right] = &self.dims;
        if crop {
            place(top, v.x, v.y, v.w, a.y - v.y)?;
            place(bottom, v.x, a.bottom(), v.w, v.bottom() - a.bottom())?;
            place(left, v.x, a.y, a.x - v.x, a.h)?;
            place(right, a.right(), a.y, v.right() - a.right(), a.h)?;
            place(&self.fill, 0.0, 0.0, 0.0, 0.0)?;
        } else {
            for dim in &self.dims {
                place(dim, 0.0, 0.0, 0.0, 0.0)?;
            }
            place(&self.fill, a.x, a.y, a.w, a.h)?;
        }
        // A frame of one thickness on screen at any zoom.
        self.outline.SetOffset(v2(a.x, a.y))?;
        self.outline.SetSize(v2(a.w, a.h))?;
        let (brush, width) = if crop {
            (&self.white, 1.5)
        } else {
            (&self.amber, 2.0)
        };
        self.stroke.SetStrokeBrush(brush)?;
        self.stroke.SetStrokeThickness(width / zoom)?;
        self.root.SetOpacity(1.0)
    }

    fn hide(&self) -> Result<()> {
        self.root.SetOpacity(0.0)
    }
}

struct Popover {
    owner: Key,
    panel: Panel<Pick>,
    rect: (f32, f32, f32, f32),
}

impl Popover {
    fn contains(&self, (x, y): (f32, f32)) -> bool {
        let (left, top, w, h) = self.rect;
        x >= left && x <= left + w && y >= top && y <= top + h
    }
}

pub struct Editor {
    hwnd: HWND,
    /// Where the panel and popover hang (the window root) and where the marks lie (the shot canvas).
    host: ContainerVisual,
    canvas: ContainerVisual,
    m: Metrics,
    k: f32,
    width: f32,
    panel: Panel<Key>,
    popover: Option<Popover>,
    tool: Tool,
    color: usize,
    size: usize,
    history: History<Mark>,
    full: (u32, u32),
    sheet: CompositionDrawingSurface,
    sheet_visual: SpriteVisual,
    selection: Selection,
    draft: Option<Draft>,
    drawn: Option<Area>,
    armed: bool,
    hidden: bool,
    in_zone: bool,
}

impl Editor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        comp: &Comp,
        hwnd: HWND,
        host: &ContainerVisual,
        canvas: &ContainerVisual,
        (k, m): (f32, Metrics),
        width: f32,
        full: (u32, u32),
    ) -> Result<Self> {
        let (sheet, sheet_visual) = comp.sheet(full.0, full.1)?;
        canvas.Children()?.InsertAtTop(&sheet_visual)?;
        let selection = Selection::new(comp, full)?;
        canvas.Children()?.InsertAtTop(&selection.root)?;
        let mut editor = Self {
            hwnd,
            host: host.clone(),
            canvas: canvas.clone(),
            m,
            k,
            width,
            panel: Panel::new(comp)?,
            popover: None,
            tool: Tool::Brush,
            color: 0,
            size: 1,
            history: History::default(),
            full,
            sheet,
            sheet_visual,
            selection,
            draft: None,
            drawn: None,
            armed: false,
            hidden: false,
            in_zone: false,
        };
        editor.build_panel(comp)?;
        editor.panel.set_shown(comp, false, m.slide_distance());
        editor.panel.slide(comp, true, m.slide_distance());
        Ok(editor)
    }

    /// The panel leaves: the editor closes. Returns it to be removed after the animation.
    pub fn retire(&self, comp: &Comp) -> ContainerVisual {
        self.panel.slide(comp, false, self.m.slide_distance());
        self.panel.root.clone()
    }

    pub fn visible(&self) -> Area {
        edit::visible(
            self.history.done(),
            Area {
                x: 0.0,
                y: 0.0,
                w: self.full.0 as f32,
                h: self.full.1 as f32,
            },
        )
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub fn flatten(&self, image: &Image) -> Result<Image> {
        let marks: Vec<&Mark> = self.history.done().iter().collect();
        paint::flatten(image, &marks)
    }

    fn build_panel(&mut self, comp: &Comp) -> Result<()> {
        let (m, k) = (self.m, self.k);
        let ink = color(TEXT, 1.0);
        let icon = |key: Key, g: Glyph, look| -> Result<(Key, Capsule)> {
            let face = Face::Glyph(g, m.icon, ink);
            Ok((key, Capsule::new(comp, k, m.button, face, look)?))
        };
        let mut capsules = vec![
            icon(Key::Back, Glyph::Back, m.plain())?,
            icon(Key::Undo, Glyph::Undo, m.plain())?,
            icon(Key::Redo, Glyph::Redo, m.plain())?,
        ];
        for (tool, g) in TOOLS {
            let look = if tool == self.tool {
                m.selected()
            } else {
                m.plain()
            };
            capsules.push(icon(Key::Tool(tool), g, look)?);
        }
        let dot = Face::Dot(color(PALETTE[self.color], 1.0), COLOR_DOT);
        capsules.push((Key::Color, Capsule::new(comp, k, m.button, dot, m.plain())?));
        let bar = Face::Bar(ink, SIZE_BARS[self.size]);
        capsules.push((Key::Size, Capsule::new(comp, k, m.button, bar, m.plain())?));
        let h = m.button.1;
        let label = |text: &str, bold: bool| -> Result<Capsule> {
            let style = TextStyle {
                size: m.text - 0.5,
                bold,
                color: ink,
                tabular: false,
            };
            let w = Capsule::text_width(comp, text, style, h);
            let look = if bold { m.accent() } else { m.plain() };
            Capsule::new(comp, k, (w, h), Face::Text(text, style), look)
        };
        capsules.push((Key::SaveCopy, label(&tr!("viewer.save_copy"), false)?));
        capsules.push((Key::Save, label(&tr!("viewer.save"), true)?));

        let mut panel = Panel::new(comp)?;
        for (key, capsule) in capsules {
            panel.add(key, capsule)?;
        }
        let _ = self.host.Children()?.Remove(&self.panel.root);
        self.host.Children()?.InsertAtTop(&panel.root)?;
        self.panel = panel;
        self.hidden = false;
        self.sync_history();
        self.layout();
        Ok(())
    }

    fn group_gap(&self) -> f32 {
        self.m.gap * 3.0
    }

    fn space_before(&self, key: Key) -> f32 {
        if key == Key::Color {
            self.group_gap()
        } else {
            self.m.gap
        }
    }

    fn span(&self, keys: &[Key]) -> f32 {
        keys.iter()
            .enumerate()
            .filter_map(|(i, &key)| {
                let gap = if i > 0 { self.space_before(key) } else { 0.0 };
                self.panel.get(key).map(|c| gap + c.rect().2)
            })
            .sum()
    }

    pub fn min_width(&self) -> f32 {
        let groups = self.span(&LEFT) + self.span(&CENTER) + self.span(&RIGHT);
        (groups + self.group_gap() * 2.0 + self.m.margin * 2.0).ceil()
    }

    /// Three groups: Back and history on the left, tools in the middle, saving on the right.
    /// The middle one is centered in the window but doesn't run into the outer ones.
    pub fn layout(&mut self) {
        let m = self.m;
        let y = m.button_top();
        let group = self.group_gap();
        let gap = m.gap;
        let space = |key: Key| if key == Key::Color { group } else { gap };
        let right = self.span(&RIGHT);
        let center = self.span(&CENTER);
        let row = |panel: &mut Panel<Key>, keys: &[Key], mut x: f32| -> f32 {
            for (i, &key) in keys.iter().enumerate() {
                if i > 0 {
                    x += space(key);
                }
                if let Some(c) = panel.get_mut(key) {
                    c.place(x, y);
                    x += c.rect().2;
                }
            }
            x
        };
        let left_end = row(&mut self.panel, &LEFT, m.margin);
        let right_start = self.width - m.margin - right;
        row(&mut self.panel, &RIGHT, right_start);
        let x = ((self.width - center) / 2.0)
            .min(right_start - group - center)
            .max(left_end + group);
        row(&mut self.panel, &CENTER, x);
    }

    pub fn resize(&mut self, width: f32) {
        self.width = width;
        self.layout();
        self.close_popover();
    }

    pub fn restyle(&mut self, comp: &Comp, k: f32, m: Metrics, width: f32) {
        self.k = k;
        self.m = m;
        self.width = width;
        self.close_popover();
        log(self.build_panel(comp));
        self.panel.set_shown(comp, true, m.slide_distance());
    }

    pub fn wants(&self, p: (f32, f32)) -> bool {
        self.popover.as_ref().is_some_and(|pop| pop.contains(p))
            || (!self.hidden && self.panel.at(p).is_some())
    }

    pub fn cursor(&self, p: (f32, f32), on_photo: bool) -> Option<Pointer> {
        if let Some(pop) = &self.popover
            && pop.contains(p)
        {
            return pop.panel.enabled_at(p).map(|_| Pointer::Hand);
        }
        if !self.hidden && self.panel.at(p).is_some() {
            return self.panel.enabled_at(p).map(|_| Pointer::Hand);
        }
        on_photo.then_some(Pointer::Cross)
    }

    fn select(&mut self, comp: &Comp, tool: Tool) {
        let m = self.m;
        if let Some(c) = self.panel.get_mut(Key::Tool(self.tool)) {
            c.set_look(comp, m.plain());
        }
        self.tool = tool;
        if let Some(c) = self.panel.get_mut(Key::Tool(tool)) {
            c.set_look(comp, m.selected());
        }
    }

    fn sync_history(&mut self) {
        let (undo, redo) = (self.history.is_dirty(), self.history.can_redo());
        if let Some(c) = self.panel.get_mut(Key::Undo) {
            c.set_enabled(undo);
        }
        if let Some(c) = self.panel.get_mut(Key::Redo) {
            c.set_enabled(redo);
        }
    }

    fn toggle_popover(&mut self, comp: &Comp, owner: Key) {
        let same = self.popover.as_ref().is_some_and(|pop| pop.owner == owner);
        self.close_popover();
        if !same {
            log(self.open_popover(comp, owner));
        }
    }

    fn open_popover(&mut self, comp: &Comp, owner: Key) -> Result<()> {
        let Some(anchor) = self.panel.get(owner).map(Capsule::rect) else {
            return Ok(());
        };
        let (m, k) = (self.m, self.k);
        let ink = color(TEXT, 1.0);
        let picks: Vec<(Pick, Face, bool)> = match owner {
            Key::Color => PALETTE
                .iter()
                .enumerate()
                .map(|(i, hex)| {
                    let face = Face::Dot(color(hex, 1.0), COLOR_DOT);
                    (Pick::Color(i), face, i == self.color)
                })
                .collect(),
            _ => SIZE_BARS
                .iter()
                .enumerate()
                .map(|(i, &bar)| (Pick::Size(i), Face::Bar(ink, bar), i == self.size))
                .collect(),
        };
        let h = m.button.1;
        let (pad, step) = (5.0, h + 2.0);
        let n = picks.len() as f32;
        let width = 2.0 * pad + n * step - 2.0;
        let x = (anchor.0 + anchor.2 / 2.0 - width / 2.0)
            .min(self.width - m.margin - width)
            .max(m.margin);
        let rect = (x, anchor.1 + h + 8.0, width, h + 2.0 * pad);

        let mut panel = Panel::new(comp)?;
        let c = &comp.compositor;
        let back = c.CreateShapeVisual()?;
        back.SetSize(v2(rect.2, rect.3))?;
        back.SetOffset(v3(rect.0, rect.1))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(rect.2, rect.3))?;
        geometry.SetCornerRadius(v2(rect.3 / 2.0, rect.3 / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&comp.brush(color("#2C2C2E", 0.97))?)?;
        back.Shapes()?.Append(&shape)?;
        panel.root.Children()?.InsertAtBottom(&back)?;
        for (i, (pick, face, chosen)) in picks.into_iter().enumerate() {
            let look = if chosen { m.selected() } else { m.clear() };
            let mut capsule = Capsule::new(comp, k, (h, h), face, look)?;
            capsule.place(rect.0 + pad + i as f32 * step, rect.1 + pad);
            panel.add(pick, capsule)?;
        }
        panel.root.SetOpacity(0.0)?;
        self.host.Children()?.InsertAtTop(&panel.root)?;
        comp.animate_scalar(&panel.root, "Opacity", 1.0, FADE)?;
        self.popover = Some(Popover { owner, panel, rect });
        Ok(())
    }

    fn close_popover(&mut self) {
        if let Some(pop) = self.popover.take()
            && let Ok(children) = self.host.Children()
        {
            let _ = children.Remove(&pop.panel.root);
        }
    }

    fn pick(&mut self, comp: &Comp, pick: Pick) {
        match pick {
            Pick::Color(i) => {
                self.color = i;
                if let Some(c) = self.panel.get(Key::Color) {
                    c.set_dot(comp, color(PALETTE[i], 1.0));
                }
            }
            Pick::Size(i) => {
                self.size = i;
                if let Some(c) = self.panel.get(Key::Size) {
                    c.set_bar(comp, SIZE_BARS[i]);
                }
            }
        }
        self.close_popover();
    }

    /// Left press at window point `p`; `at` is the same in shot pixels if it is on the shot.
    /// True = the editor took the press, the window needs mouse capture.
    pub fn press(&mut self, comp: &Comp, p: (f32, f32), at: Option<Point>) -> bool {
        // Back is the second press of "leave without saving": the release must still see the pulse.
        if self.hidden || self.panel.at(p) != Some(Key::Back) {
            self.disarm(comp);
        }
        if let Some(pop) = &mut self.popover {
            if pop.panel.press(comp, p) || pop.contains(p) {
                return true;
            }
            let owner = pop.owner;
            match self.panel.at(p) {
                Some(key) if key == owner => {}
                Some(_) => self.close_popover(),
                None => {
                    self.close_popover();
                    return true;
                }
            }
        }
        if !self.hidden && self.panel.press(comp, p) {
            return true;
        }
        if !self.hidden && self.panel.at(p).is_some() {
            return true;
        }
        let Some(at) = at else {
            return false;
        };
        let color = hex_rgb(PALETTE[self.color]);
        self.draft = Some(match self.tool {
            Tool::Brush | Tool::Marker => {
                let marker = self.tool == Tool::Marker;
                let sizes = if marker { MARKER_SIZES } else { SIZES };
                Draft::Stroke(Mark::Stroke {
                    points: vec![at],
                    color,
                    width: sizes[self.size] * self.k,
                    marker,
                })
            }
            tool => Draft::Shape {
                tool,
                from: at,
                to: at,
            },
        });
        self.redraw_draft(comp, None, 1.0);
        true
    }

    /// `at` is the shot point, `zoom` logical pixels per shot pixel.
    pub fn moved(&mut self, comp: &Comp, p: (f32, f32), at: Point, zoom: f32) {
        if self.draft.is_some() {
            self.redraw_draft(comp, Some(at), zoom);
            return;
        }
        if let Some(pop) = &mut self.popover {
            pop.panel.hover(comp, Some(p));
        }
        let over_panel = !self.hidden && !self.panel.is_pressed();
        self.panel.hover(comp, over_panel.then_some(p));
        self.track_zone(comp, Some(p));
    }

    pub fn left(&mut self, comp: &Comp, p: Option<(f32, f32)>) {
        self.panel.hover(comp, None);
        if let Some(pop) = &mut self.popover {
            pop.panel.hover(comp, None);
        }
        self.track_zone(comp, p);
    }

    pub fn release(&mut self, comp: &Comp, p: (f32, f32), image: &Image) -> Option<Effect> {
        if let Some(pop) = &mut self.popover
            && let Some(pick) = pop.panel.release(comp, p)
        {
            self.pick(comp, pick);
            return None;
        }
        if let Some(key) = self.panel.release(comp, p) {
            return self.click(comp, key);
        }
        self.commit(comp, image)
    }

    pub fn cancel_press(&mut self, comp: &Comp) {
        self.panel.cancel(comp);
        if let Some(pop) = &mut self.popover {
            pop.panel.cancel(comp);
        }
        self.cancel_draft(comp);
    }

    fn click(&mut self, comp: &Comp, key: Key) -> Option<Effect> {
        if !matches!(key, Key::Color | Key::Size) {
            self.close_popover();
        }
        match key {
            Key::Back => self.leave(comp),
            Key::Tool(tool) => {
                self.select(comp, tool);
                None
            }
            Key::Color | Key::Size => {
                self.toggle_popover(comp, key);
                None
            }
            Key::Undo => self.undo(comp),
            Key::Redo => self.redo(comp),
            Key::SaveCopy => Some(Effect::Save(Target::Copy)),
            Key::Save => Some(self.save_same()),
        }
    }

    fn save_same(&self) -> Effect {
        if self.history.is_dirty() {
            Effect::Save(Target::Same)
        } else {
            Effect::Exit
        }
    }

    pub fn save_key(&mut self, copy: bool) -> Effect {
        if copy {
            Effect::Save(Target::Copy)
        } else {
            self.save_same()
        }
    }

    /// The mark being dragged, as it will lie on the sheet. Pixels and crop are drawn by the selection,
    /// not on the sheet.
    fn draft_mark(&self) -> Option<Mark> {
        let color = hex_rgb(PALETTE[self.color]);
        let width = SIZES[self.size] * self.k;
        match self.draft.as_ref()? {
            Draft::Stroke(mark) => Some(mark.clone()),
            Draft::Shape {
                tool: Tool::Arrow,
                from,
                to,
            } => Some(Mark::Arrow {
                from: *from,
                to: *to,
                color,
                width,
            }),
            Draft::Shape {
                tool: Tool::Frame,
                from,
                to,
            } => Some(Mark::Frame {
                area: Area::between(*from, *to),
                color,
                width,
            }),
            Draft::Shape { .. } => None,
        }
    }

    fn redraw_draft(&mut self, comp: &Comp, to: Option<Point>, zoom: f32) {
        let visible = self.visible();
        let Some(draft) = &mut self.draft else {
            return;
        };
        match draft {
            Draft::Stroke(Mark::Stroke { points, width, .. }) => {
                if let Some(p) = to {
                    let last = points[points.len() - 1];
                    let far = (p.0 - last.0).hypot(p.1 - last.1) >= STEP / zoom;
                    if !far {
                        return;
                    }
                    points.push(p);
                }
                let tail = &points[points.len().saturating_sub(3)..];
                let first = tail[0];
                let area = tail
                    .iter()
                    .fold(Area::between(first, first), |a, &q| {
                        a.union(Area::between(q, q))
                    })
                    .inflate(*width / 2.0 + 2.0);
                self.drawn = Some(self.drawn.map_or(area, |d| d.union(area)));
                self.repaint(comp, area);
            }
            Draft::Stroke(_) => {}
            Draft::Shape {
                tool,
                from,
                to: end,
            } => {
                if let Some(p) = to {
                    *end = p;
                }
                let (tool, area) = (*tool, Area::between(*from, *end));
                match tool {
                    Tool::Pixels | Tool::Crop => {
                        let shown = area.intersect(visible);
                        log(match shown {
                            Some(a) => self.selection.show(a, visible, tool == Tool::Crop, zoom),
                            None => self.selection.hide(),
                        });
                    }
                    _ => {
                        let now = self.draft_mark().and_then(|m| m.bounds());
                        let dirty = match (self.drawn, now) {
                            (Some(a), Some(b)) => Some(a.union(b)),
                            (a, b) => a.or(b),
                        };
                        self.drawn = now;
                        if let Some(dirty) = dirty {
                            self.repaint(comp, dirty);
                        }
                    }
                }
            }
        }
    }

    fn commit(&mut self, comp: &Comp, image: &Image) -> Option<Effect> {
        let mark = self.draft_mark();
        let draft = self.draft.take()?;
        log(self.selection.hide());
        let drawn = self.drawn.take();
        let visible = self.visible();
        let mark = match draft {
            Draft::Stroke(_) => mark,
            Draft::Shape { tool, from, to } => {
                let area = Area::between(from, to);
                let big = area.w.max(area.h) >= MIN_SHAPE;
                match tool {
                    Tool::Arrow | Tool::Frame => mark.filter(|_| big),
                    Tool::Pixels => area
                        .intersect(visible)
                        .and_then(|a| a.pixels(image.width, image.height))
                        .filter(|_| big)
                        .map(|rect| Mark::Pixels {
                            area: Area {
                                x: rect.0 as f32,
                                y: rect.1 as f32,
                                w: rect.2 as f32,
                                h: rect.3 as f32,
                            },
                            patch: edit::pixelate(
                                image,
                                rect,
                                (PIXEL_BLOCK * self.k).round() as u32,
                            ),
                        }),
                    Tool::Crop => area
                        .intersect(visible)
                        .filter(|a| a.w >= 2.0 * MIN_SHAPE && a.h >= 2.0 * MIN_SHAPE)
                        .map(Mark::Crop),
                    Tool::Brush | Tool::Marker => None,
                }
            }
        };
        let Some(mark) = mark else {
            if let Some(area) = drawn {
                self.repaint(comp, area);
            }
            return None;
        };
        let crop = matches!(mark, Mark::Crop(_));
        let region = mark.bounds();
        self.history.push(mark);
        self.sync_history();
        if let Some(area) = [drawn, region].into_iter().flatten().reduce(Area::union) {
            self.repaint(comp, area);
        }
        crop.then_some(Effect::Refit)
    }

    fn cancel_draft(&mut self, comp: &Comp) {
        if self.draft.take().is_none() {
            return;
        }
        log(self.selection.hide());
        if let Some(area) = self.drawn.take() {
            self.repaint(comp, area);
        }
    }

    pub fn undo(&mut self, comp: &Comp) -> Option<Effect> {
        self.disarm(comp);
        let mark = self.history.undo()?.clone();
        self.after_change(comp, &mark)
    }

    pub fn redo(&mut self, comp: &Comp) -> Option<Effect> {
        self.disarm(comp);
        let mark = self.history.redo()?.clone();
        self.after_change(comp, &mark)
    }

    fn after_change(&mut self, comp: &Comp, mark: &Mark) -> Option<Effect> {
        self.sync_history();
        if let Some(area) = mark.bounds() {
            self.repaint(comp, area);
        }
        matches!(mark, Mark::Crop(_)).then_some(Effect::Refit)
    }

    fn repaint(&self, comp: &Comp, area: Area) {
        let Some(rect) = area.pixels(self.full.0, self.full.1) else {
            return;
        };
        let draft = self.draft_mark();
        log(comp.redraw(&self.sheet, rect, |dc| {
            for mark in self.history.done().iter().chain(draft.as_ref()) {
                if mark.bounds().and_then(|b| b.intersect(area)).is_some() {
                    paint::draw(dc, mark)?;
                }
            }
            Ok(())
        }));
    }

    /// Esc: what has been started (draft, popover) is cancelled first, then the exit.
    pub fn escape(&mut self, comp: &Comp) -> Option<Effect> {
        if self.draft.is_some() {
            self.cancel_draft(comp);
            return None;
        }
        if self.popover.is_some() {
            self.close_popover();
            return None;
        }
        self.leave(comp)
    }

    /// Back or Esc: with unsaved edits, "Save" pulses first.
    fn leave(&mut self, comp: &Comp) -> Option<Effect> {
        if self.history.is_dirty() && !self.armed {
            self.arm(comp);
            return None;
        }
        Some(Effect::Exit)
    }

    pub fn may_close(&mut self, comp: &Comp) -> bool {
        if self.history.is_dirty() && !self.armed {
            self.arm(comp);
            return false;
        }
        true
    }

    pub fn save_failed(&mut self, comp: &Comp) {
        self.arm(comp);
    }

    fn arm(&mut self, comp: &Comp) {
        self.armed = true;
        self.show_panel(comp);
        if let Some(save) = self.panel.get_mut(Key::Save) {
            save.shine(comp, true);
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_DISARM, DISARM_MS, None);
        }
    }

    fn disarm(&mut self, comp: &Comp) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        if let Some(save) = self.panel.get_mut(Key::Save) {
            save.shine(comp, false);
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_DISARM);
        }
    }

    pub fn on_timer(&mut self, comp: &Comp, id: usize) {
        match id {
            TIMER_DISARM => self.disarm(comp),
            TIMER_HIDE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_HIDE);
                }
                if self.in_zone && self.popover.is_none() && !self.armed {
                    self.hidden = true;
                    self.panel.hover(comp, None);
                    log(comp.animate_scalar(&self.panel.root, "Opacity", 0.0, FADE));
                }
            }
            _ => {}
        }
    }

    fn track_zone(&mut self, comp: &Comp, p: Option<(f32, f32)>) {
        if !self.m.floating() {
            return;
        }
        let inside = p.is_some_and(|p| self.in_zone_at(p));
        if inside == self.in_zone {
            return;
        }
        self.in_zone = inside;
        unsafe {
            if inside {
                SetTimer(Some(self.hwnd), TIMER_HIDE, HIDE_AFTER_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_HIDE);
            }
        }
        if !inside {
            self.show_panel(comp);
        }
    }

    fn in_zone_at(&self, p: (f32, f32)) -> bool {
        (0..3)
            .filter_map(|group| self.panel.bounds(|k| k.group() == group))
            .any(|zone| {
                let z = zone.inflate(ZONE);
                p.0 >= z.x && p.0 <= z.right() && p.1 >= z.y && p.1 <= z.bottom()
            })
    }

    fn show_panel(&mut self, comp: &Comp) {
        if std::mem::take(&mut self.hidden) {
            log(comp.animate_scalar(&self.panel.root, "Opacity", 1.0, FADE));
        }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_HIDE);
            let _ = KillTimer(Some(self.hwnd), TIMER_DISARM);
        }
        self.close_popover();
        if let Ok(children) = self.canvas.Children() {
            let _ = children.Remove(&self.sheet_visual);
            let _ = children.Remove(&self.selection.root);
        }
    }
}

fn hex_rgb(hex: &str) -> [u8; 3] {
    let c = color(hex, 1.0);
    [c.R, c.G, c.B]
}
