//! Device notices on the island (`crate::devices`):
//! - **connected**, large, 340 wide: the name on top; the case in the middle and the buds left
//!   and right of it, looking away from it (`device_art`), or headphones without a case alone.
//!   The pictures are lines seen through silver: a bright band crosses them once as they come
//!   up. Under them a battery ring with the percentage, when Windows knows it;
//! - **disconnected**, **low battery** and **sound switched**, small: the device's glyph, its
//!   name and what happened. A long name is cut.
//!
//! Every motion is the compositor's; with animations off the parts stand still in plain silver.

use super::super::comp::{color, v2, v3};
use super::super::glyph::Glyph;
use super::device_art::{self, Part};
use super::system::{CHARGE, CRITICAL, DIGITS, EASE_OUT, LINEAR, WARN};
use super::{Content, MUTED, Model, Scene, TEXT, style};
use crate::{
    devices::{Flow, Form, Look, Notice},
    lang::tr,
};
use windows::{
    UI::Composition::{
        CompositionGradientExtendMode, CompositionMappingMode, CompositionObject,
        CompositionPropertySet, CompositionStrokeCap, ContainerVisual, Visual,
    },
    core::{HSTRING, Interface, Result},
};

pub struct DeviceModel<'a> {
    pub notice: &'a Notice,
    pub decor: bool,
}

const WIDTH: f32 = 340.0;
const TOP: f32 = 12.0;
const HEAD: f32 = 36.0;
const GAP: f32 = 6.0;
const STAGE: f32 = 104.0;
const LEVEL: f32 = 24.0;
const BOTTOM: f32 = 14.0;
const STAGE_TOP: f32 = TOP + HEAD + GAP;
/// The slots' middles across the island: the left bud, the case, the right bud.
const LEFT_X: f32 = 62.0;
const CASE_X: f32 = 170.0;
const RIGHT_X: f32 = 278.0;
const BUD_W: f32 = 64.0;
const CASE_W: f32 = 112.0;
const ALONE_W: f32 = 140.0;
const LINE: f32 = 5.0;
const RING: f32 = 20.0;

const SMALL_LEFT: f32 = 11.0;
const SMALL_GLYPH: f32 = 18.0;
const SMALL_GAP: f32 = 8.0;
const VALUE_GAP: f32 = 6.0;
const SMALL_RIGHT: f32 = 14.0;
/// As wide as the other small notices may grow: past it the window would cut the text.
const SMALL_MAX: f32 = 380.0;

/// When each part comes up, ms after the notice appears; the island opens meanwhile.
const CASE_IN: u32 = 120;
const BUDS_IN: u32 = 220;
const FADE_MS: u32 = 650;
const LEVELS_IN: u32 = 480;

/// The silver ramp's length in DIPs; it is mirrored past its ends, so it repeats every two.
const RAMP: f32 = 320.0;
/// Where on the ramp its brightest band lies.
const PEAK: f32 = 0.40;
/// The band's direction: 22° below the horizontal.
const DIR: (f32, f32) = (0.927, 0.375);
const GLOW: f32 = 52.0;
/// The band's one pass, from off the pictures' left to off their right: the next band of the
/// mirrored ramp stays off them on both ends while it lasts.
const SWEEP: (f32, f32) = (-160.0, 500.0);
const SWEEP_MS: u32 = 2000;

const SILVER: [(f32, &str); 10] = [
    (0.0, "#7F7F87"),
    (0.14, "#B4B4BB"),
    (0.26, "#E2E2E6"),
    (0.33, "#F2DDE9"),
    (PEAK, "#FFFFFF"),
    (0.47, "#D7EBF5"),
    (0.56, "#EFE7D2"),
    (0.67, "#C8C8CE"),
    (0.83, "#9A9AA2"),
    (1.0, "#85858D"),
];

fn ring_hex(percent: u8) -> &'static str {
    match percent {
        0..=10 => CRITICAL,
        11..=20 => WARN,
        _ => CHARGE,
    }
}

