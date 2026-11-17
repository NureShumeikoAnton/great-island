//! The pre-start panel of the recording: the control center's own width, opened by its
//! record button.
//! - **main page**: the start circle in the corner, what is recorded and a summary of the
//!   folded choices; the area (lit while one is picked), countdown and "more" circles; the
//!   sound segment, whose microphone positions carry an arrow to the microphones page;
//!   unfolded, quality, codec, frames per second and the dissolve after a pause, each row
//!   with its icon;
//! - **microphones page**: `microphones.rs`.
//!
//! Choices change in place: the white pill slides to the new position, the labels swap
//! colors. Only switching pages builds a new view.

use super::super::comp::{Motion, Text, TextStyle, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::{
    Button, Content, FADE_IN, FADE_OUT, Hit, IconButton, MUTED, Model, Rect, Scene, TEXT, snap,
    style,
};
use crate::lang::tr;
use crate::record::{
    choice::{Choice, Codec, Quality, Rate},
    panel::{Microphone, Page, Press},
    sound::Sound,
};
use windows::{
    UI::{
        Color,
        Composition::{CompositionColorBrush, ContainerVisual, ShapeVisual, Visual},
    },
    core::{Interface, Result},
};

pub const WIDTH: f32 = super::menu::WIDTH;

pub(super) const PAD: f32 = 14.0;
const START: f32 = 48.0;
const START_RING: f32 = 3.0;
const START_DOT: f32 = 36.0;
const ROUND: f32 = 36.0;
const ROUND_GAP: f32 = 8.0;
const ROUND_ICON: f32 = 18.0;
const TEXT_GAP: f32 = 12.0;
const ROW_GAP: f32 = 10.0;
const SOUND_H: f32 = 44.0;
const SOUND_INSET: f32 = 3.0;
const SOUND_ICON: f32 = 18.0;
const SMALL_H: f32 = 28.0;
const SMALL_INSET: f32 = 2.0;
const SMALL_GAP: f32 = 8.0;
const BADGE: f32 = 16.0;
const BADGE_ICON: f32 = 10.0;
/// The badge takes clicks a little around itself: 16 px is a small target.
const BADGE_HIT: f32 = 24.0;
const SWITCH_W: f32 = 34.0;
const SWITCH_H: f32 = 20.0;
pub(super) const KNOB: f32 = 16.0;

const ROUND_Y: f32 = PAD + (START - ROUND) / 2.0;
const MORE_X: f32 = WIDTH - PAD - ROUND;
const COUNT_X: f32 = MORE_X - ROUND_GAP - ROUND;
const AREA_X: f32 = COUNT_X - ROUND_GAP - ROUND;
const TEXT_X: f32 = PAD + START + TEXT_GAP;
const TEXT_W: f32 = AREA_X - ROUND_GAP - TEXT_X;
const SOUND_Y: f32 = PAD + START + ROW_GAP;
const DETAILS_Y: f32 = SOUND_Y + SOUND_H + ROW_GAP;
const QUALITY_Y: f32 = DETAILS_Y + 1.0 + ROW_GAP;
const CODEC_Y: f32 = QUALITY_Y + SMALL_H + SMALL_GAP;
const FADE_Y: f32 = CODEC_Y + SMALL_H + SMALL_GAP;
const MAIN_H: f32 = SOUND_Y + SOUND_H + PAD;
const MORE_H: f32 = FADE_Y + SMALL_H + PAD;
pub(super) const INNER: f32 = WIDTH - 2.0 * PAD;
const ROW_ICON: f32 = 16.0;
const ROW_ICON_W: f32 = 26.0;

const RED: &str = "#FF375F";
const DARK: &str = "#000000";
pub(super) const FILL: &str = "#1C1C1E";
pub(super) const FILL_HOVER: &str = "#2C2C2E";
const ON_HOVER: &str = "#E5E5EA";
const SWITCH_OFF: &str = "#3A3A3C";

const SWITCH: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);
pub(super) const SLIDE: Motion = Motion::curve(260, 0.22, 1.0, 0.36, 1.0);
const FLY_MS: u32 = 460;
const FLY_STEPS: usize = 12;

