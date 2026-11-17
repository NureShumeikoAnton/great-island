//! System state on the island:
//! - **indicator**: a short 30 px capsule for 2 s when sound or microphone was toggled not from the island
//!   (key, tray): the icon and what happened;
//! - **mode badges**: a small moon and cup in the collapsed island while Do Not Disturb and Keep awake are on,
//!   and a circle with "!" while there is a problem not yet seen.

use super::super::glyph::Glyph;
use super::{Content, MUTED, Scene, TEXT, menu, snap, style};
use crate::control::ControlId;
use crate::lang::tr;
use std::borrow::Cow;
use windows::{UI::Composition::ContainerVisual, core::Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub quiet: bool,
    pub awake: bool,
    pub problem: bool,
}

impl Modes {
    fn glyphs(self) -> impl Iterator<Item = Glyph> {
        [
            (self.problem, Glyph::Problem),
            (self.quiet, Glyph::Moon),
            (self.awake, Glyph::Coffee),
        ]
        .into_iter()
        .filter_map(|(on, glyph)| on.then_some(glyph))
    }
}

const BADGE: f32 = 12.0;
const BADGE_GAP: f32 = 5.0;
const BADGES_AFTER: f32 = 10.0;

pub fn badges_width(modes: Modes) -> f32 {
    let n = modes.glyphs().count() as f32;
    if n == 0.0 {
        0.0
    } else {
        n * BADGE + (n - 1.0) * BADGE_GAP + BADGES_AFTER
    }
}

const INDICATOR_ICON: f32 = 16.0;
const INDICATOR_LEFT: f32 = 12.0;
const INDICATOR_GAP: f32 = 8.0;
const INDICATOR_RIGHT: f32 = 14.0;

fn indicator_text(id: ControlId, off: bool) -> (Cow<'static, str>, Glyph) {
    match (id, off) {
        (ControlId::Microphone, true) => (tr!("status.microphone_off"), Glyph::MicrophoneOff),
        (ControlId::Microphone, false) => (tr!("status.microphone_on"), Glyph::Microphone),
        (_, true) => (tr!("status.muted"), Glyph::Muted),
        (_, false) => (tr!("status.sound_on"), Glyph::Volume),
    }
}

impl Scene {
    pub(super) fn badges(&self, root: &ContainerVisual, x: f32, modes: Modes) -> Result<()> {
        for (i, glyph) in modes.glyphs().enumerate() {
            let icon = self
                .comp
                .glyph(glyph, BADGE, super::color(MUTED, 1.0), self.scale)?;
            self.place(
                root,
                &icon,
                x + i as f32 * (BADGE + BADGE_GAP),
                (30.0 - BADGE) / 2.0,
            )?;
        }
        Ok(())
    }

    pub(super) fn indicator_width(&self, id: ControlId, off: bool) -> f32 {
        let (text, _) = indicator_text(id, off);
        INDICATOR_LEFT
            + INDICATOR_ICON
            + INDICATOR_GAP
            + self.comp.measure(&text, style(13.0, true, TEXT))
            + INDICATOR_RIGHT
    }

    pub(super) fn build_indicator(
        &self,
        id: ControlId,
        off: bool,
        content: &mut Content,
    ) -> Result<()> {
        let root = content.root.clone();
        content.width = self.indicator_width(id, off).round();
        content.height = 30.0;
        let (text, glyph) = indicator_text(id, off);
        let tint = if off { menu::MUTE_ICON } else { TEXT };
        let icon = self
            .comp
            .glyph(glyph, INDICATOR_ICON, super::color(tint, 1.0), self.scale)?;
        self.place(&root, &icon, INDICATOR_LEFT, (30.0 - INDICATOR_ICON) / 2.0)?;
        let label = self.comp.text(&text, style(13.0, true, TEXT), self.scale)?;
        let x = INDICATOR_LEFT + INDICATOR_ICON + INDICATOR_GAP;
        self.place(&root, &label.visual, x, 0.0)?;
        label.visual.SetOffset(super::v3(
            snap(x, self.scale),
            snap((30.0 - label.height) / 2.0, self.scale),
        ))?;
        Ok(())
    }

    pub(super) fn discarded_width(&self) -> f32 {
        INDICATOR_LEFT
            + INDICATOR_ICON
            + INDICATOR_GAP
            + self.comp.measure(&discarded(), style(13.0, true, TEXT))
            + INDICATOR_RIGHT
    }

    pub(super) fn build_discarded(&self, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        content.width = self.discarded_width().round();
        content.height = 30.0;
        let icon = self.comp.glyph(
            Glyph::Trash,
            INDICATOR_ICON,
            super::color(MUTED, 1.0),
            self.scale,
        )?;
        self.place(&root, &icon, INDICATOR_LEFT, (30.0 - INDICATOR_ICON) / 2.0)?;
        let label = self
            .comp
            .text(&discarded(), style(13.0, true, TEXT), self.scale)?;
        let x = INDICATOR_LEFT + INDICATOR_ICON + INDICATOR_GAP;
        self.place(&root, &label.visual, x, 0.0)?;
        label.visual.SetOffset(super::v3(
            snap(x, self.scale),
            snap((30.0 - label.height) / 2.0, self.scale),
        ))?;
        Ok(())
    }
}

fn discarded() -> Cow<'static, str> {
    tr!("status.recording_deleted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_take_room_only_when_on() {
        assert_eq!(badges_width(Modes::default()), 0.0);
        let both = Modes {
            quiet: true,
            awake: true,
            problem: false,
        };
        assert_eq!(badges_width(both), 12.0 * 2.0 + 5.0 + 10.0);
    }
}
