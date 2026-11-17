//! Status on the island while a viewer window is open: a capsule as tall as the collapsed island:
//! the thumbnail and an arrow in a circle; for a video, ▶/❚❚ in a circle before them.
//! - a click on the thumbnail brings the viewer to the front (it may have gone behind other windows);
//! - ▶/❚❚ plays or pauses the video in its window;
//! - the arrow tucks the shot back into the island: the window closes and the shot flies into the card, as right
//!   after a screenshot. The file stays. In the editor the arrow must be held for half a second: the circle fills
//!   with white bottom to top; released earlier, it shudders.
//!
//! This is a state, not a notice: the island doesn't dim in it (`Mode::dims`).

use super::super::comp::{Motion, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::{Button, Content, Hit, IconButton, Model, Rect, Scene, TEXT, snap};
use crate::capture::Image;
use windows::{
    UI::Composition::Visual,
    core::{Interface, Result},
};

pub const HOLD_MS: u32 = 500;
const UNFILL: Motion = Motion::curve(160, 0.4, 0.0, 1.0, 1.0);
const SHAKE_MS: u32 = 320;

const HEIGHT: f32 = 30.0;
const THUMB_INSET: f32 = 6.0;
const THUMB_H: f32 = HEIGHT - 2.0 * THUMB_INSET;
const THUMB_RADIUS: f32 = 4.0;
const GAP: f32 = 8.0;
const ARROW: f32 = 22.0;
const ARROW_INSET: f32 = (HEIGHT - ARROW) / 2.0;
const ARROW_ICON: f32 = 14.0;
const PLAY_ICON: f32 = 12.0;

pub(super) struct Parts {
    body: Visual,
    x: f32,
    fill: Visual,
    /// A video's ▶/❚❚: the pause and the play icon, one of them shown.
    play: Option<(Visual, Visual)>,
}

/// Where the thumbnail starts: after ▶/❚❚ for a video.
fn lead(model: &Model) -> f32 {
    if model.viewing_play.is_some() {
        ARROW_INSET + ARROW + GAP
    } else {
        THUMB_INSET
    }
}

fn thumb_rect(image: &Image, x: f32) -> Rect {
    let aspect = image.width as f32 / image.height.max(1) as f32;
    let w = (THUMB_H * aspect)
        .clamp(THUMB_H * 0.6, THUMB_H * 2.4)
        .round();
    Rect {
        x,
        y: THUMB_INSET,
        w,
        h: THUMB_H,
    }
}

impl Scene {
    pub(super) fn viewing_size(&self, model: &Model) -> (f32, f32) {
        let x = lead(model);
        let thumb_w = model
            .viewing
            .map_or(THUMB_H, |image| thumb_rect(image, x).w);
        (x + thumb_w + GAP + ARROW + ARROW_INSET, HEIGHT)
    }

    pub(super) fn build_viewing(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.viewing_size(model);
        content.width = width;
        content.height = height;
        let Some(image) = model.viewing else {
            return Ok(());
        };
        let root = content.root.clone();

        let play = match model.viewing_play {
            Some(running) => Some(self.build_play(content, running)?),
            None => None,
        };

        let rect = thumb_rect(image, lead(model));
        let thumb = self.comp.image_rounded(
            &image.pixels,
            (image.width, image.height),
            (rect.w, rect.h),
            THUMB_RADIUS,
            self.scale,
        )?;
        thumb.SetCenterPoint(v3(rect.w / 2.0, rect.h / 2.0))?;
        self.place(&root, &thumb, rect.x, rect.y)?;
        content.icon_buttons.push(IconButton {
            visual: thumb.cast()?,
            hit: Hit::Viewer,
        });
        let hit_x = if play.is_some() {
            rect.x - GAP / 2.0
        } else {
            0.0
        };
        content.hits.push((
            Rect {
                x: hit_x,
                y: 0.0,
                w: rect.x + rect.w + GAP / 2.0 - hit_x,
                h: HEIGHT,
            },
            Hit::Viewer,
        ));

        let c = &self.comp.compositor;
        let x = width - ARROW_INSET - ARROW;
        let size = v2(ARROW, ARROW);
        let body = self.comp.container()?;
        body.SetSize(size)?;
        body.SetCenterPoint(v3(ARROW / 2.0, ARROW / 2.0))?;
        let circle = c.CreateEllipseGeometry()?;
        circle.SetCenter(v2(ARROW / 2.0, ARROW / 2.0))?;
        circle.SetRadius(v2(ARROW / 2.0, ARROW / 2.0))?;
        body.SetClip(&c.CreateGeometricClipWithGeometry(&circle)?)?;

        let back = c.CreateShapeVisual()?;
        back.SetSize(size)?;
        let shape = c.CreateSpriteShapeWithGeometry(&circle)?;
        let (base, hover) = (color("#FFFFFF", 0.12), color("#FFFFFF", 0.2));
        let fill_brush = self.comp.brush(base)?;
        shape.SetFillBrush(&fill_brush)?;
        back.Shapes()?.Append(&shape)?;
        body.Children()?.InsertAtTop(&back)?;

        let fill = c.CreateSpriteVisual()?;
        fill.SetBrush(&self.comp.brush(color("#FFFFFF", 0.5))?)?;
        fill.SetSize(size)?;
        fill.SetCenterPoint(v3(0.0, ARROW))?;
        fill.SetScale(scale(1.0, 0.0))?;
        body.Children()?.InsertAtTop(&fill)?;

        let icon = self
            .comp
            .glyph(Glyph::ArrowUp, ARROW_ICON, color(TEXT, 1.0), self.scale)?;
        let inset = (ARROW - ARROW_ICON) / 2.0;
        icon.SetOffset(v3(inset, inset))?;
        body.Children()?.InsertAtTop(&icon)?;
        self.place(&root, &body, x, ARROW_INSET)?;

        let hit = Rect {
            x: x - GAP / 2.0,
            y: 0.0,
            w: width - x + GAP / 2.0,
            h: HEIGHT,
        };
        content.hits.push((hit, Hit::ToIsland));
        content.buttons.push(Button {
            fill: fill_brush,
            rect: hit,
            hit: Hit::ToIsland,
            base,
            hover,
        });
        content.icon_buttons.push(IconButton {
            visual: body.cast()?,
            hit: Hit::ToIsland,
        });
        content.viewing = Some(Parts {
            body: body.cast()?,
            x: snap(x, self.scale),
            fill: fill.cast()?,
            play,
        });
        Ok(())
    }

    fn build_play(&self, content: &mut Content, running: bool) -> Result<(Visual, Visual)> {
        let c = &self.comp.compositor;
        let size = v2(ARROW, ARROW);
        let body = self.comp.container()?;
        body.SetSize(size)?;
        body.SetCenterPoint(v3(ARROW / 2.0, ARROW / 2.0))?;
        let circle = c.CreateEllipseGeometry()?;
        circle.SetCenter(v2(ARROW / 2.0, ARROW / 2.0))?;
        circle.SetRadius(v2(ARROW / 2.0, ARROW / 2.0))?;
        let back = c.CreateShapeVisual()?;
        back.SetSize(size)?;
        let shape = c.CreateSpriteShapeWithGeometry(&circle)?;
        let (base, hover) = (color("#FFFFFF", 0.12), color("#FFFFFF", 0.2));
        let fill = self.comp.brush(base)?;
        shape.SetFillBrush(&fill)?;
        back.Shapes()?.Append(&shape)?;
        body.Children()?.InsertAtTop(&back)?;

        let inset = (ARROW - PLAY_ICON) / 2.0;
        let mut icons = Vec::with_capacity(2);
        for (glyph, shown) in [(Glyph::Pause, running), (Glyph::Play, !running)] {
            let icon = self
                .comp
                .glyph(glyph, PLAY_ICON, color(TEXT, 1.0), self.scale)?;
            icon.SetOffset(v3(inset, inset))?;
            icon.SetOpacity(if shown { 1.0 } else { 0.0 })?;
            body.Children()?.InsertAtTop(&icon)?;
            icons.push(icon.cast::<Visual>()?);
        }
        self.place(&content.root, &body, ARROW_INSET, ARROW_INSET)?;

        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: ARROW_INSET + ARROW + GAP / 2.0,
            h: HEIGHT,
        };
        content.hits.push((rect, Hit::ViewerPlay));
        content.buttons.push(Button {
            fill,
            rect,
            hit: Hit::ViewerPlay,
            base,
            hover,
        });
        content.icon_buttons.push(IconButton {
            visual: body.cast()?,
            hit: Hit::ViewerPlay,
        });
        let play = icons.pop().expect("two icons were made");
        let pause = icons.pop().expect("two icons were made");
        Ok((pause, play))
    }

    /// The video started or paused: the icon changes in place.
    pub(super) fn update_viewing(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (
            Some(running),
            Some(Parts {
                play: Some((pause, play)),
                ..
            }),
        ) = (model.viewing_play, content.viewing.as_ref())
        else {
            return Ok(());
        };
        pause.SetOpacity(if running { 1.0 } else { 0.0 })?;
        play.SetOpacity(if running { 0.0 } else { 1.0 })
    }

    /// Holding the arrow: the fill grows (`true`) or rolls back with a shudder (`false`).
    /// `done` = held to the end: the fill just fades.
    pub fn hold_arrow(&self, holding: bool, done: bool) -> Result<()> {
        let Some(parts) = self.content.as_ref().and_then(|c| c.viewing.as_ref()) else {
            return Ok(());
        };
        let (body, x, fill) = (&parts.body, parts.x, &parts.fill);
        if holding {
            return self.comp.animate_vector3(
                fill,
                "Scale",
                scale(1.0, 1.0),
                Motion::linear(HOLD_MS),
            );
        }
        self.comp
            .animate_vector3(fill, "Scale", scale(1.0, 0.0), UNFILL)?;
        if done {
            return Ok(());
        }
        self.comp.keyframes(
            body,
            "Offset.X",
            &[
                (0.0, x),
                (0.15, x - 4.0),
                (0.35, x + 4.0),
                (0.55, x - 3.0),
                (0.75, x + 2.0),
                (1.0, x),
            ],
            SHAKE_MS,
        )
    }
}