pub struct PanelModel<'a> {
    pub page: Page,
    pub more: bool,
    /// The picked area's size; `None` — the whole screen. A change builds the page anew.
    pub area: Option<(u32, u32)>,
    pub choice: &'a Choice,
    pub devices: &'a [Microphone],
    pub selected: usize,
    /// About how many megabytes a minute of this recording takes.
    pub weight_mb: u32,
    /// The checked microphone answers; otherwise there are no bars, only a word.
    pub hearing: bool,
}

pub(super) enum Parts {
    Main(Box<MainParts>),
    Microphones(super::microphones::MicParts),
}

struct Slot {
    x: f32,
    off: Visual,
    on: Visual,
}

struct Segments {
    pill: Visual,
    y: f32,
    slots: Vec<Slot>,
    shown: Option<usize>,
}

/// What the main page shows now; a part is redrawn only when its own value changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shown {
    seconds: u32,
    more: bool,
    fade: bool,
}

pub(super) struct MainParts {
    start: Visual,
    /// Everything but the start circle: it fades in while the circle flies.
    rest: Visual,
    summary: Text,
    sound: Segments,
    badges: [Visual; 2],
    count_fill: CompositionColorBrush,
    count_icon: Visual,
    count_text: Text,
    more_fill: CompositionColorBrush,
    details: Visual,
    quality: Segments,
    codec: Segments,
    rate: Segments,
    switch_fill: CompositionColorBrush,
    knob: Visual,
    knob_fill: CompositionColorBrush,
    /// The summary as it was before fitting: measuring the text again is skipped while
    /// the choice stays.
    summary_of: String,
    shown: Option<Shown>,
}

fn sound_index(sound: Sound) -> usize {
    match sound {
        Sound::Off => 0,
        Sound::System => 1,
        Sound::Microphone => 2,
        Sound::Both => 3,
    }
}

fn quality_label(quality: Quality) -> String {
    match quality {
        Quality::High => tr!("record.quality.high"),
        Quality::Medium => tr!("record.quality.medium"),
        Quality::Low => tr!("record.quality.low"),
    }
    .into_owned()
}

fn codec_label(codec: Codec) -> String {
    match codec {
        Codec::H264 => tr!("record.codec.h264"),
        Codec::Hevc => tr!("record.codec.hevc"),
    }
    .into_owned()
}

fn rate_label(rate: Rate) -> String {
    match rate {
        Rate::Thirty => tr!("record.rate.30"),
        Rate::Sixty => tr!("record.rate.60"),
    }
    .into_owned()
}

/// What is folded under "more", in one figure: about how heavy a minute of it is.
fn summary(weight_mb: u32) -> String {
    tr!("record.panel.weight", mb = weight_mb).into_owned()
}

pub(super) fn bold(size: f32, hex: &str) -> TextStyle {
    style(size, true, hex)
}

/// Center of the start circle when the panel stands still: where the flight ends.
const START_AT: (f32, f32) = (PAD, PAD);

impl Scene {
    pub(super) fn panel_size(&self, model: &Model) -> (f32, f32) {
        match model.panel.as_ref().map(|p| (p.page, p.more)) {
            Some((Page::Microphones, _)) => self.microphones_size(model),
            Some((Page::Main, true)) => (WIDTH, MORE_H),
            _ => (WIDTH, MAIN_H),
        }
    }

    pub(super) fn build_panel(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.panel_size(model);
        content.width = width;
        content.height = height;
        let Some(panel) = &model.panel else {
            return Ok(());
        };
        let parts = match panel.page {
            Page::Main => Parts::Main(Box::new(self.build_main(panel, content)?)),
            Page::Microphones => Parts::Microphones(self.build_microphones(panel, content)?),
        };
        content.panel = Some(parts);
        Ok(())
    }

