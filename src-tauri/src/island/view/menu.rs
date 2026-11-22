//! Control-center view: round buttons, four per row, a label under each. Sizes and colors come from the
//! mockup's "Control menu" section; the columns are wider than the mockup's so that the
//! "Do Not Disturb" label fits.
//!
//! The view knows buttons only by [`ControlId`] and state: what they do is decided by `control`. State is not
//! part of the view key: on a press the button flows in place: the fill changes color, the icon and label
//! swap by cross-fading.
//!
//! Holding the screenshot button draws a ring around it: when the ring closes, the area
//! selection opens.

use super::super::comp::{Motion, Text, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::{Content, HOLD_MS, HOVER, Hit, LATCH, MUTED, Model, Rect, Scene, TEXT, strip, style};
use crate::control::{ControlId, ControlState, Item};
use crate::lang::tr;
use std::borrow::Cow;
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionColorBrush, CompositionEllipseGeometry, CompositionStrokeCap,
            ContainerVisual, Visual,
        },
    },
    core::{Interface, Result},
};

const COLUMNS: usize = 4;
const PAD: f32 = 14.0;
const COLUMN: f32 = 68.0;
const GAP: f32 = 6.0;
pub(super) const CIRCLE: f32 = 44.0;
/// The same for any number of buttons: the pre-start panel takes it too.
pub(super) const WIDTH: f32 = 2.0 * PAD + COLUMNS as f32 * COLUMN + (COLUMNS - 1) as f32 * GAP;
const ICON: f32 = 20.0;
const LABEL_GAP: f32 = 6.0;
const LABEL_LINE: f32 = 14.0;
const ROW: f32 = CIRCLE + LABEL_GAP + LABEL_LINE;
const ROW_GAP: f32 = 12.0;

const SWITCH: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);
const UNHOLD: Motion = Motion::curve(160, 0.4, 0.0, 1.0, 1.0);
const HOLD_RING: f32 = 54.0;
/// A click is shorter than this, so it shows no ring; the ring still closes at [`HOLD_MS`].
const HOLD_QUIET_MS: u32 = 150;
const HOLD_STROKE: f32 = 2.5;
const PRESSED_SCALE: f32 = 0.9;

const IDLE_FILL: &str = "#1C1C1E";
const IDLE_HOVER: &str = "#2C2C2E";
const ON_HOVER: &str = "#E5E5EA";
/// Muted sound and microphone: the fill and icon are slightly red.
pub(super) const MUTE_RED: &str = "#FF453A";
pub(super) const MUTE_ICON: &str = "#FF6961";
pub(super) const MUTE_FILL: f32 = 0.24;
pub(super) const MUTE_HOVER: f32 = 0.32;
const REC: &str = "#FF375F";
const REC_HOVER: &str = "#FF5C7A";

pub fn size(count: usize) -> (f32, f32) {
    let rows = count.div_ceil(COLUMNS).max(1) as f32;
    (WIDTH, 2.0 * PAD + rows * ROW + (rows - 1.0) * ROW_GAP)
}

pub(super) fn menu_size(model: &Model) -> (f32, f32) {
    let (width, height) = size(model.controls.len());
    match model.strip {
        Some(_) => (width, height + strip::GAP + strip::HEIGHT),
        None => (width, height),
    }
}

/// The circles' outer edge: the bottom line keeps to it.
const EDGE: f32 = PAD + (COLUMN - CIRCLE) / 2.0;

struct Look {
    fill: Color,
    hover: Color,
    glyph: Glyph,
    icon: Color,
    /// `None` = the recording time instead.
    label: Option<Cow<'static, str>>,
    label_color: Color,
}

