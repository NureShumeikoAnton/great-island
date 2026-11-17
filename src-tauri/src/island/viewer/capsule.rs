//! Viewer and editor capsule button: a rounded backing with an icon, a label or a colored dot on it.
//! Lightens under the cursor, shrinks on press, pale when unavailable. [`Panel`] is a row of such
//! capsules with hover and press.

use super::super::comp::{Comp, Motion, TextStyle, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::super::log;
use crate::lang::tr;
use crate::shot::edit::Area;
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionColorBrush, CompositionEllipseGeometry, CompositionRoundedRectangleGeometry,
            ContainerVisual, SpriteVisual, Visual,
        },
    },
    core::{Interface, Result},
};

const HOVER: Motion = Motion::curve(150, 0.25, 0.1, 0.25, 1.0);
const LATCH: Motion = Motion::curve(120, 0.22, 1.0, 0.36, 1.0);
const PRESSED: f32 = 0.9;
const DISABLED: f32 = 0.35;
/// The glint on "Save": band width in capsule heights, brightness, slant; it runs across in a fraction
/// of the cycle, the rest of the time the capsule rests.
const GLINT_WIDTH: f32 = 1.2;
const GLINT_ALPHA: f32 = 0.5;
const GLINT_TILT: f32 = 20.0;
const GLINT_RUN: f32 = 0.45;
const GLINT_MS: u32 = 1700;
const SLIDE_OUT: Motion = Motion::curve(180, 0.4, 0.0, 1.0, 1.0);
const SLIDE_IN: Motion = Motion::curve(0, 0.0, 0.0, 0.2, 1.0);
const SLIDE_IN_MS: u32 = 400;
const SLIDE_WAIT: f32 = 0.4;

#[derive(Clone, Copy, PartialEq)]
pub struct Look {
    pub idle: Color,
    pub hover: Color,
}

impl Look {
    pub fn new(hex: &str, alpha: f32, hover_hex: &str, hover_alpha: f32) -> Self {
        Self {
            idle: color(hex, alpha),
            hover: color(hover_hex, hover_alpha),
        }
    }
}

pub enum Face<'a> {
    Glyph(Glyph, f32, Color),
    Text(&'a str, TextStyle),
    Dot(Color, f32),
    Bar(Color, f32),
}

const BAR_LENGTH: f32 = 16.0;

pub struct Capsule {
    /// x, y, width, height: logical window pixels.
    rect: (f32, f32, f32, f32),
    pub body: ContainerVisual,
    fill: CompositionColorBrush,
    back: Visual,
    glint: Option<SpriteVisual>,
    /// A share of the capsule filled from the left (Save while saving).
    progress: Option<SpriteVisual>,
    pub face: Visual,
    dot: Option<(CompositionEllipseGeometry, CompositionColorBrush)>,
    bar: Option<CompositionRoundedRectangleGeometry>,
    look: Look,
    hovered: bool,
    enabled: bool,
}

