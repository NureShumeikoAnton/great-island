//! Signs on the video window's picture: ▶ or ❚❚ flashes in its middle when the video is played or
//! paused from the picture, the keyboard or the island; "2× ▶▶" stands at its top while a press on
//! it is held; a spinner turns in its middle while the video opens again after the editor.
//! Compositor animations; at rest all are invisible.

use super::super::comp::{Comp, Motion, TextStyle, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::super::log;
use super::lens::Room;
use super::{Metrics, TEXT};
use crate::play::{BOOST, rate_label};
use windows::{
    UI::Composition::{CompositionStrokeCap, ContainerVisual, ShapeVisual, Visual},
    core::{Interface, Result},
};

const FLASH: f32 = 64.0;
const FLASH_ICON: f32 = 26.0;
/// The flash stays for the first part of this, then fades and grows.
const FLASH_MS: u32 = 650;
const FLASH_GROWTH: f32 = 1.2;
const FAST_H: f32 = 32.0;
const FAST_PAD: f32 = 14.0;
const FAST_GAP: f32 = 6.0;
const FAST_ICON: f32 = 14.0;
const FAST_FADE: Motion = Motion::curve(150, 0.25, 0.1, 0.25, 1.0);
const FAST_POP: f32 = 0.9;
const BACK: (&str, f32) = ("#000000", 0.55);

pub struct Signs {
    pub root: ContainerVisual,
    flash: ContainerVisual,
    pause: Visual,
    play: Visual,
    fast: ContainerVisual,
    fast_w: f32,
    pub spinner: Spinner,
}

impl Signs {
    pub fn new(comp: &Comp, k: f32, m: Metrics) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;

        let flash = comp.container()?;
        flash.SetSize(v2(FLASH, FLASH))?;
        flash.SetCenterPoint(v3(FLASH / 2.0, FLASH / 2.0))?;
        flash.SetOpacity(0.0)?;
        let circle = c.CreateEllipseGeometry()?;
        circle.SetCenter(v2(FLASH / 2.0, FLASH / 2.0))?;
        circle.SetRadius(v2(FLASH / 2.0, FLASH / 2.0))?;
        flash
            .Children()?
            .InsertAtTop(&backing(comp, &circle.cast()?, (FLASH, FLASH))?)?;
        let inset = (FLASH - FLASH_ICON) / 2.0;
        let icon = |glyph: Glyph| -> Result<Visual> {
            let icon = comp.glyph(glyph, FLASH_ICON, color(TEXT, 1.0), k)?;
            icon.SetOffset(v3(inset, inset))?;
            flash.Children()?.InsertAtTop(&icon)?;
            icon.cast()
        };
        let pause = icon(Glyph::Pause)?;
        let play = icon(Glyph::Play)?;
        root.Children()?.InsertAtTop(&flash)?;

        let style = TextStyle {
            size: m.text,
            bold: true,
            color: color(TEXT, 1.0),
            tabular: false,
        };
        let label = comp.text(&rate_label(BOOST), style, k)?;
        let fast_w = FAST_PAD + label.width + FAST_GAP + FAST_ICON + FAST_PAD;
        let fast = comp.container()?;
        fast.SetSize(v2(fast_w, FAST_H))?;
        fast.SetCenterPoint(v3(fast_w / 2.0, FAST_H / 2.0))?;
        fast.SetOpacity(0.0)?;
        let pill = c.CreateRoundedRectangleGeometry()?;
        pill.SetSize(v2(fast_w, FAST_H))?;
        pill.SetCornerRadius(v2(FAST_H / 2.0, FAST_H / 2.0))?;
        fast.Children()?
            .InsertAtTop(&backing(comp, &pill.cast()?, (fast_w, FAST_H))?)?;
        let snap = |v: f32| (v * k).round() / k;
        label.visual.SetOffset(v3(
            snap(FAST_PAD),
            snap(FAST_H / 2.0 - comp.cap_middle(style)),
        ))?;
        fast.Children()?.InsertAtTop(&label.visual)?;
        let arrows = comp.glyph(Glyph::Forward, FAST_ICON, color(TEXT, 1.0), k)?;
        arrows.SetOffset(v3(
            snap(FAST_PAD + label.width + FAST_GAP),
            snap((FAST_H - FAST_ICON) / 2.0),
        ))?;
        fast.Children()?.InsertAtTop(&arrows)?;
        root.Children()?.InsertAtTop(&fast)?;
        let spinner = Spinner::new(comp)?;
        root.Children()?.InsertAtTop(&spinner.root)?;

        Ok(Self {
            root,
            flash,
            pause,
            play,
            fast,
            fast_w,
            spinner,
        })
    }

    /// The flash and the spinner in the middle of `area`, the 2× sign at `top` above its middle.
    pub fn layout(&self, area: Room, top: f32) -> Result<()> {
        let middle = area.0 + area.2 / 2.0;
        let center = area.1 + area.3 / 2.0;
        self.flash
            .SetOffset(v3(middle - FLASH / 2.0, center - FLASH / 2.0))?;
        self.spinner
            .root
            .SetOffset(v3(middle - SPINNER / 2.0, center - SPINNER / 2.0))?;
        self.fast.SetOffset(v3(middle - self.fast_w / 2.0, top))
    }

    /// `running` = the video now plays: ▶, else ❚❚.
    pub fn flash(&self, comp: &Comp, running: bool) {
        let (pause, play) = if running { (0.0, 1.0) } else { (1.0, 0.0) };
        log(self.pause.SetOpacity(pause));
        log(self.play.SetOpacity(play));
        log(comp.stop(&self.flash, "Scale"));
        log(self.flash.SetScale(scale(1.0, 1.0)));
        log(comp.keyframes(
            &self.flash,
            "Opacity",
            &[(0.0, 1.0), (0.4, 1.0), (1.0, 0.0)],
            FLASH_MS,
        ));
        log(comp.animate_vector3(
            &self.flash,
            "Scale",
            scale(FLASH_GROWTH, FLASH_GROWTH),
            Motion::curve(FLASH_MS, 0.25, 0.1, 0.25, 1.0),
        ));
    }

    pub fn set_fast(&self, comp: &Comp, on: bool) {
        let (opacity, size) = if on { (1.0, 1.0) } else { (0.0, FAST_POP) };
        if on {
            log(comp.stop(&self.fast, "Scale"));
            log(self.fast.SetScale(scale(FAST_POP, FAST_POP)));
        }
        log(comp.animate_scalar(&self.fast, "Opacity", opacity, FAST_FADE));
        log(comp.animate_vector3(&self.fast, "Scale", scale(size, size), FAST_FADE));
    }
}

