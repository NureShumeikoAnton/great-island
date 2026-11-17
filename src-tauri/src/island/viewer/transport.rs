//! The video window's bottom panel, left to right: ▶/❚❚, −5 s, +5 s, sound, the time, the progress
//! bar, the length, the speed, frame to screenshot, full screen. With the strip it lies on a dark
//! strip; without it every element is a capsule of its own.
//!
//! - the cursor on the sound slides a volume slider out of it to the right; the rest moves aside;
//! - a click on the speed unfolds its steps in a row in its place (the bar gets shorter); the row
//!   folds when a step is chosen or, by the window's timer, after the cursor leaves it;
//! - the cursor on the bar thickens it, shows its knob and the time under the cursor above it.
//!
//! The bar runs by itself on the compositor: [`Transport::set_bar`] says where it stands and how long
//! it runs to the end. The panel decides nothing: it says what is under a point ([`Spot`]).

use super::super::comp::{Comp, Motion, Text, TextStyle, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::super::log;
use super::capsule::{Capsule, Face, Look, Panel};
use super::{CHECK, FILL, Metrics, TEXT};
use crate::play::{self, SPEEDS, speed_label};
use windows::{
    UI::Composition::{
        CompositionEllipseGeometry, CompositionObject, CompositionRoundedRectangleGeometry,
        ContainerVisual, ShapeVisual, Visual,
    },
    core::{Interface, Result},
};

const MOVE: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);
const FADE: Motion = Motion::curve(150, 0.25, 0.1, 0.25, 1.0);
const RAIL: f32 = 4.0;
const RAIL_HOVER: f32 = 1.5;
const KNOB: f32 = 12.0;
const SLIDER: f32 = 64.0;
/// The sound capsule grows by this when its slider is out.
const SLIDER_ROOM: f32 = SLIDER + 14.0;
const VOLUME_KNOB: f32 = 10.0;
/// Inside the floating pill around the time, the bar and the length.
const PAD: f32 = 12.0;
const TEXT_GAP: f32 = 10.0;
const MIN_BAR: f32 = 120.0;
/// A bar shorter than this (a narrow window with the row open) is not shown.
const HIDDEN_BAR: f32 = 24.0;
/// The bar takes the cursor this far above and below its line.
const BAR_REACH: f32 = 8.0;
const TIP_H: f32 = 24.0;
const TIP_PAD: f32 = 8.0;
const TIP_GAP: f32 = 8.0;
const STEP_INSET: f32 = 2.0;
/// The shapes of the bar and the slider are drawn in one visual each; wider than any window.
const SHAPES_WIDTH: f32 = 10_000.0;
const TRACK: (&str, f32) = ("#FFFFFF", 0.22);
const GHOST: (&str, f32) = ("#FFFFFF", 0.38);
const TIP: (&str, f32) = ("#1C1C1E", 0.92);
const MINUS: &str = "\u{2212}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Play,
    Back,
    Forward,
    Sound,
    Speed,
    Shot,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    Button(Button),
    Step(usize),
    /// 0–1 along the bar.
    Bar(f64),
    /// 0–1 along the volume slider.
    Volume(f64),
    Length,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Show {
    pub running: bool,
    pub ended: bool,
    pub muted: bool,
    pub volume: f64,
    pub speed: usize,
    pub full: bool,
}

struct Shape {
    visual: ShapeVisual,
    geometry: CompositionRoundedRectangleGeometry,
}

pub struct Transport {
    pub root: ContainerVisual,
    m: Metrics,
    k: f32,
    show: Show,
    duration: f64,
    buttons: Panel<Button>,
    steps: Option<Panel<usize>>,
    /// The row just folded, on its way into the speed button.
    folding: Option<Panel<usize>>,
    /// The bar's width before an animated layout: the next [`Transport::set_bar`] glides from it.
    glide_from: Option<f32>,
    row_back: Shape,
    sound_back: Shape,
    /// Without the strip: the capsule under the time, the bar and the length.
    pill: Option<Shape>,
    time: Text,
    length: Text,
    rail: ContainerVisual,
    track: CompositionRoundedRectangleGeometry,
    ghost: CompositionRoundedRectangleGeometry,
    fill: CompositionRoundedRectangleGeometry,
    knob: ShapeVisual,
    slider: ContainerVisual,
    volume_fill: CompositionRoundedRectangleGeometry,
    volume_knob: CompositionEllipseGeometry,
    tip: ContainerVisual,
    tip_back: CompositionRoundedRectangleGeometry,
    tip_text: Text,
    shot_done: Visual,
    width: f32,
    /// x and width of the bar, of the sound with its slider, of the speed (or its row).
    bar: (f32, f32),
    sound: (f32, f32),
    speed: (f32, f32),
    length_box: (f32, f32),
    time_w: f32,
    length_w: f32,
    slider_open: bool,
    bar_hovered: bool,
    tip_shown: bool,
}