    fn build_main(&self, panel: &PanelModel, content: &mut Content) -> Result<MainParts> {
        let c = &self.comp.compositor;
        let rest = self.comp.container()?;
        content.root.Children()?.InsertAtTop(&rest)?;

        let start = self.comp.container()?;
        start.SetSize(v2(START, START))?;
        start.SetCenterPoint(v3(START / 2.0, START / 2.0))?;
        let ring = c.CreateShapeVisual()?;
        ring.SetSize(v2(START, START))?;
        let outline = c.CreateEllipseGeometry()?;
        outline.SetCenter(v2(START / 2.0, START / 2.0))?;
        let r = (START - START_RING) / 2.0;
        outline.SetRadius(v2(r, r))?;
        let outline = c.CreateSpriteShapeWithGeometry(&outline)?;
        outline.SetStrokeBrush(&self.comp.brush(color(TEXT, 1.0))?)?;
        outline.SetStrokeThickness(START_RING)?;
        ring.Shapes()?.Append(&outline)?;
        let dot = c.CreateEllipseGeometry()?;
        dot.SetCenter(v2(START / 2.0, START / 2.0))?;
        dot.SetRadius(v2(START_DOT / 2.0, START_DOT / 2.0))?;
        let dot = c.CreateSpriteShapeWithGeometry(&dot)?;
        dot.SetFillBrush(&self.comp.brush(color(RED, 1.0))?)?;
        ring.Shapes()?.Append(&dot)?;
        start.Children()?.InsertAtTop(&ring)?;
        self.place(&content.root, &start, START_AT.0, START_AT.1)?;
        let start_rect = Rect {
            x: START_AT.0,
            y: START_AT.1,
            w: START,
            h: START,
        };
        content.hits.push((start_rect, Hit::Panel(Press::Start)));
        content.icon_buttons.push(IconButton {
            visual: start.cast()?,
            hit: Hit::Panel(Press::Start),
        });

        let title = match panel.area {
            Some((width, height)) => tr!("record.panel.area", width = width, height = height),
            None => tr!("record.panel.screen"),
        };
        let title = self.comp.fit(&title, bold(15.0, TEXT), TEXT_W);
        let title = self.comp.text(&title, bold(15.0, TEXT), self.scale)?;
        let summary = self.comp.text("", style(12.0, false, MUTED), self.scale)?;
        let block = title.height + 2.0 + summary.height;
        let top = PAD + (START - block) / 2.0;
        self.place(&rest, &title.visual, TEXT_X, top)?;
        self.place(&rest, &summary.visual, TEXT_X, top + title.height + 2.0)?;

        let lit = panel.area.is_some();
        let (area, area_fill) = self.round(&rest, AREA_X, if lit { TEXT } else { FILL })?;
        let area_icon = self.comp.glyph(
            Glyph::Screenshot,
            ROUND_ICON,
            color(if lit { DARK } else { TEXT }, 1.0),
            self.scale,
        )?;
        area_icon.SetOffset(v3((ROUND - ROUND_ICON) / 2.0, (ROUND - ROUND_ICON) / 2.0))?;
        area.Children()?.InsertAtTop(&area_icon)?;
        self.round_hit(content, AREA_X, area_fill, Press::Area)?;
        if lit {
            let (base, hover) = (color(TEXT, 1.0), color(ON_HOVER, 1.0));
            self.restyle(&mut content.buttons, &Press::Area, base, hover);
        }

        let (count, count_fill) = self.round(&rest, COUNT_X, FILL)?;
        let count_icon = self
            .comp
            .glyph(Glyph::Timer, ROUND_ICON, color(TEXT, 1.0), self.scale)?;
        count_icon.SetOffset(v3((ROUND - ROUND_ICON) / 2.0, (ROUND - ROUND_ICON) / 2.0))?;
        count.Children()?.InsertAtTop(&count_icon)?;
        let count_text = self.comp.text("", bold(12.0, DARK), self.scale)?;
        count.Children()?.InsertAtTop(&count_text.visual)?;
        self.round_hit(content, COUNT_X, count_fill.clone(), Press::Countdown)?;

        let (more, more_fill) = self.round(&rest, MORE_X, FILL)?;
        let more_icon = self
            .comp
            .glyph(Glyph::More, ROUND_ICON, color(TEXT, 1.0), self.scale)?;
        more_icon.SetOffset(v3((ROUND - ROUND_ICON) / 2.0, (ROUND - ROUND_ICON) / 2.0))?;
        more.Children()?.InsertAtTop(&more_icon)?;
        self.round_hit(content, MORE_X, more_fill.clone(), Press::More)?;

        let sound_rect = Rect {
            x: PAD,
            y: SOUND_Y,
            w: INNER,
            h: SOUND_H,
        };
        let sound_items = [
            (Sound::Off, Glyph::Muted, tr!("record.sound.off")),
            (Sound::System, Glyph::Monitor, tr!("record.sound.system")),
            (
                Sound::Microphone,
                Glyph::Microphone,
                tr!("record.sound.microphone"),
            ),
            (Sound::Both, Glyph::MonitorMic, tr!("record.sound.both")),
        ];
        // The badges' clicks come first: they lie over their segments and take the click.
        let slot_w = (INNER - 2.0 * SOUND_INSET) / sound_items.len() as f32;
        let badge_at: Vec<(f32, f32)> = [2usize, 3]
            .into_iter()
            .map(|i| {
                let x = PAD + SOUND_INSET + (i as f32 + 1.0) * slot_w - BADGE - 4.0;
                (x, SOUND_Y + SOUND_INSET + 3.0)
            })
            .collect();
        for &(x, y) in &badge_at {
            let grow = (BADGE_HIT - BADGE) / 2.0;
            content.hits.push((
                Rect {
                    x: x - grow,
                    y: y - grow,
                    w: BADGE_HIT,
                    h: BADGE_HIT,
                },
                Hit::Panel(Press::Microphones),
            ));
        }
        let sound = self.segments(
            &rest,
            sound_rect,
            SOUND_INSET,
            None,
            &sound_items
                .iter()
                .map(|(_, glyph, label)| (Some(*glyph), label.to_string()))
                .collect::<Vec<_>>(),
            sound_items
                .iter()
                .map(|(s, ..)| Hit::Panel(Press::Sound(*s)))
                .collect(),
            content,
        )?;
        // Made after the segments: over the pill. A visual goes into its parent only once:
        // a second insert is refused (E_INVALIDARG) and the whole view is not built.
        let mut badges = Vec::new();
        for &(x, y) in &badge_at {
            badges.push(self.badge(&rest, x, y)?);
        }

        let details = self.comp.container()?;
        details.SetOpacity(0.0)?;
        rest.Children()?.InsertAtTop(&details)?;
        let line = c.CreateShapeVisual()?;
        line.SetSize(v2(INNER, 1.0))?;
        let line_geometry = c.CreateRectangleGeometry()?;
        line_geometry.SetSize(v2(INNER, 1.0))?;
        let line_shape = c.CreateSpriteShapeWithGeometry(&line_geometry)?;
        line_shape.SetFillBrush(&self.comp.brush(color("#FFFFFF", 0.08))?)?;
        line.Shapes()?.Append(&line_shape)?;
        self.place(&details, &line, PAD, DETAILS_Y)?;

        let words = |labels: Vec<String>| labels.into_iter().map(|l| (None, l)).collect::<Vec<_>>();
        let quality = self.segments(
            &details,
            Rect {
                x: PAD,
                y: QUALITY_Y,
                w: INNER,
                h: SMALL_H,
            },
            SMALL_INSET,
            Some(Glyph::Picture),
            &words(Quality::ALL.into_iter().map(quality_label).collect()),
            Quality::ALL
                .into_iter()
                .map(|q| Hit::Panel(Press::Quality(q)))
                .collect(),
            content,
        )?;
        let half = (INNER - SMALL_GAP) / 2.0;
        let codec = self.segments(
            &details,
            Rect {
                x: PAD,
                y: CODEC_Y,
                w: half,
                h: SMALL_H,
            },
            SMALL_INSET,
            Some(Glyph::Film),
            &words(Codec::ALL.into_iter().map(codec_label).collect()),
            Codec::ALL
                .into_iter()
                .map(|c| Hit::Panel(Press::Codec(c)))
                .collect(),
            content,
        )?;
        let rate = self.segments(
            &details,
            Rect {
                x: PAD + half + SMALL_GAP,
                y: CODEC_Y,
                w: half,
                h: SMALL_H,
            },
            SMALL_INSET,
            Some(Glyph::Gauge),
            &words(Rate::ALL.into_iter().map(rate_label).collect()),
            Rate::ALL
                .into_iter()
                .map(|r| Hit::Panel(Press::Rate(r)))
                .collect(),
            content,
        )?;

        let fade = self.comp.text(
            &tr!("record.panel.fade"),
            style(12.0, false, MUTED),
            self.scale,
        )?;
        self.row_icon(
            &details,
            Glyph::Dissolve,
            PAD + SMALL_INSET,
            FADE_Y,
            SMALL_H,
        )?;
        self.place(
            &details,
            &fade.visual,
            PAD + SMALL_INSET + ROW_ICON_W,
            FADE_Y + (SMALL_H - fade.height) / 2.0,
        )?;
        let switch_x = WIDTH - PAD - SWITCH_W;
        let switch_y = FADE_Y + (SMALL_H - SWITCH_H) / 2.0;
        let switch_fill = self.comp.brush(color(SWITCH_OFF, 1.0))?;
        let track = self.pill(SWITCH_W, SWITCH_H, &switch_fill)?;
        self.place(&details, &track, switch_x, switch_y)?;
        let knob_fill = self.comp.brush(color(MUTED, 1.0))?;
        let knob = self.disc(KNOB, &knob_fill)?;
        self.place(&details, &knob, switch_x + 2.0, switch_y + 2.0)?;
        content.hits.push((
            Rect {
                x: PAD,
                y: FADE_Y,
                w: INNER,
                h: SMALL_H,
            },
            Hit::Panel(Press::Fade),
        ));

        let parts = MainParts {
            start: start.cast()?,
            rest: rest.cast()?,
            summary,
            sound,
            badges: [badges[0].clone(), badges[1].clone()],
            count_fill,
            count_icon: count_icon.cast()?,
            count_text,
            more_fill,
            details: details.cast()?,
            quality,
            codec,
            rate,
            switch_fill,
            knob: knob.cast()?,
            knob_fill,
            summary_of: String::new(),
            shown: None,
        };
        Ok(parts)
    }