/// A small notice's parts: the title, the dim part, the value and its color; and which of the
/// two texts is the device's name, the one cut when it doesn't fit.
struct Small {
    title: String,
    dim: Option<String>,
    value: Option<(String, &'static str)>,
    name_is_title: bool,
}

fn small_text(notice: &Notice) -> Small {
    let (title, dim, value, name_is_title) = match notice {
        Notice::Disconnected { name, .. } => (
            name.clone(),
            Some(tr!("device.disconnected").into_owned()),
            None,
            true,
        ),
        Notice::Low { name, percent, .. } => (
            name.clone(),
            Some(tr!("device.low").into_owned()),
            Some((format!("{percent}%"), ring_hex(*percent))),
            true,
        ),
        Notice::Switched { flow, name, .. } => {
            let title = match flow {
                Flow::Output => tr!("device.sound"),
                Flow::Input => tr!("device.microphone"),
            };
            (title.into_owned(), Some(name.clone()), None, false)
        }
        Notice::Connected { name, .. } => (name.clone(), None, None, true),
    };
    Small {
        title,
        dim,
        value,
        name_is_title,
    }
}

fn form_glyph(form: Form) -> Glyph {
    match form {
        Form::Headphones | Form::Headset => Glyph::Headphones,
        Form::Display => Glyph::Monitor,
        Form::Microphone => Glyph::Microphone,
        Form::Speakers | Form::LineOut | Form::Other => Glyph::Speaker,
    }
}

fn look_glyph(look: Look) -> Glyph {
    match look {
        Look::Speaker => Glyph::Speaker,
        Look::Mouse => Glyph::Mouse,
        Look::Keyboard => Glyph::Keyboard,
        Look::Gamepad => Glyph::Gamepad,
        Look::Device => Glyph::Phone,
        Look::AirPodsPro | Look::GalaxyBuds | Look::GalaxyBuds3 | Look::OverEar | Look::Earbuds => {
            Glyph::Headphones
        }
    }
}

impl Scene {
    fn small_fitted(&self, notice: &Notice) -> (Small, f32) {
        let mut small = small_text(notice);
        let title_style = style(13.0, true, TEXT);
        let dim_style = style(13.0, false, MUTED);
        let mut fixed = SMALL_LEFT + SMALL_GLYPH + SMALL_GAP + SMALL_RIGHT;
        if let Some((value, _)) = &small.value {
            fixed += VALUE_GAP + self.comp.measure(value, title_style);
        }
        let room = SMALL_MAX - fixed;
        let dim_w = |d: &Option<String>| {
            d.as_ref()
                .map_or(0.0, |d| self.comp.measure(&format!(" {d}"), dim_style))
        };
        let title_w = self.comp.measure(&small.title, title_style);
        if title_w + dim_w(&small.dim) > room {
            if small.name_is_title {
                small.title = self
                    .comp
                    .fit(&small.title, title_style, room - dim_w(&small.dim));
            } else if let Some(dim) = &small.dim {
                let space = self.comp.measure(" ", dim_style);
                small.dim = Some(self.comp.fit(dim, dim_style, room - title_w - space));
            }
        }
        let width = fixed + self.comp.measure(&small.title, title_style) + dim_w(&small.dim);
        (small, width.round())
    }

    pub(super) fn device_size(&self, model: &Model) -> (f32, f32) {
        let Some(device) = &model.device else {
            return (30.0, 30.0);
        };
        if let Notice::Connected { battery, .. } = device.notice {
            let levels = if battery.is_some() { GAP + LEVEL } else { 0.0 };
            return (WIDTH, STAGE_TOP + STAGE + levels + BOTTOM);
        }
        (self.small_fitted(device.notice).1, 30.0)
    }

    pub(super) fn build_device(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.device_size(model);
        content.width = width;
        content.height = height;
        let Some(device) = &model.device else {
            return Ok(());
        };
        let root = content.root.clone();
        match device.notice {
            Notice::Connected {
                name,
                look,
                battery,
                ..
            } => {
                let light =
                    self.build_connected(&root, name, *look, *battery, device.decor, height)?;
                content._light = Some(light);
                Ok(())
            }
            notice => self.build_small_device(&root, notice),
        }
    }