fn look(id: ControlId, active: bool) -> Look {
    let idle = |glyph, label| Look {
        fill: color(IDLE_FILL, 1.0),
        hover: color(IDLE_HOVER, 1.0),
        glyph,
        icon: color(TEXT, 1.0),
        label: Some(label),
        label_color: color(MUTED, 1.0),
    };
    let on = |glyph, label| Look {
        fill: color(TEXT, 1.0),
        hover: color(ON_HOVER, 1.0),
        glyph,
        icon: color("#000000", 1.0),
        label: Some(label),
        label_color: color(TEXT, 1.0),
    };
    let muted = |glyph, label| Look {
        fill: color(MUTE_RED, MUTE_FILL),
        hover: color(MUTE_RED, MUTE_HOVER),
        glyph,
        icon: color(MUTE_ICON, 1.0),
        label: Some(label),
        label_color: color(TEXT, 1.0),
    };
    match (id, active) {
        (ControlId::Sound, false) => idle(Glyph::Volume, tr!("menu.sound")),
        (ControlId::Sound, true) => muted(Glyph::Muted, tr!("menu.muted")),
        (ControlId::Microphone, false) => idle(Glyph::Microphone, tr!("menu.microphone")),
        (ControlId::Microphone, true) => muted(Glyph::MicrophoneOff, tr!("menu.microphone_off")),
        (ControlId::DoNotDisturb, false) => idle(Glyph::Moon, tr!("menu.do_not_disturb")),
        (ControlId::DoNotDisturb, true) => on(Glyph::Moon, tr!("menu.do_not_disturb")),
        (ControlId::KeepAwake, false) => idle(Glyph::Coffee, tr!("menu.keep_awake")),
        (ControlId::KeepAwake, true) => on(Glyph::Coffee, tr!("menu.keep_awake")),
        (ControlId::Screenshot, _) => idle(Glyph::Screenshot, tr!("menu.screenshot")),
        (ControlId::Scan, _) => idle(Glyph::ScanText, tr!("menu.scan")),
        (ControlId::Record, false) => Look {
            icon: color(REC, 1.0),
            ..idle(Glyph::RecordDot, tr!("menu.record"))
        },
        (ControlId::Record, true) => Look {
            fill: color(REC, 1.0),
            hover: color(REC_HOVER, 1.0),
            glyph: Glyph::RecordStop,
            icon: color("#FFFFFF", 1.0),
            label: None,
            label_color: color(REC, 1.0),
        },
        (ControlId::Settings, _) => idle(Glyph::Gear, tr!("menu.settings")),
    }
}

fn switchable(id: ControlId) -> bool {
    !matches!(
        id,
        ControlId::Screenshot | ControlId::Scan | ControlId::Settings
    )
}

struct Layer {
    icon: Visual,
    label: Visual,
    look: Look,
}

pub(super) struct MenuButton {
    id: ControlId,
    circle: ContainerVisual,
    fill: CompositionColorBrush,
    idle: Layer,
    active: Option<Layer>,
    clock: Option<Text>,
    label_at: (f32, f32),
    /// The position shown now. `None` = not shown yet.
    shown: Option<bool>,
}

impl MenuButton {
    fn look(&self, active: bool) -> &Look {
        match (&self.active, active) {
            (Some(layer), true) => &layer.look,
            _ => &self.idle.look,
        }
    }
}

pub(super) struct Parts {
    buttons: Vec<MenuButton>,
    /// Around each button that can be held: fills while it is.
    holds: Vec<(ControlId, CompositionEllipseGeometry)>,
    strip: Option<strip::Parts>,
}

/// The middle of a button's circle, in the menu's own coordinates.
pub fn button_center(controls: &[Item], id: ControlId) -> Option<(f32, f32)> {
    let i = controls.iter().position(|c| c.id == id)?;
    let count = controls.len();
    let last = count % COLUMNS;
    let shift = if last != 0 && i >= count - last {
        (COLUMNS - last) as f32 * (COLUMN + GAP) / 2.0
    } else {
        0.0
    };
    Some((
        PAD + (i % COLUMNS) as f32 * (COLUMN + GAP) + shift + COLUMN / 2.0,
        PAD + (i / COLUMNS) as f32 * (ROW + ROW_GAP) + CIRCLE / 2.0,
    ))
}

fn is_active(model: &Model, id: ControlId) -> Option<(bool, Option<u64>)> {
    model
        .controls
        .iter()
        .find(|i| i.id == id)
        .map(|i| match i.state {
            ControlState::Running { since } => (true, Some(since)),
            state => (state.is_active(), None),
        })
}