    pub(super) fn update_panel(&self, model: &Model, content: &mut Content) -> Result<()> {
        // Unfolding "more" changes the height in place: the capsule follows it.
        (content.width, content.height) = self.panel_size(model);
        let (Some(panel), Some(parts)) = (&model.panel, &mut content.panel) else {
            return Ok(());
        };
        match parts {
            Parts::Main(parts) => self.update_main(panel, parts, &mut content.buttons),
            Parts::Microphones(parts) => self.update_microphones(panel, parts),
        }
    }

    fn update_main(
        &self,
        panel: &PanelModel,
        parts: &mut MainParts,
        buttons: &mut [Button],
    ) -> Result<()> {
        let s = self.scale;
        let choice = panel.choice;
        let animate = parts.shown.is_some();
        let sound = sound_index(choice.sound);
        self.select(&mut parts.sound, sound)?;
        for (badge, at) in parts.badges.iter().zip([2, 3]) {
            badge.SetOpacity(if at == sound { 1.0 } else { 0.0 })?;
        }
        let folded = summary(panel.weight_mb);
        if folded != parts.summary_of {
            let text = self.comp.fit(&folded, style(12.0, false, MUTED), TEXT_W);
            self.comp.set_text(&mut parts.summary, &text, s)?;
            parts.summary_of = folded;
        }
        self.select(
            &mut parts.quality,
            Quality::ALL
                .iter()
                .position(|&q| q == choice.quality)
                .unwrap_or(0),
        )?;
        self.select(
            &mut parts.codec,
            Codec::ALL
                .iter()
                .position(|&c| c == choice.codec)
                .unwrap_or(0),
        )?;
        self.select(
            &mut parts.rate,
            Rate::ALL
                .iter()
                .position(|&r| r == choice.rate)
                .unwrap_or(0),
        )?;

        let shown = Shown {
            seconds: choice.countdown.seconds(),
            more: panel.more,
            fade: choice.fade,
        };
        let before = parts.shown;
        parts.shown = Some(shown);
        if before.is_none_or(|b| b.seconds != shown.seconds) {
            self.show_countdown(parts, buttons, shown.seconds, animate)?;
        }
        if before.is_none_or(|b| b.more != shown.more) {
            self.show_more(parts, buttons, shown.more, animate)?;
        }
        if before.is_none_or(|b| b.fade != shown.fade) {
            self.show_fade(parts, shown.fade, animate)?;
        }
        Ok(())
    }

