//! What an app shows outside its page: its notice (`Mode::AppNotice`: a tinted disc with the
//! sign, the title and a line) and its page of the ribbon (`Mode::Ribbon`: collapsed the sign and
//! the value, under the cursor the whole card). Any change is a new view: the value changes once
//! a minute.

use super::super::comp::{color, v3};
use super::super::glyph::Glyph;
use super::{Content, Hit, MUTED, Model, Rect, Scene, TEXT, dots_space, snap, style};
use crate::apps::ui::{Symbol, Tint};
use windows::core::Result;

pub struct LiveModel<'a> {
    pub symbol: Symbol,
    pub tint: Tint,
    pub title: &'a str,
    pub text: &'a str,
    /// The ribbon's value (a duration); empty for a notice.
    pub value: &'a str,
}

const CARD_WIDTH: f32 = 300.0;
const CARD_HEIGHT: f32 = 60.0;
const PAD: f32 = 12.0;
const DISC: f32 = 36.0;
const SIGN: f32 = 18.0;
const TEXT_GAP: f32 = 12.0;
const VALUE_GAP: f32 = 10.0;
const COLLAPSED_SIGN: f32 = 16.0;
const COLLAPSED_LEFT: f32 = 12.0;
const COLLAPSED_GAP: f32 = 7.0;
const COLLAPSED_RIGHT: f32 = 14.0;

pub fn glyph(symbol: Symbol) -> Glyph {
    match symbol {
        Symbol::Backspace => Glyph::Backspace,
        Symbol::Siren => Glyph::Siren,
        Symbol::Warning => Glyph::Warning,
        Symbol::Check => Glyph::Check,
    }
}

fn tint_colors(tint: Tint) -> (&'static str, &'static str) {
    match tint {
        Tint::Alarm => ("#FF453A", "#FFFFFF"),
        // White on yellow doesn't read.
        Tint::Caution => ("#FFD60A", "#000000"),
        Tint::Clear => ("#30D158", "#FFFFFF"),
    }
}

fn value_style() -> super::super::comp::TextStyle {
    super::super::comp::TextStyle {
        tabular: true,
        ..style(13.0, true, TEXT)
    }
}

impl Scene {
    pub(super) fn live_size(&self, model: &Model, open: bool) -> (f32, f32) {
        match model.live.as_ref() {
            Some(live) if !open => (
                (COLLAPSED_LEFT
                    + COLLAPSED_SIGN
                    + COLLAPSED_GAP
                    + self.comp.measure(live.value, value_style())
                    + COLLAPSED_RIGHT)
                    .round(),
                30.0,
            ),
            _ => (CARD_WIDTH, CARD_HEIGHT + dots_space(model)),
        }
    }

    /// A notice, or the ribbon's page under the cursor: under the ribbon's dots, like every page
    /// of the ribbon.
    pub(super) fn build_live_card(
        &self,
        model: &Model,
        hit: Option<Hit>,
        content: &mut Content,
    ) -> Result<()> {
        let dots = dots_space(model);
        (content.width, content.height) = (CARD_WIDTH, CARD_HEIGHT + dots);
        let Some(live) = model.live.as_ref() else {
            return Ok(());
        };
        let root = content.root.clone();
        self.dots(&root, CARD_WIDTH, model)?;
        let (fill, sign) = tint_colors(live.tint);
        let c = &self.comp.compositor;
        let disc = c.CreateShapeVisual()?;
        disc.SetSize(super::v2(DISC, DISC))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(super::v2(DISC / 2.0, DISC / 2.0))?;
        geometry.SetRadius(super::v2(DISC / 2.0, DISC / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&self.comp.brush(color(fill, 1.0))?)?;
        disc.Shapes()?.Append(&shape)?;
        let top = dots + (CARD_HEIGHT - DISC) / 2.0;
        self.place(&root, &disc, PAD, top)?;
        let icon = self
            .comp
            .glyph(glyph(live.symbol), SIGN, color(sign, 1.0), self.scale)?;
        self.place(
            &root,
            &icon,
            PAD + (DISC - SIGN) / 2.0,
            top + (DISC - SIGN) / 2.0,
        )?;

        let left = PAD + DISC + TEXT_GAP;
        let mut right = CARD_WIDTH - PAD - 2.0;
        if !live.value.is_empty() {
            let width = self.comp.measure(live.value, value_style());
            right -= width;
            let value = self.comp.text(live.value, value_style(), self.scale)?;
            let y = dots + CARD_HEIGHT / 2.0 - self.comp.cap_middle(value_style());
            self.place(&root, &value.visual, right, y)?;
            right -= VALUE_GAP;
        }
        let title_style = style(14.0, true, TEXT);
        let text_style = style(12.0, false, MUTED);
        let title = self.comp.fit(live.title, title_style, right - left);
        let text = self.comp.fit(live.text, text_style, right - left);
        let title = self.comp.text(&title, title_style, self.scale)?;
        if live.text.is_empty() {
            let y = dots + CARD_HEIGHT / 2.0 - self.comp.cap_middle(title_style);
            self.place(&root, &title.visual, left, y)?;
        } else {
            let text = self.comp.text(&text, text_style, self.scale)?;
            let y = snap(
                dots + self.comp.stack_top(&title, &text, 2.0, CARD_HEIGHT),
                self.scale,
            );
            self.place(&root, &title.visual, left, y)?;
            self.place(&root, &text.visual, left, y + title.height + 2.0)?;
        }
        if let Some(hit) = hit {
            content.hits.push((
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: CARD_WIDTH,
                    h: CARD_HEIGHT + dots,
                },
                hit,
            ));
        }
        Ok(())
    }

    /// The ribbon's page at rest: the sign in its color and the value.
    pub(super) fn build_live_collapsed(&self, model: &Model, content: &mut Content) -> Result<()> {
        let Some(live) = model.live.as_ref() else {
            (content.width, content.height) = (30.0, 30.0);
            return Ok(());
        };
        (content.width, content.height) = self.live_size(model, false);
        let root = content.root.clone();
        let (fill, _) = tint_colors(live.tint);
        let icon = self.comp.glyph(
            glyph(live.symbol),
            COLLAPSED_SIGN,
            color(fill, 1.0),
            self.scale,
        )?;
        self.place(&root, &icon, COLLAPSED_LEFT, (30.0 - COLLAPSED_SIGN) / 2.0)?;
        let value = self.comp.text(live.value, value_style(), self.scale)?;
        let x = COLLAPSED_LEFT + COLLAPSED_SIGN + COLLAPSED_GAP;
        value.visual.SetOffset(v3(
            snap(x, self.scale),
            snap(15.0 - self.comp.cap_middle(value_style()), self.scale),
        ))?;
        root.Children()?.InsertAtTop(&value.visual)?;
        Ok(())
    }
}

pub(super) fn key(live: Option<&LiveModel>) -> String {
    live.map_or_else(String::new, |l| {
        format!(
            "{:?}|{:?}|{}|{}|{}",
            l.symbol, l.tint, l.title, l.text, l.value
        )
    })
}