    fn build_connected(
        &self,
        root: &ContainerVisual,
        name: &str,
        look: Look,
        battery: Option<u8>,
        decor: bool,
        height: f32,
    ) -> Result<CompositionPropertySet> {
        let title_style = style(14.0, true, TEXT);
        let title = self.comp.fit(name, title_style, WIDTH - 36.0);
        let title = self.comp.text(&title, title_style, self.scale)?;
        self.place(root, &title.visual, (WIDTH - title.width) / 2.0, TOP)?;
        let sub = self.comp.text(
            &tr!("device.connected"),
            style(12.0, false, MUTED),
            self.scale,
        )?;
        let sub_y = TOP + title.height + 1.0;
        self.place(root, &sub.visual, (WIDTH - sub.width) / 2.0, sub_y)?;

        let light = self.comp.compositor.CreatePropertySet()?;
        // Lively, the band crosses the middle of the pictures; still, it waits above the top
        // left, off the pictures, so they show plain silver.
        let (x, y) = if decor {
            (SWEEP.0, STAGE_TOP + STAGE * 0.45)
        } else {
            (WIDTH * 0.18, -height * 0.6)
        };
        light.InsertScalar(&HSTRING::from("X"), x)?;
        light.InsertScalar(&HSTRING::from("Y"), y)?;

        if look.has_case() {
            self.piece(
                root,
                &light,
                decor,
                look,
                Part::Case,
                CASE_X,
                CASE_W,
                CASE_IN,
            )?;
            self.piece(
                root,
                &light,
                decor,
                look,
                Part::Left,
                LEFT_X,
                BUD_W,
                BUDS_IN,
            )?;
            self.piece(
                root,
                &light,
                decor,
                look,
                Part::Right,
                RIGHT_X,
                BUD_W,
                BUDS_IN,
            )?;
        } else {
            let w = ALONE_W.min(STAGE * device_art::aspect(look, Part::Whole));
            self.piece(root, &light, decor, look, Part::Whole, CASE_X, w, CASE_IN)?;
        }
        if decor {
            self.comp.scalar_keys(
                &light,
                "X",
                &[(0.0, SWEEP.0, LINEAR), (1.0, SWEEP.1, EASE_OUT)],
                SWEEP_MS,
                BUDS_IN,
                Some(1),
            )?;
        }
        if let Some(percent) = battery {
            self.level(
                root,
                CASE_X,
                STAGE_TOP + STAGE + GAP,
                percent,
                decor,
                LEVELS_IN,
            )?;
        }
        Ok(light)
    }

    #[allow(clippy::too_many_arguments)]
    fn piece(
        &self,
        root: &ContainerVisual,
        light: &CompositionPropertySet,
        decor: bool,
        look: Look,
        part: Part,
        middle: f32,
        width: f32,
        delay: u32,
    ) -> Result<()> {
        let c = &self.comp.compositor;
        let height = width / device_art::aspect(look, part);
        let art = self.comp.picture(width, height, self.scale, |dc, base| {
            device_art::draw(dc, base, look, part, width, LINE, "#FFFFFF")
        })?;
        let (x, y) = (middle - width / 2.0, STAGE_TOP + (STAGE - height) / 2.0);
        let lines = art.Brush()?;
        let light: CompositionObject = light.cast()?;
        let at = [("ox", x), ("oy", y)];

        let metal = c.CreateLinearGradientBrush()?;
        metal.SetMappingMode(CompositionMappingMode::Absolute)?;
        metal.SetExtendMode(CompositionGradientExtendMode::Mirror)?;
        let stops = metal.ColorStops()?;
        for (offset, hex) in SILVER {
            stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(offset, color(hex, 1.0))?)?;
        }
        let (dx, dy) = (DIR.0 * RAMP, DIR.1 * RAMP);
        for (property, along) in [("StartPoint", -PEAK), ("EndPoint", 1.0 - PEAK)] {
            self.comp.bind(
                &metal,
                property,
                "Vector2(l.X - ox + dx * a, l.Y - oy + dy * a)",
                &[("l", &light)],
                &[at[0], at[1], ("dx", dx), ("dy", dy), ("a", along)],
            )?;
        }
        let silver = c.CreateMaskBrush()?;
        silver.SetMask(&lines)?;
        silver.SetSource(&metal)?;
        art.SetBrush(&silver)?;

        let glow =
            self.comp
                .radial_wash(v2(0.0, 0.0), v2(GLOW, GLOW), color("#FFFFFF", 1.0), 0.75)?;
        self.comp.bind(
            &glow,
            "EllipseCenter",
            "Vector2(l.X - ox, l.Y - oy)",
            &[("l", &light)],
            &at,
        )?;
        let shine = c.CreateMaskBrush()?;
        shine.SetMask(&lines)?;
        shine.SetSource(&glow)?;
        let highlight = c.CreateSpriteVisual()?;
        highlight.SetSize(art.Size()?)?;
        highlight.SetBrush(&shine)?;
        art.Children()?.InsertAtTop(&highlight)?;

        art.SetCenterPoint(v3(width / 2.0, height / 2.0))?;
        self.place(root, &art, x, y)?;
        if !decor {
            return Ok(());
        }
        let once = Some(1);
        self.comp.scalar_keys(
            &art,
            "Opacity",
            &[(0.0, 0.0, LINEAR), (1.0, 1.0, EASE_OUT)],
            FADE_MS,
            delay,
            once,
        )?;
        for axis in ["Scale.X", "Scale.Y"] {
            self.comp.scalar_keys(
                &art,
                axis,
                &[(0.0, 0.95, LINEAR), (1.0, 1.0, EASE_OUT)],
                FADE_MS,
                delay,
                once,
            )?;
        }
        Ok(())
    }

