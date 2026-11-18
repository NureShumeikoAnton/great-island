//! System notices on the island (`crate::notify`), in the island's standard sizes:
//! - **small**, 30 px: a battery, what happened and the percentage (low charge, Energy saver,
//!   plugged, unplugged, charged, slow charging);
//! - **medium**, 300 × 52: a glyph, two lines and a big value on the right (critical charge,
//!   fast drain, high load);
//! - **large**, 340 wide: under the cursor the fast drain and the high load open with up to
//!   three apps.
//!
//! The battery is built of parts the compositor moves: the fill grows or runs down, chevrons
//! run to the tip, the bolt and the Energy saver plus pop in, a shine passes. The gauge's
//! needle swings up and trembles at the top, the processor's core pulses. Nothing runs in our
//! process; with animations off every part stands where it ends.

use super::super::comp::{Motion, TextStyle, color, v2, v3};
use super::super::glyph::{BATTERY_VIEW, Glyph};
use super::{Content, MUTED, Model, Scene, TEXT, snap, style};
use crate::{
    lang::tr,
    media::Icon,
    notify::{Notice, usage},
};
use std::borrow::Cow;
use windows::{
    UI::{
        Color,
        Composition::{ContainerVisual, ShapeVisual},
    },
    core::{Interface, Result},
};

pub struct SystemModel<'a> {
    pub notice: &'a Notice,
    pub apps: Vec<AppModel<'a>>,
    pub decor: bool,
}

pub struct AppModel<'a> {
    pub name: &'a str,
    pub icon: Option<&'a Icon>,
    pub percent: u8,
}

pub(super) const WARN: &str = "#FF9F0A";
pub(super) const CRITICAL: &str = "#FF453A";
pub(super) const CHARGE: &str = "#30D158";
pub(super) const DIGITS: &str = "#D1D1D6";

/// The battery body in viewBox units, out of `BATTERY_VIEW`.
const BODY_W: f32 = 26.0;
const BODY_TOP: f32 = 10.0;
const BODY_MIDDLE: f32 = 16.0;
/// The right end of the tip, and of the Energy saver plus.
const TIP_END: f32 = 25.0;
const PLUS_END: f32 = 30.4;

pub(super) const SMALL_BATTERY: f32 = 20.0;
const MEDIUM_BATTERY: f32 = 28.0;
pub const BADGE_BATTERY: f32 = 18.0;

const SMALL_LEFT: f32 = 12.0;
const SMALL_GAP: f32 = 8.0;
const VALUE_GAP: f32 = 12.0;
const SMALL_RIGHT: f32 = 14.0;

const MEDIUM: (f32, f32) = (300.0, 52.0);
const LARGE_WIDTH: f32 = 340.0;
const LEFT: f32 = 16.0;
const GLYPH_SLOT: f32 = 28.0;
const ICON: f32 = 22.0;
const TEXT_X: f32 = LEFT + GLYPH_SLOT + 12.0;
const RIGHT: f32 = 18.0;
/// The middle of the glyph row, the same in the medium and the large size.
const ROW_MIDDLE: f32 = 26.0;
const LARGE_TOP: f32 = 8.0;
const HEAD: f32 = 36.0;
const ROWS_GAP: f32 = 12.0;
const ROW: f32 = 22.0;
const ROW_SPACE: f32 = 8.0;
const APP_ICON: f32 = 20.0;
/// App icons are centered under the glyph; names align with the title (TEXT_X).
const ROWS_X: f32 = LEFT + (GLYPH_SLOT - APP_ICON) / 2.0;
const BAR: (f32, f32) = (64.0, 4.0);
const PERCENT_W: f32 = 34.0;
const LARGE_BOTTOM: f32 = 14.0;
const NONE_GAP: f32 = 10.0;
const NONE_LINE: f32 = 16.0;
/// How long the load has held when the notice comes (`notify::usage`'s window).
const LOAD_MINUTES: u32 = 5;

pub(super) const EASE_OUT: Motion = Motion::curve(0, 0.22, 1.0, 0.36, 1.0);
const IN_OUT: Motion = Motion::curve(0, 0.42, 0.0, 0.58, 1.0);
/// The island's own overshoot.
const SPRING: Motion = Motion::curve(0, 0.32, 1.28, 0.5, 1.0);
pub(super) const LINEAR: Motion = Motion::linear(0);

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Fx {
    Still,
    /// Starts this many times wider and runs down to the charge.
    Drain(f32),
    Blink,
    Charge,
    Full,
    Slow,
}