impl Transport {
    pub fn band(m: Metrics) -> f32 {
        if m.floating() {
            m.button.1 + m.margin
        } else {
            m.strip
        }
    }

    pub fn new(comp: &Comp, k: f32, m: Metrics, duration: f64, show: Show) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;
        let (bw, bh) = m.button;
        let sound_back = shape(comp, (bw, bh), m.plain().idle)?;
        sound_back.visual.SetOpacity(0.0)?;
        let row_back = shape(comp, (bw, bh), m.plain().idle)?;
        row_back.visual.SetOpacity(0.0)?;
        let pill = if m.floating() {
            Some(shape(comp, (bw, bh), m.plain().idle)?)
        } else {
            None
        };
        for back in [&sound_back, &row_back].into_iter().chain(pill.as_ref()) {
            root.Children()?.InsertAtTop(&back.visual)?;
        }

        let mut buttons = Panel::new(comp)?;
        for button in [
            Button::Play,
            Button::Back,
            Button::Forward,
            Button::Sound,
            Button::Speed,
            Button::Shot,
            Button::Full,
        ] {
            buttons.add(button, capsule(comp, k, m, button, show)?)?;
        }
        root.Children()?.InsertAtTop(&buttons.root)?;
        let shot_done = {
            let shot = buttons
                .get(Button::Shot)
                .ok_or_else(windows::core::Error::empty)?;
            let check = comp.glyph(Glyph::Check, m.icon, color(CHECK, 1.0), k)?;
            check.SetOffset(shot.face.Offset()?)?;
            check.SetOpacity(0.0)?;
            shot.body.Children()?.InsertAtTop(&check)?;
            check.cast::<Visual>()?
        };

        let style = time_style(m);
        let time = comp.text("", style, k)?;
        let length = comp.text("", style, k)?;
        root.Children()?.InsertAtTop(&time.visual)?;
        root.Children()?.InsertAtTop(&length.visual)?;

        let rail = comp.container()?;
        rail.SetCenterPoint(v3(0.0, RAIL / 2.0))?;
        let shapes = c.CreateShapeVisual()?;
        shapes.SetSize(v2(SHAPES_WIDTH, RAIL))?;
        let track = rounded(comp, &shapes, color(TRACK.0, TRACK.1))?;
        let ghost = rounded(comp, &shapes, color(GHOST.0, GHOST.1))?;
        let fill = rounded(comp, &shapes, color(TEXT, 1.0))?;
        rail.Children()?.InsertAtTop(&shapes)?;
        root.Children()?.InsertAtTop(&rail)?;
        let knob = circle(comp, KNOB, color("#FFFFFF", 1.0))?;
        knob.SetOpacity(0.0)?;
        root.Children()?.InsertAtTop(&knob)?;

        let slider = comp.container()?;
        slider.SetOpacity(0.0)?;
        let volume_shapes = c.CreateShapeVisual()?;
        volume_shapes.SetSize(v2(SLIDER + VOLUME_KNOB, VOLUME_KNOB))?;
        let volume_track = rounded(comp, &volume_shapes, color(TRACK.0, TRACK.1))?;
        volume_track.SetOffset(v2(VOLUME_KNOB / 2.0, (VOLUME_KNOB - RAIL) / 2.0))?;
        volume_track.SetSize(v2(SLIDER, RAIL))?;
        let volume_fill = rounded(comp, &volume_shapes, color(TEXT, 1.0))?;
        volume_fill.SetOffset(v2(VOLUME_KNOB / 2.0, (VOLUME_KNOB - RAIL) / 2.0))?;
        let volume_knob = c.CreateEllipseGeometry()?;
        volume_knob.SetRadius(v2(VOLUME_KNOB / 2.0, VOLUME_KNOB / 2.0))?;
        let knob_shape = c.CreateSpriteShapeWithGeometry(&volume_knob)?;
        knob_shape.SetFillBrush(&comp.brush(color("#FFFFFF", 1.0))?)?;
        volume_shapes.Shapes()?.Append(&knob_shape)?;
        slider.Children()?.InsertAtTop(&volume_shapes)?;
        root.Children()?.InsertAtTop(&slider)?;

