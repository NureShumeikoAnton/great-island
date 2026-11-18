//! "Now playing" activity view: collapsed, track announce and expanded card. Sizes and colors come from
//! `design/now-playing.html`.
//!
//! Status (playing or paused) is not part of the view key: on a change the view isn't rebuilt but flows:
//! the bars converge into a pause sign, the cover dims, the button swaps.

mod bars;

use super::super::comp::{Comp, Motion, Text, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::{
    Content, FADE_IN, FADE_OUT, Hit, IconButton, Liveness, MUTED, Model, Modes, PlayerForm,
    PlayerHit, Rect, Scene, TEXT, clock_style, format_clock, style,
};
use crate::lang::tr;
use crate::media::{Level, NEUTRAL_TINT, Session, Timeline};
use bars::{ANNOUNCE_BARS, Bars, BarsMode, COLLAPSED_BARS, EXPANDED_BARS};
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionObject, CompositionRoundedRectangleGeometry, ContainerVisual, ShapeVisual,
            SpriteVisual, Visual,
        },
    },
    core::{Interface, Result},
};

const TRACK_TIME: Liveness = Liveness::Live;
const BARS: Liveness = Liveness::Freezable;

const STATUS: Motion = Motion::curve(320, 0.32, 1.28, 0.5, 1.0);
/// Loudness is sampled 30 times a second; the compositor eases the bars between samples.
pub const SAMPLE_MS: u32 = 33;
const NO_COVER: &str = "#2C2C2E";
const TRACK_ALPHA: f32 = 0.16;

struct Progress {
    visual: ShapeVisual,
    fill: CompositionRoundedRectangleGeometry,
    rect: Rect,
    applied: Option<(Timeline, bool)>,
}

impl Progress {
    fn set(&mut self, comp: &Comp, timeline: Timeline, playing: bool, now: u64) -> Result<()> {
        if self.applied == Some((timeline, playing)) {
            return Ok(());
        }
        self.applied = Some((timeline, playing));
        let duration = timeline.duration_ms.max(1);
        let position = timeline.position_at(now, playing);
        self.show_fraction(comp, position as f32 / duration as f32)?;
        if playing && position < duration {
            comp.animate_vector2(
                &self.fill,
                "Size",
                v2(self.rect.w, 4.0),
                Motion::linear((duration - position) as u32),
            )?;
        }
        Ok(())
    }

    fn show_fraction(&self, comp: &Comp, fraction: f32) -> Result<()> {
        comp.stop(&self.fill, "Size")?;
        self.fill
            .SetSize(v2(self.rect.w * fraction.clamp(0.0, 1.0), 4.0))
    }
}

pub(super) struct Parts {
    form: PlayerForm,
    bars: Bars,
    cover: Visual,
    /// Cover opacity when paused: 50% collapsed, 75% in the card.
    cover_paused: f32,
    playing: Option<bool>,
    time: Option<Text>,
    time_shown: Option<bool>,
    elapsed: Option<Text>,
    remaining: Option<Text>,
    progress: Option<Progress>,
    toggle: Option<(SpriteVisual, SpriteVisual)>,
    /// The card's artist line and the copied note in its place; swapped in place, not by a rebuild,
    /// which would cross-fade the whole card.
    artist: Option<(SpriteVisual, SpriteVisual)>,
    copied: Option<bool>,
}

impl Parts {
    pub fn wants_levels(&self) -> bool {
        self.bars.is_live()
    }

    pub fn seek_fraction(&self, x: f32) -> Option<f32> {
        let p = self.progress.as_ref()?;
        Some(((x - p.rect.x) / p.rect.w).clamp(0.0, 1.0))
    }
}

/// Mode badges stand first, where the timers view has them.
const BADGES_X: f32 = 13.0;

fn tint_of(session: &Session) -> Color {
    let [r, g, b] = session.cover.as_ref().map_or(NEUTRAL_TINT, |c| c.tint);
    Color {
        A: 255,
        R: r,
        G: g,
        B: b,
    }
}

fn artist_line(session: &Session) -> String {
    match (session.artist.is_empty(), session.album.is_empty()) {
        (false, false) => format!("{} · {}", session.artist, session.album),
        (false, true) => session.artist.clone(),
        (true, false) => session.album.clone(),
        (true, true) => String::new(),
    }
}

