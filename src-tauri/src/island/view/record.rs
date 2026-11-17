//! Screen recording on the island.
//! - **countdown** (a setting): a 30 px circle, a number and a red ring running out;
//! - **recording**: pulsing dot, REC, time, pause and stop. The island keeps its width:
//!   the time sits between REC and the buttons, and as it gets longer the gaps around it
//!   shrink (the island grows only past an hour, when the gaps are at their minimum).
//!   On hover the island widens by the same amount on each side: file size appears on the
//!   left, a track with a ✕ knob on the right — sliding it to the end cancels the
//!   recording. The middle does not move.

use super::super::comp::{Motion, Text, color, v2, v3};
use super::super::glyph::Glyph;
use super::{
    Button, Content, FADE_IN, FADE_OUT, Hit, IconButton, MUTED, Model, Rect, Scene, TEXT,
    clock_style, format_clock, format_size, snap, style,
};
use windows::{
    UI::Composition::{
        CompositionColorBrush, CompositionSpriteShape, CompositionStrokeCap, ContainerVisual,
        ShapeVisual, Visual,
    },
    core::{Interface, Result},
};

/// Same red as the record button in the control center.
const RED: &str = "#FF375F";
const HEIGHT: f32 = 30.0;
const PAD: f32 = 12.0;
const DOT: f32 = 8.0;
const DOT_GAP: f32 = 6.0;
const GAP: f32 = 10.0;
const MIN_GAP: f32 = 5.0;
const BUTTON: f32 = 22.0;
const BUTTON_GAP: f32 = 4.0;
/// Button circles are concentric with the capsule's rounding.
const INSET: f32 = (HEIGHT - BUTTON) / 2.0;
const SIDE: f32 = 60.0;
const SIZE_LEFT: f32 = 14.0;
const TRACK_GAP: f32 = 6.0;
const ICON: f32 = 12.0;
const STOP_ICON: f32 = 16.0;
const CANCEL_ICON: f32 = 12.0;
const CHEVRON: f32 = 12.0;
const PULSE_MS: u32 = 1600;
const BUTTON_FILL: &str = "#2C2C2E";
const BUTTON_HOVER: &str = "#3A3A3C";
const ROLL_BACK: Motion = Motion::curve(260, 0.32, 1.28, 0.5, 1.0);
/// Chevrons get brighter towards the end of the track: that is where to slide.
const CHEVRONS: [f32; 3] = [0.35, 0.65, 1.0];

const COUNTDOWN: f32 = 30.0;

pub struct RecordModel {
    pub elapsed_ms: u64,
    pub paused: bool,
    pub bytes: u64,
    pub hover: bool,
}

pub(super) struct Parts {
    canvas: ContainerVisual,
    dot: ShapeVisual,
    dot_shape: CompositionSpriteShape,
    dot_brush: CompositionColorBrush,
    rec: Text,
    time: Text,
    size: Text,
    pause: Visual,
    pause_icon: Visual,
    play_icon: Visual,
    stop: Visual,
    track: Visual,
    knob: Visual,
    chevrons: Vec<(Visual, f32, f32)>,
    /// What is shown: paused, hover. `None` — just built.
    shown: Option<(bool, bool)>,
}

fn rec_style(hex: &str) -> super::super::comp::TextStyle {
    style(11.0, true, hex)
}

/// Width between REC and the buttons and the gap on each side of the time.
/// Pure arithmetic: `time` is the width of the time text, `shortest` of "0:00".
fn middle(time: f32, shortest: f32) -> (f32, f32) {
    let width = (shortest + 2.0 * GAP).max(time + 2.0 * MIN_GAP);
    (width, (width - time) / 2.0)
}

const TRACK: f32 = SIDE - TRACK_GAP;
const TRAVEL: f32 = TRACK - BUTTON;

impl Scene {
    fn core(&self, time: &str) -> (f32, f32) {
        let rec = self.comp.measure("REC", rec_style(RED));
        let style = clock_style(13.0, TEXT);
        let (middle, gap) = middle(
            self.comp.measure(time, style),
            self.comp.measure("0:00", style),
        );
        let width = PAD + DOT + DOT_GAP + rec + middle + BUTTON + BUTTON_GAP + BUTTON + INSET;
        (width, gap)
    }

