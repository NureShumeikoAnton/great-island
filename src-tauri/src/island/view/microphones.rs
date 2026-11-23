//! The microphones page of the pre-start panel: "as in Windows" and the plugged-in
//! microphones, the checked one lit and listened to (its bars jump with its sound). When
//! both inputs are recorded, a slider below sets the voice against the computer's sound.

use super::super::comp::{Text, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::panel::{FILL, FILL_HOVER, INNER, KNOB, PAD, PanelModel, Parts, SLIDE, WIDTH, bold};
use super::{Button, Content, Hit, MUTED, Model, Rect, Scene, TEXT, snap, style};
use crate::lang::tr;
use crate::record::{
    meter::BARS,
    panel::{Page, Press},
    sound::Sound,
};
use windows::{
    UI::Composition::{ContainerVisual, Visual},
    core::{Interface, Result},
};

/// More rows would make the island taller than the screen part it may take.
pub const MAX_MICROPHONES: usize = 6;
const BACK: f32 = 28.0;
const BACK_ICON: f32 = 14.0;
const ROW: f32 = 34.0;
const ROW_STEP: f32 = 36.0;
const CHECK: f32 = 14.0;
const BAR_W: f32 = 2.0;
const BAR_GAP: f32 = 2.0;
const BAR_H: f32 = 12.0;
/// An empty bar still shows as a dot: the meter is there, the room is quiet.
const BAR_MIN: f32 = 0.15;
const ROWS_Y: f32 = PAD + BACK + 6.0;
/// The window is made this tall: the microphones page with every row is the panel's tallest.
pub const TALLEST: f32 =
    ROWS_Y + (MAX_MICROPHONES + 1) as f32 * ROW_STEP - (ROW_STEP - ROW) + BALANCE_H + PAD;
/// The balance row under the microphones: a line, then the slider.
const BALANCE_GAP: f32 = 8.0;
const BALANCE_ROW: f32 = 28.0;
const BALANCE_H: f32 = BALANCE_GAP + 1.0 + BALANCE_GAP + BALANCE_ROW;
const BALANCE_ICON: f32 = 16.0;
const SLIDER_X0: f32 = PAD + 8.0 + BALANCE_ICON + 12.0;
const SLIDER_X1: f32 = WIDTH - PAD - 8.0 - BALANCE_ICON - 12.0;
const SLIDER_H: f32 = 4.0;
const GREEN: &str = "#30D158";

pub(super) struct MicParts {
    /// The knob of the balance slider, when both inputs are recorded.
    balance: Option<Visual>,
    balance_y: f32,
    highlight: Visual,
    checks: Vec<Visual>,
    bars: Visual,
    bar_visuals: Vec<Visual>,
    unheard: Text,
    shown: Option<(usize, bool)>,
}

fn rows(model: &Model) -> usize {
    1 + model
        .panel
        .as_ref()
        .map_or(0, |p| p.devices.len().min(MAX_MICROPHONES))
}

/// Where the last microphone row ends.
fn rows_end(rows: usize) -> f32 {
    ROWS_Y + rows as f32 * ROW_STEP - (ROW_STEP - ROW)
}

impl Scene {
    pub(super) fn microphones_size(&self, model: &Model) -> (f32, f32) {
        let both = model
            .panel
            .as_ref()
            .is_some_and(|p| p.page == Page::Microphones && p.choice.sound == Sound::Both);
        (
            WIDTH,
            rows_end(rows(model)) + if both { BALANCE_H } else { 0.0 } + PAD,
        )
    }

    pub(super) fn build_microphones(
        &self,
        panel: &PanelModel,
        content: &mut Content,
    ) -> Result<MicParts> {
        let root = content.root.clone();
        let back_fill = self.comp.brush(color(FILL, 1.0))?;
        let back = self.disc(BACK, &back_fill)?;
        let back_icon = self
            .comp
            .glyph(Glyph::Back, BACK_ICON, color(MUTED, 1.0), self.scale)?;
        back_icon.SetOffset(v3((BACK - BACK_ICON) / 2.0, (BACK - BACK_ICON) / 2.0))?;
        let holder = self.comp.container()?;
        holder.Children()?.InsertAtTop(&back)?;
        holder.Children()?.InsertAtTop(&back_icon)?;
        self.place(&root, &holder, PAD, PAD)?;
        let back_rect = Rect {
            x: PAD,
            y: PAD,
            w: BACK,
            h: BACK,
        };
        content.hits.push((back_rect, Hit::Panel(Press::Back)));
        content.buttons.push(Button {
            fill: back_fill,
            rect: back_rect,
            hit: Hit::Panel(Press::Back),
            base: color(FILL, 1.0),
            hover: color(FILL_HOVER, 1.0),
        });
        let title = self.comp.text(
            &tr!("record.panel.microphone"),
            bold(13.0, TEXT),
            self.scale,
        )?;
        self.place(
            &root,
            &title.visual,
            PAD + BACK + 8.0,
            PAD + self.comp.center_top(&title, BACK),
        )?;

        let highlight = self.pill(INNER, ROW, &self.comp.brush(color(FILL, 1.0))?)?;
        root.Children()?.InsertAtTop(&highlight)?;

        let names: Vec<String> = std::iter::once(tr!("record.panel.windows_mic").into_owned())
            .chain(
                panel
                    .devices
                    .iter()
                    .take(MAX_MICROPHONES)
                    .map(|d| d.name.clone()),
            )
            .collect();
        let name_x = PAD + 12.0 + CHECK + 10.0;
        let name_w = WIDTH - PAD - 12.0 - 4.0 * (BAR_W + BAR_GAP) - 10.0 - name_x;
        let mut checks = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let y = ROWS_Y + i as f32 * ROW_STEP;
            let rect = Rect {
                x: PAD,
                y,
                w: INNER,
                h: ROW,
            };
            let hover_fill = self.comp.brush(color(FILL, 0.0))?;
            let hover = self.pill(INNER, ROW, &hover_fill)?;
            self.place(&root, &hover, PAD, y)?;
            content.buttons.push(Button {
                fill: hover_fill,
                rect,
                hit: Hit::Panel(Press::Microphone(i)),
                base: color(FILL, 0.0),
                hover: color(FILL, 0.6),
            });
            content.hits.push((rect, Hit::Panel(Press::Microphone(i))));
            let check = self
                .comp
                .glyph(Glyph::Check, CHECK, color(TEXT, 1.0), self.scale)?;
            check.SetOpacity(0.0)?;
            self.place(&root, &check, PAD + 12.0, y + (ROW - CHECK) / 2.0)?;
            checks.push(check.cast()?);
            let st = style(13.0, false, TEXT);
            let text = self
                .comp
                .text(&self.comp.fit(name, st, name_w), st, self.scale)?;
            self.place(
                &root,
                &text.visual,
                name_x,
                y + self.comp.center_top(&text, ROW),
            )?;
        }
        let bars = self.comp.container()?;
        let mut bar_visuals = Vec::new();
        let c = &self.comp.compositor;
        let green = self.comp.brush(color(GREEN, 1.0))?;
        for i in 0..BARS {
            let bar = c.CreateShapeVisual()?;
            bar.SetSize(v2(BAR_W, BAR_H))?;
            let geometry = c.CreateRoundedRectangleGeometry()?;
            geometry.SetSize(v2(BAR_W, BAR_H))?;
            geometry.SetCornerRadius(v2(BAR_W / 2.0, BAR_W / 2.0))?;
            let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
            shape.SetFillBrush(&green)?;
            bar.Shapes()?.Append(&shape)?;
            // Grows from the bottom.
            bar.SetCenterPoint(v3(BAR_W / 2.0, BAR_H))?;
            bar.SetScale(scale(1.0, BAR_MIN))?;
            self.place(&bars, &bar, i as f32 * (BAR_W + BAR_GAP), 0.0)?;
            bar_visuals.push(bar.cast()?);
        }
        root.Children()?.InsertAtTop(&bars)?;
        let unheard = self.comp.text(
            &tr!("record.panel.unheard"),
            style(11.0, false, MUTED),
            self.scale,
        )?;
        unheard.visual.SetOpacity(0.0)?;
        root.Children()?.InsertAtTop(&unheard.visual)?;
        let balance_y = rows_end(names.len()) + BALANCE_GAP + 1.0 + BALANCE_GAP;
        let balance = match panel.choice.sound {
            Sound::Both => Some(self.build_balance(&root, balance_y, content)?),
            _ => None,
        };
        Ok(MicParts {
            balance,
            balance_y,
            highlight: highlight.cast()?,
            checks,
            bars: bars.cast()?,
            bar_visuals,
            unheard,
            shown: None,
        })
    }

    /// Under the microphones: a line, then the voice and the computer at the ends of a slider.
    fn build_balance(
        &self,
        root: &ContainerVisual,
        y: f32,
        content: &mut Content,
    ) -> Result<Visual> {
        let c = &self.comp.compositor;
        let line = self.pill(INNER, 1.0, &self.comp.brush(color("#FFFFFF", 0.08))?)?;
        self.place(root, &line, PAD, y - BALANCE_GAP - 1.0)?;
        let icon_y = y + (BALANCE_ROW - BALANCE_ICON) / 2.0;
        for (glyph, x) in [
            (Glyph::Microphone, PAD + 8.0),
            (Glyph::Monitor, WIDTH - PAD - 8.0 - BALANCE_ICON),
        ] {
            let icon = self
                .comp
                .glyph(glyph, BALANCE_ICON, color(MUTED, 1.0), self.scale)?;
            self.place(root, &icon, x, icon_y)?;
        }
        let track = self.pill(
            SLIDER_X1 - SLIDER_X0,
            SLIDER_H,
            &self.comp.brush(color("#FFFFFF", 0.16))?,
        )?;
        self.place(root, &track, SLIDER_X0, y + (BALANCE_ROW - SLIDER_H) / 2.0)?;
        // The middle is marked: there both stay as loud as they are.
        let tick = c.CreateShapeVisual()?;
        tick.SetSize(v2(2.0, 10.0))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(2.0, 10.0))?;
        geometry.SetCornerRadius(v2(1.0, 1.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&self.comp.brush(color("#FFFFFF", 0.28))?)?;
        tick.Shapes()?.Append(&shape)?;
        self.place(
            root,
            &tick,
            (SLIDER_X0 + SLIDER_X1) / 2.0 - 1.0,
            y + (BALANCE_ROW - 10.0) / 2.0,
        )?;
        let knob = self.disc(KNOB, &self.comp.brush(color(TEXT, 1.0))?)?;
        root.Children()?.InsertAtTop(&knob)?;
        content.hits.push((
            Rect {
                x: SLIDER_X0 - KNOB,
                y,
                w: SLIDER_X1 - SLIDER_X0 + 2.0 * KNOB,
                h: BALANCE_ROW,
            },
            Hit::Panel(Press::Balance),
        ));
        knob.cast()
    }

    /// Where on the balance slider the pointer is: 0 — the voice end, 1 — the computer's.
    /// `None` — no slider is shown.
    pub fn balance_at(&self, x: f32) -> Option<f32> {
        let content = self.content.as_ref()?;
        let Some(Parts::Microphones(parts)) = &content.panel else {
            return None;
        };
        parts.balance.as_ref()?;
        let local = x - (self.frame_width - content.width) / 2.0;
        Some(((local - SLIDER_X0) / (SLIDER_X1 - SLIDER_X0)).clamp(0.0, 1.0))
    }

    pub(super) fn update_microphones(
        &self,
        panel: &PanelModel,
        parts: &mut MicParts,
    ) -> Result<()> {
        let s = self.scale;
        if let Some(knob) = &parts.balance {
            let x = SLIDER_X0 + panel.choice.balance.fraction() * (SLIDER_X1 - SLIDER_X0);
            knob.SetOffset(v3(
                snap(x - KNOB / 2.0, s),
                snap(parts.balance_y + (BALANCE_ROW - KNOB) / 2.0, s),
            ))?;
        }
        let shown = (
            panel.selected.min(parts.checks.len().saturating_sub(1)),
            panel.hearing,
        );
        if parts.shown == Some(shown) {
            return Ok(());
        }
        let (row, hearing) = shown;
        let y = ROWS_Y + row as f32 * ROW_STEP;
        let at = v3(snap(PAD, s), snap(y, s));
        if parts.shown.is_some() {
            self.comp
                .animate_vector3(&parts.highlight, "Offset", at, SLIDE)?;
        } else {
            parts.highlight.SetOffset(at)?;
        }
        for (i, check) in parts.checks.iter().enumerate() {
            check.SetOpacity(if i == row { 1.0 } else { 0.0 })?;
        }
        let bars_w = BARS as f32 * (BAR_W + BAR_GAP) - BAR_GAP;
        parts.bars.SetOffset(v3(
            snap(WIDTH - PAD - 12.0 - bars_w, s),
            snap(y + (ROW - BAR_H) / 2.0, s),
        ))?;
        parts.unheard.visual.SetOffset(v3(
            snap(WIDTH - PAD - 12.0 - parts.unheard.width, s),
            snap(y + self.comp.center_top(&parts.unheard, ROW), s),
        ))?;
        parts.bars.SetOpacity(if hearing { 1.0 } else { 0.0 })?;
        parts
            .unheard
            .visual
            .SetOpacity(if hearing { 0.0 } else { 1.0 })?;
        if row != parts.shown.map_or(usize::MAX, |(r, _)| r) {
            for bar in &parts.bar_visuals {
                bar.SetScale(scale(1.0, BAR_MIN))?;
            }
        }
        parts.shown = Some(shown);
        Ok(())
    }

    /// The bars of the listened microphone, heights 0 to 1. Called by a timer while the
    /// microphones page is open: set directly, no animation.
    pub fn set_bars(&self, heights: [f32; BARS]) -> Result<()> {
        let Some(Parts::Microphones(parts)) = self.content.as_ref().and_then(|c| c.panel.as_ref())
        else {
            return Ok(());
        };
        for (bar, height) in parts.bar_visuals.iter().zip(heights) {
            bar.SetScale(scale(1.0, height.max(BAR_MIN)))?;
        }
        Ok(())
    }
}