impl Scene {
    /// Without badges the cover is concentric with the capsule's rounding.
    fn collapsed_cover_x(&self, modes: Modes) -> f32 {
        match self.badges_width(modes) {
            0.0 => 6.0,
            badges => BADGES_X + badges,
        }
    }

    pub(super) fn build_player(
        &self,
        model: &Model,
        form: PlayerForm,
        content: &mut Content,
    ) -> Result<()> {
        (content.width, content.height) = form.size();
        let Some(player) = &model.player else {
            return Ok(());
        };
        let parts = match form {
            PlayerForm::Collapsed => {
                self.build_player_collapsed(player.session, model.modes, content)?
            }
            PlayerForm::Announce => self.build_announce(player.session, content)?,
            PlayerForm::Expanded { timeline } => {
                self.build_player_card(model, player.session, player.headphones, timeline, content)?
            }
        };
        content.player = Some(parts);
        Ok(())
    }

    fn cover(&self, session: &Session, size: f32) -> Result<Visual> {
        if let Some(cover) = &session.cover {
            return self
                .comp
                .image(
                    &cover.bgra,
                    (cover.size, cover.size),
                    size,
                    true,
                    self.scale,
                )?
                .cast();
        }
        let c = &self.comp.compositor;
        let holder = c.CreateContainerVisual()?;
        holder.SetSize(v2(size, size))?;
        let circle = c.CreateShapeVisual()?;
        circle.SetSize(v2(size, size))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(size / 2.0, size / 2.0))?;
        geometry.SetRadius(v2(size / 2.0, size / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&self.comp.brush(color(NO_COVER, 1.0))?)?;
        circle.Shapes()?.Append(&shape)?;
        holder.Children()?.InsertAtTop(&circle)?;
        let glyph = if session.video {
            Glyph::Video
        } else {
            Glyph::Note
        };
        let mark = (size * 0.5).round();
        let icon = self
            .comp
            .glyph(glyph, mark, color(MUTED, 1.0), self.scale)?;
        icon.SetOffset(v3((size - mark) / 2.0, (size - mark) / 2.0))?;
        holder.Children()?.InsertAtTop(&icon)?;
        holder.cast()
    }

    fn parts(&self, form: PlayerForm, bars: Bars, cover: Visual, cover_paused: f32) -> Parts {
        Parts {
            form,
            bars,
            cover,
            cover_paused,
            playing: None,
            time: None,
            time_shown: None,
            elapsed: None,
            remaining: None,
            progress: None,
            toggle: None,
            artist: None,
            copied: None,
        }
    }

    /// Collapsed: the mode badges and an 18 px cover on the left, the remaining track time, bars on the right.
    /// The view is anchored to the capsule's left edge, the bars to the right one: as the time appears and
    /// disappears the capsule changes width while the cover and bars stay at their edges.
    fn build_player_collapsed(
        &self,
        session: &Session,
        modes: Modes,
        content: &mut Content,
    ) -> Result<Parts> {
        let root = content.root.clone();
        content.anchor_left = true;
        self.badges(&root, BADGES_X, modes)?;
        let cover = self.cover(session, 18.0)?;
        self.place(&root, &cover, self.collapsed_cover_x(modes), 6.0)?;

        let time = self.comp.text("", clock_style(13.0, TEXT), self.scale)?;
        time.visual.SetOpacity(0.0)?;
        root.Children()?.InsertAtTop(&time.visual)?;

        let bars = Bars::new(&self.comp, COLLAPSED_BARS, tint_of(session))?;
        root.Children()?.InsertAtTop(&bars.visual)?;
        let g: CompositionObject = self.geometry.cast()?;
        self.comp.bind(
            &bars.visual,
            "Offset",
            "Vector3(g.Size.X - 12 - B, 9, 0)",
            &[("g", &g)],
            &[("B", COLLAPSED_BARS.width())],
        )?;

        let mut parts = self.parts(PlayerForm::Collapsed, bars, cover, 0.5);
        parts.time = Some(time);
        Ok(parts)
    }

