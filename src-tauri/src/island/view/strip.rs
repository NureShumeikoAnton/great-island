//! The control center's bottom line, under the buttons, with no backing of its own: the clock on
//! the left, the time on battery (or to a full charge) in the middle, the battery's state on
//! the right as icons, the percentage only when the setting asks for it. Without a battery
//! only the clock, in the middle.
//!
//! Each character of the clock sits in its own clipped cell: a changed digit rolls up into
//! place, the others stay. The battery's part is redrawn alone when its state changes, so the
//! menu around it doesn't cross-fade.

use super::super::comp::{TextStyle, color, v2, v3};
use super::super::glyph::Glyph;
use super::system::{
    Battery, CHARGE, CRITICAL, DIGITS, EASE_OUT, Fx, LINEAR, SMALL_BATTERY, WARN, battery_top,
    battery_width, duration,
};
use super::{MUTED, Scene, TEXT, style};
use crate::{
    lang::tr,
    notify::battery::{self as power, Gauge, Minutes, Supply},
    platform::WallClock,
};
use windows::{
    UI::Composition::{ContainerVisual, Visual},
    core::{Interface, Result},
};

#[derive(Clone, Copy)]
pub struct StripModel {
    pub clock: WallClock,
    pub seconds: bool,
    /// `None` without a battery.
    pub gauge: Option<Gauge>,
    pub percent: bool,
    /// The low-charge threshold: the battery turns yellow from it.
    pub low: u8,
    pub decor: bool,
}

/// Under the last row of buttons.
pub(super) const GAP: f32 = 12.0;
pub(super) const HEIGHT: f32 = 20.0;
const ITEM_GAP: f32 = 5.0;
const PLUG: f32 = 14.0;
const ROLL_MS: u32 = 260;
/// How far a digit travels as it rolls, in line heights.
const ROLL: f32 = 0.7;

struct Cell {
    holder: ContainerVisual,
    ch: char,
    shown: Visual,
    /// The digit that rolled out; removed at the next change, its animation long over.
    leaving: Option<Visual>,
    width: f32,
}

pub(super) struct Parts {
    cells: Vec<Cell>,
    gauge: ContainerVisual,
    drawn: Option<(Option<Gauge>, bool, u8)>,
    top: f32,
    clock_right: f32,
    line: f32,
}

fn clock_style() -> TextStyle {
    TextStyle {
        tabular: true,
        ..style(13.0, true, TEXT)
    }
}

fn clock_text(model: &StripModel) -> String {
    let c = model.clock;
    if model.seconds {
        format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second)
    } else {
        format!("{:02}:{:02}", c.hour, c.minute)
    }
}

/// The structure the menu is built for: a change rebuilds the menu.
pub(super) fn key(model: &StripModel) -> String {
    format!("{}|{}", model.seconds, model.gauge.is_some())
}

fn tint(gauge: &Gauge, low: u8) -> &'static str {
    match gauge.supply {
        Supply::Charging => CHARGE,
        _ if gauge.percent <= power::CRITICAL => CRITICAL,
        _ if gauge.percent <= low => WARN,
        _ => TEXT,
    }
}

impl Scene {
    /// `top`: where the line starts; `width`, `edge`: the menu's width and the buttons' outer edge.
    pub(super) fn build_strip(
        &self,
        root: &ContainerVisual,
        model: &StripModel,
        top: f32,
        (width, edge): (f32, f32),
    ) -> Result<Parts> {
        let st = clock_style();
        let text = clock_text(model);
        let widths: Vec<f32> = text
            .chars()
            .map(|ch| self.comp.measure(if ch == ':' { ":" } else { "0" }, st))
            .collect();
        let total: f32 = widths.iter().sum();
        let mut x = match model.gauge {
            Some(_) => edge,
            None => (width - total) / 2.0,
        };
        let mut cells = Vec::new();
        let mut line = 0.0f32;
        for (ch, w) in text.chars().zip(widths) {
            let glyph = self.comp.text(&ch.to_string(), st, self.scale)?;
            line = line.max(glyph.height);
            let holder = self.comp.container()?;
            holder.SetSize(v2(w, glyph.height))?;
            holder.SetClip(&self.comp.compositor.CreateInsetClip()?)?;
            glyph.visual.SetOffset(v3((w - glyph.width) / 2.0, 0.0))?;
            holder.Children()?.InsertAtTop(&glyph.visual)?;
            self.place(root, &holder, x, top + (HEIGHT - glyph.height) / 2.0)?;
            cells.push(Cell {
                holder,
                ch,
                shown: glyph.visual.cast()?,
                leaving: None,
                width: w,
            });
            x += w;
        }
        let gauge = self.comp.container()?;
        root.Children()?.InsertAtTop(&gauge)?;
        let mut parts = Parts {
            cells,
            gauge,
            drawn: None,
            top,
            clock_right: x,
            line,
        };
        self.draw_gauge(&mut parts, model, (width, edge))?;
        Ok(parts)
    }