    pub(super) fn recording_size(&self, model: &Model) -> (f32, f32) {
        let Some(record) = &model.record else {
            return (COUNTDOWN, HEIGHT);
        };
        let (core, _) = self.core(&format_clock(record.elapsed_ms as i64));
        let side = if record.hover { 2.0 * SIDE } else { 0.0 };
        ((core + side).round(), HEIGHT)
    }

    pub(super) fn build_recording(&self, content: &mut Content) -> Result<()> {
        content.height = HEIGHT;
        let c = &self.comp.compositor;
        let canvas = self.comp.container()?;
        content.root.Children()?.InsertAtTop(&canvas)?;

        let dot = c.CreateShapeVisual()?;
        dot.SetSize(v2(DOT, DOT))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(DOT / 2.0, DOT / 2.0))?;
        geometry.SetRadius(v2(DOT / 2.0, DOT / 2.0))?;
        let dot_shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        let dot_brush = self.comp.brush(color(RED, 1.0))?;
        dot_shape.SetFillBrush(&dot_brush)?;
        dot_shape.SetCenterPoint(v2(DOT / 2.0, DOT / 2.0))?;
        dot.Shapes()?.Append(&dot_shape)?;
        canvas.Children()?.InsertAtTop(&dot)?;

        let rec = self.comp.text("REC", rec_style(RED), self.scale)?;
        canvas.Children()?.InsertAtTop(&rec.visual)?;
        let time = self.comp.text("", clock_style(13.0, TEXT), self.scale)?;
        canvas.Children()?.InsertAtTop(&time.visual)?;
        let size = self.comp.text("", style(12.0, true, MUTED), self.scale)?;
        size.visual.SetOpacity(0.0)?;
        canvas.Children()?.InsertAtTop(&size.visual)?;

        let (pause, pause_fill) = self.round_button(&canvas)?;
        let pause_icon = self.icon_in(&pause, Glyph::Pause, ICON, TEXT)?;
        let play_icon = self.icon_in(&pause, Glyph::Play, ICON, TEXT)?;
        play_icon.SetOpacity(0.0)?;
        let (stop, stop_fill) = self.round_button(&canvas)?;
        self.icon_in(&stop, Glyph::RecordStop, STOP_ICON, RED)?;

        let track = self.comp.container()?;
        track.SetOpacity(0.0)?;
        canvas.Children()?.InsertAtTop(&track)?;
        let groove = c.CreateShapeVisual()?;
        groove.SetSize(v2(TRACK, BUTTON))?;
        let pill = c.CreateRoundedRectangleGeometry()?;
        pill.SetSize(v2(TRACK, BUTTON))?;
        pill.SetCornerRadius(v2(BUTTON / 2.0, BUTTON / 2.0))?;
        let groove_shape = c.CreateSpriteShapeWithGeometry(&pill)?;
        groove_shape.SetFillBrush(&self.comp.brush(color("#FFFFFF", 0.08))?)?;
        groove.Shapes()?.Append(&groove_shape)?;
        track.Children()?.InsertAtTop(&groove)?;
        let mut chevrons = Vec::new();
        let step = TRAVEL / CHEVRONS.len() as f32;
        for (i, opacity) in CHEVRONS.into_iter().enumerate() {
            // A drawn chevron, not the text one: a font puts it low, under the button's middle.
            let chevron =
                self.comp
                    .glyph(Glyph::Chevron, CHEVRON, color(MUTED, 1.0), self.scale)?;
            let middle = BUTTON + step * (i as f32 + 0.5);
            chevron.SetOffset(v3(
                snap(middle - CHEVRON / 2.0, self.scale),
                snap((BUTTON - CHEVRON) / 2.0, self.scale),
            ))?;
            chevron.SetOpacity(opacity)?;
            track.Children()?.InsertAtTop(&chevron)?;
            chevrons.push((chevron.cast()?, middle, opacity));
        }
        let (knob, knob_fill) = self.round_button(&track)?;
        self.icon_in(&knob, Glyph::Close, CANCEL_ICON, TEXT)?;