    fn build_announce(&self, session: &Session, content: &mut Content) -> Result<Parts> {
        let root = content.root.clone();
        let cover = self.cover(session, 36.0)?;
        self.place(&root, &cover, 8.0, 8.0)?;

        let text_x = 8.0 + 36.0 + 10.0;
        let bars_x = 300.0 - 18.0 - ANNOUNCE_BARS.width();
        let avail = bars_x - 10.0 - text_x;
        let title_style = style(14.0, true, TEXT);
        let artist_style = style(12.0, false, MUTED);
        let title = self.comp.fit(&session.title, title_style, avail);
        let title = self.comp.text(&title, title_style, self.scale)?;
        let artist = self.comp.fit(&session.artist, artist_style, avail);
        let artist = self.comp.text(&artist, artist_style, self.scale)?;
        let top = (52.0 - (title.height + artist.height)) / 2.0;
        self.place(&root, &title.visual, text_x, top)?;
        self.place(&root, &artist.visual, text_x, top + title.height)?;

        let bars = Bars::new(&self.comp, ANNOUNCE_BARS, tint_of(session))?;
        self.place(
            &root,
            &bars.visual,
            bars_x,
            (52.0 - ANNOUNCE_BARS.height) / 2.0,
        )?;
        Ok(self.parts(PlayerForm::Announce, bars, cover, 1.0))
    }

    fn build_player_card(
        &self,
        model: &Model,
        session: &Session,
        headphones: bool,
        timeline: bool,
        content: &mut Content,
    ) -> Result<Parts> {
        let root = content.root.clone();
        let width = content.width;
        let tint = tint_of(session);

        if session.cover.is_some() {
            let wash = self.comp.compositor.CreateSpriteVisual()?;
            wash.SetSize(v2(width, content.height))?;
            wash.SetBrush(
                &self
                    .comp
                    .radial_wash(v2(42.0, 42.0), v2(140.0, 110.0), tint, 0.16)?,
            )?;
            root.Children()?.InsertAtBottom(&wash)?;
        }
        self.dots(&root, width, model)?;

        let cover = self.cover(session, 52.0)?;
        self.place(&root, &cover, 16.0, 16.0)?;
        content.hits.push((
            Rect {
                x: 16.0,
                y: 16.0,
                w: 52.0,
                h: 52.0,
            },
            Hit::Player(PlayerHit::Open),
        ));

        let bars_x = width - 16.0 - EXPANDED_BARS.width();
        let text_x = 16.0 + 52.0 + 12.0;
        let avail = bars_x - 12.0 - text_x;
        let title_style = style(15.0, true, TEXT);
        let artist_style = style(13.0, false, MUTED);
        let artist = self.comp.fit(&artist_line(session), artist_style, avail);
        let artist = self.comp.text(&artist, artist_style, self.scale)?;
        let copied = self
            .comp
            .text(&tr!("player.copied"), artist_style, self.scale)?;
        let title_h = self.comp.text("Ag", title_style, self.scale)?.height;
        let top = 16.0 + (52.0 - (title_h + artist.height)) / 2.0;
        self.title(
            &root,
            &session.title,
            title_style,
            text_x,
            top,
            avail,
            title_h,
        )?;
        self.place(&root, &artist.visual, text_x, top + title_h)?;
        self.place(&root, &copied.visual, text_x, top + title_h)?;
        content.hits.push((
            Rect {
                x: text_x,
                y: top,
                w: avail,
                h: title_h + artist.height,
            },
            Hit::Player(PlayerHit::Copy),
        ));

        let bars = Bars::new(&self.comp, EXPANDED_BARS, tint)?;
        self.place(
            &root,
            &bars.visual,
            bars_x,
            16.0 + (52.0 - EXPANDED_BARS.height) / 2.0,
        )?;
        let mut parts = self.parts(PlayerForm::Expanded { timeline }, bars, cover, 0.75);
        parts.artist = Some((artist.visual.clone(), copied.visual.clone()));

        let mut controls_y = 16.0 + 52.0 + 12.0;
        if timeline {
            let row_y = 16.0 + 52.0 + 14.0;
            let elapsed = self.comp.text("", clock_style(11.0, TEXT), self.scale)?;
            let remaining = self.comp.text("", clock_style(11.0, MUTED), self.scale)?;
            for text in [&elapsed, &remaining] {
                root.Children()?.InsertAtTop(&text.visual)?;
            }
            parts.elapsed = Some(elapsed);
            parts.remaining = Some(remaining);

            let rect = Rect {
                x: 16.0 + 34.0 + 8.0,
                y: row_y + 5.0,
                w: width - 2.0 * (16.0 + 34.0 + 8.0),
                h: 4.0,
            };
            let progress = self.progress(&root, rect, tint)?;
            if session.controls.seek {
                content.hits.push((
                    Rect {
                        x: rect.x - 4.0,
                        y: rect.y - 8.0,
                        w: rect.w + 8.0,
                        h: rect.h + 16.0,
                    },
                    Hit::Player(PlayerHit::Seek),
                ));
            }
            parts.progress = Some(progress);
            controls_y = row_y + 14.0 + 10.0;
        }

        let controls = session.controls;
        let mid_x = (width - (3.0 * 40.0 + 2.0 * 20.0)) / 2.0;
        let buttons = [
            (
                Glyph::Previous,
                18.0,
                PlayerHit::Previous,
                controls.previous,
            ),
            (Glyph::Play, 22.0, PlayerHit::PlayPause, controls.play_pause),
            (Glyph::Next, 18.0, PlayerHit::Next, controls.next),
        ];
        for (i, (glyph, size, hit, enabled)) in buttons.into_iter().enumerate() {
            let rect = Rect {
                x: mid_x + i as f32 * 60.0,
                y: controls_y,
                w: 40.0,
                h: 32.0,
            };
            let holder = self.comp.container()?;
            holder.SetSize(v2(rect.w, rect.h))?;
            holder.SetCenterPoint(v3(rect.w / 2.0, rect.h / 2.0))?;
            let icon = self.comp.glyph(glyph, size, color(TEXT, 1.0), self.scale)?;
            icon.SetOffset(v3((rect.w - size) / 2.0, (rect.h - size) / 2.0))?;
            holder.Children()?.InsertAtTop(&icon)?;
            if hit == PlayerHit::PlayPause {
                let pause = self
                    .comp
                    .glyph(Glyph::Pause, size, color(TEXT, 1.0), self.scale)?;
                pause.SetOffset(v3((rect.w - size) / 2.0, (rect.h - size) / 2.0))?;
                pause.SetOpacity(0.0)?;
                holder.Children()?.InsertAtTop(&pause)?;
                for v in [&icon, &pause] {
                    v.SetCenterPoint(v3(size / 2.0, size / 2.0))?;
                }
                parts.toggle = Some((icon, pause));
            }
            self.place(&root, &holder, rect.x, rect.y)?;
            if enabled {
                content.hits.push((rect, Hit::Player(hit)));
                content.icon_buttons.push(IconButton {
                    visual: holder.cast()?,
                    hit: Hit::Player(hit),
                });
            } else {
                holder.SetOpacity(0.35)?;
            }
        }

        let device = if headphones {
            Glyph::Headphones
        } else {
            Glyph::Speaker
        };
        let device = self
            .comp
            .glyph(device, 16.0, color(MUTED, 1.0), self.scale)?;
        self.place(&root, &device, 16.0, controls_y + 8.0)?;
        if let Some(icon) = &session.icon {
            let app = self.comp.image(
                &icon.bgra,
                (icon.width, icon.height),
                16.0,
                false,
                self.scale,
            )?;
            let rect = Rect {
                x: width - 16.0 - 32.0,
                y: controls_y,
                w: 32.0,
                h: 32.0,
            };
            self.place(&root, &app, width - 16.0 - 16.0, controls_y + 8.0)?;
            content.hits.push((rect, Hit::Player(PlayerHit::Open)));
        }
        Ok(parts)
    }