        let tip = comp.container()?;
        tip.SetOpacity(0.0)?;
        let tip_shapes = c.CreateShapeVisual()?;
        tip_shapes.SetSize(v2(200.0, TIP_H))?;
        let tip_back = rounded(comp, &tip_shapes, color(TIP.0, TIP.1))?;
        tip_back.SetCornerRadius(v2(TIP_H / 2.0, TIP_H / 2.0))?;
        tip.Children()?.InsertAtTop(&tip_shapes)?;
        let tip_text = comp.text("", style, k)?;
        tip.Children()?.InsertAtTop(&tip_text.visual)?;
        root.Children()?.InsertAtTop(&tip)?;

        let mut transport = Self {
            root,
            m,
            k,
            show,
            duration: 0.0,
            buttons,
            steps: None,
            folding: None,
            glide_from: None,
            row_back,
            sound_back,
            pill,
            time,
            length,
            rail,
            track,
            ghost,
            fill,
            knob,
            slider,
            volume_fill,
            volume_knob,
            tip,
            tip_back,
            tip_text,
            shot_done,
            width: 0.0,
            bar: (0.0, 0.0),
            sound: (0.0, 0.0),
            speed: (0.0, 0.0),
            length_box: (0.0, 0.0),
            time_w: 0.0,
            length_w: 0.0,
            slider_open: false,
            bar_hovered: false,
            tip_shown: false,
        };
        transport.set_duration(comp, duration);
        transport.draw_volume()?;
        Ok(transport)
    }

    fn top(&self) -> f32 {
        if self.m.floating() {
            0.0
        } else {
            (self.m.strip - self.m.button.1) / 2.0
        }
    }

    fn middle(&self) -> f32 {
        self.top() + self.m.button.1 / 2.0
    }

    /// The text widths are reserved for the longest time of this video, so the bar doesn't twitch.
    pub fn set_duration(&mut self, comp: &Comp, duration: f64) {
        self.duration = duration;
        let template: String = play::clock(duration, duration)
            .chars()
            .map(|c| if c.is_ascii_digit() { '8' } else { c })
            .collect();
        let style = time_style(self.m);
        self.time_w = comp.measure(&template, style).ceil();
        self.length_w = comp.measure(&format!("{MINUS}{template}"), style).ceil();
    }

    pub fn min_width(&self, comp: &Comp) -> f32 {
        let (bw, _) = self.m.button;
        let g = self.m.gap;
        let speed = speed_width(comp, self.m, self.show.speed);
        let pad = if self.m.floating() { 2.0 * PAD } else { 0.0 };
        2.0 * self.m.margin
            + 4.0 * (bw + g)
            + g
            + pad
            + self.time_w
            + self.length_w
            + 2.0 * TEXT_GAP
            + MIN_BAR
            + 2.0 * g
            + speed
            + 2.0 * (bw + g)
    }

    pub fn layout(&mut self, comp: &Comp, width: f32, animate: bool) -> Result<()> {
        self.width = width;
        let m = self.m;
        let (bw, bh) = m.button;
        let g = m.gap;
        let y = self.top();
        let cy = self.middle();
        let place = |panel: &mut Panel<Button>, key: Button, x: f32| {
            if let Some(capsule) = panel.get_mut(key) {
                if animate {
                    capsule.glide(comp, x, y, MOVE);
                } else {
                    capsule.place(x, y);
                }
            }
        };
        let mut x = m.margin;
        for key in [Button::Play, Button::Back, Button::Forward, Button::Sound] {
            place(&mut self.buttons, key, x);
            x += bw + g;
        }
        let sound_x = x - bw - g;
        let sound_w = bw + if self.slider_open { SLIDER_ROOM } else { 0.0 };
        self.sound = (sound_x, sound_w);
        let left = sound_x + sound_w + 2.0 * g;

        let mut right = width - m.margin;
        for key in [Button::Full, Button::Shot] {
            right -= bw;
            place(&mut self.buttons, key, right);
            right -= g;
        }
        let speed_w = match &self.steps {
            Some(_) => row_width(comp, m),
            None => speed_width(comp, m, self.show.speed),
        };
        right -= speed_w;
        place(&mut self.buttons, Button::Speed, right);
        self.speed = (right, speed_w);
        let right = right - 2.0 * g;

        let pad = if m.floating() { PAD } else { 0.0 };
        let time_x = left + pad;
        let length_end = right - pad;
        let bar_x = time_x + self.time_w + TEXT_GAP;
        let bar_w = (length_end - self.length_w - TEXT_GAP - bar_x).max(0.0);
        if !animate {
            self.glide_from = None;
        } else if self.glide_from.is_none() && (bar_w - self.bar.1).abs() > 0.5 {
            self.glide_from = Some(self.bar.1);
        }
        self.bar = (bar_x, bar_w);
        self.length_box = (length_end - self.length_w, self.length_w);

        let at = |visual: &Visual, x: f32, y: f32| {
            if animate {
                log(comp.animate_vector3(visual, "Offset", v3(x, y), MOVE));
            } else {
                log(visual.SetOffset(v3(x, y)));
            }
        };
        let text_y = snap(cy - comp.cap_middle(time_style(m)), self.k);
        at(&self.time.visual.cast()?, snap(time_x, self.k), text_y);
        self.place_length(comp, animate)?;
        at(&self.rail.cast()?, bar_x, cy - RAIL / 2.0);
        let shown = if bar_w >= HIDDEN_BAR { 1.0 } else { 0.0 };
        log(comp.animate_scalar(&self.rail, "Opacity", shown, FADE));
        resize(comp, &self.track, bar_w, animate);
        self.bind_knob(comp)?;

        if let Some(pill) = &self.pill {
            let pill_x = left;
            let pill_w = (right - left).max(0.0);
            at(&pill.visual.cast()?, pill_x, y);
            pill.visual.SetSize(v2(SHAPES_WIDTH, bh))?;
            resize(comp, &pill.geometry, pill_w, animate);
        }
        at(&self.sound_back.visual.cast()?, sound_x, y);
        self.sound_back.visual.SetSize(v2(SHAPES_WIDTH, bh))?;
        resize(comp, &self.sound_back.geometry, sound_w, animate);
        at(
            &self.slider.cast()?,
            sound_x + bw - VOLUME_KNOB / 2.0 + 2.0,
            cy - VOLUME_KNOB / 2.0,
        );
        at(&self.row_back.visual.cast()?, self.speed.0, y);
        self.row_back.visual.SetSize(v2(SHAPES_WIDTH, bh))?;
        resize(comp, &self.row_back.geometry, speed_w, animate);
        self.place_steps(comp, animate);
        Ok(())
    }

    /// The length keeps to the right of its box: "1:05" and "−0:25" end in one place.
    fn place_length(&mut self, comp: &Comp, animate: bool) -> Result<()> {
        let (x, w) = self.length_box;
        let y = snap(self.middle() - comp.cap_middle(time_style(self.m)), self.k);
        let at = v3(snap(x + w - self.length.width, self.k), y);
        if animate {
            comp.animate_vector3(&self.length.visual, "Offset", at, MOVE)
        } else {
            comp.stop(&self.length.visual, "Offset")?;
            self.length.visual.SetOffset(at)
        }
    }

    fn place_steps(&mut self, comp: &Comp, animate: bool) {
        let (x0, _) = self.speed;
        let y = self.top() + STEP_INSET;
        let m = self.m;
        let Some(steps) = &mut self.steps else {
            return;
        };
        let mut x = x0 + STEP_INSET;
        for (i, capsule) in steps.items_mut() {
            if animate {
                capsule.glide(comp, x, y, MOVE);
            } else {
                capsule.place(x, y);
            }
            x += step_width(comp, m, i);
        }
    }

    /// The knob rides on the end of the running fill: the compositor moves it, not we.
    fn bind_knob(&self, comp: &Comp) -> Result<()> {
        let fill: CompositionObject = self.fill.cast()?;
        comp.bind(
            &self.knob,
            "Offset",
            "Vector3(x + g.Size.X - r, y, 0)",
            &[("g", &fill)],
            &[
                ("x", self.bar.0),
                ("y", self.middle() - KNOB / 2.0),
                ("r", KNOB / 2.0),
            ],
        )
    }

    /// Where the bar stands and, while playing, its run to the end. After the bar changed its
    /// width the fill glides to it along with the track.
    pub fn set_bar(&mut self, comp: &Comp, bar: play::Bar) {
        let w = self.bar.1;
        let f = (bar.fraction as f32).clamp(0.0, 1.0);
        let old = self.glide_from.take().unwrap_or(w);
        log(comp.stop(&self.fill, "Size"));
        log(self.fill.SetSize(v2(f * old, RAIL)));
        let run = bar
            .run_ms
            .filter(|_| w > f * w)
            .map(|ms| ms.clamp(1, u64::from(u32::MAX)) as u32);
        let moving = (old - w).abs() > 0.5;
        match run {
            Some(ms) if moving && ms > MOVE.ms => {
                let p = MOVE.ms as f32 / ms as f32;
                let mid = (f + p * (1.0 - f)) * w;
                log(comp.path_vector2(
                    &self.fill,
                    "Size",
                    &[
                        (p, v2(mid, RAIL), MOVE),
                        (1.0, v2(w, RAIL), Motion::linear(0)),
                    ],
                    ms,
                ));
            }
            Some(ms) => {
                log(comp.animate_vector2(&self.fill, "Size", v2(w, RAIL), Motion::linear(ms)));
            }
            None if moving => {
                log(comp.animate_vector2(&self.fill, "Size", v2(f * w, RAIL), MOVE));
            }
            None => log(self.fill.SetSize(v2(f * w, RAIL))),
        }
    }

    /// The bar's width changed in an animated layout and its fill waits for [`Transport::set_bar`].
    pub fn bar_moved(&self) -> bool {
        self.glide_from.is_some()
    }

    pub fn set_times(&mut self, comp: &Comp, time: &str, length: &str) -> Result<()> {
        comp.set_text(&mut self.time, time, self.k)?;
        comp.set_text(&mut self.length, length, self.k)?;
        self.place_length(comp, false)
    }

    pub fn set_show(&mut self, comp: &Comp, show: Show) -> Result<()> {
        let old = self.show;
        self.show = show;
        let faces = [
            (
                Button::Play,
                face_of(old, Button::Play) != face_of(show, Button::Play),
            ),
            (Button::Sound, old.muted != show.muted),
            (Button::Full, old.full != show.full),
            (Button::Speed, old.speed != show.speed),
        ];
        for (button, changed) in faces {
            if changed {
                let fresh = capsule(comp, self.k, self.m, button, show)?;
                self.buttons.replace(comp, button, fresh)?;
            }
        }
        if self.slider_open
            && let Some(sound) = self.buttons.get_mut(Button::Sound)
        {
            sound.set_look(comp, self.m.clear());
        }
        if old.volume != show.volume || old.muted != show.muted {
            self.draw_volume()?;
        }
        if old.speed != show.speed {
            if self.steps.is_some() {
                self.close_row(comp)?;
            } else {
                self.layout(comp, self.width, true)?;
            }
        }
        Ok(())
    }

    fn draw_volume(&self) -> Result<()> {
        let v = if self.show.muted {
            0.0
        } else {
            self.show.volume as f32
        };
        let x = v.clamp(0.0, 1.0) * SLIDER;
        self.volume_fill.SetSize(v2(x, RAIL))?;
        self.volume_knob
            .SetCenter(v2(VOLUME_KNOB / 2.0 + x, VOLUME_KNOB / 2.0))
    }

    pub fn show_shot_done(&self, comp: &Comp, on: bool) {
        let Some(shot) = self.buttons.get(Button::Shot) else {
            return;
        };
        let (icon, check) = if on { (0.0, 1.0) } else { (1.0, 0.0) };
        log(comp.animate_scalar(&shot.face, "Opacity", icon, FADE));
        log(comp.animate_scalar(&self.shot_done, "Opacity", check, FADE));
    }

    pub fn row_open(&self) -> bool {
        self.steps.is_some()
    }

    pub fn slider_open(&self) -> bool {
        self.slider_open
    }

    /// The speed steps unfold in place of the speed button; the current one on a white backing.
    pub fn open_row(&mut self, comp: &Comp) -> Result<()> {
        if self.steps.is_some() {
            return Ok(());
        }
        let m = self.m;
        let h = m.button.1 - 2.0 * STEP_INSET;
        let mut steps = Panel::new(comp)?;
        for i in 0..SPEEDS.len() {
            let on = i == self.show.speed;
            let ink = if on { "#1C1C1E" } else { TEXT };
            let style = label_style(m, ink);
            let label = speed_label(i);
            let look = if on {
                Look::new(TEXT, 1.0, TEXT, 1.0)
            } else {
                Look::new("#FFFFFF", 0.0, "#FFFFFF", FILL.1)
            };
            let w = step_width(comp, m, i);
            let mut step = Capsule::new(comp, self.k, (w, h), Face::Text(&label, style), look)?;
            step.place(self.speed.0 + STEP_INSET, self.top() + STEP_INSET);
            steps.add(i, step)?;
        }
        self.drop_folding();
        if let Some(speed) = self.buttons.get(Button::Speed) {
            log(comp.animate_scalar(&speed.body, "Opacity", 0.0, FADE));
        }
        steps.root.SetOpacity(0.0)?;
        self.root.Children()?.InsertAtTop(&steps.root)?;
        log(comp.animate_scalar(&steps.root, "Opacity", 1.0, MOVE));
        self.steps = Some(steps);
        log(comp.animate_scalar(&self.row_back.visual, "Opacity", 1.0, FADE));
        self.layout(comp, self.width, true)
    }

    pub fn close_row(&mut self, comp: &Comp) -> Result<()> {
        let Some(mut steps) = self.steps.take() else {
            return Ok(());
        };
        if let Some(speed) = self.buttons.get(Button::Speed) {
            log(comp.animate_scalar(&speed.body, "Opacity", 1.0, FADE));
        }
        log(comp.animate_scalar(&self.row_back.visual, "Opacity", 0.0, FADE));
        self.layout(comp, self.width, true)?;
        let (x, y) = (self.speed.0 + STEP_INSET, self.top() + STEP_INSET);
        for (_, step) in steps.items_mut() {
            step.glide(comp, x, y, MOVE);
        }
        log(comp.animate_scalar(&steps.root, "Opacity", 0.0, MOVE));
        self.drop_folding();
        self.folding = Some(steps);
        Ok(())
    }

    /// A folded row is invisible by now or about to be replaced: it leaves the tree.
    fn drop_folding(&mut self) {
        if let Some(old) = self.folding.take()
            && let Ok(children) = self.root.Children()
        {
            let _ = children.Remove(&old.root);
        }
    }

    fn set_slider(&mut self, comp: &Comp, open: bool) -> Result<()> {
        if open == self.slider_open {
            return Ok(());
        }
        self.slider_open = open;
        let opacity = if open { 1.0 } else { 0.0 };
        log(comp.animate_scalar(&self.sound_back.visual, "Opacity", opacity, FADE));
        log(comp.animate_scalar(&self.slider, "Opacity", opacity, FADE));
        let look = if open { self.m.clear() } else { self.m.plain() };
        if let Some(sound) = self.buttons.get_mut(Button::Sound) {
            sound.set_look(comp, look);
        }
        self.layout(comp, self.width, true)
    }

    fn within(&self, (x, y): (f32, f32), (left, w): (f32, f32), reach: f32) -> bool {
        let top = self.top();
        x >= left && x <= left + w && y >= top - reach && y <= top + self.m.button.1 + reach
    }

    pub fn fraction_at(&self, x: f32) -> f64 {
        let (left, w) = self.bar;
        if w <= 0.0 {
            return 0.0;
        }
        f64::from(((x - left) / w).clamp(0.0, 1.0))
    }

    pub fn volume_at(&self, x: f32) -> f64 {
        let left = self.sound.0 + self.m.button.0 + 2.0;
        f64::from(((x - left) / SLIDER).clamp(0.0, 1.0))
    }

    fn on_slider(&self, p: (f32, f32)) -> bool {
        let x = self.sound.0 + self.m.button.0;
        self.slider_open && self.within(p, (x, self.sound.1 - self.m.button.0), 0.0)
    }

    pub fn at(&self, p: (f32, f32)) -> Option<Spot> {
        if let Some(steps) = &self.steps
            && let Some(i) = steps.enabled_at(p)
        {
            return Some(Spot::Step(i));
        }
        if self.on_slider(p) {
            return Some(Spot::Volume(self.volume_at(p.0)));
        }
        if let Some(button) = self.buttons.enabled_at(p) {
            // The speed button sits under its row while the row is open.
            if !(button == Button::Speed && self.steps.is_some()) {
                return Some(Spot::Button(button));
            }
        }
        if self.bar.1 >= HIDDEN_BAR
            && self.within(p, (self.bar.0 - KNOB / 2.0, self.bar.1 + KNOB), BAR_REACH)
        {
            return Some(Spot::Bar(self.fraction_at(p.0)));
        }
        if self.within(p, self.length_box, 0.0) {
            return Some(Spot::Length);
        }
        None
    }

    pub fn on_row(&self, p: (f32, f32)) -> bool {
        self.steps.is_some() && self.within(p, self.speed, BAR_REACH)
    }

    pub fn covers(&self, (_, y): (f32, f32)) -> bool {
        y >= self.top() - BAR_REACH && y <= self.top() + self.m.button.1 + BAR_REACH
    }

    /// `p` in the panel's coordinates; `holding` = the volume is being dragged: the slider stays.
    /// `None` = the pointer left the panel.
    pub fn hover(&mut self, comp: &Comp, p: Option<(f32, f32)>, holding: bool) -> Result<()> {
        self.buttons.hover(comp, p);
        if let Some(steps) = &mut self.steps {
            steps.hover(comp, p);
        }
        let on_sound = p.is_some_and(|p| self.within(p, self.sound, 0.0));
        if !holding {
            self.set_slider(comp, on_sound)?;
        }
        let on_bar = p.and_then(|p| match self.at(p) {
            Some(Spot::Bar(f)) => Some(f),
            _ => None,
        });
        self.set_bar_hover(comp, on_bar.is_some());
        self.set_tip(comp, on_bar)
    }

    fn set_bar_hover(&mut self, comp: &Comp, on: bool) {
        if on == self.bar_hovered {
            return;
        }
        self.bar_hovered = on;
        let k = if on { RAIL_HOVER } else { 1.0 };
        log(comp.animate_vector3(&self.rail, "Scale", scale(1.0, k), FADE));
        log(comp.animate_scalar(&self.knob, "Opacity", if on { 1.0 } else { 0.0 }, FADE));
        if !on {
            log(self.ghost.SetSize(v2(0.0, RAIL)));
        }
    }

    /// The time under the bar point `fraction` above the bar; `None` hides it. Also while the bar is
    /// dragged: then the knob stays out.
    pub fn set_tip(&mut self, comp: &Comp, fraction: Option<f64>) -> Result<()> {
        let Some(f) = fraction else {
            if self.tip_shown {
                self.tip_shown = false;
                log(comp.animate_scalar(&self.tip, "Opacity", 0.0, FADE));
            }
            return Ok(());
        };
        let text = play::clock(f * self.duration, self.duration);
        comp.set_text(&mut self.tip_text, &text, self.k)?;
        let w = self.tip_text.width + 2.0 * TIP_PAD;
        self.tip_back.SetSize(v2(w, TIP_H))?;
        let style = time_style(self.m);
        self.tip_text.visual.SetOffset(v3(
            snap(TIP_PAD, self.k),
            snap(TIP_H / 2.0 - comp.cap_middle(style), self.k),
        ))?;
        let (left, bw) = self.bar;
        let cursor = left + f as f32 * bw;
        let x = (cursor - w / 2.0).clamp(self.m.margin, (self.width - self.m.margin - w).max(0.0));
        let y = self.top() - TIP_H - TIP_GAP;
        self.tip.SetOffset(v3(snap(x, self.k), snap(y, self.k)))?;
        self.ghost.SetSize(v2(cursor - left, RAIL))?;
        if !self.tip_shown {
            self.tip_shown = true;
            log(comp.animate_scalar(&self.tip, "Opacity", 1.0, FADE));
        }
        Ok(())
    }

    /// While the bar is dragged it stays thick with its knob, wherever the cursor goes.
    pub fn hold_bar(&mut self, comp: &Comp, on: bool) {
        self.set_bar_hover(comp, on);
    }

    pub fn press(&mut self, comp: &Comp, p: (f32, f32)) -> bool {
        if let Some(steps) = &mut self.steps
            && steps.press(comp, p)
        {
            return true;
        }
        self.buttons.press(comp, p)
    }

    pub fn release(&mut self, comp: &Comp, p: (f32, f32)) -> Option<Spot> {
        if let Some(steps) = &mut self.steps
            && steps.is_pressed()
        {
            return steps.release(comp, p).map(Spot::Step);
        }
        self.buttons.release(comp, p).map(Spot::Button)
    }

    pub fn cancel(&mut self, comp: &Comp) {
        if let Some(steps) = &mut self.steps {
            steps.cancel(comp);
        }
        self.buttons.cancel(comp);
    }
}