impl Capsule {
    pub fn new(comp: &Comp, k: f32, (w, h): (f32, f32), face: Face, look: Look) -> Result<Self> {
        let c = &comp.compositor;
        let body = c.CreateContainerVisual()?;
        body.SetSize(v2(w, h))?;
        body.SetCenterPoint(v3(w / 2.0, h / 2.0))?;

        let capsule = c.CreateShapeVisual()?;
        capsule.SetSize(v2(w, h))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(w, h))?;
        geometry.SetCornerRadius(v2(h / 2.0, h / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        let fill = comp.brush(look.idle)?;
        shape.SetFillBrush(&fill)?;
        capsule.Shapes()?.Append(&shape)?;
        body.Children()?.InsertAtTop(&capsule)?;

        let (mut dot, mut bar) = (None, None);
        let face: Visual = match face {
            Face::Glyph(glyph, size, ink) => {
                let icon = comp.glyph(glyph, size, ink, k)?;
                icon.SetOffset(v3((w - size) / 2.0, (h - size) / 2.0))?;
                icon.cast()?
            }
            Face::Text(text, style) => {
                let label = comp.text(text, style, k)?;
                let (x, y) = ((w - label.width) / 2.0, h / 2.0 - comp.cap_middle(style));
                label
                    .visual
                    .SetOffset(v3((x * k).round() / k, (y * k).round() / k))?;
                label.visual.cast()?
            }
            Face::Dot(ink, size) => {
                let visual = c.CreateShapeVisual()?;
                visual.SetSize(v2(w, h))?;
                let geometry = c.CreateEllipseGeometry()?;
                geometry.SetCenter(v2(w / 2.0, h / 2.0))?;
                geometry.SetRadius(v2(size / 2.0, size / 2.0))?;
                let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
                let brush = comp.brush(ink)?;
                shape.SetFillBrush(&brush)?;
                // A thin light rim: a black dot is visible on a dark capsule too.
                shape.SetStrokeBrush(&comp.brush(color("#FFFFFF", 0.45))?)?;
                shape.SetStrokeThickness(1.0)?;
                visual.Shapes()?.Append(&shape)?;
                dot = Some((geometry, brush));
                visual.cast()?
            }
            Face::Bar(ink, size) => {
                let visual = c.CreateShapeVisual()?;
                visual.SetSize(v2(w, h))?;
                let geometry = c.CreateRoundedRectangleGeometry()?;
                geometry.SetOffset(v2((w - BAR_LENGTH) / 2.0, (h - size) / 2.0))?;
                geometry.SetSize(v2(BAR_LENGTH, size))?;
                geometry.SetCornerRadius(v2(size / 2.0, size / 2.0))?;
                let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
                shape.SetFillBrush(&comp.brush(ink)?)?;
                visual.Shapes()?.Append(&shape)?;
                bar = Some(geometry);
                visual.cast()?
            }
        };
        body.Children()?.InsertAtTop(&face)?;
        Ok(Self {
            rect: (0.0, 0.0, w, h),
            body,
            fill,
            back: capsule.cast()?,
            glint: None,
            progress: None,
            face,
            dot,
            bar,
            look,
            hovered: false,
            enabled: true,
        })
    }

    pub fn text_width(comp: &Comp, text: &str, style: TextStyle, h: f32) -> f32 {
        (comp.measure(text, style) + h * 1.2).ceil()
    }

    pub fn place(&mut self, x: f32, y: f32) {
        self.rect.0 = x;
        self.rect.1 = y;
        log(self.body.SetOffset(v3(x, y)));
    }

    /// Like [`Capsule::place`], moving there smoothly.
    pub fn glide(&mut self, comp: &Comp, x: f32, y: f32, motion: Motion) {
        self.rect.0 = x;
        self.rect.1 = y;
        log(comp.animate_vector3(&self.body, "Offset", v3(x, y), motion));
    }

    pub fn rect(&self) -> (f32, f32, f32, f32) {
        self.rect
    }

    pub fn area(&self) -> Area {
        let (x, y, w, h) = self.rect;
        Area { x, y, w, h }
    }

    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        let (left, top, w, h) = self.rect;
        x >= left && x <= left + w && y >= top && y <= top + h
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.enabled = enabled;
            log(self.body.SetOpacity(if enabled { 1.0 } else { DISABLED }));
        }
    }

    pub fn set_hover(&mut self, comp: &Comp, on: bool) {
        if on != self.hovered {
            self.hovered = on;
            let target = if on { self.look.hover } else { self.look.idle };
            log(comp.animate_color(&self.fill, target, HOVER));
        }
    }

    pub fn set_look(&mut self, comp: &Comp, look: Look) {
        if look != self.look {
            self.look = look;
            let target = if self.hovered { look.hover } else { look.idle };
            log(comp.animate_color(&self.fill, target, HOVER));
        }
    }

    pub fn squeeze(&self, comp: &Comp, pressed: bool) {
        let k = if pressed { PRESSED } else { 1.0 };
        log(comp.animate_vector3(&self.body, "Scale", scale(k, k), LATCH));
    }

    pub fn set_dot(&self, comp: &Comp, ink: Color) {
        if let Some((_, brush)) = &self.dot {
            log(comp.animate_color(brush, ink, HOVER));
        }
    }

    pub fn set_bar(&self, comp: &Comp, size: f32) {
        let Some(geometry) = &self.bar else {
            return;
        };
        let (_, _, w, h) = self.rect;
        let at = v2((w - BAR_LENGTH) / 2.0, (h - size) / 2.0);
        log(comp.animate_vector2(geometry, "Offset", at, HOVER));
        log(comp.animate_vector2(geometry, "Size", v2(BAR_LENGTH, size), HOVER));
        log(comp.animate_vector2(geometry, "CornerRadius", v2(size / 2.0, size / 2.0), HOVER));
    }

    /// Fills the capsule from the left by `share` in `ink`; `None` takes the fill away.
    pub fn set_progress(&mut self, comp: &Comp, share: Option<f32>, ink: Color) {
        let Some(share) = share else {
            if let Some(fill) = self.progress.take()
                && let Ok(children) = self.body.Children()
            {
                let _ = children.Remove(&fill);
            }
            return;
        };
        let (_, _, w, h) = self.rect;
        if self.progress.is_none() {
            let made = (|| -> Result<SpriteVisual> {
                let c = &comp.compositor;
                let shape = c.CreateRoundedRectangleGeometry()?;
                shape.SetSize(v2(w, h))?;
                shape.SetCornerRadius(v2(h / 2.0, h / 2.0))?;
                self.body
                    .SetClip(&c.CreateGeometricClipWithGeometry(&shape)?)?;
                let fill = c.CreateSpriteVisual()?;
                fill.SetBrush(&comp.brush(ink)?)?;
                fill.SetSize(v2(0.0, h))?;
                self.body.Children()?.InsertAbove(&fill, &self.back)?;
                Ok(fill)
            })();
            match made {
                Ok(fill) => self.progress = Some(fill),
                Err(e) => log(Err(e)),
            }
        }
        if let Some(fill) = &self.progress {
            log(comp.animate_vector2(fill, "Size", v2(w * share.clamp(0.0, 1.0), h), HOVER));
        }
    }

    pub fn shine(&mut self, comp: &Comp, on: bool) {
        match (on, &self.glint) {
            (true, None) => match self.make_glint(comp) {
                Ok(glint) => self.glint = Some(glint),
                Err(e) => crate::journal::warn(tr!("journal.glint_failed", error = e)),
            },
            (false, Some(glint)) => {
                if let Ok(children) = self.body.Children() {
                    let _ = children.Remove(glint);
                }
                self.glint = None;
            }
            _ => {}
        }
    }

    fn make_glint(&self, comp: &Comp) -> Result<SpriteVisual> {
        let c = &comp.compositor;
        let (_, _, w, h) = self.rect;
        let shape = c.CreateRoundedRectangleGeometry()?;
        shape.SetSize(v2(w, h))?;
        shape.SetCornerRadius(v2(h / 2.0, h / 2.0))?;
        self.body
            .SetClip(&c.CreateGeometricClipWithGeometry(&shape)?)?;
        // A slanted band of light taller than the capsule: the slant doesn't expose its ends.
        let band = h * GLINT_WIDTH;
        let glint = c.CreateSpriteVisual()?;
        glint.SetBrush(&comp.glint(GLINT_ALPHA)?)?;
        glint.SetSize(v2(band, h * 2.0))?;
        glint.SetCenterPoint(v3(band / 2.0, h))?;
        glint.SetRotationAngleInDegrees(GLINT_TILT)?;
        glint.SetOffset(v3(-band * 1.5, -h / 2.0))?;
        self.body.Children()?.InsertAbove(&glint, &self.back)?;
        let ease = Motion::curve(0, 0.45, 0.0, 0.3, 1.0);
        let wait = Motion::linear(0);
        comp.loop_scalar(
            &glint,
            "Offset.X",
            &[
                (0.0, -band * 1.5, wait),
                (GLINT_RUN, w + band * 0.5, ease),
                (1.0, w + band * 0.5, wait),
            ],
            GLINT_MS,
        )?;
        Ok(glint)
    }
}