    fn paint(&self, brush: &CompositionColorBrush, to: Color, animate: bool) -> Result<()> {
        if animate {
            self.comp.animate_color(brush, to, SWITCH)
        } else {
            brush.SetColor(to)
        }
    }

    fn fade_to(&self, visual: &Visual, opacity: f32, animate: bool) -> Result<()> {
        if animate {
            self.comp.animate_scalar(visual, "Opacity", opacity, SWITCH)
        } else {
            visual.SetOpacity(opacity)
        }
    }

    fn show_countdown(
        &self,
        parts: &mut MainParts,
        buttons: &mut [Button],
        seconds: u32,
        animate: bool,
    ) -> Result<()> {
        let s = self.scale;
        let counting = seconds > 0;
        if counting {
            let label = tr!("record.panel.seconds", n = seconds);
            self.comp.set_text(&mut parts.count_text, &label, s)?;
            parts.count_text.visual.SetOffset(v3(
                snap((ROUND - parts.count_text.width) / 2.0, s),
                snap((ROUND - parts.count_text.height) / 2.0, s),
            ))?;
        }
        let (base, hover) = if counting {
            (color(TEXT, 1.0), color(ON_HOVER, 1.0))
        } else {
            (color(FILL, 1.0), color(FILL_HOVER, 1.0))
        };
        self.restyle(buttons, &Press::Countdown, base, hover);
        let under = self.hovered == Some(Hit::Panel(Press::Countdown));
        self.paint(&parts.count_fill, if under { hover } else { base }, animate)?;
        self.fade_to(&parts.count_icon, if counting { 0.0 } else { 1.0 }, animate)?;
        self.fade_to(
            &parts.count_text.visual.cast()?,
            if counting { 1.0 } else { 0.0 },
            animate,
        )
    }