    /// Title: a long one is truncated with an ellipsis, and 1.2 s after the card opens it scrolls through once
    /// (40 px/s) and returns to the ellipsis. All the motion is one compositor animation.
    #[allow(clippy::too_many_arguments)]
    fn title(
        &self,
        root: &ContainerVisual,
        title: &str,
        st: super::TextStyle,
        x: f32,
        y: f32,
        avail: f32,
        line: f32,
    ) -> Result<()> {
        let full_w = self.comp.measure(title, st);
        if full_w <= avail {
            let text = self.comp.text(title, st, self.scale)?;
            return self.place(root, &text.visual, x, y);
        }
        let c = &self.comp.compositor;
        let box_ = c.CreateContainerVisual()?;
        box_.SetSize(v2(avail, line + 2.0))?;
        box_.SetClip(&c.CreateInsetClip()?)?;
        self.place(root, &box_, x, y)?;

        let short = self
            .comp
            .text(&self.comp.fit(title, st, avail), st, self.scale)?;
        let full = self.comp.text(title, st, self.scale)?;
        full.visual.SetOpacity(0.0)?;
        box_.Children()?.InsertAtTop(&short.visual)?;
        box_.Children()?.InsertAtTop(&full.visual)?;

        let overflow = full_w - avail;
        let (delay, fade, hold) = (1200.0, 150.0, 600.0);
        let slide = overflow / 40.0 * 1000.0;
        let total = delay + fade + slide + hold + fade;
        let at = |ms: f32| ms / total;
        let shown = [
            (0.0, 0.0),
            (at(delay), 0.0),
            (at(delay + fade), 1.0),
            (at(total - fade), 1.0),
            (1.0, 0.0),
        ];
        let hidden: Vec<(f32, f32)> = shown.iter().map(|&(t, v)| (t, 1.0 - v)).collect();
        let slide_keys = [
            (0.0, 0.0),
            (at(delay + fade), 0.0),
            (at(delay + fade + slide), -overflow),
            (at(total - fade), -overflow),
            (1.0, -overflow),
        ];
        let ms = total as u32;
        self.comp.keyframes(&full.visual, "Opacity", &shown, ms)?;
        self.comp.keyframes(&short.visual, "Opacity", &hidden, ms)?;
        self.comp
            .keyframes(&full.visual, "Offset.X", &slide_keys, ms)
    }

