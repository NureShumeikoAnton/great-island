//! What the island says about updates (`crate::update`):
//! - **capsule** — 30 px, like the sound indicator: a spinner while a check runs, a green check when
//!   nothing new was found;
//! - **card** — 64 px, laid out like the problem card: a badge, what happened and what to do. It tells that an
//!   update was postponed (and till when) or skipped.
//!
//! The update window itself is another process (`update_app.rs`).

use super::super::comp::{Motion, color, v2};
use super::super::glyph::Glyph;
use super::{Content, MUTED, Model, Scene, TEXT, snap, style};
use windows::core::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateLook {
    Checking,
    UpToDate,
    Postponed,
    Skipped,
}

impl UpdateLook {
    fn is_card(self) -> bool {
        matches!(self, Self::Postponed | Self::Skipped)
    }
}

pub struct UpdateModel<'a> {
    pub look: UpdateLook,
    pub title: &'a str,
    pub reason: &'a str,
}

const OK: &str = "#30D158";

const ICON: f32 = 16.0;
const LEFT: f32 = 12.0;
const GAP: f32 = 8.0;
const RIGHT: f32 = 14.0;
const SPIN_MS: u32 = 900;

const HEIGHT: f32 = 64.0;
const PAD: f32 = 12.0;
const BADGE: f32 = 40.0;
const GLYPH: f32 = 20.0;
const BADGE_TINT: f32 = 0.10;
const TEXT_GAP: f32 = 12.0;
const CARD_RIGHT: f32 = 16.0;
const MIN_WIDTH: f32 = 240.0;
const MAX_WIDTH: f32 = 380.0;

impl Scene {
    pub(super) fn update_size(&self, model: &Model) -> (f32, f32) {
        let Some(update) = &model.update else {
            return (30.0, 30.0);
        };
        if !update.look.is_card() {
            let text = self.comp.measure(update.title, style(13.0, true, TEXT));
            return ((LEFT + ICON + GAP + text + RIGHT).round(), 30.0);
        }
        let text = self
            .comp
            .measure(update.title, style(14.0, true, TEXT))
            .max(self.comp.measure(update.reason, style(12.0, false, MUTED)));
        let width = PAD + BADGE + TEXT_GAP + text + CARD_RIGHT;
        (width.clamp(MIN_WIDTH, MAX_WIDTH).round(), HEIGHT)
    }

    pub(super) fn build_update(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.update_size(model);
        content.width = width;
        content.height = height;
        let Some(update) = &model.update else {
            return Ok(());
        };
        if update.look.is_card() {
            self.build_update_card(update, content)
        } else {
            self.build_update_capsule(update, content)
        }
    }

    fn build_update_capsule(&self, update: &UpdateModel, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let y = (30.0 - ICON) / 2.0;
        if update.look == UpdateLook::Checking {
            // A quarter of a ring turning on the compositor: nothing is drawn per frame by us.
            let ring = self.ring("#FFFFFF", ICON, 2.0)?;
            ring.arc.SetTrimEnd(0.27)?;
            self.place(&root, &ring.visual, LEFT, y)?;
            self.comp.loop_scalar(
                &ring.visual,
                "RotationAngleInDegrees",
                &[
                    (0.0, 0.0, Motion::linear(SPIN_MS)),
                    (1.0, 360.0, Motion::linear(SPIN_MS)),
                ],
                SPIN_MS,
            )?;
        } else {
            let check = self
                .comp
                .glyph(Glyph::Check, ICON, color(OK, 1.0), self.scale)?;
            self.place(&root, &check, LEFT, y)?;
        }
        let label = self
            .comp
            .text(update.title, style(13.0, true, TEXT), self.scale)?;
        let x = LEFT + ICON + GAP;
        self.place(&root, &label.visual, x, (30.0 - label.height) / 2.0)
    }

    fn build_update_card(&self, update: &UpdateModel, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let c = &self.comp.compositor;
        let top = (HEIGHT - BADGE) / 2.0;

        let badge = c.CreateShapeVisual()?;
        badge.SetSize(v2(BADGE, BADGE))?;
        let disc = c.CreateEllipseGeometry()?;
        disc.SetCenter(v2(BADGE / 2.0, BADGE / 2.0))?;
        disc.SetRadius(v2(BADGE / 2.0, BADGE / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&disc)?;
        shape.SetFillBrush(&self.comp.brush(color("#FFFFFF", BADGE_TINT))?)?;
        badge.Shapes()?.Append(&shape)?;
        self.place(&root, &badge, PAD, top)?;
        let glyph = if update.look == UpdateLook::Skipped {
            Glyph::Next
        } else {
            Glyph::Timer
        };
        let glyph = self
            .comp
            .glyph(glyph, GLYPH, color(TEXT, 1.0), self.scale)?;
        let inset = (BADGE - GLYPH) / 2.0;
        self.place(&root, &glyph, PAD + inset, top + inset)?;

        let x = PAD + BADGE + TEXT_GAP;
        let room = content.width - x - CARD_RIGHT;
        let title_style = style(14.0, true, TEXT);
        let reason_style = style(12.0, false, MUTED);
        let title = self.comp.fit(update.title, title_style, room);
        let reason = self.comp.fit(update.reason, reason_style, room);
        let title = self.comp.text(&title, title_style, self.scale)?;
        let reason = self.comp.text(&reason, reason_style, self.scale)?;
        let block = title.height + 2.0 + reason.height;
        let y = snap((HEIGHT - block) / 2.0, self.scale);
        self.place(&root, &title.visual, x, y)?;
        self.place(&root, &reason.visual, x, y + title.height + 2.0)
    }
}