#[derive(Clone, Copy)]
pub(super) struct Battery<'a> {
    pub(super) percent: u8,
    pub(super) hex: &'a str,
    pub(super) fx: Fx,
    /// Energy saver: a plus in place of the tip.
    pub(super) plus: bool,
}

enum Look {
    Small {
        battery: Battery<'static>,
        title: String,
        dim: Option<String>,
        value: String,
        tint: &'static str,
    },
    Medium {
        glyph: MediumGlyph,
        title: String,
        sub: String,
        big: String,
        tint: &'static str,
    },
}

enum MediumGlyph {
    Battery(u8),
    Gauge,
    Chip,
}

fn percent(p: u8) -> String {
    format!("{p}%")
}

pub(super) fn duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => tr!("time.minutes", m = m),
        (h, 0) => tr!("time.hours", h = h),
        (h, m) => tr!("time.hours_minutes", h = h, m = m),
    }
    .into_owned()
}

fn left(minutes: Option<u32>) -> Option<String> {
    minutes.map(|m| tr!("notify.left", time = duration(m)).into_owned())
}

fn small(battery: Battery<'static>, title: Cow<str>, dim: Option<String>) -> Look {
    Look::Small {
        battery,
        title: title.into_owned(),
        dim,
        value: percent(battery.percent),
        tint: battery.hex,
    }
}