/// The icon of a button; `None` = the speed, which shows its label.
fn face_of(show: Show, button: Button) -> Option<Glyph> {
    Some(match button {
        Button::Play if show.ended => Glyph::Replay,
        Button::Play if show.running => Glyph::Pause,
        Button::Play => Glyph::Play,
        Button::Back => Glyph::Rewind,
        Button::Forward => Glyph::Forward,
        Button::Sound if show.muted => Glyph::Muted,
        Button::Sound => Glyph::Volume,
        Button::Shot => Glyph::Camera,
        Button::Full if show.full => Glyph::ExitFullScreen,
        Button::Full => Glyph::FullScreen,
        Button::Speed => return None,
    })
}

fn capsule(comp: &Comp, k: f32, m: Metrics, button: Button, show: Show) -> Result<Capsule> {
    match face_of(show, button) {
        Some(glyph) => Capsule::new(
            comp,
            k,
            m.button,
            Face::Glyph(glyph, m.icon, color(TEXT, 1.0)),
            m.plain(),
        ),
        None => {
            let label = speed_label(show.speed);
            let w = speed_width(comp, m, show.speed);
            Capsule::new(
                comp,
                k,
                (w, m.button.1),
                Face::Text(&label, label_style(m, TEXT)),
                m.plain(),
            )
        }
    }
}