    pub(super) fn update_strip(
        &self,
        parts: &mut Parts,
        model: &StripModel,
        frame: (f32, f32),
    ) -> Result<()> {
        let st = clock_style();
        let line = parts.line;
        for (cell, ch) in parts.cells.iter_mut().zip(clock_text(model).chars()) {
            if cell.ch == ch {
                continue;
            }
            cell.ch = ch;
            if let Some(old) = cell.leaving.take() {
                cell.holder.Children()?.Remove(&old)?;
            }
            let glyph = self.comp.text(&ch.to_string(), st, self.scale)?;
            glyph
                .visual
                .SetOffset(v3((cell.width - glyph.width) / 2.0, 0.0))?;
            cell.holder.Children()?.InsertAtTop(&glyph.visual)?;
            let old = std::mem::replace(&mut cell.shown, glyph.visual.cast()?);
            if !model.decor {
                cell.holder.Children()?.Remove(&old)?;
                continue;
            }
            let travel = line * ROLL;
            let keys = |from: f32, to: f32| [(0.0, from, LINEAR), (1.0, to, EASE_OUT)];
            glyph.visual.SetOpacity(0.0)?;
            let once = Some(1);
            self.comp.scalar_keys(
                &glyph.visual,
                "Offset.Y",
                &keys(travel, 0.0),
                ROLL_MS,
                0,
                once,
            )?;
            self.comp
                .scalar_keys(&glyph.visual, "Opacity", &keys(0.0, 1.0), ROLL_MS, 0, once)?;
            self.comp
                .scalar_keys(&old, "Offset.Y", &keys(0.0, -travel), ROLL_MS, 0, once)?;
            self.comp
                .scalar_keys(&old, "Opacity", &keys(1.0, 0.0), ROLL_MS, 0, once)?;
            cell.leaving = Some(old);
        }
        self.draw_gauge(parts, model, frame)
    }

    fn draw_gauge(
        &self,
        parts: &mut Parts,
        model: &StripModel,
        (width, edge): (f32, f32),
    ) -> Result<()> {
        let state = (model.gauge, model.percent, model.low);
        if parts.drawn == Some(state) {
            return Ok(());
        }
        parts.drawn = Some(state);
        parts.gauge.Children()?.RemoveAll()?;
        let Some(gauge) = model.gauge else {
            return Ok(());
        };
        let root = &parts.gauge;
        let middle = parts.top + HEIGHT / 2.0;
        let hex = tint(&gauge, model.low);
        let battery = Battery {
            percent: gauge.percent,
            hex,
            fx: if gauge.supply == Supply::Charging {
                Fx::Charge
            } else {
                Fx::Still
            },
            plus: gauge.saver,
        };
        let mut x = width - edge - battery_width(SMALL_BATTERY, battery.plus);
        // Still: the bolt of a charging battery stands where its animation would end.
        self.battery(
            root,
            (x, battery_top(SMALL_BATTERY, middle)),
            SMALL_BATTERY,
            battery,
            false,
        )?;
        if model.percent {
            let pc_hex = if hex == TEXT || hex == CHARGE {
                DIGITS
            } else {
                hex
            };
            let st = TextStyle {
                tabular: true,
                ..style(12.0, true, pc_hex)
            };
            let pc = self
                .comp
                .text(&format!("{}%", gauge.percent), st, self.scale)?;
            x -= ITEM_GAP + pc.width;
            self.place(root, &pc.visual, x, middle - pc.height / 2.0)?;
        }
        if gauge.supply == Supply::Plugged {
            let plug = self
                .comp
                .glyph(Glyph::Plug, PLUG, color(MUTED, 1.0), self.scale)?;
            x -= ITEM_GAP + PLUG;
            self.place(root, &plug, x, middle - PLUG / 2.0)?;
        }
        let center = match gauge.minutes {
            Some(Minutes::Left(m)) => tr!("strip.left", time = duration(m)),
            Some(Minutes::ToFull(m)) => tr!("strip.to_full", time = duration(m)),
            None => return Ok(()),
        };
        let st = style(12.0, false, MUTED);
        let half = width / 2.0;
        let room = 2.0 * (half - parts.clock_right).min(x - half) - 2.0 * ITEM_GAP;
        let center = self.comp.fit(&center, st, room);
        let text = self.comp.text(&center, st, self.scale)?;
        self.place(
            root,
            &text.visual,
            half - text.width / 2.0,
            middle - text.height / 2.0,
        )
    }
}
