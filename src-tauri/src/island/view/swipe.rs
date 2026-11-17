//! Swipe between activities. The finger (or cursor, touchpad) drives the view 1:1, the neighbouring activity
//! slides in behind it lagging by a page, and the capsule flows between their sizes. The old one fades in
//! the first half of the way, the new one appears in the second: they don't mix in the middle.
//!
//! After release: a transition (view 320 ms, capsule 420 ms with overshoot), a return without a spring (260 ms)
//! or a bounce off the ribbon's edge.

use super::super::comp::{Motion, v2};
use super::{Content, FADE_IN, FADE_OUT, MORPH, Model, Scene, content_key};
use crate::gesture::{Dir, PAGE, Settle};
use windows::core::{HSTRING, Result};

const COMMIT: Motion = Motion::curve(320, 0.22, 1.0, 0.36, 1.0);
const RETURN: Motion = Motion::curve(260, 0.22, 1.0, 0.36, 1.0);
const BOUNCE: Motion = MORPH;
const BUMP_PX: f32 = 12.0;
const BUMP_MS: u32 = 300;

fn entry(dir: Dir) -> f32 {
    match dir {
        Dir::Next => PAGE,
        Dir::Prev => -PAGE,
    }
}

fn set_slide(content: &Content, x: f32) -> Result<()> {
    content.slide.InsertScalar(&HSTRING::from("X"), x)
}

impl Scene {
    /// `offset` already includes rubber. `incoming` is the neighbour in the swipe direction and its model;
    /// `None` if there is none (rubber at the edge).
    pub fn drag(&mut self, offset: f32, incoming: Option<(Dir, &Model)>) -> Result<()> {
        if self.content.is_none() {
            return Ok(());
        }
        self.dragging = true;
        self.hovered = None;
        match incoming {
            Some((dir, model)) => self.prepare_incoming(dir, model)?,
            None => self.drop_incoming()?,
        }

        let current = self.content.as_ref().expect("the island has a view");
        self.comp.stop(&current.slide, "X")?;
        set_slide(current, offset)?;
        let from = v2(current.width, current.height);
        self.comp.stop(&self.geometry, "Size")?;
        match &self.incoming {
            Some((dir, next)) => {
                let p = (offset.abs() / PAGE).min(1.0);
                self.comp.stop(&next.slide, "X")?;
                set_slide(next, offset + entry(*dir))?;
                self.comp.stop(&current.root, "Opacity")?;
                self.comp.stop(&next.root, "Opacity")?;
                current.root.SetOpacity((1.0 - 2.0 * p).max(0.0))?;
                next.root.SetOpacity((2.0 * p - 1.0).max(0.0))?;
                let to = v2(next.width, next.height);
                self.geometry.SetSize(v2(
                    from.X + (to.X - from.X) * p,
                    from.Y + (to.Y - from.Y) * p,
                ))
            }
            None => {
                current.root.SetOpacity(1.0)?;
                self.geometry.SetSize(from)
            }
        }
    }

    /// The capsule's largest size during the gesture: the window must hold both the view and the neighbour
    /// (or the menu), or the window area clips the capsule into a rectangle.
    pub fn gesture_size(&self) -> Option<(f32, f32)> {
        let current = self.content.as_ref()?;
        let others = self.incoming.iter().map(|(_, c)| c).chain(&self.pulled);
        Some(others.fold((current.width, current.height), |(w, h), c| {
            (w.max(c.width), h.max(c.height))
        }))
    }

    pub fn settle(&mut self, settle: Settle) -> Result<()> {
        self.dragging = false;
        match settle {
            Settle::Commit(dir) if self.incoming.as_ref().is_some_and(|(d, _)| *d == dir) => {
                self.commit(dir)
            }
            Settle::Commit(_) | Settle::Cancel => self.cancel(RETURN),
            Settle::Bounce => self.cancel(BOUNCE),
        }
    }

    pub fn step(&mut self, dir: Dir, model: &Model) -> Result<()> {
        if self.content.is_none() {
            return Ok(());
        }
        self.drop_incoming()?;
        self.prepare_incoming(dir, model)?;
        self.commit(dir)
    }

    pub fn bump(&mut self, dir: Dir) -> Result<()> {
        let Some(current) = &self.content else {
            return Ok(());
        };
        let away = -entry(dir).signum() * BUMP_PX;
        self.comp.keyframes(
            &current.slide,
            "X",
            &[(0.0, 0.0), (0.35, away), (1.0, 0.0)],
            BUMP_MS,
        )
    }

    fn prepare_incoming(&mut self, dir: Dir, model: &Model) -> Result<()> {
        let key = content_key(model);
        if self
            .incoming
            .as_ref()
            .is_some_and(|(d, c)| *d == dir && c.key == key)
        {
            return Ok(());
        }
        self.drop_incoming()?;
        let next = self.build(model, key)?;
        self.sync_content_rings(&next, model)?;
        next.root.SetOpacity(0.0)?;
        set_slide(&next, entry(dir))?;
        self.island.Children()?.InsertAtTop(&next.root)?;
        self.bind_offset(&next)?;
        self.incoming = Some((dir, next));
        Ok(())
    }

    fn drop_incoming(&mut self) -> Result<()> {
        if let Some((_, old)) = self.incoming.take() {
            self.island.Children()?.Remove(&old.root)?;
        }
        Ok(())
    }

    fn commit(&mut self, dir: Dir) -> Result<()> {
        let (_, next) = self.incoming.take().expect("the neighbor view was built");
        if let Some(old) = self.content.replace(next) {
            self.comp
                .animate_scalar(&old.slide, "X", -entry(dir), COMMIT)?;
            self.comp
                .animate_scalar(&old.root, "Opacity", 0.0, FADE_OUT)?;
            self.dying.push(old.root);
        }
        let current = self.content.as_ref().expect("the island has a view");
        self.comp.animate_scalar(&current.slide, "X", 0.0, COMMIT)?;
        self.comp
            .animate_scalar(&current.root, "Opacity", 1.0, FADE_IN)?;
        self.hovered = None;
        self.morph()
    }

    fn cancel(&mut self, motion: Motion) -> Result<()> {
        let Some(current) = &self.content else {
            return Ok(());
        };
        self.comp.animate_scalar(&current.slide, "X", 0.0, motion)?;
        self.comp
            .animate_scalar(&current.root, "Opacity", 1.0, FADE_IN)?;
        self.comp.animate_vector2(
            &self.geometry,
            "Size",
            v2(current.width, current.height),
            motion,
        )?;
        if let Some((dir, next)) = self.incoming.take() {
            self.comp
                .animate_scalar(&next.slide, "X", entry(dir), RETURN)?;
            self.comp
                .animate_scalar(&next.root, "Opacity", 0.0, FADE_OUT)?;
            self.dying.push(next.root);
        }
        Ok(())
    }
}