pub const SPINNER: f32 = 36.0;
const SPIN_MS: u32 = 900;
const SPINNER_FADE: Motion = Motion::curve(180, 0.25, 0.1, 0.25, 1.0);

/// A dark disc with a light arc turning in it: the picture is on its way. Shared by the window and
/// the editor; hidden, it doesn't turn.
pub struct Spinner {
    pub root: ContainerVisual,
    arc: ShapeVisual,
}

impl Spinner {
    pub fn new(comp: &Comp) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;
        root.SetOpacity(0.0)?;
        let disc = c.CreateShapeVisual()?;
        disc.SetSize(v2(SPINNER, SPINNER))?;
        let back = c.CreateEllipseGeometry()?;
        back.SetCenter(v2(SPINNER / 2.0, SPINNER / 2.0))?;
        back.SetRadius(v2(SPINNER / 2.0, SPINNER / 2.0))?;
        let fill = c.CreateSpriteShapeWithGeometry(&back)?;
        fill.SetFillBrush(&comp.brush(color("#1C1C1E", 0.72))?)?;
        disc.Shapes()?.Append(&fill)?;
        root.Children()?.InsertAtTop(&disc)?;
        let arc = c.CreateShapeVisual()?;
        arc.SetSize(v2(SPINNER, SPINNER))?;
        arc.SetCenterPoint(v3(SPINNER / 2.0, SPINNER / 2.0))?;
        let ring = c.CreateEllipseGeometry()?;
        ring.SetCenter(v2(SPINNER / 2.0, SPINNER / 2.0))?;
        ring.SetRadius(v2(9.0, 9.0))?;
        ring.SetTrimEnd(0.28)?;
        let stroke = c.CreateSpriteShapeWithGeometry(&ring)?;
        stroke.SetStrokeBrush(&comp.brush(color("#F5F5F7", 1.0))?)?;
        stroke.SetStrokeThickness(2.4)?;
        stroke.SetStrokeStartCap(CompositionStrokeCap::Round)?;
        stroke.SetStrokeEndCap(CompositionStrokeCap::Round)?;
        arc.Shapes()?.Append(&stroke)?;
        root.Children()?.InsertAtTop(&arc)?;
        Ok(Self { root, arc })
    }

    /// Turns and shows: at once, or fading in after `delay_ms` (a short wait shows nothing).
    pub fn show(&self, comp: &Comp, delay_ms: u32) {
        let turn = Motion::linear(0);
        log(comp.loop_scalar(
            &self.arc,
            "RotationAngleInDegrees",
            &[(0.0, 0.0, turn), (1.0, 360.0, turn)],
            SPIN_MS,
        ));
        if delay_ms == 0 {
            log(comp.stop(&self.root, "Opacity"));
            log(self.root.SetOpacity(1.0));
        } else {
            log(comp.animate_scalar_after(&self.root, "Opacity", 1.0, SPINNER_FADE, delay_ms));
        }
    }

    /// Fades out; it stops turning at once, which a fading spinner doesn't show.
    pub fn hide(&self, comp: &Comp) {
        log(comp.stop(&self.arc, "RotationAngleInDegrees"));
        log(comp.animate_scalar(&self.root, "Opacity", 0.0, SPINNER_FADE));
    }
}

fn backing(
    comp: &Comp,
    geometry: &windows::UI::Composition::CompositionGeometry,
    (w, h): (f32, f32),
) -> Result<ShapeVisual> {
    let c = &comp.compositor;
    let visual = c.CreateShapeVisual()?;
    visual.SetSize(v2(w, h))?;
    let shape = c.CreateSpriteShapeWithGeometry(geometry)?;
    shape.SetFillBrush(&comp.brush(color(BACK.0, BACK.1))?)?;
    visual.Shapes()?.Append(&shape)?;
    Ok(visual)
}