fn elapsed(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Scene {
    pub(super) fn build_menu(&self, model: &Model, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let (width, buttons_height) = size(model.controls.len());
        (content.width, content.height) = menu_size(model);
        let mut buttons = Vec::with_capacity(model.controls.len());
        for (i, item) in model.controls.iter().enumerate() {
            let rect = Rect {
                x: PAD + (i % COLUMNS) as f32 * (COLUMN + GAP),
                y: PAD + (i / COLUMNS) as f32 * (ROW + ROW_GAP),
                w: COLUMN,
                h: ROW,
            };
            buttons.push(self.menu_button(&root, *item, rect)?);
            content.hits.push((rect, Hit::Control(item.id)));
        }
        let last = model.controls.len() % COLUMNS;
        if last != 0 {
            let shift = (COLUMNS - last) as f32 * (COLUMN + GAP) / 2.0;
            let start = model.controls.len() - last;
            for (button, (rect, _)) in buttons[start..]
                .iter_mut()
                .zip(content.hits[start..].iter_mut())
            {
                rect.x += shift;
                button.label_at.0 += shift;
                for visual in [button.circle.cast::<Visual>()?, button.idle.label.clone()]
                    .into_iter()
                    .chain(button.active.as_ref().map(|l| l.label.clone()))
                {
                    let at = visual.Offset()?;
                    visual.SetOffset(v3(at.X + shift, at.Y))?;
                }
            }
        }
        let mut holds = Vec::new();
        for item in model.controls.iter().filter(|i| i.id.holds()) {
            if let Some(center) = button_center(model.controls, item.id) {
                holds.push((item.id, self.hold_ring(&root, center)?));
            }
        }
        let strip = match &model.strip {
            Some(line) => {
                let top = buttons_height - PAD + strip::GAP;
                Some(self.build_strip(&root, line, top, (width, EDGE))?)
            }
            None => None,
        };
        content.menu = Some(Parts {
            buttons,
            holds,
            strip,
        });
        Ok(())
    }

    fn hold_ring(
        &self,
        root: &ContainerVisual,
        (x, y): (f32, f32),
    ) -> Result<CompositionEllipseGeometry> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(HOLD_RING, HOLD_RING))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(HOLD_RING / 2.0, HOLD_RING / 2.0))?;
        let r = (HOLD_RING - HOLD_STROKE) / 2.0;
        geometry.SetRadius(v2(r, r))?;
        geometry.SetTrimEnd(0.0)?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetStrokeBrush(&self.comp.brush(color(TEXT, 1.0))?)?;
        shape.SetStrokeThickness(HOLD_STROKE)?;
        shape.SetStrokeStartCap(CompositionStrokeCap::Round)?;
        shape.SetStrokeEndCap(CompositionStrokeCap::Round)?;
        visual.Shapes()?.Append(&shape)?;
        self.place(root, &visual, x - HOLD_RING / 2.0, y - HOLD_RING / 2.0)?;
        Ok(geometry)
    }

    /// A button is held: its ring closes by [`HOLD_MS`]. Let go, it runs back.
    pub fn hold_button(&self, id: ControlId, holding: bool) -> Result<()> {
        let Some(ring) = self
            .content
            .as_ref()
            .and_then(|c| c.menu.as_ref())
            .and_then(|m| m.holds.iter().find(|(held, _)| *held == id))
            .map(|(_, ring)| ring)
        else {
            return Ok(());
        };
        if holding {
            self.comp.animate_scalar_after(
                ring,
                "TrimEnd",
                1.0,
                Motion::linear(HOLD_MS - HOLD_QUIET_MS),
                HOLD_QUIET_MS,
            )
        } else {
            self.comp.animate_scalar(ring, "TrimEnd", 0.0, UNHOLD)
        }
    }

    fn menu_button(&self, root: &ContainerVisual, item: Item, rect: Rect) -> Result<MenuButton> {
        let c = &self.comp.compositor;
        let circle = c.CreateContainerVisual()?;
        circle.SetSize(v2(CIRCLE, CIRCLE))?;
        circle.SetCenterPoint(v3(CIRCLE / 2.0, CIRCLE / 2.0))?;

        let idle_look = look(item.id, false);
        let fill = self.comp.brush(idle_look.fill)?;
        let disc = c.CreateShapeVisual()?;
        disc.SetSize(v2(CIRCLE, CIRCLE))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(CIRCLE / 2.0, CIRCLE / 2.0))?;
        geometry.SetRadius(v2(CIRCLE / 2.0, CIRCLE / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&fill)?;
        disc.Shapes()?.Append(&shape)?;
        circle.Children()?.InsertAtTop(&disc)?;

        let center = rect.x + rect.w / 2.0;
        let label_y = rect.y + CIRCLE + LABEL_GAP;
        let idle = self.menu_layer(&circle, root, idle_look, center, label_y)?;
        let (active, clock) = if switchable(item.id) {
            let layer = self.menu_layer(&circle, root, look(item.id, true), center, label_y)?;
            layer.icon.SetOpacity(0.0)?;
            layer.label.SetOpacity(0.0)?;
            let clock = match layer.look.label {
                Some(_) => None,
                None => {
                    let st = super::clock_style(11.0, REC);
                    let text = self.comp.text("0:00", st, self.scale)?;
                    text.visual.SetOpacity(0.0)?;
                    self.place(root, &text.visual, center - text.width / 2.0, label_y)?;
                    Some(text)
                }
            };
            (Some(layer), clock)
        } else {
            (None, None)
        };
        self.place(root, &circle, center - CIRCLE / 2.0, rect.y)?;
        Ok(MenuButton {
            id: item.id,
            circle,
            fill,
            idle,
            active,
            clock,
            label_at: (center, label_y),
            shown: None,
        })
    }

    fn menu_layer(
        &self,
        circle: &ContainerVisual,
        root: &ContainerVisual,
        look: Look,
        center: f32,
        label_y: f32,
    ) -> Result<Layer> {
        let icon = self.comp.glyph(look.glyph, ICON, look.icon, self.scale)?;
        icon.SetOffset(v3((CIRCLE - ICON) / 2.0, (CIRCLE - ICON) / 2.0))?;
        circle.Children()?.InsertAtTop(&icon)?;
        let st = super::TextStyle {
            color: look.label_color,
            ..style(11.0, false, MUTED)
        };
        let label: Visual = match &look.label {
            Some(text) => {
                let text = self.comp.text(text, st, self.scale)?;
                self.place(root, &text.visual, center - text.width / 2.0, label_y)?;
                text.visual.cast()?
            }
            None => {
                let empty = self.comp.container()?;
                root.Children()?.InsertAtTop(&empty)?;
                empty.cast()?
            }
        };
        Ok(Layer {
            icon: icon.cast()?,
            label,
            look,
        })
    }

    pub(super) fn update_menu(&self, model: &Model, content: &mut Content) -> Result<()> {
        let Some(parts) = &mut content.menu else {
            return Ok(());
        };
        if let (Some(strip), Some(line)) = (&mut parts.strip, &model.strip) {
            self.update_strip(strip, line, (WIDTH, EDGE))?;
        }
        let hovered = self.hovered.clone();
        for button in &mut parts.buttons {
            let Some((active, since)) = is_active(model, button.id) else {
                continue;
            };
            if let (Some(clock), Some(since)) = (&mut button.clock, since) {
                self.comp.set_text(
                    clock,
                    &elapsed(model.now_ms.saturating_sub(since)),
                    self.scale,
                )?;
                clock.visual.SetOffset(v3(
                    super::snap(button.label_at.0 - clock.width / 2.0, self.scale),
                    super::snap(button.label_at.1, self.scale),
                ))?;
            }
            if button.shown == Some(active) {
                continue;
            }
            let animate = button.shown.is_some();
            button.shown = Some(active);
            let under_cursor = hovered == Some(Hit::Control(button.id));
            let look = button.look(active);
            let fill = if under_cursor { look.hover } else { look.fill };
            let Some(layer) = &button.active else {
                continue;
            };
            let (on, off) = if active { (1.0, 0.0) } else { (0.0, 1.0) };
            let mut fades: Vec<(Visual, f32)> = vec![
                (layer.icon.clone(), on),
                (layer.label.clone(), on),
                (button.idle.icon.clone(), off),
                (button.idle.label.clone(), off),
            ];
            if let Some(clock) = &button.clock {
                fades.push((clock.visual.cast()?, on));
            }
            if animate {
                self.comp.animate_color(&button.fill, fill, SWITCH)?;
                for (visual, opacity) in fades {
                    self.comp
                        .animate_scalar(&visual, "Opacity", opacity, SWITCH)?;
                }
            } else {
                button.fill.SetColor(fill)?;
                for (visual, opacity) in fades {
                    visual.SetOpacity(opacity)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn hover_menu(
        &self,
        parts: &Parts,
        was: Option<&Hit>,
        now: Option<&Hit>,
    ) -> Result<()> {
        for button in &parts.buttons {
            let hit = Hit::Control(button.id);
            let (before, after) = (was == Some(&hit), now == Some(&hit));
            if before == after {
                continue;
            }
            let look = button.look(button.shown.unwrap_or(false));
            let target = if after { look.hover } else { look.fill };
            self.comp.animate_color(&button.fill, target, HOVER)?;
        }
        Ok(())
    }

    pub(super) fn press_menu(
        &self,
        parts: &Parts,
        was: Option<&Hit>,
        now: Option<&Hit>,
    ) -> Result<()> {
        for button in &parts.buttons {
            let hit = Hit::Control(button.id);
            let (before, after) = (was == Some(&hit), now == Some(&hit));
            if before == after {
                continue;
            }
            let k = if after { PRESSED_SCALE } else { 1.0 };
            self.comp
                .animate_vector3(&button.circle, "Scale", scale(k, k), LATCH)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_buttons_take_two_rows() {
        assert_eq!(size(4), (318.0, 92.0));
        assert_eq!(size(7), (318.0, 168.0));
    }

    #[test]
    fn the_record_button_sits_in_the_middle_of_the_second_row() {
        use ControlId::*;
        let items: Vec<Item> = [
            Sound,
            Microphone,
            DoNotDisturb,
            KeepAwake,
            Screenshot,
            Record,
            Settings,
        ]
        .into_iter()
        .map(|id| Item {
            id,
            state: ControlState::Off,
        })
        .collect();
        assert_eq!(button_center(&items, Record), Some((WIDTH / 2.0, 112.0)));
    }

    #[test]
    fn recording_time_is_floored() {
        assert_eq!(elapsed(999), "0:00");
        assert_eq!(elapsed(61_500), "1:01");
    }
}