pub struct Panel<K> {
    pub root: ContainerVisual,
    items: Vec<(K, Capsule)>,
    hovered: Option<K>,
    pressed: Option<K>,
}

impl<K: Copy + PartialEq> Panel<K> {
    pub fn new(comp: &Comp) -> Result<Self> {
        Ok(Self {
            root: comp.container()?,
            items: Vec::new(),
            hovered: None,
            pressed: None,
        })
    }

    pub fn add(&mut self, key: K, capsule: Capsule) -> Result<()> {
        self.root.Children()?.InsertAtTop(&capsule.body)?;
        self.items.push((key, capsule));
        Ok(())
    }

    /// Puts `capsule` in the place of the one under `key` (a new face); hover carries over.
    pub fn replace(&mut self, comp: &Comp, key: K, mut capsule: Capsule) -> Result<()> {
        let Some(slot) = self.items.iter_mut().find(|(k, _)| *k == key) else {
            return self.add(key, capsule);
        };
        let (x, y, _, _) = slot.1.rect();
        capsule.place(x, y);
        capsule.set_enabled(slot.1.enabled());
        capsule.set_hover(comp, self.hovered == Some(key));
        self.root
            .Children()?
            .InsertAbove(&capsule.body, &slot.1.body)?;
        self.root.Children()?.Remove(&slot.1.body)?;
        slot.1 = capsule;
        Ok(())
    }

