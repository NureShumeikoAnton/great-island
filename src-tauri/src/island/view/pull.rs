//! Vertical pull: the control center opens downward and closes upward.
//!
//! While pulled (travel 60 px) everything follows the finger 1:1: the capsule size linearly by the travel share p,
//! the outgoing view shifts 25% of the travel and fades (1 − p/0.7), the incoming one appears ((p − 0.3)/0.7)
//! and slides into place from 12 px. The menu comes from the top, the activity view on closing from the bottom.
//!
//! After release: settle (content 320 ms, capsule 420 ms with overshoot) or return without a spring (260 ms).

use super::super::comp::{Motion, v2};
use super::{Content, FADE_IN, FADE_OUT, Model, Scene, content_key};
use crate::gesture::{PULL_TRAVEL, Pull};
use windows::core::{HSTRING, Result};

const COMMIT: Motion = Motion::curve(320, 0.22, 1.0, 0.36, 1.0);
const RETURN: Motion = Motion::curve(260, 0.22, 1.0, 0.36, 1.0);
const LEAVE_SHARE: f32 = 0.25;
const ARRIVE_FROM: f32 = 12.0;

fn sign(pull: Pull) -> f32 {
    match pull {
        Pull::Down => 1.0,
        Pull::Up => -1.0,
    }
}

fn set_y(content: &Content, y: f32) -> Result<()> {
    content.slide.InsertScalar(&HSTRING::from("Y"), y)
}

impl Scene {
    /// `offset` already includes rubber. `target` is the view that will arrive: the menu on opening
    /// or the activity on closing.
    pub fn pull(&mut self, pull: Pull, offset: f32, target: &Model) -> Result<()> {
        if self.content.is_none() {
            return Ok(());
        }
        self.dragging = true;
        self.hovered = None;
        self.prepare_pulled(target)?;
        let s = sign(pull);
        let p = (offset / PULL_TRAVEL).clamp(0.0, 1.0);
        let current = self.content.as_ref().expect("the island has a view");
        let next = self.pulled.as_ref().expect("the pulled view was built");
        for (content, y, opacity) in [
            (current, s * LEAVE_SHARE * offset, 1.0 - p / 0.7),
            (next, -s * ARRIVE_FROM * (1.0 - p), (p - 0.3) / 0.7),
        ] {
            self.comp.stop(&content.slide, "Y")?;
            self.comp.stop(&content.root, "Opacity")?;
            set_y(content, y)?;
            content.root.SetOpacity(opacity.clamp(0.0, 1.0))?;
        }
        self.comp.stop(&self.geometry, "Size")?;
        let (from, to) = ((current.width, current.height), (next.width, next.height));
        self.geometry.SetSize(v2(
            from.0 + (to.0 - from.0) * p,
            from.1 + (to.1 - from.1) * p,
        ))
    }

    /// `commit` = the arriving view becomes the island's view.
    pub fn settle_pull(&mut self, pull: Pull, commit: bool) -> Result<()> {
        self.dragging = false;
        let s = sign(pull);
        let Some(next) = self.pulled.take() else {
            return Ok(());
        };
        if commit {
            if let Some(old) = self.content.replace(next) {
                self.comp
                    .animate_scalar(&old.slide, "Y", s * LEAVE_SHARE * PULL_TRAVEL, COMMIT)?;
                self.comp
                    .animate_scalar(&old.root, "Opacity", 0.0, FADE_OUT)?;
                self.dying.push(old.root);
            }
            let current = self.content.as_ref().expect("the island has a view");
            self.comp.animate_scalar(&current.slide, "Y", 0.0, COMMIT)?;
            self.comp
                .animate_scalar(&current.root, "Opacity", 1.0, FADE_IN)?;
            self.hovered = None;
            return self.morph();
        }
        self.comp
            .animate_scalar(&next.slide, "Y", -s * ARRIVE_FROM, RETURN)?;
        self.comp
            .animate_scalar(&next.root, "Opacity", 0.0, FADE_OUT)?;
        self.dying.push(next.root);
        let Some(current) = &self.content else {
            return Ok(());
        };
        self.comp.animate_scalar(&current.slide, "Y", 0.0, RETURN)?;
        self.comp
            .animate_scalar(&current.root, "Opacity", 1.0, FADE_IN)?;
        self.comp.animate_vector2(
            &self.geometry,
            "Size",
            v2(current.width, current.height),
            RETURN,
        )
    }

    fn prepare_pulled(&mut self, model: &Model) -> Result<()> {
        let key = content_key(model);
        if self.pulled.as_ref().is_some_and(|c| c.key == key) {
            return Ok(());
        }
        if let Some(old) = self.pulled.take() {
            self.island.Children()?.Remove(&old.root)?;
        }
        let next = self.build(model, key)?;
        self.sync_content_rings(&next, model)?;
        next.root.SetOpacity(0.0)?;
        self.island.Children()?.InsertAtTop(&next.root)?;
        self.bind_offset(&next)?;
        self.pulled = Some(next);
        Ok(())
    }
}