fn look(model: &SystemModel) -> Look {
    let leader = || {
        let shares: Vec<usage::Share> = model
            .apps
            .iter()
            .map(|a| usage::Share {
                exe: a.name.to_string(),
                percent: a.percent,
            })
            .collect();
        usage::leader(&shares).map(|s| s.exe.clone())
    };
    let battery = |percent, hex, fx| Battery {
        percent,
        hex,
        fx,
        plus: false,
    };
    match *model.notice {
        Notice::Low { percent: p, saver } => {
            let from = (f32::from(p) + 12.0) / f32::from(p.max(1));
            let b = Battery {
                plus: saver,
                ..battery(p, WARN, Fx::Drain(from))
            };
            let title = if saver {
                tr!("notify.saver")
            } else {
                tr!("notify.low")
            };
            small(b, title, None)
        }
        Notice::Plugged { percent: p } => {
            small(battery(p, CHARGE, Fx::Charge), tr!("notify.plugged"), None)
        }
        Notice::Unplugged {
            percent: p,
            minutes_left,
        } => small(
            battery(p, TEXT, Fx::Still),
            tr!("notify.unplugged"),
            minutes_left.map(|m| format!("~{}", duration(m))),
        ),
        Notice::Charged { percent: p } => {
            small(battery(p, CHARGE, Fx::Full), tr!("notify.charged"), None)
        }
        Notice::SlowCharge { percent: p } => {
            small(battery(p, WARN, Fx::Slow), tr!("notify.slow"), None)
        }
        Notice::Critical {
            percent: p,
            minutes_left,
        } => Look::Medium {
            glyph: MediumGlyph::Battery(p),
            title: tr!("notify.critical").into_owned(),
            sub: left(minutes_left).unwrap_or_default(),
            big: percent(p),
            tint: CRITICAL,
        },
        Notice::Drain {
            minutes_left,
            watts,
            ..
        } => Look::Medium {
            glyph: MediumGlyph::Gauge,
            title: tr!("notify.drain").into_owned(),
            sub: [left(minutes_left), leader()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
            big: tr!("notify.watts", n = watts.round() as u32).into_owned(),
            tint: WARN,
        },
        Notice::Load { cpu, .. } => Look::Medium {
            glyph: MediumGlyph::Chip,
            title: tr!("notify.load").into_owned(),
            sub: match leader() {
                Some(app) => tr!("notify.load_app", app = app, m = LOAD_MINUTES),
                None => tr!("notify.load_long", m = LOAD_MINUTES),
            }
            .into_owned(),
            big: percent(cpu),
            tint: WARN,
        },
    }
}

/// Opens into the large size under the cursor.
pub fn has_list(notice: &Notice) -> bool {
    matches!(notice, Notice::Drain { .. } | Notice::Load { .. })
}

fn large_height(rows: usize) -> f32 {
    let head = LARGE_TOP + HEAD;
    if rows == 0 {
        return head + NONE_GAP + NONE_LINE + LARGE_BOTTOM;
    }
    let n = rows as f32;
    head + ROWS_GAP + n * ROW + (n - 1.0) * ROW_SPACE + LARGE_BOTTOM
}

fn digits(size: f32, bold: bool, hex: &str) -> TextStyle {
    TextStyle {
        tabular: true,
        ..style(size, bold, hex)
    }
}

pub(super) fn battery_width(w: f32, plus: bool) -> f32 {
    w / BODY_W * if plus { PLUS_END } else { TIP_END }
}

/// Where a battery `w` wide sits for its body's middle to be at `middle`.
pub(super) fn battery_top(w: f32, middle: f32) -> f32 {
    middle - (BODY_MIDDLE - BODY_TOP) * w / BODY_W
}

impl Scene {
    pub(super) fn system_size(&self, model: &Model, open: bool) -> (f32, f32) {
        let Some(system) = &model.system else {
            return (30.0, 30.0);
        };
        match look(system) {
            Look::Small {
                battery,
                title,
                dim,
                value,
                ..
            } => {
                let mut width = SMALL_LEFT + battery_width(SMALL_BATTERY, battery.plus) + SMALL_GAP;
                width += self.comp.measure(&title, style(13.0, true, TEXT));
                if let Some(dim) = &dim {
                    width += self
                        .comp
                        .measure(&format!(" {dim}"), style(13.0, false, MUTED));
                }
                width += VALUE_GAP + self.comp.measure(&value, digits(13.0, true, TEXT));
                ((width + SMALL_RIGHT).round(), 30.0)
            }
            Look::Medium { .. } if open && has_list(system.notice) => {
                (LARGE_WIDTH, large_height(system.apps.len()))
            }
            Look::Medium { .. } => MEDIUM,
        }
    }

    pub(super) fn build_system(
        &self,
        model: &Model,
        open: bool,
        content: &mut Content,
    ) -> Result<()> {
        let (width, height) = self.system_size(model, open);
        content.width = width;
        content.height = height;
        let Some(system) = &model.system else {
            return Ok(());
        };
        let root = content.root.clone();
        match look(system) {
            Look::Small {
                battery,
                title,
                dim,
                value,
                tint,
            } => {
                let top = battery_top(SMALL_BATTERY, 15.0);
                self.battery(
                    &root,
                    (SMALL_LEFT, top),
                    SMALL_BATTERY,
                    battery,
                    system.decor,
                )?;
                let mut x = SMALL_LEFT + battery_width(SMALL_BATTERY, battery.plus) + SMALL_GAP;
                let label = self
                    .comp
                    .text(&title, style(13.0, true, TEXT), self.scale)?;
                self.place(&root, &label.visual, x, (30.0 - label.height) / 2.0)?;
                x += label.width;
                if let Some(dim) = dim {
                    let dim_style = style(13.0, false, MUTED);
                    let dim = self.comp.text(&format!(" {dim}"), dim_style, self.scale)?;
                    self.place(&root, &dim.visual, x, (30.0 - dim.height) / 2.0)?;
                    x += dim.width;
                }
                let value = self
                    .comp
                    .text(&value, digits(13.0, true, tint), self.scale)?;
                self.place(
                    &root,
                    &value.visual,
                    x + VALUE_GAP,
                    (30.0 - value.height) / 2.0,
                )?;
            }
            Look::Medium {
                glyph,
                title,
                sub,
                big,
                tint,
            } => {
                let large = height > MEDIUM.1;
                let middle = if large { ROW_MIDDLE } else { height / 2.0 };
                self.medium_glyph(&root, glyph, middle, tint, system.decor)?;

                let big = self.comp.text(&big, digits(22.0, true, tint), self.scale)?;
                let right = width - RIGHT;
                self.place(
                    &root,
                    &big.visual,
                    right - big.width,
                    middle - big.height / 2.0,
                )?;

                let room = right - big.width - 12.0 - TEXT_X;
                let title_style = style(14.0, true, TEXT);
                let sub_style = style(12.0, false, MUTED);
                let title = self.comp.fit(&title, title_style, room);
                let title = self.comp.text(&title, title_style, self.scale)?;
                let sub = if sub.is_empty() {
                    None
                } else {
                    let sub = self.comp.fit(&sub, sub_style, room);
                    Some(self.comp.text(&sub, sub_style, self.scale)?)
                };
                let lines = title.height + sub.as_ref().map_or(0.0, |s| 1.0 + s.height);
                let y = snap(middle - lines / 2.0, self.scale);
                self.place(&root, &title.visual, TEXT_X, y)?;
                if let Some(sub) = &sub {
                    self.place(&root, &sub.visual, TEXT_X, y + title.height + 1.0)?;
                }

                if large {
                    self.system_rows(&root, system, tint)?;
                }
            }
        }
        Ok(())
    }

    fn medium_glyph(
        &self,
        root: &ContainerVisual,
        glyph: MediumGlyph,
        middle: f32,
        tint: &str,
        decor: bool,
    ) -> Result<()> {
        let (x, y) = (LEFT + (GLYPH_SLOT - ICON) / 2.0, middle - ICON / 2.0);
        match glyph {
            MediumGlyph::Battery(p) => {
                let x = LEFT + (GLYPH_SLOT - battery_width(MEDIUM_BATTERY, false)) / 2.0;
                let battery = Battery {
                    percent: p,
                    hex: tint,
                    fx: Fx::Blink,
                    plus: false,
                };
                let at = (x, battery_top(MEDIUM_BATTERY, middle));
                self.battery(root, at, MEDIUM_BATTERY, battery, decor)
            }
            MediumGlyph::Gauge => {
                let dial = self
                    .comp
                    .glyph(Glyph::Dial, ICON, color(tint, 1.0), self.scale)?;
                self.place(root, &dial, x, y)?;
                let needle = self
                    .comp
                    .glyph(Glyph::Needle, ICON, color(TEXT, 1.0), self.scale)?;
                let k = ICON / 20.0;
                needle.SetCenterPoint(v3(10.0 * k, 12.2 * k))?;
                self.place(root, &needle, x, y)?;
                if decor {
                    self.revs(&needle)?;
                }
                Ok(())
            }
            MediumGlyph::Chip => {
                let frame = self
                    .comp
                    .glyph(Glyph::Chip, ICON, color(tint, 1.0), self.scale)?;
                self.place(root, &frame, x, y)?;
                let core = self
                    .comp
                    .glyph(Glyph::ChipCore, ICON, color(tint, 1.0), self.scale)?;
                self.place(root, &core, x, y)?;
                if decor {
                    let pulse = [
                        (0.0, 1.0, LINEAR),
                        (0.25, 0.2, IN_OUT),
                        (0.5, 1.0, IN_OUT),
                        (0.75, 0.2, IN_OUT),
                        (1.0, 1.0, IN_OUT),
                    ];
                    self.comp
                        .scalar_keys(&core, "Opacity", &pulse, 2200, 250, Some(1))?;
                }
                Ok(())
            }
        }
    }

    fn revs(&self, needle: &impl Interface) -> Result<()> {
        const TOTAL: f32 = 1620.0;
        let at = |ms: f32| ms / TOTAL;
        let keys = [
            (0.0, -95.0, LINEAR),
            (at(150.0), -95.0, LINEAR),
            (at(1100.0), 0.0, SPRING),
            (at(1162.0), 4.0, LINEAR),
            (at(1235.0), -2.0, LINEAR),
            (at(1308.0), 4.5, LINEAR),
            (at(1381.0), -1.5, LINEAR),
            (at(1454.0), 3.5, LINEAR),
            (at(1537.0), -1.0, LINEAR),
            (1.0, 0.0, LINEAR),
        ];
        self.comp.scalar_keys(
            needle,
            "RotationAngleInDegrees",
            &keys,
            TOTAL as u32,
            0,
            Some(1),
        )
    }

    fn system_rows(&self, root: &ContainerVisual, system: &SystemModel, tint: &str) -> Result<()> {
        let mut y = LARGE_TOP + HEAD;
        if system.apps.is_empty() {
            let none =
                self.comp
                    .text(&tr!("notify.many"), style(12.0, false, MUTED), self.scale)?;
            return self.place(root, &none.visual, TEXT_X, y + NONE_GAP);
        }
        y += ROWS_GAP;
        let right = LARGE_WIDTH - RIGHT;
        let bar_x = right - PERCENT_W - 10.0 - BAR.0;
        let name_x = TEXT_X;
        let name_style = style(13.0, false, TEXT);
        for app in &system.apps {
            let icon_y = y + (ROW - APP_ICON) / 2.0;
            match app.icon {
                Some(icon) => {
                    let picture = self.comp.image_rounded(
                        &icon.bgra,
                        (icon.width, icon.height),
                        (APP_ICON, APP_ICON),
                        5.0,
                        self.scale,
                    )?;
                    self.place(root, &picture, ROWS_X, icon_y)?;
                }
                None => {
                    let tile = self.rounded_rect(APP_ICON, APP_ICON, 5.0, color("#2C2C2E", 1.0))?;
                    self.place(root, &tile, ROWS_X, icon_y)?;
                }
            }
            let name = self.comp.fit(app.name, name_style, bar_x - 10.0 - name_x);
            let name = self.comp.text(&name, name_style, self.scale)?;
            self.place(root, &name.visual, name_x, y + (ROW - name.height) / 2.0)?;

            let bar_y = y + (ROW - BAR.1) / 2.0;
            let track = self.rounded_rect(BAR.0, BAR.1, BAR.1 / 2.0, color("#FFFFFF", 0.12))?;
            self.place(root, &track, bar_x, bar_y)?;
            let filled = (BAR.0 * f32::from(app.percent.min(100)) / 100.0).max(BAR.1);
            let fill = self.rounded_rect(filled, BAR.1, BAR.1 / 2.0, color(tint, 1.0))?;
            self.place(root, &fill, bar_x, bar_y)?;
            if system.decor {
                let grow = [(0.0, 0.0, LINEAR), (1.0, 1.0, EASE_OUT)];
                self.comp
                    .scalar_keys(&fill, "Scale.X", &grow, 600, 200, Some(1))?;
            }
            let pc = self.comp.text(
                &percent(app.percent),
                digits(12.0, true, DIGITS),
                self.scale,
            )?;
            self.place(
                root,
                &pc.visual,
                right - pc.width,
                y + (ROW - pc.height) / 2.0,
            )?;
            y += ROW + ROW_SPACE;
        }
        Ok(())
    }

    pub(super) fn rounded_rect(&self, w: f32, h: f32, r: f32, fill: Color) -> Result<ShapeVisual> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(w, h))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(w, h))?;
        geometry.SetCornerRadius(v2(r, r))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&self.comp.brush(fill)?)?;
        visual.Shapes()?.Append(&shape)?;
        Ok(visual)
    }

    /// A battery `w` wide whose body's top-left is at `at`. Without `animate` every part stands
    /// where it ends: a charging battery keeps its bolt, a slow one its chevrons.
    pub(super) fn battery(
        &self,
        root: &ContainerVisual,
        at: (f32, f32),
        w: f32,
        battery: Battery,
        animate: bool,
    ) -> Result<()> {
        let Battery {
            percent,
            hex,
            fx,
            plus,
        } = battery;
        let c = &self.comp.compositor;
        let k = w / BODY_W;
        let size = BATTERY_VIEW * k;
        let once = Some(1);
        let holder = self.comp.container()?;
        self.place(root, &holder, at.0, at.1 - BODY_TOP * k)?;
        let outline = color("#FFFFFF", 0.5);
        let shell = self
            .comp
            .glyph(Glyph::BatteryShell, size, outline, self.scale)?;
        holder.Children()?.InsertAtTop(&shell)?;
        if !plus {
            let tip = self
                .comp
                .glyph(Glyph::BatteryTip, size, outline, self.scale)?;
            holder.Children()?.InsertAtTop(&tip)?;
        }

        // Scaled from its left edge, which is its own origin.
        let fill_w = 18.0 * f32::from(percent.clamp(4, 100)) / 100.0;
        let fill = self.rounded_rect(fill_w * k, 7.0 * k, 1.6 * k, color(hex, 1.0))?;
        fill.SetOffset(v3(2.5 * k, 12.5 * k))?;
        holder.Children()?.InsertAtTop(&fill)?;

        // The wash, the chevrons and the shine stay inside the body.
        let inside = self.comp.container()?;
        let body = c.CreateRoundedRectangleGeometry()?;
        body.SetOffset(v2(2.0 * k, 12.0 * k))?;
        body.SetSize(v2(19.0 * k, 8.0 * k))?;
        body.SetCornerRadius(v2(1.8 * k, 1.8 * k))?;
        inside.SetClip(&c.CreateGeometricClipWithGeometry(&body)?)?;
        inside.SetSize(v2(size, size))?;
        holder.Children()?.InsertAtTop(&inside)?;

        if fx == Fx::Slow || (fx == Fx::Charge && animate) {
            let wash = self.rounded_rect(19.0 * k, 8.0 * k, 1.8 * k, color("#FFFFFF", 0.28))?;
            wash.SetOffset(v3(2.0 * k, 12.0 * k))?;
            inside.Children()?.InsertAtTop(&wash)?;
            let white = color("#FFFFFF", 1.0);
            let chevrons = self
                .comp
                .glyph(Glyph::BatteryChevrons, size, white, self.scale)?;
            chevrons.SetOffset(v3(2.0 * k, 0.0))?;
            inside.Children()?.InsertAtTop(&chevrons)?;
            if animate {
                let (period, times) = if fx == Fx::Slow {
                    (2200, None)
                } else {
                    let keys = [
                        (0.0, 0.0, LINEAR),
                        (0.12, 1.0, IN_OUT),
                        (0.8, 1.0, LINEAR),
                        (1.0, 0.0, IN_OUT),
                    ];
                    self.comp
                        .scalar_keys(&wash, "Opacity", &keys, 2100, 0, once)?;
                    (900, Some(2))
                };
                let run = [(0.0, -10.0 * k, LINEAR), (1.0, 14.0 * k, LINEAR)];
                self.comp
                    .scalar_keys(&chevrons, "Offset.X", &run, period, 0, times)?;
                let show = [
                    (0.0, 0.0, LINEAR),
                    (0.25, 1.0, LINEAR),
                    (0.75, 1.0, LINEAR),
                    (1.0, 0.0, LINEAR),
                ];
                self.comp
                    .scalar_keys(&chevrons, "Opacity", &show, period, 0, times)?;
            }
        }
        if fx == Fx::Full && animate {
            let shine = c.CreateSpriteVisual()?;
            shine.SetBrush(&self.comp.glint(0.75)?)?;
            shine.SetSize(v2(6.0 * k, 12.0 * k))?;
            shine.SetOffset(v3(-10.0 * k, BODY_TOP * k))?;
            inside.Children()?.InsertAtTop(&shine)?;
            let pass = [(0.0, -10.0 * k, LINEAR), (1.0, 28.0 * k, EASE_OUT)];
            self.comp
                .scalar_keys(&shine, "Offset.X", &pass, 1100, 650, once)?;
        }
        if matches!(fx, Fx::Charge | Fx::Full) {
            let white = color("#FFFFFF", 1.0);
            let bolt = self
                .comp
                .glyph(Glyph::BatteryBolt, size, white, self.scale)?;
            bolt.SetCenterPoint(v3(12.2 * k, 15.5 * k))?;
            holder.Children()?.InsertAtTop(&bolt)?;
            if animate {
                self.pop(&bolt, 0.4, 380, 1750)?;
            }
        }
        if plus {
            let saver = self
                .comp
                .glyph(Glyph::BatteryPlus, size, color(TEXT, 1.0), self.scale)?;
            saver.SetCenterPoint(v3(27.0 * k, 16.0 * k))?;
            holder.Children()?.InsertAtTop(&saver)?;
            if animate {
                self.pop(&saver, 0.0, 420, 700)?;
            }
        }
        if !animate {
            return Ok(());
        }
        match fx {
            Fx::Drain(from) => {
                let keys = [(0.0, from, LINEAR), (1.0, 1.0, EASE_OUT)];
                self.comp
                    .scalar_keys(&fill, "Scale.X", &keys, 1400, 380, once)?;
            }
            Fx::Blink => {
                let keys = [
                    (0.0, 1.0, LINEAR),
                    (0.25, 0.25, IN_OUT),
                    (0.5, 1.0, IN_OUT),
                    (0.75, 0.25, IN_OUT),
                    (1.0, 1.0, IN_OUT),
                ];
                self.comp
                    .scalar_keys(&fill, "Opacity", &keys, 2600, 350, once)?;
            }
            Fx::Charge | Fx::Full => {
                let grow = [(0.0, 0.0, LINEAR), (1.0, 1.0, EASE_OUT)];
                self.comp
                    .scalar_keys(&fill, "Scale.X", &grow, 700, 120, once)?;
            }
            Fx::Still | Fx::Slow => {}
        }
        Ok(())
    }

    fn pop(&self, visual: &impl Interface, from: f32, ms: u32, delay: u32) -> Result<()> {
        let fade = [(0.0, 0.0, LINEAR), (1.0, 1.0, EASE_OUT)];
        self.comp
            .scalar_keys(visual, "Opacity", &fade, ms, delay, Some(1))?;
        let grow = [(0.0, from, LINEAR), (1.0, 1.0, SPRING)];
        for axis in ["Scale.X", "Scale.Y"] {
            self.comp
                .scalar_keys(visual, axis, &grow, ms, delay, Some(1))?;
        }
        Ok(())
    }
}