    pub fn get(&self, key: K) -> Option<&Capsule> {
        self.items.iter().find(|(k, _)| *k == key).map(|(_, c)| c)
    }

    pub fn get_mut(&mut self, key: K) -> Option<&mut Capsule> {
        self.items
            .iter_mut()
            .find(|(k, _)| *k == key)
            .map(|(_, c)| c)
    }

    pub fn items_mut(&mut self) -> impl Iterator<Item = (K, &mut Capsule)> {
        self.items.iter_mut().map(|(k, c)| (*k, c))
    }

    pub fn at(&self, p: (f32, f32)) -> Option<K> {
        self.items
            .iter()
            .find(|(_, c)| c.contains(p))
            .map(|(k, _)| *k)
    }

    pub fn enabled_at(&self, p: (f32, f32)) -> Option<K> {
        self.items
            .iter()
            .find(|(_, c)| c.enabled() && c.contains(p))
            .map(|(k, _)| *k)
    }

    pub fn hover(&mut self, comp: &Comp, p: Option<(f32, f32)>) {
        let now = p.and_then(|p| self.enabled_at(p));
        if now == self.hovered {
            return;
        }
        for (key, capsule) in &mut self.items {
            capsule.set_hover(comp, Some(*key) == now);
        }
        self.hovered = now;
    }

    pub fn press(&mut self, comp: &Comp, p: (f32, f32)) -> bool {
        let Some(key) = self.enabled_at(p) else {
            return false;
        };
        self.pressed = Some(key);
        if let Some(capsule) = self.get(key) {
            capsule.squeeze(comp, true);
        }
        true
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed.is_some()
    }

    pub fn pressed(&self) -> Option<K> {
        self.pressed
    }

    pub fn release(&mut self, comp: &Comp, p: (f32, f32)) -> Option<K> {
        let key = self.pressed.take()?;
        if let Some(capsule) = self.get(key) {
            capsule.squeeze(comp, false);
        }
        (self.enabled_at(p) == Some(key)).then_some(key)
    }

    pub fn cancel(&mut self, comp: &Comp) {
        if let Some(key) = self.pressed.take()
            && let Some(capsule) = self.get(key)
        {
            capsule.squeeze(comp, false);
        }
    }

    pub fn bounds(&self, which: impl Fn(K) -> bool) -> Option<Area> {
        self.items
            .iter()
            .filter(|(k, _)| which(*k))
            .map(|(_, c)| c.area())
            .reduce(Area::union)
    }

    pub fn slide(&self, comp: &Comp, shown: bool, distance: f32) {
        let away = v3(0.0, -distance);
        if shown {
            let wait = Motion::linear(0);
            log(comp.path_vector3(
                &self.root,
                "Offset",
                &[
                    (0.0, away, wait),
                    (SLIDE_WAIT, away, wait),
                    (1.0, v3(0.0, 0.0), SLIDE_IN),
                ],
                SLIDE_IN_MS,
            ));
            log(comp.keyframes(
                &self.root,
                "Opacity",
                &[(0.0, 0.0), (SLIDE_WAIT, 0.0), (1.0, 1.0)],
                SLIDE_IN_MS,
            ));
        } else {
            log(comp.animate_vector3(&self.root, "Offset", away, SLIDE_OUT));
            log(comp.animate_scalar(&self.root, "Opacity", 0.0, SLIDE_OUT));
        }
    }

    pub fn set_shown(&self, comp: &Comp, shown: bool, distance: f32) {
        log(comp.stop(&self.root, "Offset"));
        log(comp.stop(&self.root, "Opacity"));
        log(self
            .root
            .SetOffset(v3(0.0, if shown { 0.0 } else { -distance })));
        log(self.root.SetOpacity(if shown { 1.0 } else { 0.0 }));
    }
}