    fn show_more(
        &self,
        parts: &mut MainParts,
        buttons: &mut [Button],
        more: bool,
        animate: bool,
    ) -> Result<()> {
        let (base, hover) = if more {
            (color(FILL_HOVER, 1.0), color(SWITCH_OFF, 1.0))
        } else {
            (color(FILL, 1.0), color(FILL_HOVER, 1.0))
        };
        self.restyle(buttons, &Press::More, base, hover);
        let under = self.hovered == Some(Hit::Panel(Press::More));
        self.paint(&parts.more_fill, if under { hover } else { base }, animate)?;
        // Like a view coming in anywhere on the island: shows over the capsule's resize.
        match (animate, more) {
            (false, _) => parts.details.SetOpacity(if more { 1.0 } else { 0.0 }),
            (true, true) => self
                .comp
                .animate_scalar(&parts.details, "Opacity", 1.0, FADE_IN),
            (true, false) => self
                .comp
                .animate_scalar(&parts.details, "Opacity", 0.0, FADE_OUT),
        }
    }

    fn show_fade(&self, parts: &mut MainParts, fade: bool, animate: bool) -> Result<()> {
        let s = self.scale;
        self.paint(
            &parts.switch_fill,
            color(if fade { TEXT } else { SWITCH_OFF }, 1.0),
            animate,
        )?;
        self.paint(
            &parts.knob_fill,
            color(if fade { DARK } else { MUTED }, 1.0),
            animate,
        )?;
        let knob_x = WIDTH - PAD - SWITCH_W + if fade { SWITCH_W - KNOB - 2.0 } else { 2.0 };
        let knob_y = FADE_Y + (SMALL_H - SWITCH_H) / 2.0 + 2.0;
        let at = v3(snap(knob_x, s), snap(knob_y, s));
        if animate {
            self.comp.animate_vector3(&parts.knob, "Offset", at, SLIDE)
        } else {
            parts.knob.SetOffset(at)
        }
    }

    /// Right after the panel replaced the control center: the record button flies into the
    /// start circle along a curve, up and to the left; the rest of the panel fades in.
    /// `from` — the center of the record button in the menu.
    pub fn fly_panel_in(&self, from: (f32, f32)) -> Result<()> {
        let Some(content) = &self.content else {
            return Ok(());
        };
        let Some(Parts::Main(parts)) = &content.panel else {
            return Ok(());
        };
        // The whole view faded in from nothing; the circle must be there from the start.
        self.comp.stop(&content.root, "Opacity")?;
        content.root.SetOpacity(1.0)?;
        self.comp
            .animate_scalar_from(&parts.rest, "Opacity", 0.0, 1.0, FADE_IN)?;

        let begin = (from.0 - START / 2.0, from.1 - START / 2.0);
        let end = START_AT;
        // Up first, then to the left: the control point sits above the start, level with the end.
        let control = (begin.0, end.1);
        let linear = Motion::linear(0);
        let keys: Vec<_> = (0..=FLY_STEPS)
            .map(|i| {
                let t = i as f32 / FLY_STEPS as f32;
                let e = 1.0 - (1.0 - t).powi(3);
                let point = |a: f32, b: f32, c: f32| {
                    (1.0 - e) * (1.0 - e) * a + 2.0 * (1.0 - e) * e * b + e * e * c
                };
                let x = point(begin.0, control.0, end.0);
                let y = point(begin.1, control.1, end.1);
                (t, v3(x, y), linear)
            })
            .collect();
        self.comp
            .path_vector3(&parts.start, "Offset", &keys, FLY_MS)?;
        let k = super::menu::CIRCLE / START;
        parts.start.SetScale(scale(k, k))?;
        self.comp.animate_vector3(
            &parts.start,
            "Scale",
            scale(1.0, 1.0),
            Motion::curve(FLY_MS, 0.22, 1.0, 0.36, 1.0),
        )
    }