fn speed_width(comp: &Comp, m: Metrics, speed: usize) -> f32 {
    Capsule::text_width(comp, &speed_label(speed), label_style(m, TEXT), m.button.1).max(m.button.0)
}

fn step_width(comp: &Comp, m: Metrics, step: usize) -> f32 {
    let h = m.button.1 - 2.0 * STEP_INSET;
    Capsule::text_width(comp, &speed_label(step), label_style(m, TEXT), h).max(h * 1.4)
}

fn row_width(comp: &Comp, m: Metrics) -> f32 {
    (0..SPEEDS.len())
        .map(|i| step_width(comp, m, i))
        .sum::<f32>()
        + 2.0 * STEP_INSET
}

fn time_style(m: Metrics) -> TextStyle {
    TextStyle {
        size: m.text,
        bold: false,
        color: color(TEXT, 1.0),
        tabular: true,
    }
}

fn label_style(m: Metrics, ink: &str) -> TextStyle {
    TextStyle {
        size: m.text - 1.0,
        bold: true,
        color: color(ink, 1.0),
        tabular: true,
    }
}

fn snap(v: f32, k: f32) -> f32 {
    (v * k).round() / k
}

fn resize(comp: &Comp, geometry: &CompositionRoundedRectangleGeometry, w: f32, animate: bool) {
    let h = geometry.Size().map_or(RAIL, |s| s.Y);
    if animate {
        log(comp.animate_vector2(geometry, "Size", v2(w, h), MOVE));
    } else {
        log(comp.stop(geometry, "Size"));
        log(geometry.SetSize(v2(w, h)));
    }
}