        let base = color(BUTTON_FILL, 1.0);
        let hover = color(BUTTON_HOVER, 1.0);
        for (visual, brush, hit) in [
            (&pause, pause_fill, Hit::RecordPause),
            (&stop, stop_fill, Hit::RecordStop),
            (&knob, knob_fill, Hit::RecordCancel),
        ] {
            content.buttons.push(Button {
                fill: brush,
                rect: Rect::default(),
                hit: hit.clone(),
                base,
                hover,
            });
            content.icon_buttons.push(IconButton {
                visual: visual.cast()?,
                hit,
            });
        }
        content.record = Some(Parts {
            canvas,
            dot,
            dot_shape,
            dot_brush,
            rec,
            time,
            size,
            pause: pause.cast()?,
            pause_icon,
            play_icon,
            stop: stop.cast()?,
            track: track.cast()?,
            knob: knob.cast()?,
            chevrons,
            shown: None,
        });
        Ok(())
    }

    fn round_button(
        &self,
        parent: &ContainerVisual,
    ) -> Result<(ContainerVisual, CompositionColorBrush)> {
        let c = &self.comp.compositor;
        let body = self.comp.container()?;
        body.SetSize(v2(BUTTON, BUTTON))?;
        body.SetCenterPoint(v3(BUTTON / 2.0, BUTTON / 2.0))?;
        let back = c.CreateShapeVisual()?;
        back.SetSize(v2(BUTTON, BUTTON))?;
        let circle = c.CreateEllipseGeometry()?;
        circle.SetCenter(v2(BUTTON / 2.0, BUTTON / 2.0))?;
        circle.SetRadius(v2(BUTTON / 2.0, BUTTON / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&circle)?;
        let brush = self.comp.brush(color(BUTTON_FILL, 1.0))?;
        shape.SetFillBrush(&brush)?;
        back.Shapes()?.Append(&shape)?;
        body.Children()?.InsertAtTop(&back)?;
        parent.Children()?.InsertAtTop(&body)?;
        Ok((body, brush))
    }

    fn icon_in(
        &self,
        body: &ContainerVisual,
        glyph: Glyph,
        size: f32,
        hex: &str,
    ) -> Result<Visual> {
        let icon = self.comp.glyph(glyph, size, color(hex, 1.0), self.scale)?;
        let inset = (BUTTON - size) / 2.0;
        icon.SetOffset(v3(inset, inset))?;
        body.Children()?.InsertAtTop(&icon)?;
        icon.cast()
    }

    pub(super) fn update_recording(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (Some(record), Some(parts)) = (&model.record, &mut content.record) else {
            return Ok(());
        };
        let s = self.scale;
        let clock = format_clock(record.elapsed_ms as i64);
        self.comp.set_text(&mut parts.time, &clock, s)?;
        self.comp
            .set_text(&mut parts.size, &format_size(record.bytes), s)?;

        // Layout: the core starts SIDE in; the canvas is always the hovered width and
        // slides so that the core stays in the middle of the capsule.
        let (core, gap) = self.core(&clock);
        let full = core + 2.0 * SIDE;
        let mut x = SIDE + PAD;
        parts
            .dot
            .SetOffset(v3(snap(x, s), snap((HEIGHT - DOT) / 2.0, s)))?;
        x += DOT + DOT_GAP;
        parts
            .rec
            .visual
            .SetOffset(v3(snap(x, s), snap((HEIGHT - parts.rec.height) / 2.0, s)))?;
        x += parts.rec.width + gap;
        parts
            .time
            .visual
            .SetOffset(v3(snap(x, s), snap((HEIGHT - parts.time.height) / 2.0, s)))?;
        x += parts.time.width + gap;
        let pause_x = x;
        let stop_x = x + BUTTON + BUTTON_GAP;
        let track_x = stop_x + BUTTON + TRACK_GAP;
        parts
            .pause
            .SetOffset(v3(snap(pause_x, s), snap(INSET, s)))?;
        parts.stop.SetOffset(v3(snap(stop_x, s), snap(INSET, s)))?;
        parts
            .track
            .SetOffset(v3(snap(track_x, s), snap(INSET, s)))?;
        parts.size.visual.SetOffset(v3(
            snap(SIZE_LEFT, s),
            snap((HEIGHT - parts.size.height) / 2.0, s),
        ))?;

        let width = (core + if record.hover { 2.0 * SIDE } else { 0.0 }).round();
        let shift = (width - full) / 2.0;
        parts.canvas.SetOffset(v3(snap(shift, s), 0.0))?;
        content.width = width;

        content.hits.clear();
        let half = BUTTON_GAP / 2.0;
        content.hits.push((
            Rect {
                x: shift + pause_x - gap / 2.0,
                y: 0.0,
                w: BUTTON + gap / 2.0 + half,
                h: HEIGHT,
            },
            Hit::RecordPause,
        ));
        content.hits.push((
            Rect {
                x: shift + stop_x - half,
                y: 0.0,
                w: BUTTON + half + TRACK_GAP / 2.0,
                h: HEIGHT,
            },
            Hit::RecordStop,
        ));
        if record.hover {
            // Only the knob is grabbed; the rest of the track is where it goes.
            content.hits.push((
                Rect {
                    x: shift + track_x - TRACK_GAP / 2.0,
                    y: 0.0,
                    w: BUTTON + TRACK_GAP / 2.0,
                    h: HEIGHT,
                },
                Hit::RecordCancel,
            ));
        }

        let want = (record.paused, record.hover);
        let first = parts.shown.is_none();
        let (was_paused, was_hover) = parts.shown.unwrap_or((!record.paused, !record.hover));
        if record.paused != was_paused {
            let hex = if record.paused { MUTED } else { TEXT };
            let rec_hex = if record.paused { MUTED } else { RED };
            self.comp
                .set_text_color(&mut parts.time, color(hex, 1.0), s)?;
            self.comp
                .set_text_color(&mut parts.rec, color(rec_hex, 1.0), s)?;
            parts.dot_brush.SetColor(color(rec_hex, 1.0))?;
            let (pause, play) = if record.paused {
                (0.0, 1.0)
            } else {
                (1.0, 0.0)
            };
            parts.pause_icon.SetOpacity(pause)?;
            parts.play_icon.SetOpacity(play)?;
            if record.paused {
                self.comp.stop(&parts.dot, "Opacity")?;
                self.comp.stop(&parts.dot_shape, "Scale")?;
                parts.dot.SetOpacity(1.0)?;
                parts.dot_shape.SetScale(v2(1.0, 1.0))?;
            } else {
                self.pulse(parts)?;
            }
        }
        if record.hover != was_hover {
            let opacity = if record.hover { 1.0 } else { 0.0 };
            for visual in [&parts.size.visual.cast::<Visual>()?, &parts.track] {
                if first {
                    visual.SetOpacity(opacity)?;
                } else {
                    let motion = if record.hover { FADE_IN } else { FADE_OUT };
                    self.comp
                        .animate_scalar(visual, "Opacity", opacity, motion)?;
                }
            }
        }
        parts.shown = Some(want);
        Ok(())
    }

    pub fn cancel_travel(&self) -> f32 {
        TRAVEL
    }

    pub fn slide_cancel(&self, offset: f32) -> Result<()> {
        let Some(parts) = self.content.as_ref().and_then(|c| c.record.as_ref()) else {
            return Ok(());
        };
        let offset = offset.clamp(0.0, TRAVEL);
        parts.knob.SetOffset(v3(snap(offset, self.scale), 0.0))?;
        let edge = BUTTON + offset;
        for (chevron, middle, opacity) in &parts.chevrons {
            let left = ((middle + 3.0 - edge) / 6.0).clamp(0.0, 1.0);
            chevron.SetOpacity(opacity * left)?;
        }
        Ok(())
    }

    pub fn slide_back(&self) -> Result<()> {
        let Some(parts) = self.content.as_ref().and_then(|c| c.record.as_ref()) else {
            return Ok(());
        };
        self.comp
            .animate_vector3(&parts.knob, "Offset", v3(0.0, 0.0), ROLL_BACK)?;
        for (chevron, _, opacity) in &parts.chevrons {
            self.comp
                .animate_scalar(chevron, "Opacity", *opacity, FADE_IN)?;
        }
        Ok(())
    }

    /// The dot breathes: the compositor runs it, the process sleeps.
    fn pulse(&self, parts: &Parts) -> Result<()> {
        let ease = Motion::curve(0, 0.45, 0.0, 0.55, 1.0);
        self.comp.loop_scalar(
            &parts.dot,
            "Opacity",
            &[(0.0, 1.0, ease), (0.5, 0.35, ease), (1.0, 1.0, ease)],
            PULSE_MS,
        )?;
        self.comp.loop_vector2(
            &parts.dot_shape,
            "Scale",
            &[
                (0.0, v2(1.0, 1.0)),
                (0.5, v2(0.85, 0.85)),
                (1.0, v2(1.0, 1.0)),
            ],
            PULSE_MS,
        )
    }

    /// The ring empties over the whole countdown: the view is built at its first second.
    pub(super) fn build_countdown(&self, model: &Model, content: &mut Content) -> Result<()> {
        content.width = COUNTDOWN;
        content.height = COUNTDOWN;
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(COUNTDOWN, COUNTDOWN))?;
        let center = v2(COUNTDOWN / 2.0, COUNTDOWN / 2.0);
        let radius = 12.0;
        let circle = || -> Result<_> {
            let geometry = c.CreateEllipseGeometry()?;
            geometry.SetCenter(center)?;
            geometry.SetRadius(v2(radius, radius))?;
            Ok(geometry)
        };
        let track = c.CreateSpriteShapeWithGeometry(&circle()?)?;
        track.SetStrokeBrush(&self.comp.brush(color(TEXT, 0.25))?)?;
        track.SetStrokeThickness(2.2)?;
        visual.Shapes()?.Append(&track)?;
        let arc_geometry = circle()?;
        let arc = c.CreateSpriteShapeWithGeometry(&arc_geometry)?;
        arc.SetStrokeBrush(&self.comp.brush(color(RED, 1.0))?)?;
        arc.SetStrokeThickness(2.2)?;
        arc.SetStrokeStartCap(CompositionStrokeCap::Round)?;
        arc.SetStrokeEndCap(CompositionStrokeCap::Round)?;
        visual.Shapes()?.Append(&arc)?;
        self.comp.animate_scalar_from(
            &arc_geometry,
            "TrimEnd",
            1.0,
            0.0,
            Motion::linear(model.countdown.unwrap_or(1).max(1) * 1000),
        )?;
        content.root.Children()?.InsertAtTop(&visual)?;

        let number = self.comp.text("", clock_style(15.0, TEXT), self.scale)?;
        content.root.Children()?.InsertAtTop(&number.visual)?;
        content.clocks.push(super::Clock {
            text: number,
            timer: None,
            right: None,
            y: 0.0,
        });
        content.hits.push((
            Rect {
                x: 0.0,
                y: 0.0,
                w: COUNTDOWN,
                h: COUNTDOWN,
            },
            Hit::RecordNow,
        ));
        Ok(())
    }

    pub(super) fn update_countdown(&self, model: &Model, content: &mut Content) -> Result<()> {
        let s = self.scale;
        if let Some(clock) = content.clocks.first_mut() {
            let left = model.countdown.unwrap_or(0).to_string();
            self.comp.set_text(&mut clock.text, &left, s)?;
            clock.text.visual.SetOffset(v3(
                snap((COUNTDOWN - clock.text.width) / 2.0, s),
                snap((COUNTDOWN - clock.text.height) / 2.0, s),
            ))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_time_eats_the_gaps_not_the_island() {
        assert_eq!(middle(28.0, 28.0), (48.0, 10.0));
        assert_eq!(middle(36.0, 28.0), (48.0, 6.0));
        assert_eq!(middle(50.0, 28.0), (60.0, 5.0));
    }
}