    fn round(
        &self,
        parent: &ContainerVisual,
        x: f32,
        hex: &str,
    ) -> Result<(ContainerVisual, CompositionColorBrush)> {
        let body = self.comp.container()?;
        body.SetSize(v2(ROUND, ROUND))?;
        let fill = self.comp.brush(color(hex, 1.0))?;
        body.Children()?.InsertAtTop(&self.disc(ROUND, &fill)?)?;
        self.place(parent, &body, x, ROUND_Y)?;
        Ok((body, fill))
    }

    fn round_hit(
        &self,
        content: &mut Content,
        x: f32,
        fill: CompositionColorBrush,
        press: Press,
    ) -> Result<()> {
        let rect = Rect {
            x,
            y: ROUND_Y,
            w: ROUND,
            h: ROUND,
        };
        content.hits.push((rect, Hit::Panel(press.clone())));
        content.buttons.push(Button {
            fill,
            rect,
            hit: Hit::Panel(press),
            base: color(FILL, 1.0),
            hover: color(FILL_HOVER, 1.0),
        });
        Ok(())
    }

    /// A button's colors change with its state: hover then follows the new ones.
    fn restyle(&self, buttons: &mut [Button], press: &Press, base: Color, hover: Color) {
        let hit = Hit::Panel(press.clone());
        for button in buttons.iter_mut().filter(|b| b.hit == hit) {
            button.base = base;
            button.hover = hover;
        }
    }

    fn badge(&self, parent: &ContainerVisual, x: f32, y: f32) -> Result<Visual> {
        let holder = self.comp.container()?;
        holder.SetOpacity(0.0)?;
        holder
            .Children()?
            .InsertAtTop(&self.disc(BADGE, &self.comp.brush(color(DARK, 0.1))?)?)?;
        let icon = self
            .comp
            .glyph(Glyph::ChevronDown, BADGE_ICON, color(DARK, 1.0), self.scale)?;
        icon.SetOffset(v3((BADGE - BADGE_ICON) / 2.0, (BADGE - BADGE_ICON) / 2.0))?;
        holder.Children()?.InsertAtTop(&icon)?;
        self.place(parent, &holder, x, y)?;
        holder.cast()
    }

