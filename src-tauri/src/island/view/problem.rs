//! A problem on the island (`crate::fault`):
//! - **capsule** — 30 px, like the sound indicator: a yellow triangle for a warning,
//!   a red disc for a failure, and what went wrong. A failure also makes the island
//!   shudder once;
//! - **card** — under the cursor the capsule opens: a badge, what went wrong and why.
//!   A click closes it.
//!
//! The small mark in the collapsed island (a problem nobody has looked at yet) is one
//! of the mode badges in `status.rs`.

use super::super::comp::{color, v2};
use super::super::glyph::Glyph;
use super::{Content, Hit, MUTED, Model, Rect, Scene, TEXT, snap, style};
use windows::core::Result;

pub struct ProblemModel<'a> {
    pub critical: bool,
    pub title: &'a str,
    pub reason: &'a str,
}

const WARNING: &str = "#FF9F0A";
/// System red; the recording's own red (#FF375F) is a different one on purpose.
const FAILURE: &str = "#FF453A";

const ICON: f32 = 16.0;
const LEFT: f32 = 12.0;
const GAP: f32 = 8.0;
const RIGHT: f32 = 14.0;

const HEIGHT: f32 = 64.0;
const PAD: f32 = 12.0;
const BADGE: f32 = 40.0;
const BANG: f32 = 20.0;
const BADGE_TINT: f32 = 0.18;
const TEXT_GAP: f32 = 12.0;
const CARD_RIGHT: f32 = 16.0;
const MIN_WIDTH: f32 = 240.0;
const MAX_WIDTH: f32 = 380.0;
const SHAKE_MS: u32 = 360;

fn tint(critical: bool) -> &'static str {
    if critical { FAILURE } else { WARNING }
}

impl Scene {
    pub(super) fn problem_size(&self, model: &Model, open: bool) -> (f32, f32) {
        let Some(problem) = &model.problem else {
            return (30.0, 30.0);
        };
        if !open {
            let text = self.comp.measure(problem.title, style(13.0, true, TEXT));
            return ((LEFT + ICON + GAP + text + RIGHT).round(), 30.0);
        }
        let text = self
            .comp
            .measure(problem.title, style(14.0, true, TEXT))
            .max(self.comp.measure(problem.reason, style(12.0, false, MUTED)));
        let width = PAD + BADGE + TEXT_GAP + text + CARD_RIGHT;
        (width.clamp(MIN_WIDTH, MAX_WIDTH).round(), HEIGHT)
    }

    pub(super) fn build_problem(
        &self,
        model: &Model,
        open: bool,
        content: &mut Content,
    ) -> Result<()> {
        let (width, height) = self.problem_size(model, open);
        content.width = width;
        content.height = height;
        let Some(problem) = &model.problem else {
            return Ok(());
        };
        if open {
            self.build_problem_card(problem, content)
        } else {
            self.build_problem_capsule(problem, content)
        }
    }

    fn build_problem_capsule(&self, problem: &ProblemModel, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let (shape, mark) = if problem.critical {
            (Glyph::Disc, Glyph::DiscMark)
        } else {
            (Glyph::Caution, Glyph::CautionMark)
        };
        let y = (30.0 - ICON) / 2.0;
        let fill = color(tint(problem.critical), 1.0);
        let shape = self.comp.glyph(shape, ICON, fill, self.scale)?;
        self.place(&root, &shape, LEFT, y)?;
        // The "!" is the island's own black showing through the shape.
        let mark = self
            .comp
            .glyph(mark, ICON, color("#000000", 1.0), self.scale)?;
        self.place(&root, &mark, LEFT, y)?;

        let label = self
            .comp
            .text(problem.title, style(13.0, true, TEXT), self.scale)?;
        let x = LEFT + ICON + GAP;
        self.place(&root, &label.visual, x, (30.0 - label.height) / 2.0)
    }

    fn build_problem_card(&self, problem: &ProblemModel, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let c = &self.comp.compositor;
        let hex = tint(problem.critical);
        let top = (HEIGHT - BADGE) / 2.0;

        let badge = c.CreateShapeVisual()?;
        badge.SetSize(v2(BADGE, BADGE))?;
        let disc = c.CreateEllipseGeometry()?;
        disc.SetCenter(v2(BADGE / 2.0, BADGE / 2.0))?;
        disc.SetRadius(v2(BADGE / 2.0, BADGE / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&disc)?;
        shape.SetFillBrush(&self.comp.brush(color(hex, BADGE_TINT))?)?;
        badge.Shapes()?.Append(&shape)?;
        self.place(&root, &badge, PAD, top)?;
        let bang = self
            .comp
            .glyph(Glyph::Bang, BANG, color(hex, 1.0), self.scale)?;
        let inset = (BADGE - BANG) / 2.0;
        self.place(&root, &bang, PAD + inset, top + inset)?;

        let x = PAD + BADGE + TEXT_GAP;
        let room = content.width - x - CARD_RIGHT;
        let title_style = style(14.0, true, TEXT);
        let reason_style = style(12.0, false, MUTED);
        let title = self.comp.fit(problem.title, title_style, room);
        let reason = self.comp.fit(problem.reason, reason_style, room);
        let title = self.comp.text(&title, title_style, self.scale)?;
        let reason = self.comp.text(&reason, reason_style, self.scale)?;
        let block = title.height + 2.0 + reason.height;
        let y = snap((HEIGHT - block) / 2.0, self.scale);
        self.place(&root, &title.visual, x, y)?;
        self.place(&root, &reason.visual, x, y + title.height + 2.0)?;

        content.hits.push((
            Rect {
                x: 0.0,
                y: 0.0,
                w: content.width,
                h: HEIGHT,
            },
            Hit::Problem,
        ));
        Ok(())
    }

    pub fn shake(&self) -> Result<()> {
        self.comp.keyframes(
            &self.shudder,
            "X",
            &[
                (0.0, 0.0),
                (0.15, -5.0),
                (0.35, 5.0),
                (0.55, -3.0),
                (0.75, 2.0),
                (1.0, 0.0),
            ],
            SHAKE_MS,
        )
    }
}