    /// A ring that fills to the charge and the percentage beside it, centered on `middle`.
    fn level(
        &self,
        root: &ContainerVisual,
        middle: f32,
        top: f32,
        percent: u8,
        decor: bool,
        delay: u32,
    ) -> Result<()> {
        let c = &self.comp.compositor;
        let group = self.comp.container()?;
        let shapes = c.CreateShapeVisual()?;
        shapes.SetSize(v2(RING, RING))?;
        let centre = v2(RING / 2.0, RING / 2.0);
        let radius = v2(8.0, 8.0);
        let track = c.CreateEllipseGeometry()?;
        track.SetCenter(centre)?;
        track.SetRadius(radius)?;
        let track_shape = c.CreateSpriteShapeWithGeometry(&track)?;
        track_shape.SetStrokeBrush(&self.comp.brush(color("#FFFFFF", 0.14))?)?;
        track_shape.SetStrokeThickness(2.4)?;
        shapes.Shapes()?.Append(&track_shape)?;
        let arc = c.CreateEllipseGeometry()?;
        arc.SetCenter(centre)?;
        arc.SetRadius(radius)?;
        let filled = f32::from(percent.min(100)) / 100.0;
        arc.SetTrimEnd(filled)?;
        let arc_shape = c.CreateSpriteShapeWithGeometry(&arc)?;
        arc_shape.SetStrokeBrush(&self.comp.brush(color(ring_hex(percent), 1.0))?)?;
        arc_shape.SetStrokeThickness(2.4)?;
        arc_shape.SetStrokeStartCap(CompositionStrokeCap::Round)?;
        arc_shape.SetStrokeEndCap(CompositionStrokeCap::Round)?;
        // An ellipse's outline starts at three o'clock: turned back a quarter it starts at the top.
        arc_shape.SetCenterPoint(centre)?;
        arc_shape.SetRotationAngleInDegrees(-90.0)?;
        shapes.Shapes()?.Append(&arc_shape)?;
        self.place(&group, &shapes, 0.0, (LEVEL - RING) / 2.0)?;

        let text = self.comp.text(
            &format!("{percent}%"),
            super::super::comp::TextStyle {
                tabular: true,
                ..style(12.0, true, DIGITS)
            },
            self.scale,
        )?;
        let text_x = RING + 6.0;
        self.place(&group, &text.visual, text_x, (LEVEL - text.height) / 2.0)?;
        let width = text_x + text.width;
        group.SetSize(v2(width, LEVEL))?;
        self.place(root, &group, middle - width / 2.0, top)?;
        if !decor {
            return Ok(());
        }
        let once = Some(1);
        self.comp.scalar_keys(
            &group,
            "Opacity",
            &[(0.0, 0.0, LINEAR), (1.0, 1.0, EASE_OUT)],
            520,
            delay,
            once,
        )?;
        let visual: Visual = group.cast()?;
        let y = visual.Offset()?.Y;
        self.comp.scalar_keys(
            &group,
            "Offset.Y",
            &[(0.0, y + 4.0, LINEAR), (1.0, y, EASE_OUT)],
            520,
            delay,
            once,
        )?;
        self.comp.scalar_keys(
            &arc,
            "TrimEnd",
            &[(0.0, 0.0, LINEAR), (1.0, filled, EASE_OUT)],
            800,
            delay,
            once,
        )
    }

    fn build_small_device(&self, root: &ContainerVisual, notice: &Notice) -> Result<()> {
        let middle = 15.0;
        let glyph = match notice {
            Notice::Switched { form, .. } => form_glyph(*form),
            Notice::Disconnected { look, .. }
            | Notice::Low { look, .. }
            | Notice::Connected { look, .. } => look_glyph(*look),
        };
        let glyph = self
            .comp
            .glyph(glyph, SMALL_GLYPH, color(TEXT, 1.0), self.scale)?;
        self.place(root, &glyph, SMALL_LEFT, middle - SMALL_GLYPH / 2.0)?;
        let (small, _) = self.small_fitted(notice);
        let mut x = SMALL_LEFT + SMALL_GLYPH + SMALL_GAP;
        let label = self
            .comp
            .text(&small.title, style(13.0, true, TEXT), self.scale)?;
        self.place(root, &label.visual, x, (30.0 - label.height) / 2.0)?;
        x += label.width;
        if let Some(dim) = small.dim {
            let dim = self
                .comp
                .text(&format!(" {dim}"), style(13.0, false, MUTED), self.scale)?;
            self.place(root, &dim.visual, x, (30.0 - dim.height) / 2.0)?;
            x += dim.width;
        }
        if let Some((value, hex)) = small.value {
            let value = self.comp.text(&value, style(13.0, true, hex), self.scale)?;
            self.place(
                root,
                &value.visual,
                x + VALUE_GAP,
                (30.0 - value.height) / 2.0,
            )?;
        }
        Ok(())
    }
}