    pub(super) fn disc(&self, size: f32, brush: &CompositionColorBrush) -> Result<ShapeVisual> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(size, size))?;
        let geometry = c.CreateEllipseGeometry()?;
        geometry.SetCenter(v2(size / 2.0, size / 2.0))?;
        geometry.SetRadius(v2(size / 2.0, size / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(brush)?;
        visual.Shapes()?.Append(&shape)?;
        Ok(visual)
    }

    pub(super) fn pill(
        &self,
        w: f32,
        h: f32,
        brush: &CompositionColorBrush,
    ) -> Result<ShapeVisual> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(w, h))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(w, h))?;
        geometry.SetCornerRadius(v2(h / 2.0, h / 2.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(brush)?;
        visual.Shapes()?.Append(&shape)?;
        Ok(visual)
    }

    #[allow(clippy::too_many_arguments)]
    fn segments(
        &self,
        parent: &ContainerVisual,
        rect: Rect,
        inset: f32,
        icon: Option<Glyph>,
        items: &[(Option<Glyph>, String)],
        hits: Vec<Hit>,
        content: &mut Content,
    ) -> Result<Segments> {
        let back = self.pill(rect.w, rect.h, &self.comp.brush(color(FILL, 1.0))?)?;
        self.place(parent, &back, rect.x, rect.y)?;
        let lead = match icon {
            Some(glyph) => {
                self.row_icon(parent, glyph, rect.x + inset, rect.y, rect.h)?;
                ROW_ICON_W
            }
            None => 0.0,
        };
        let slot_w = (rect.w - 2.0 * inset - lead) / items.len() as f32;
        let inner_h = rect.h - 2.0 * inset;
        let pill = self.pill(slot_w, inner_h, &self.comp.brush(color(TEXT, 1.0))?)?;
        parent.Children()?.InsertAtTop(&pill)?;
        let label_size = if items.iter().any(|(g, _)| g.is_some()) {
            11.0
        } else {
            12.0
        };
        let mut slots = Vec::new();
        for (i, ((glyph, label), hit)) in items.iter().zip(hits).enumerate() {
            let x = rect.x + inset + lead + i as f32 * slot_w;
            let y = rect.y + inset;
            let layer = |hex: &str, strong: bool| -> Result<Visual> {
                let holder = self.comp.container()?;
                let st = style(label_size, strong, hex);
                let text =
                    self.comp
                        .text(&self.comp.fit(label, st, slot_w - 6.0), st, self.scale)?;
                let (icon_h, gap) = match glyph {
                    Some(_) => (SOUND_ICON, 2.0),
                    None => (0.0, 0.0),
                };
                let top = (inner_h - icon_h - gap - text.height) / 2.0;
                if let Some(glyph) = glyph {
                    let icon = self
                        .comp
                        .glyph(*glyph, SOUND_ICON, color(hex, 1.0), self.scale)?;
                    icon.SetOffset(v3(
                        snap((slot_w - SOUND_ICON) / 2.0, self.scale),
                        snap(top, self.scale),
                    ))?;
                    holder.Children()?.InsertAtTop(&icon)?;
                }
                self.place(
                    &holder,
                    &text.visual,
                    (slot_w - text.width) / 2.0,
                    top + icon_h + gap,
                )?;
                self.place(parent, &holder, x, y)?;
                holder.cast()
            };
            let off = layer(MUTED, false)?;
            let on = layer(DARK, true)?;
            on.SetOpacity(0.0)?;
            slots.push(Slot { x, off, on });
            content.hits.push((
                Rect {
                    x,
                    y: rect.y,
                    w: slot_w,
                    h: rect.h,
                },
                hit,
            ));
        }
        Ok(Segments {
            pill: pill.cast()?,
            y: rect.y + inset,
            slots,
            shown: None,
        })
    }

    /// What a row of the details is about, in grey at its left.
    fn row_icon(
        &self,
        parent: &ContainerVisual,
        glyph: Glyph,
        x: f32,
        y: f32,
        h: f32,
    ) -> Result<()> {
        let icon = self
            .comp
            .glyph(glyph, ROW_ICON, color(MUTED, 1.0), self.scale)?;
        self.place(
            parent,
            &icon,
            x + (ROW_ICON_W - ROW_ICON) / 2.0,
            y + (h - ROW_ICON) / 2.0,
        )
    }

    fn select(&self, segments: &mut Segments, index: usize) -> Result<()> {
        if segments.shown == Some(index) {
            return Ok(());
        }
        let Some(slot) = segments.slots.get(index) else {
            return Ok(());
        };
        let at = v3(snap(slot.x, self.scale), snap(segments.y, self.scale));
        let animate = segments.shown.is_some();
        if animate {
            self.comp
                .animate_vector3(&segments.pill, "Offset", at, SLIDE)?;
        } else {
            segments.pill.SetOffset(at)?;
        }
        for (i, slot) in segments.slots.iter().enumerate() {
            let (on, off) = if i == index { (1.0, 0.0) } else { (0.0, 1.0) };
            if animate {
                self.comp.animate_scalar(&slot.on, "Opacity", on, SWITCH)?;
                self.comp
                    .animate_scalar(&slot.off, "Opacity", off, SWITCH)?;
            } else {
                slot.on.SetOpacity(on)?;
                slot.off.SetOpacity(off)?;
            }
        }
        segments.shown = Some(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_keeps_the_control_center_width_and_grid() {
        assert_eq!(WIDTH, 318.0);
        assert_eq!(MAIN_H, 130.0);
        assert_eq!(MORE_H, 251.0);
        assert_eq!(TEXT_W, 98.0);
        assert_eq!(MORE_X + ROUND + PAD, WIDTH);
        const { assert!(super::super::microphones::TALLEST >= MORE_H) };
    }

    #[test]
    fn the_summary_is_the_weight() {
        crate::lang::with(crate::lang::Lang::Ru, || {
            assert_eq!(summary(13), "≈ 13 МБ/мин");
        });
    }
}