    fn progress(&self, root: &ContainerVisual, rect: Rect, tint: Color) -> Result<Progress> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(rect.w, rect.h))?;
        visual.SetCenterPoint(v3(0.0, rect.h / 2.0))?;
        let track = c.CreateRoundedRectangleGeometry()?;
        track.SetSize(v2(rect.w, rect.h))?;
        track.SetCornerRadius(v2(2.0, 2.0))?;
        let track_shape = c.CreateSpriteShapeWithGeometry(&track)?;
        track_shape.SetFillBrush(&self.comp.brush(color("#FFFFFF", TRACK_ALPHA))?)?;
        let fill = c.CreateRoundedRectangleGeometry()?;
        fill.SetSize(v2(0.0, rect.h))?;
        fill.SetCornerRadius(v2(2.0, 2.0))?;
        let fill_shape = c.CreateSpriteShapeWithGeometry(&fill)?;
        fill_shape.SetFillBrush(&self.comp.brush(tint)?)?;
        visual.Shapes()?.Append(&track_shape)?;
        visual.Shapes()?.Append(&fill_shape)?;
        self.place(root, &visual, rect.x, rect.y)?;
        Ok(Progress {
            visual,
            fill,
            rect,
            applied: None,
        })
    }

    pub(super) fn update_player(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (Some(parts), Some(player)) = (content.player.as_mut(), &model.player) else {
            return Ok(());
        };
        let session = player.session;
        let playing = session.is_playing();
        let mode = if !playing {
            BarsMode::Paused
        } else if !BARS.moving(model.frozen) {
            BarsMode::Frozen
        } else if player.decorative {
            BarsMode::Decorative
        } else {
            BarsMode::Live
        };
        parts.bars.set_mode(&self.comp, mode)?;

        if parts.playing != Some(playing) {
            let first = parts.playing.is_none();
            parts.playing = Some(playing);
            let opacity = if playing { 1.0 } else { parts.cover_paused };
            if first {
                parts.cover.SetOpacity(opacity)?;
            } else {
                self.comp
                    .animate_scalar(&parts.cover, "Opacity", opacity, STATUS)?;
            }
            if let Some((play, pause)) = &parts.toggle {
                let (show, hide) = if playing {
                    (pause, play)
                } else {
                    (play, pause)
                };
                if first {
                    show.SetOpacity(1.0)?;
                    hide.SetOpacity(0.0)?;
                } else {
                    self.comp.animate_scalar(show, "Opacity", 1.0, FADE_IN)?;
                    self.comp.animate_scalar(hide, "Opacity", 0.0, FADE_OUT)?;
                    show.SetScale(scale(0.8, 0.8))?;
                    self.comp
                        .animate_vector3(show, "Scale", scale(1.0, 1.0), STATUS)?;
                }
            }
        }

        if let Some((artist, note)) = &parts.artist
            && parts.copied != Some(player.copied)
        {
            let first = parts.copied.is_none();
            parts.copied = Some(player.copied);
            let (show, hide) = if player.copied {
                (note, artist)
            } else {
                (artist, note)
            };
            if first {
                show.SetOpacity(1.0)?;
                hide.SetOpacity(0.0)?;
            } else {
                self.comp.animate_scalar(show, "Opacity", 1.0, FADE_IN)?;
                self.comp.animate_scalar(hide, "Opacity", 0.0, FADE_OUT)?;
            }
        }

        let now = player.now_unix_ms;
        let timeline = session.timeline;
        let left = |t: Timeline| t.duration_ms - t.position_at(now, playing);
        match parts.form {
            PlayerForm::Collapsed => {
                let show = TRACK_TIME.visible(model.frozen) && timeline.is_some();
                let after_cover = self.collapsed_cover_x(model.modes) + 18.0 + 12.0;
                let mut width = after_cover + COLLAPSED_BARS.width() + 12.0;
                if let (Some(text), Some(t)) = (parts.time.as_mut(), timeline) {
                    self.comp
                        .set_text(text, &format!("−{}", format_clock(left(t))), self.scale)?;
                    text.visual
                        .SetOffset(v3(after_cover, ((30.0 - text.height) / 2.0).round()))?;
                    if show {
                        width += text.width + 12.0;
                    }
                }
                if parts.time_shown != Some(show) {
                    let first = parts.time_shown.is_none();
                    parts.time_shown = Some(show);
                    if let Some(text) = &parts.time {
                        let (opacity, motion) = if show {
                            (1.0, FADE_IN)
                        } else {
                            (0.0, FADE_OUT)
                        };
                        if first {
                            text.visual.SetOpacity(opacity)?;
                        } else {
                            self.comp
                                .animate_scalar(&text.visual, "Opacity", opacity, motion)?;
                        }
                    }
                }
                content.width = width.round();
            }
            PlayerForm::Expanded { .. } => {
                if let (Some(t), Some(progress)) = (timeline, parts.progress.as_mut()) {
                    progress.set(&self.comp, t, playing, now)?;
                    let rect = progress.rect;
                    let row_y = rect.y - 5.0;
                    if let Some(text) = parts.elapsed.as_mut() {
                        self.comp.set_text(
                            text,
                            &format_clock(t.position_at(now, playing)),
                            self.scale,
                        )?;
                        text.visual
                            .SetOffset(v3(16.0, row_y + (14.0 - text.height) / 2.0))?;
                    }
                    if let Some(text) = parts.remaining.as_mut() {
                        self.comp.set_text(
                            text,
                            &format!("−{}", format_clock(left(t))),
                            self.scale,
                        )?;
                        let right = rect.x + rect.w + 8.0 + 34.0;
                        text.visual.SetOffset(v3(
                            right - text.width,
                            row_y + (14.0 - text.height) / 2.0,
                        ))?;
                    }
                }
            }
            PlayerForm::Announce => {}
        }
        Ok(())
    }

    pub(super) fn hover_progress(&self, parts: &Parts, hovered: bool) -> Result<()> {
        if let Some(p) = &parts.progress {
            let k = if hovered { 1.5 } else { 1.0 };
            self.comp
                .animate_vector3(&p.visual, "Scale", scale(1.0, k), super::HOVER)?;
        }
        Ok(())
    }

    pub fn feed_levels(&mut self, level: Level, dt_ms: f32) -> Result<()> {
        let comp = &self.comp;
        match self.content.as_mut().and_then(|c| c.player.as_mut()) {
            Some(parts) => parts.bars.feed(comp, level, dt_ms),
            None => Ok(()),
        }
    }

    pub fn wants_levels(&self) -> bool {
        self.content
            .as_ref()
            .and_then(|c| c.player.as_ref())
            .is_some_and(Parts::wants_levels)
    }

    pub fn seek_fraction(&self, x: f32) -> Option<f32> {
        let content = self.content.as_ref()?;
        let local_x = x - (self.frame_width - content.width) / 2.0;
        content.player.as_ref()?.seek_fraction(local_x)
    }

    pub fn preview_seek(&mut self, fraction: f32) -> Result<()> {
        let comp = &self.comp;
        if let Some(progress) = self
            .content
            .as_mut()
            .and_then(|c| c.player.as_mut())
            .and_then(|p| p.progress.as_mut())
        {
            // The next player snapshot restarts the motion from the new position.
            progress.applied = None;
            progress.show_fraction(comp, fraction)?;
        }
        Ok(())
    }
}