fn shape(comp: &Comp, (w, h): (f32, f32), ink: windows::UI::Color) -> Result<Shape> {
    let c = &comp.compositor;
    let visual = c.CreateShapeVisual()?;
    visual.SetSize(v2(SHAPES_WIDTH, h))?;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetSize(v2(w, h))?;
    geometry.SetCornerRadius(v2(h / 2.0, h / 2.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&geometry)?;
    sprite.SetFillBrush(&comp.brush(ink)?)?;
    visual.Shapes()?.Append(&sprite)?;
    Ok(Shape { visual, geometry })
}

fn rounded(
    comp: &Comp,
    into: &ShapeVisual,
    ink: windows::UI::Color,
) -> Result<CompositionRoundedRectangleGeometry> {
    let c = &comp.compositor;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetSize(v2(0.0, RAIL))?;
    geometry.SetCornerRadius(v2(RAIL / 2.0, RAIL / 2.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&geometry)?;
    sprite.SetFillBrush(&comp.brush(ink)?)?;
    into.Shapes()?.Append(&sprite)?;
    Ok(geometry)
}

fn circle(comp: &Comp, d: f32, ink: windows::UI::Color) -> Result<ShapeVisual> {
    let c = &comp.compositor;
    let visual = c.CreateShapeVisual()?;
    visual.SetSize(v2(d, d))?;
    let geometry = c.CreateEllipseGeometry()?;
    geometry.SetCenter(v2(d / 2.0, d / 2.0))?;
    geometry.SetRadius(v2(d / 2.0, d / 2.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&geometry)?;
    sprite.SetFillBrush(&comp.brush(ink)?)?;
    visual.Shapes()?.Append(&sprite)?;
    Ok(visual)
}
