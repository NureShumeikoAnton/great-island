//! The capture card: what a screenshot and a finished recording end in. The thumbnail and a hint of what can
//! be done with the file. A click on the thumbnail opens it, a click on the text copies it. After copying
//! (click, Ctrl+C or Ctrl+X) a tick appears over the thumbnail and the labels change in place.
//!
//! - a screenshot lands in a capsule in the middle of the island, and only then does it widen into the card
//!   ([`CardPhase`]; the flight is in `island/flight.rs`);
//! - a recording ([`Video`]) has no flight: the card opens at once, with a play badge and the duration on
//!   the thumbnail, the length and size in the title.

use super::super::comp::{Motion, Text, TextStyle, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::{Content, Hit, IconButton, LATCH, MUTED, Model, Rect, Scene, TEXT, snap, style};
use crate::capture::{Image, Status};
use crate::lang::tr;
use std::{borrow::Cow, sync::Arc};
use windows::{
    UI::Composition::{ContainerVisual, Visual},
    core::{Interface, Result},
};

const HEIGHT: f32 = 64.0;
const PAD: f32 = 12.0;
const THUMB_H: f32 = 40.0;
const THUMB_RADIUS: f32 = 6.0;
const TEXT_GAP: f32 = 12.0;
const RIGHT: f32 = 18.0;
const MIN_WIDTH: f32 = 240.0;
const CHECK: f32 = 22.0;
const CHECK_ICON: f32 = 14.0;
const CHECK_FILL: &str = "#30D158";
const THUMB_DIMMED: f32 = 0.55;
const FEEDBACK: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardPhase {
    /// The shot is approaching: the island turns from the shutter into a capsule sized for the thumbnail,
    /// in the middle, and waits for it empty.
    Catching,
    Caught,
    Open,
}

pub struct CardModel<'a> {
    pub thumb: &'a Arc<Image>,
    pub status: Status,
    pub phase: CardPhase,
    pub video: Option<Video>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Video {
    pub duration_ms: u64,
    pub bytes: u64,
}

fn meta(video: Video) -> String {
    format!(
        " · {} · {}",
        super::format_clock(video.duration_ms as i64),
        super::format_size(video.bytes)
    )
}

fn meta_style() -> TextStyle {
    style(14.0, false, MUTED)
}

pub(super) struct Parts {
    title: Text,
    meta: Option<Text>,
    subtitle: Text,
    picture: Visual,
    check: Visual,
    /// What is already shown: thumbnail opacity, whether the text and tick are visible.
    /// `None` = the view was just built, values are set at once with no animation.
    shown: Option<(f32, bool, bool)>,
}

const CAUGHT: Motion = Motion::linear(120);
const TEXT_IN: Motion = Motion::curve(260, 0.0, 0.0, 0.2, 1.0);

fn texts(status: Status, video: bool) -> (Cow<'static, str>, Cow<'static, str>) {
    match (status, video) {
        (Status::Fresh, false) => (tr!("shot.title"), tr!("shot.hint")),
        (Status::Fresh, true) => (tr!("shot.title_video"), tr!("shot.hint")),
        (Status::Copied, false) => (tr!("shot.copied"), tr!("shot.copied_file_follows")),
        // The video is already in its folder.
        (Status::Copied, true) => (tr!("shot.copied"), tr!("shot.copied_file_saved")),
        (Status::ClipboardOnly, _) => (tr!("shot.clipboard"), tr!("shot.clipboard_no_file")),
    }
}

const BADGE: f32 = 16.0;
const BADGE_ICON: f32 = 9.0;

fn title_style() -> TextStyle {
    style(14.0, true, TEXT)
}

fn subtitle_style() -> TextStyle {
    style(12.0, false, MUTED)
}

fn thumb_rect(image: &Image) -> Rect {
    let aspect = image.width as f32 / image.height.max(1) as f32;
    let w = (THUMB_H * aspect)
        .clamp(THUMB_H * 0.6, THUMB_H * 2.4)
        .round();
    Rect {
        x: PAD,
        y: (HEIGHT - THUMB_H) / 2.0,
        w,
        h: THUMB_H,
    }
}

impl Scene {
    /// Until the shot lands: a capsule sized for the thumbnail, then the whole card.
    pub(super) fn card_size(&self, model: &Model) -> (f32, f32) {
        let Some(shot) = &model.card else {
            return (MIN_WIDTH, HEIGHT);
        };
        if shot.phase != CardPhase::Open {
            return (thumb_rect(shot.thumb).w + 2.0 * PAD, HEIGHT);
        }
        (self.card_width(shot), HEIGHT)
    }

    fn card_width(&self, shot: &CardModel) -> f32 {
        // The width follows the longest label: after Ctrl+C the card doesn't jump.
        let meta_w = shot
            .video
            .map_or(0.0, |video| self.comp.measure(&meta(video), meta_style()));
        let text_w = [Status::Fresh, Status::Copied, Status::ClipboardOnly]
            .into_iter()
            .map(|status| {
                let (title, subtitle) = texts(status, shot.video.is_some());
                (self.comp.measure(&title, title_style()) + meta_w)
                    .max(self.comp.measure(&subtitle, subtitle_style()))
            })
            .fold(0.0, f32::max);
        let thumb = thumb_rect(shot.thumb);
        let width = PAD + thumb.w + TEXT_GAP + text_w + RIGHT;
        width.max(MIN_WIDTH).round()
    }

    /// Where the thumbnail is on the island window (logical pixels). While the shot flies or has landed: in the
    /// middle, in the capsule that meets it; in the open card: at its left edge, where the shot flies out into the
    /// viewer window.
    pub fn card_slot(&self, model: &Model) -> Option<(f32, f32, f32, f32)> {
        let shot = model.card.as_ref()?;
        let thumb = thumb_rect(shot.thumb);
        let left = match shot.phase {
            CardPhase::Open => (self.frame_width - self.card_width(shot)) / 2.0 + thumb.x,
            _ => (self.frame_width - thumb.w) / 2.0,
        };
        Some((left, thumb.y, thumb.w, thumb.h))
    }

    pub(super) fn build_card(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.card_size(model);
        content.width = width;
        content.height = height;
        let Some(shot) = &model.card else {
            return Ok(());
        };
        // The view holds onto the capsule's left edge: when it widens from the middle, the thumbnail moves left
        // with the edge, and the text opens on the right.
        content.anchor_left = true;
        let root = content.root.clone();
        let rect = thumb_rect(shot.thumb);

        // The thumbnail is a button: slightly more transparent under the cursor, shrinks on press.
        // The dimming under the tick belongs to the wrapper so it doesn't fight the hover.
        let holder = self.comp.container()?;
        holder.SetSize(v2(rect.w, rect.h))?;
        let thumb = self.comp.image_rounded(
            &shot.thumb.pixels,
            (shot.thumb.width, shot.thumb.height),
            (rect.w, rect.h),
            THUMB_RADIUS,
            self.scale,
        )?;
        thumb.SetCenterPoint(v3(rect.w / 2.0, rect.h / 2.0))?;
        holder.Children()?.InsertAtTop(&thumb)?;
        if let Some(video) = shot.video {
            self.video_badge(&holder, rect, video.duration_ms)?;
        }
        self.place(&root, &holder, rect.x, rect.y)?;
        content.icon_buttons.push(IconButton {
            visual: thumb.cast()?,
            hit: Hit::Card,
        });
        content.hits.push((rect, Hit::Card));

        let check = self.check_badge()?;
        check.SetOpacity(0.0)?;
        check.SetScale(scale(0.6, 0.6))?;
        self.place(
            &root,
            &check,
            rect.x + (rect.w - CHECK) / 2.0,
            rect.y + (rect.h - CHECK) / 2.0,
        )?;

        let text_x = rect.x + rect.w + TEXT_GAP;
        let text_rect = Rect {
            x: text_x,
            y: 0.0,
            w: self.card_width(shot) - text_x - RIGHT / 2.0,
            h: HEIGHT,
        };
        let label = self.comp.container()?;
        label.SetSize(v2(text_rect.w, text_rect.h))?;
        label.SetCenterPoint(v3(text_rect.w / 2.0, text_rect.h / 2.0))?;
        self.place(&root, &label, text_rect.x, text_rect.y)?;
        let title = self.comp.text("", title_style(), self.scale)?;
        let subtitle = self.comp.text("", subtitle_style(), self.scale)?;
        label.Children()?.InsertAtTop(&title.visual)?;
        label.Children()?.InsertAtTop(&subtitle.visual)?;
        let meta = match shot.video {
            Some(video) => {
                let text = self.comp.text(&meta(video), meta_style(), self.scale)?;
                label.Children()?.InsertAtTop(&text.visual)?;
                Some(text)
            }
            None => None,
        };
        content.icon_buttons.push(IconButton {
            visual: label.cast()?,
            hit: Hit::CardCopy,
        });
        content.hits.push((text_rect, Hit::CardCopy));
        content.card = Some(Parts {
            title,
            meta,
            subtitle,
            picture: holder.cast()?,
            check: check.cast()?,
            shown: None,
        });
        Ok(())
    }

    fn video_badge(&self, holder: &ContainerVisual, rect: Rect, duration: u64) -> Result<()> {
        let c = &self.comp.compositor;
        let s = self.scale;
        let badge = c.CreateShapeVisual()?;
        badge.SetSize(v2(BADGE, BADGE))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(BADGE / 2.0, BADGE / 2.0))?;
        geometry.SetRadius(v2(BADGE / 2.0, BADGE / 2.0))?;
        let fill = c.CreateSpriteShapeWithGeometry(&geometry)?;
        fill.SetFillBrush(&self.comp.brush(color("#000000", 0.55))?)?;
        badge.Shapes()?.Append(&fill)?;
        let (bx, by) = ((rect.w - BADGE) / 2.0, (rect.h - BADGE) / 2.0);
        badge.SetOffset(v3(snap(bx, s), snap(by, s)))?;
        holder.Children()?.InsertAtTop(&badge)?;
        let icon = self
            .comp
            .glyph(Glyph::Play, BADGE_ICON, color("#FFFFFF", 1.0), s)?;
        let inset = (BADGE - BADGE_ICON) / 2.0;
        icon.SetOffset(v3(snap(bx + inset + 0.5, s), snap(by + inset, s)))?;
        holder.Children()?.InsertAtTop(&icon)?;

        let label = self.comp.text(
            &super::format_clock(duration as i64),
            style(9.0, true, "#FFFFFF"),
            s,
        )?;
        let (pad, w, h) = (3.0, label.width + 6.0, label.height);
        let pill = c.CreateShapeVisual()?;
        pill.SetSize(v2(w, h))?;
        let shape = c.CreateRoundedRectangleGeometry()?;
        shape.SetSize(v2(w, h))?;
        shape.SetCornerRadius(v2(3.0, 3.0))?;
        let back = c.CreateSpriteShapeWithGeometry(&shape)?;
        back.SetFillBrush(&self.comp.brush(color("#000000", 0.6))?)?;
        pill.Shapes()?.Append(&back)?;
        let (px, py) = (rect.w - w - 2.0, rect.h - h - 2.0);
        pill.SetOffset(v3(snap(px, s), snap(py, s)))?;
        holder.Children()?.InsertAtTop(&pill)?;
        label.visual.SetOffset(v3(snap(px + pad, s), snap(py, s)))?;
        holder.Children()?.InsertAtTop(&label.visual)
    }

    fn check_badge(&self) -> Result<Visual> {
        let c = &self.comp.compositor;
        let badge = c.CreateShapeVisual()?;
        badge.SetSize(v2(CHECK, CHECK))?;
        badge.SetCenterPoint(v3(CHECK / 2.0, CHECK / 2.0))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(CHECK / 2.0, CHECK / 2.0))?;
        geometry.SetRadius(v2(CHECK / 2.0, CHECK / 2.0))?;
        let fill = c.CreateSpriteShapeWithGeometry(&geometry)?;
        fill.SetFillBrush(&self.comp.brush(color(CHECK_FILL, 1.0))?)?;
        badge.Shapes()?.Append(&fill)?;

        let holder = self.comp.container()?;
        holder.SetSize(v2(CHECK, CHECK))?;
        holder.SetCenterPoint(v3(CHECK / 2.0, CHECK / 2.0))?;
        holder.Children()?.InsertAtTop(&badge)?;
        let icon = self
            .comp
            .glyph(Glyph::Check, CHECK_ICON, color("#FFFFFF", 1.0), self.scale)?;
        let inset = (CHECK - CHECK_ICON) / 2.0;
        icon.SetOffset(v3(inset, inset))?;
        holder.Children()?.InsertAtTop(&icon)?;
        holder.cast()
    }

    pub(super) fn update_card(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (Some(shot), Some(parts)) = (&model.card, &mut content.card) else {
            return Ok(());
        };
        let s = self.scale;
        let (title, subtitle) = texts(shot.status, shot.video.is_some());
        self.comp.set_text(&mut parts.title, &title, s)?;
        self.comp.set_text(&mut parts.subtitle, &subtitle, s)?;
        let x = 0.0;
        let top = self
            .comp
            .stack_top(&parts.title, &parts.subtitle, 2.0, HEIGHT);
        parts.title.visual.SetOffset(v3(snap(x, s), snap(top, s)))?;
        if let Some(meta) = &parts.meta {
            meta.visual
                .SetOffset(v3(snap(x + parts.title.width, s), snap(top, s)))?;
        }
        parts
            .subtitle
            .visual
            .SetOffset(v3(snap(x, s), snap(top + parts.title.height + 2.0, s)))?;

        // The tick is visible only when the thumbnail is already in place.
        let caught = shot.phase != CardPhase::Catching;
        let checked = caught && shot.status != Status::Fresh;
        let picture = match (caught, checked) {
            (false, _) => 0.0,
            (true, true) => THUMB_DIMMED,
            (true, false) => 1.0,
        };
        let text = shot.phase == CardPhase::Open;
        let want = (picture, text, checked);
        let text_opacity = if text { 1.0 } else { 0.0 };
        match parts.shown {
            None => {
                parts.picture.SetOpacity(picture)?;
                parts.title.visual.SetOpacity(text_opacity)?;
                parts.subtitle.visual.SetOpacity(text_opacity)?;
                if let Some(meta) = &parts.meta {
                    meta.visual.SetOpacity(text_opacity)?;
                }
                if checked {
                    parts.check.SetOpacity(1.0)?;
                    parts.check.SetScale(scale(1.0, 1.0))?;
                }
            }
            Some((was_picture, was_text, was_checked)) => {
                if was_picture != picture {
                    let motion = if checked { FEEDBACK } else { CAUGHT };
                    self.comp
                        .animate_scalar(&parts.picture, "Opacity", picture, motion)?;
                }
                if was_text != text {
                    let meta = parts.meta.as_ref().map(|m| &m.visual);
                    for label in [
                        Some(&parts.title.visual),
                        Some(&parts.subtitle.visual),
                        meta,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.comp
                            .animate_scalar(label, "Opacity", text_opacity, TEXT_IN)?;
                    }
                }
                if checked && !was_checked {
                    self.comp
                        .animate_scalar(&parts.check, "Opacity", 1.0, FEEDBACK)?;
                    self.comp
                        .animate_vector3(&parts.check, "Scale", scale(1.0, 1.0), LATCH)?;
                }
            }
        }
        parts.shown = Some(want);
        content.width = self.card_size(model).0;
        Ok(())
    }
}
