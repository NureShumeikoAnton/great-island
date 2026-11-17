//! The editor's bottom panel: ▶/❚❚ and the time of the result on top, the ruler and the timeline
//! under them. With the strip it lies on a dark band of its own; without it on a dark rounded
//! panel over the picture. Its height is fixed whatever the window's.
//!
//! - one block per fragment: thumbnails above, the sound as a wave of bars below, badges for the
//!   fragment's speed, zoom and muted sound; the fragment under the caret is outlined;
//! - the caret runs on without jumps: a moment of the result lies at its share of the width, and
//!   a join is a narrow empty gap the caret glides over in the last and first moments around it.
//!   With a transition each block gives up half of the overlap, its button sits in the gap;
//! - the caret (the playhead) runs over everything by the compositor; the scissors draw their own
//!   yellow line, and near the caret the caret itself lights up with the scissors on its head;
//! - a dragged block lifts and follows the cursor, the others move aside for it; while it is
//!   dragged a trash target shows above the panel.
//!
//! Blocks are drawn once per change with Direct2D (`comp.picture`), nothing per frame. The panel
//! decides nothing: it says what is under a point ([`Hit`]) and where a moment lies.

use super::super::super::comp::{Comp, Motion, Text, TextStyle, color, scale, v2, v3};
use super::super::super::glyph::{self, Glyph};
use super::super::super::log;
use super::super::capsule::{Capsule, Face, Panel};
use super::super::{MUTED, Metrics, STRIP, TEXT};
use super::pictures::Pictures;
use crate::lang::tr;
use crate::montage::{Fragment, Montage, strip};
use crate::play::rate_label;
use std::collections::HashMap;
use windows::{
    UI::Composition::{
        CompositionColorBrush, CompositionRoundedRectangleGeometry, ContainerVisual, ShapeVisual,
        SpriteVisual, Visual,
    },
    Win32::Graphics::{
        Direct2D::{
            Common::{
                D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
                D2D1_PIXEL_FORMAT,
            },
            D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR, ID2D1Bitmap1,
            ID2D1DeviceContext,
        },
        Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    },
    core::{Interface, Result},
};

const ROW_Y: f32 = 6.0;
const RULER_Y: f32 = 40.0;
pub const BLOCKS_Y: f32 = 56.0;
const BLOCK_H: f32 = 56.0;
const THUMB_H: f32 = 36.0;
pub const HEIGHT: f32 = BLOCKS_Y + BLOCK_H + 10.0;
/// Without the strip the panel keeps this far from the window's edges.
const FLOAT_MARGIN: f32 = 10.0;
const FLOAT_RADIUS: f32 = 14.0;
const PAD: f32 = 14.0;
/// The join's button and the reach of a press on it.
const JOIN: f32 = 14.0;
/// The empty gap between two blocks.
const GAP: f32 = 8.0;
/// The caret glides over a gap in this long before and after the join (less in a short block).
const GLIDE: f64 = 0.12;
const SHIMMER_W: f32 = 180.0;
const SHIMMER_MS: u32 = 1400;
const RADIUS: f32 = 8.0;
const TILE_GAP: f32 = 2.0;
const BAR_W: f32 = 3.0;
const BAR_STEP: f32 = 5.0;
/// A tick of the ruler at least this far from the next.
const TICK_ROOM: f32 = 64.0;
const TICK_STEPS: [f64; 10] = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
const CARET_TOP: f32 = RULER_Y - 4.0;
const CARET_BOTTOM: f32 = BLOCKS_Y + BLOCK_H + 4.0;
const HEAD: (f32, f32) = (24.0, 18.0);
const KNOB: f32 = 8.0;
const TRASH_H: f32 = 40.0;
const TRASH_GAP: f32 = 14.0;
const BADGE_H: f32 = 18.0;
const JOIN_BUTTON: f32 = 22.0;
const LIFT: f32 = 1.03;
/// The panel comes up from this far below when the editor opens.
const ARRIVE: f32 = 28.0;
const TILT: f32 = -1.2;

const BLOCK_BACK: &str = "#2C2C2E";
const WAVE_BACK: &str = "#232325";
const BAR: &str = "#8E8E93";
const MUTED_WAVE: f32 = 0.32;
const YELLOW: &str = "#FFD60A";
const RED_TEXT: &str = "#FF6961";
const JOIN_LINE: (&str, f32) = ("#FFFFFF", 0.16);
const JOIN_FILL: &str = "#3A3A3C";
const PANEL_ALPHA: f32 = 0.86;

const FADE: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);
/// A light spring: a small overshoot and back.
pub const SPRING: Motion = Motion::curve(380, 0.3, 1.25, 0.5, 1.0);
const FLY_MS: u32 = 340;
pub const RETIRE_MS: u32 = 420;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Play,
    Block(usize),
    Join(usize),
    Ruler,
}

#[derive(Clone, PartialEq)]
struct Drawn {
    width: i32,
    fragment: Fragment,
    thumbs: usize,
    peaks: bool,
}

struct Block {
    key: u64,
    body: ContainerVisual,
    content: Option<SpriteVisual>,
    badges: Option<ContainerVisual>,
    clip: CompositionRoundedRectangleGeometry,
    outline: ShapeVisual,
    outline_shape: CompositionRoundedRectangleGeometry,
    x: f32,
    w: f32,
    drawn: Option<Drawn>,
}

struct Join {
    root: ContainerVisual,
    button: ContainerVisual,
}

struct Caret {
    root: ContainerVisual,
    line: CompositionColorBrush,
    knob: ShapeVisual,
    head: ContainerVisual,
    lit: bool,
}

pub struct Timeline {
    pub root: ContainerVisual,
    m: Metrics,
    k: f32,
    back: ShapeVisual,
    back_shape: CompositionRoundedRectangleGeometry,
    play: Panel<Hit>,
    running: bool,
    time: Text,
    length: Text,
    hint: Text,
    ruler: ContainerVisual,
    shelf: ContainerVisual,
    blocks: Vec<Block>,
    joins: Vec<Join>,
    caret: Caret,
    scissors: SpriteVisual,
    trash: ContainerVisual,
    trash_size: (f32, f32),
    trash_hot: bool,
    retired: Vec<Visual>,
    /// The panel in the window: x, y, width.
    place: (f32, f32, f32),
    /// The layout of the montage shown: block x and width, seconds' width, result starts.
    xs: Vec<f32>,
    ws: Vec<f32>,
    per_second: f32,
    starts: Vec<f64>,
    lengths: Vec<f64>,
    duration: f64,
    /// The joins' middles.
    cs: Vec<f32>,
    /// Per join, the result moments over which the caret crosses its gap.
    ramps: Vec<(f64, f64)>,
    dragging: Option<usize>,
    slot: Option<usize>,
    /// The thumbnails are still coming: a glint runs over the blocks.
    shimmer: ContainerVisual,
    glint: SpriteVisual,
    loading: bool,
}

impl Timeline {
    /// The room the panel takes at the window's bottom.
    pub fn band(m: Metrics) -> f32 {
        if m.floating() {
            HEIGHT + FLOAT_MARGIN
        } else {
            HEIGHT
        }
    }

    pub fn new(comp: &Comp, k: f32, m: Metrics) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;
        let back = c.CreateShapeVisual()?;
        back.SetSize(v2(10_000.0, HEIGHT))?;
        let back_shape = c.CreateRoundedRectangleGeometry()?;
        let radius = if m.floating() { FLOAT_RADIUS } else { 0.0 };
        back_shape.SetCornerRadius(v2(radius, radius))?;
        let fill = c.CreateSpriteShapeWithGeometry(&back_shape)?;
        let alpha = if m.floating() { PANEL_ALPHA } else { 1.0 };
        fill.SetFillBrush(&comp.brush(color(STRIP, alpha))?)?;
        back.Shapes()?.Append(&fill)?;
        root.Children()?.InsertAtTop(&back)?;

        let mut play = Panel::new(comp)?;
        let mut button = play_capsule(comp, k, m, false)?;
        button.place(PAD, ROW_Y);
        play.add(Hit::Play, button)?;
        root.Children()?.InsertAtTop(&play.root)?;
        let time = comp.text("", time_style(TEXT), k)?;
        let length = comp.text("", time_style(MUTED), k)?;
        let hint = comp.text("", hint_style(), k)?;
        for t in [&time, &length, &hint] {
            root.Children()?.InsertAtTop(&t.visual)?;
        }
        let ruler = comp.container()?;
        root.Children()?.InsertAtTop(&ruler)?;
        let shelf = comp.container()?;
        root.Children()?.InsertAtTop(&shelf)?;

        let caret = make_caret(comp, k)?;
        root.Children()?.InsertAtTop(&caret.root)?;
        let scissors = c.CreateSpriteVisual()?;
        scissors.SetBrush(&comp.brush(color(YELLOW, 1.0))?)?;
        scissors.SetSize(v2(2.0, CARET_BOTTOM - CARET_TOP))?;
        scissors.SetOpacity(0.0)?;
        root.Children()?.InsertAtTop(&scissors)?;
        let (trash, trash_size) = make_trash(comp, k)?;
        root.Children()?.InsertAtTop(&trash)?;
        let shimmer = comp.container()?;
        shimmer.SetClip(&c.CreateInsetClip()?)?;
        shimmer.SetOffset(v3(PAD, BLOCKS_Y))?;
        shimmer.SetOpacity(0.0)?;
        let glint = c.CreateSpriteVisual()?;
        glint.SetBrush(&comp.glint(0.14)?)?;
        glint.SetSize(v2(SHIMMER_W, BLOCK_H))?;
        shimmer.Children()?.InsertAtTop(&glint)?;
        root.Children()?.InsertAbove(&shimmer, &shelf)?;

        Ok(Self {
            root,
            m,
            k,
            back,
            back_shape,
            play,
            running: false,
            time,
            length,
            hint,
            ruler,
            shelf,
            blocks: Vec::new(),
            joins: Vec::new(),
            caret,
            scissors,
            trash,
            trash_size,
            trash_hot: false,
            retired: Vec::new(),
            place: (0.0, 0.0, 0.0),
            xs: Vec::new(),
            ws: Vec::new(),
            per_second: 1.0,
            starts: Vec::new(),
            lengths: Vec::new(),
            duration: 0.0,
            cs: Vec::new(),
            ramps: Vec::new(),
            dragging: None,
            slot: None,
            shimmer,
            glint,
            loading: false,
        })
    }

    /// The panel for a window `size`; the blocks are laid out again by [`Timeline::sync`].
    pub fn resize(&mut self, comp: &Comp, size: (f32, f32)) -> Result<()> {
        let (x, w) = if self.m.floating() {
            (FLOAT_MARGIN, size.0 - 2.0 * FLOAT_MARGIN)
        } else {
            (0.0, size.0)
        };
        let y = size.1 - Self::band(self.m);
        self.place = (x, y, w.max(1.0));
        comp.stop(&self.root, "Offset")?;
        comp.stop(&self.root, "Opacity")?;
        self.root.SetOffset(v3(x, y))?;
        self.root.SetOpacity(1.0)?;
        self.back_shape.SetSize(v2(w.max(1.0), HEIGHT))?;
        let _ = &self.back;
        self.trash
            .SetOffset(v3((w - self.trash_size.0) / 2.0, -TRASH_GAP - TRASH_H))?;
        self.shimmer
            .SetSize(v2((w - 2.0 * PAD).max(1.0), BLOCK_H))?;
        if self.loading {
            self.run_shimmer(comp)?;
        }
        self.place_hint(comp)
    }

    /// While the thumbnails come a glint runs over the timeline, as over Save.
    pub fn set_loading(&mut self, comp: &Comp, on: bool) {
        if on == self.loading {
            return;
        }
        self.loading = on;
        log(comp.animate_scalar(&self.shimmer, "Opacity", if on { 1.0 } else { 0.0 }, FADE));
        if on {
            log(self.run_shimmer(comp));
        }
    }

    fn run_shimmer(&self, comp: &Comp) -> Result<()> {
        let room = (self.place.2 - 2.0 * PAD).max(1.0);
        let ease = Motion::curve(0, 0.45, 0.0, 0.55, 1.0);
        comp.loop_scalar(
            &self.glint,
            "Offset.X",
            &[(0.0, -SHIMMER_W, ease), (1.0, room, ease)],
            SHIMMER_MS,
        )
    }

    /// `p` in window coordinates to the panel's.
    pub fn local(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0 - self.place.0, p.1 - self.place.1)
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        let (x, y) = self.local(p);
        (0.0..=self.place.2).contains(&x) && (0.0..=HEIGHT).contains(&y)
    }

    fn inner(&self) -> (f32, f32) {
        (PAD, (self.place.2 - 2.0 * PAD).max(1.0))
    }

    pub fn set_running(&mut self, comp: &Comp, running: bool) -> Result<()> {
        if running == self.running {
            return Ok(());
        }
        self.running = running;
        let fresh = play_capsule(comp, self.k, self.m, running)?;
        self.play.replace(comp, Hit::Play, fresh)
    }

    pub fn set_times(&mut self, comp: &Comp, time: &str, length: &str) -> Result<()> {
        comp.set_text(&mut self.time, time, self.k)?;
        comp.set_text(&mut self.length, &format!("/ {length}"), self.k)?;
        let x = PAD + self.m.button.0 + 10.0;
        let y = snap(
            ROW_Y + self.m.button.1 / 2.0 - comp.cap_middle(time_style(TEXT)),
            self.k,
        );
        self.time.visual.SetOffset(v3(snap(x, self.k), y))?;
        self.length
            .visual
            .SetOffset(v3(snap(x + self.time.width + 6.0, self.k), y))
    }

    fn place_hint(&mut self, comp: &Comp) -> Result<()> {
        let left = PAD + self.m.button.0 + 10.0 + 150.0;
        let room = self.place.2 - PAD - left;
        let text = comp.fit(&tr!("editor.hint"), hint_style(), room.max(0.0));
        comp.set_text(&mut self.hint, &text, self.k)?;
        let x = self.place.2 - PAD - self.hint.width;
        let y = ROW_Y + self.m.button.1 / 2.0 - comp.cap_middle(hint_style());
        self.hint
            .visual
            .SetOffset(v3(snap(x, self.k), snap(y, self.k)))
    }

    /// Lays the montage out again. `motion`: the blocks glide to their places (a spring), or
    /// jump. A block that is new shows up where its part of a former block was.
    pub fn sync(
        &mut self,
        comp: &Comp,
        montage: &Montage,
        pictures: &Pictures,
        current: usize,
        motion: Option<Motion>,
    ) -> Result<()> {
        let fragments = montage.fragments();
        let (left, room) = self.inner();
        let lengths: Vec<f64> = fragments.iter().map(Fragment::length).collect();
        let old_per_second = self.per_second;
        self.duration = montage.duration();
        let gaps = GAP * montage.joins().len() as f32;
        self.per_second = ((room - gaps).max(1.0) / self.duration.max(0.001) as f32).max(0.0001);
        self.starts = montage.starts();
        self.lengths = lengths;
        self.xs.clear();
        self.ws.clear();
        // A transition's overlap is cut at its middle: each block keeps its own half, so no block lies over another
        // and the join's button sits in the gap between them.
        let trims: Vec<(f64, f64)> = (0..fragments.len())
            .map(|i| {
                let before = i.checked_sub(1).map_or(0.0, |j| montage.overlap(j) / 2.0);
                (before, montage.overlap(i) / 2.0)
            })
            .collect();
        for (i, (start, len)) in self.starts.iter().zip(&self.lengths).enumerate() {
            let (before, after) = trims[i];
            self.xs
                .push(left + (*start + before) as f32 * self.per_second + i as f32 * GAP);
            self.ws
                .push(((*len - before - after) as f32 * self.per_second).max(2.0));
        }
        // A transition's gap is crossed over its overlap, a plain one around the join.
        self.ramps = (0..montage.joins().len())
            .map(|j| {
                let t = self.starts[j + 1];
                match montage.overlap(j) {
                    o if o > 0.0 => (t, t + o),
                    _ => {
                        let e = GLIDE
                            .min(self.lengths[j] / 4.0)
                            .min(self.lengths[j + 1] / 4.0);
                        (t - e, t + e)
                    }
                }
            })
            .collect();
        // A join's middle: the middle of the gap.
        self.cs = (1..self.xs.len())
            .map(|i| (self.xs[i] + self.xs[i - 1] + self.ws[i - 1]) / 2.0)
            .collect();

        let mut old: Vec<Block> = std::mem::take(&mut self.blocks);
        let olds: Vec<(f32, Fragment)> = old
            .iter()
            .filter_map(|b| b.drawn.as_ref().map(|d| (b.x, d.fragment)))
            .collect();
        for (i, f) in fragments.iter().enumerate() {
            let key = f.from.to_bits();
            let (x, w) = (self.xs[i], self.ws[i]);
            let mut block = match old.iter().position(|b| b.key == key) {
                Some(at) => old.remove(at),
                None => {
                    let block = make_block(comp, key)?;
                    // Where this piece was inside a block it came out of (a cut).
                    let born = olds
                        .iter()
                        .find(|(_, o)| o.from < f.from && f.from < o.to)
                        .map(|(ox, o)| ox + ((f.from - o.from) / o.speed) as f32 * old_per_second);
                    block.body.SetOffset(v3(born.unwrap_or(x), BLOCKS_Y))?;
                    if born.is_none() {
                        block.body.SetOpacity(0.0)?;
                        comp.animate_scalar(&block.body, "Opacity", 1.0, FADE)?;
                    }
                    self.shelf.Children()?.InsertAtTop(&block.body)?;
                    block
                }
            };
            block.x = x;
            block.w = w;
            let drawn_w = w;
            block.body.SetSize(v2(drawn_w, BLOCK_H))?;
            block
                .body
                .SetCenterPoint(v3(drawn_w / 2.0, BLOCK_H / 2.0))?;
            block.clip.SetSize(v2(drawn_w, BLOCK_H))?;
            block
                .outline_shape
                .SetSize(v2(drawn_w - 2.0, BLOCK_H - 2.0))?;
            block
                .outline
                .SetOpacity(if i == current { 1.0 } else { 0.0 })?;
            match motion {
                Some(motion) => {
                    comp.animate_vector3(&block.body, "Offset", v3(x, BLOCKS_Y), motion)?
                }
                None => {
                    comp.stop(&block.body, "Offset")?;
                    block.body.SetOffset(v3(x, BLOCKS_Y))?;
                }
            }
            // What the block shows: the fragment less the halves given to its transitions.
            let (before, after) = trims[i];
            let shown = Fragment {
                from: f.from + before * f.speed,
                to: (f.to - after * f.speed).max(f.from + before * f.speed),
                ..*f
            };
            let drawn = Drawn {
                width: (w * self.k).round() as i32,
                fragment: shown,
                thumbs: pictures.thumbs.as_ref().map_or(0, |t| t.pixels.len()),
                peaks: pictures.peaks.is_some(),
            };
            if block.drawn.as_ref() != Some(&drawn) {
                self.draw_block(comp, &mut block, &shown, pictures)?;
                block.drawn = Some(drawn);
            }
            self.blocks.push(block);
        }
        for gone in old {
            let _ = self.shelf.Children()?.Remove(&gone.body);
        }
        // The blocks stacked in order; the joins' buttons go over them below.
        let children = self.shelf.Children()?;
        for block in &self.blocks {
            children.Remove(&block.body)?;
            children.InsertAtTop(&block.body)?;
        }
        self.dragging = None;
        self.slot = None;
        self.sync_joins(comp, montage, motion)?;
        self.draw_ruler(comp)
    }

    pub fn set_current(&self, current: usize) {
        for (i, block) in self.blocks.iter().enumerate() {
            log(block
                .outline
                .SetOpacity(if i == current { 1.0 } else { 0.0 }));
        }
    }

    fn draw_block(
        &self,
        comp: &Comp,
        block: &mut Block,
        f: &Fragment,
        pictures: &Pictures,
    ) -> Result<()> {
        let w = block.w;
        let per_second = self.per_second;
        let picture = comp.picture(w, BLOCK_H, self.k, |dc, _| {
            draw_strip(dc, w, per_second, f, pictures)
        })?;
        let children = block.body.Children()?;
        if let Some(old) = block.content.take() {
            children.Remove(&old)?;
        }
        children.InsertAtBottom(&picture)?;
        if let Some(old) = block.badges.take() {
            children.Remove(&old)?;
        }
        let badges = make_badges(comp, self.k, f)?;
        children.InsertAbove(&badges, &picture)?;
        block.content = Some(picture);
        block.badges = Some(badges);
        Ok(())
    }

    fn sync_joins(&mut self, comp: &Comp, montage: &Montage, motion: Option<Motion>) -> Result<()> {
        let count = montage.joins().len();
        while self.joins.len() > count {
            if let Some(join) = self.joins.pop() {
                let _ = self.shelf.Children()?.Remove(&join.root);
            }
        }
        while self.joins.len() < count {
            let join = make_join(comp, self.k)?;
            if let Some(c) = self.cs.get(self.joins.len()) {
                join.root.SetOffset(v3(c - JOIN / 2.0, BLOCKS_Y))?;
            }
            self.joins.push(join);
        }
        // Over the blocks: a button is wider than the gap.
        for join in &self.joins {
            let children = self.shelf.Children()?;
            let _ = children.Remove(&join.root);
            children.InsertAtTop(&join.root)?;
        }
        for (i, (join, transition)) in self.joins.iter().zip(montage.joins()).enumerate() {
            let x = self.cs[i] - JOIN / 2.0;
            match motion {
                Some(motion) => {
                    comp.animate_vector3(&join.root, "Offset", v3(x, BLOCKS_Y), motion)?
                }
                None => {
                    comp.stop(&join.root, "Offset")?;
                    join.root.SetOffset(v3(x, BLOCKS_Y))?;
                }
            }
            join.button
                .SetOpacity(if transition.is_some() { 1.0 } else { 0.0 })?;
        }
        Ok(())
    }

    /// The transition tool is being dragged: every join shows where it can go, the one under
    /// the chip (`hot`) larger. `None` = the drag is over.
    pub fn targets(&self, comp: &Comp, hot: Option<Option<usize>>, montage: &Montage) {
        for (i, (j, t)) in self.joins.iter().zip(montage.joins()).enumerate() {
            let (opacity, k) = match hot {
                None if t.is_some() => (1.0, 1.0),
                None => (0.0, 1.0),
                Some(hot) if hot == Some(i) => (1.0, 1.3),
                Some(_) => (0.55, 1.0),
            };
            log(comp.animate_scalar(&j.button, "Opacity", opacity, FADE));
            log(comp.animate_vector3(&j.button, "Scale", scale(k, k), SPRING));
        }
    }

    /// The join a chip at `p` (panel coordinates) would land on: the nearest gap within reach.
    pub fn join_near(&self, p: (f32, f32)) -> Option<usize> {
        const REACH: f32 = 24.0;
        if p.1 < BLOCKS_Y - REACH || p.1 > BLOCKS_Y + BLOCK_H + REACH {
            return None;
        }
        (0..self.joins.len())
            .map(|i| (i, (self.cs[i] - p.0).abs()))
            .filter(|(_, d)| *d <= REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The panel comes up from below and shows (the editor opens).
    pub fn arrive(&self, comp: &Comp, motion: Motion) {
        let (x, y, _) = self.place;
        log(self.root.SetOffset(v3(x, y + ARRIVE)));
        log(self.root.SetOpacity(0.0));
        log(comp.animate_vector3(&self.root, "Offset", v3(x, y), motion));
        log(comp.animate_scalar(&self.root, "Opacity", 1.0, motion));
    }

    /// The panel sinks and fades (the editor closes, its window goes full screen and back).
    pub fn depart(&self, comp: &Comp, motion: Motion) {
        let (x, y, _) = self.place;
        log(comp.animate_vector3(&self.root, "Offset", v3(x, y + ARRIVE), motion));
        log(comp.animate_scalar(&self.root, "Opacity", 0.0, motion));
    }

    /// A plain join shows its button faintly under the cursor: it can be given a transition.
    pub fn hover_join(&self, join: Option<usize>, montage: &Montage) {
        for (i, (j, t)) in self.joins.iter().zip(montage.joins()).enumerate() {
            let opacity = match (t, join == Some(i)) {
                (Some(_), _) => 1.0,
                (None, true) => 0.55,
                (None, false) => 0.0,
            };
            log(j.button.SetOpacity(opacity));
        }
    }

    fn draw_ruler(&mut self, comp: &Comp) -> Result<()> {
        self.ruler.Children()?.RemoveAll()?;
        let per_second = f64::from(self.per_second);
        let step = TICK_STEPS
            .iter()
            .copied()
            .find(|s| s * per_second >= f64::from(TICK_ROOM))
            .unwrap_or(600.0);
        let style = TextStyle {
            size: 10.0,
            bold: false,
            color: color(MUTED, 1.0),
            tabular: true,
        };
        let end = self.place.2 - PAD;
        let mut t = 0.0;
        while t <= self.duration + 1e-6 {
            let x = self.x_of(t);
            let label = comp.text(&crate::play::clock(t, self.duration), style, self.k)?;
            if x + label.width > end + PAD / 2.0 {
                break;
            }
            label.visual.SetOffset(v3(snap(x, self.k), RULER_Y - 2.0))?;
            self.ruler.Children()?.InsertAtTop(&label.visual)?;
            t += step;
        }
        Ok(())
    }

    /// Where a result moment lies on the timeline (panel coordinates): its share of the width,
    /// plus the gaps crossed by then.
    pub fn x_of(&self, result: f64) -> f32 {
        let r = result.clamp(0.0, self.duration);
        let crossed: f64 = self
            .ramps
            .iter()
            .map(|&(a, b)| {
                if b > a {
                    ((r - a) / (b - a)).clamp(0.0, 1.0)
                } else if r >= a {
                    1.0
                } else {
                    0.0
                }
            })
            .sum();
        PAD + r as f32 * self.per_second + crossed as f32 * GAP
    }

    /// The result moment under `x` (panel coordinates); in a gap, the moment the caret is there.
    pub fn result_at(&self, x: f32) -> f64 {
        let (mut lo, mut hi) = (0.0, self.duration);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if self.x_of(mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }

    /// The caret's path from `from` to `to` (result seconds): straight between the moments its
    /// pace changes, as (share of the time, x).
    pub fn caret_path(&self, from: f64, to: f64) -> Vec<(f32, f32)> {
        let mut moments = vec![from];
        for &(a, b) in &self.ramps {
            for t in [a, b] {
                if t > from && t < to {
                    moments.push(t);
                }
            }
        }
        moments.push(to);
        moments.sort_by(f64::total_cmp);
        let span = (to - from).max(1e-9);
        moments
            .into_iter()
            .map(|t| (((t - from) / span) as f32, self.x_of(t)))
            .collect()
    }

    /// `p` in panel coordinates.
    pub fn hit(&self, p: (f32, f32)) -> Option<Hit> {
        if self.play.at(p).is_some() {
            return Some(Hit::Play);
        }
        let (x, y) = p;
        if (BLOCKS_Y..=BLOCKS_Y + BLOCK_H).contains(&y) {
            if let Some(j) = self.cs.iter().position(|c| (x - c).abs() <= JOIN / 2.0) {
                return Some(Hit::Join(j));
            }
            // Neighbours may touch by a pixel: the later one wins.
            if let Some(i) = (0..self.xs.len())
                .rev()
                .find(|&i| x >= self.xs[i] && x <= self.xs[i] + self.ws[i])
            {
                return Some(Hit::Block(i));
            }
        }
        let (left, room) = self.inner();
        let on_ruler = (CARET_TOP..BLOCKS_Y).contains(&y);
        (on_ruler && x >= left - 4.0 && x <= left + room + 4.0).then_some(Hit::Ruler)
    }

    /// The join's middle in window coordinates (where its popover points).
    pub fn join_anchor(&self, join: usize) -> (f32, f32) {
        let x = self.cs.get(join).copied().unwrap_or(0.0);
        (self.place.0 + x, self.place.1 + BLOCKS_Y)
    }

    pub fn hover(&mut self, comp: &Comp, p: Option<(f32, f32)>) {
        self.play.hover(comp, p);
    }

    pub fn press_play(&mut self, comp: &Comp, p: (f32, f32)) -> bool {
        self.play.press(comp, p)
    }

    pub fn release_play(&mut self, comp: &Comp, p: (f32, f32)) -> bool {
        self.play.release(comp, p).is_some()
    }

    pub fn cancel(&mut self, comp: &Comp) {
        self.play.cancel(comp);
    }

    /// The caret at `x` (panel coordinates), running along `path` (from [`Timeline::caret_path`])
    /// in `ms` while playing.
    pub fn caret(&self, comp: &Comp, x: f32, run: Option<(Vec<(f32, f32)>, u32)>) {
        let at = v3(x - 1.0, CARET_TOP);
        log(comp.stop(&self.caret.root, "Offset"));
        log(self.caret.root.SetOffset(at));
        if let Some((path, ms)) = run {
            let keys: Vec<_> = path
                .into_iter()
                .map(|(f, x)| (f, v3(x - 1.0, CARET_TOP), Motion::linear(0)))
                .collect();
            log(comp.path_vector3(&self.caret.root, "Offset", &keys, ms));
        }
    }

    /// The scissors' line at `x`; `snapped` lights the caret instead.
    pub fn scissors(&mut self, comp: &Comp, at: Option<f32>, snapped: bool) {
        match at {
            Some(x) if !snapped => {
                log(self.scissors.SetOffset(v3(x - 1.0, CARET_TOP)));
                log(self.scissors.SetOpacity(1.0));
            }
            _ => log(self.scissors.SetOpacity(0.0)),
        }
        self.light(comp, snapped && at.is_some());
    }

    fn light(&mut self, comp: &Comp, on: bool) {
        if on == self.caret.lit {
            return;
        }
        self.caret.lit = on;
        let ink = if on { YELLOW } else { TEXT };
        log(comp.animate_color(&self.caret.line, color(ink, 1.0), FADE));
        log(comp.animate_scalar(
            &self.caret.knob,
            "Opacity",
            if on { 0.0 } else { 1.0 },
            FADE,
        ));
        log(comp.animate_scalar(
            &self.caret.head,
            "Opacity",
            if on { 1.0 } else { 0.0 },
            FADE,
        ));
        let k = if on { 1.0 } else { 0.6 };
        log(comp.animate_vector3(&self.caret.head, "Scale", scale(k, k), SPRING));
    }

    /// The cut: the scissors roll down the line at `x`, then the two edges part (the next sync).
    pub fn cut_at(&mut self, comp: &Comp, x: f32) -> Result<()> {
        let rolling = comp.glyph(Glyph::Scissors, 16.0, color(YELLOW, 1.0), self.k)?;
        rolling.SetCenterPoint(v3(8.0, 8.0))?;
        rolling.SetRotationAngleInDegrees(90.0)?;
        rolling.SetOffset(v3(x - 8.0, CARET_TOP - 8.0))?;
        self.shelf.Children()?.InsertAtTop(&rolling)?;
        let roll = Motion::curve(260, 0.4, 0.0, 0.2, 1.0);
        comp.animate_vector3(&rolling, "Offset", v3(x - 8.0, CARET_BOTTOM - 8.0), roll)?;
        comp.animate_scalar_after(&rolling, "Opacity", 0.0, FADE, 200)?;
        self.retired.push(rolling.cast()?);
        Ok(())
    }

    pub fn lift(&mut self, comp: &Comp, index: usize) {
        let Some(block) = self.blocks.get(index) else {
            return;
        };
        self.dragging = Some(index);
        self.slot = Some(index);
        if let Ok(children) = self.shelf.Children() {
            let _ = children.Remove(&block.body);
            let _ = children.InsertAtTop(&block.body);
        }
        log(comp.animate_vector3(&block.body, "Scale", scale(LIFT, LIFT), FADE));
        log(comp.animate_scalar(&block.body, "RotationAngleInDegrees", TILT, FADE));
        log(block.outline.SetOpacity(1.0));
        log(comp.animate_scalar(&self.trash, "Opacity", 1.0, FADE));
    }

    /// The dragged block's left edge follows the pointer at `x`, `y` (panel coordinates); the
    /// others make room at the place it would land. Returns that place among the others.
    pub fn drag(&mut self, comp: &Comp, x: f32, y: f32) -> usize {
        let Some(index) = self.dragging else {
            return 0;
        };
        let w = self.ws[index];
        if let Some(block) = self.blocks.get(index) {
            log(comp.stop(&block.body, "Offset"));
            log(block.body.SetOffset(v3(x, y)));
        }
        // The others laid end to end without the dragged one: the slot is among their middles.
        let center = x + w / 2.0;
        let (left, _) = self.inner();
        let others: Vec<usize> = (0..self.blocks.len()).filter(|&i| i != index).collect();
        let mut end = left;
        let mut slot = 0;
        for &i in &others {
            if end + self.ws[i] / 2.0 < center {
                slot += 1;
            }
            end += self.ws[i] + GAP;
        }
        if self.slot != Some(slot) {
            self.slot = Some(slot);
            let mut at = left;
            for (placed, &i) in others.iter().enumerate() {
                if placed == slot {
                    at += w + GAP;
                }
                let to = v3(at, BLOCKS_Y);
                log(comp.animate_vector3(&self.blocks[i].body, "Offset", to, SPRING));
                at += self.ws[i] + GAP;
            }
            for join in &self.joins {
                log(comp.animate_scalar(&join.root, "Opacity", 0.0, FADE));
            }
        }
        let hot = self.over_trash((x, x + w), y + BLOCK_H / 2.0);
        if hot != self.trash_hot {
            self.trash_hot = hot;
            let k = if hot { 1.08 } else { 1.0 };
            log(comp.animate_vector3(&self.trash, "Scale", scale(k, k), SPRING));
        }
        slot
    }

    /// A block spanning `x.0..x.1`, at height `y`, in panel coordinates: any overlap with the target counts, so a
    /// block wider than the target is not carried by its middle.
    pub fn over_trash(&self, x: (f32, f32), y: f32) -> bool {
        let (w, h) = self.trash_size;
        let x0 = (self.place.2 - w) / 2.0;
        let y0 = -TRASH_GAP - TRASH_H;
        x.1 >= x0 - 12.0 && x.0 <= x0 + w + 12.0 && y >= y0 - 12.0 && y <= y0 + h + 12.0
    }

    /// The drag is over: the block settles (the next sync moves it home).
    pub fn land(&mut self, comp: &Comp) {
        if let Some(block) = self.dragging.and_then(|i| self.blocks.get(i)) {
            log(comp.animate_vector3(&block.body, "Scale", scale(1.0, 1.0), SPRING));
            log(comp.animate_scalar(&block.body, "RotationAngleInDegrees", 0.0, FADE));
        }
        for join in &self.joins {
            log(comp.animate_scalar(&join.root, "Opacity", 1.0, FADE));
        }
        self.hide_trash(comp);
    }

    pub fn hide_trash(&mut self, comp: &Comp) {
        self.trash_hot = false;
        log(comp.animate_scalar(&self.trash, "Opacity", 0.0, FADE));
        log(comp.animate_vector3(&self.trash, "Scale", scale(1.0, 1.0), FADE));
    }

    /// Block `index` flies into the trash target and leaves the timeline; it is removed from the
    /// tree by [`Timeline::clear_retired`].
    pub fn fly_away(&mut self, comp: &Comp, index: usize) {
        if index >= self.blocks.len() {
            return;
        }
        let block = self.blocks.remove(index);
        self.xs.remove(index);
        self.ws.remove(index);
        let (tw, _) = self.trash_size;
        let target = v3(
            (self.place.2 - tw) / 2.0 + tw / 2.0 - block.w / 2.0,
            -TRASH_GAP - TRASH_H / 2.0 - BLOCK_H / 2.0,
        );
        let fly = Motion::curve(FLY_MS, 0.5, 0.0, 0.3, 1.0);
        log(comp.animate_scalar(&self.trash, "Opacity", 1.0, FADE));
        log(comp.animate_vector3(&block.body, "Offset", target, fly));
        log(comp.animate_vector3(&block.body, "Scale", scale(0.18, 0.18), fly));
        log(comp.animate_scalar_after(&block.body, "Opacity", 0.0, FADE, FLY_MS - 120));
        if let Ok(body) = block.body.cast() {
            self.retired.push(body);
        }
        self.dragging = None;
        self.trash_hot = false;
    }

    /// Visuals that finished leaving go from the tree; the trash target hides.
    pub fn clear_retired(&mut self, comp: &Comp) {
        for body in self.retired.drain(..) {
            if let Ok(children) = self.shelf.Children() {
                let _ = children.Remove(&body);
            }
        }
        if self.dragging.is_none() {
            self.hide_trash(comp);
        }
    }
}

fn snap(v: f32, k: f32) -> f32 {
    (v * k).round() / k
}

fn time_style(ink: &str) -> TextStyle {
    TextStyle {
        size: 13.0,
        bold: false,
        color: color(ink, 1.0),
        tabular: true,
    }
}

fn hint_style() -> TextStyle {
    TextStyle {
        size: 12.0,
        bold: false,
        color: color(MUTED, 1.0),
        tabular: false,
    }
}

fn play_capsule(comp: &Comp, k: f32, m: Metrics, running: bool) -> Result<Capsule> {
    let glyph = if running { Glyph::Pause } else { Glyph::Play };
    Capsule::new(
        comp,
        k,
        m.button,
        Face::Glyph(glyph, m.icon, color(TEXT, 1.0)),
        m.plain(),
    )
}

fn make_block(comp: &Comp, key: u64) -> Result<Block> {
    let c = &comp.compositor;
    let body = comp.container()?;
    let clip = c.CreateRoundedRectangleGeometry()?;
    clip.SetCornerRadius(v2(RADIUS, RADIUS))?;
    body.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
    let outline = c.CreateShapeVisual()?;
    outline.SetSize(v2(10_000.0, BLOCK_H))?;
    let outline_shape = c.CreateRoundedRectangleGeometry()?;
    outline_shape.SetOffset(v2(1.0, 1.0))?;
    outline_shape.SetCornerRadius(v2(RADIUS - 1.0, RADIUS - 1.0))?;
    let stroke = c.CreateSpriteShapeWithGeometry(&outline_shape)?;
    stroke.SetStrokeBrush(&comp.brush(color(TEXT, 1.0))?)?;
    stroke.SetStrokeThickness(2.0)?;
    outline.Shapes()?.Append(&stroke)?;
    outline.SetOpacity(0.0)?;
    body.Children()?.InsertAtTop(&outline)?;
    Ok(Block {
        key,
        body,
        content: None,
        badges: None,
        clip,
        outline,
        outline_shape,
        x: 0.0,
        w: 0.0,
        drawn: None,
    })
}

fn draw_strip(
    dc: &ID2D1DeviceContext,
    w: f32,
    per_second: f32,
    f: &Fragment,
    pictures: &Pictures,
) -> Result<()> {
    unsafe {
        let solid = |hex: &str, alpha: f32| {
            let c = color(hex, alpha);
            dc.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: f32::from(c.R) / 255.0,
                    g: f32::from(c.G) / 255.0,
                    b: f32::from(c.B) / 255.0,
                    a: alpha,
                },
                None,
            )
        };
        let rect = |l: f32, t: f32, r: f32, b: f32| D2D_RECT_F {
            left: l,
            top: t,
            right: r,
            bottom: b,
        };
        dc.FillRectangle(&rect(0.0, 0.0, w, THUMB_H), &solid(BLOCK_BACK, 1.0)?);
        dc.FillRectangle(&rect(0.0, THUMB_H, w, BLOCK_H), &solid(WAVE_BACK, 1.0)?);
        if let Some(thumbs) = &pictures.thumbs {
            let (tw, th) = thumbs.size;
            let tile = THUMB_H * tw as f32 / th.max(1) as f32;
            let mut made: HashMap<usize, ID2D1Bitmap1> = HashMap::new();
            let mut x = 0.0;
            while x < w {
                let middle = f64::from((x + tile / 2.0).min(w) / per_second);
                let source = f.from + middle * f.speed;
                if let Some(i) = thumbs.near(source.min(f.to)) {
                    let bitmap = match made.get(&i) {
                        Some(b) => b.clone(),
                        None => {
                            let b = dc.CreateBitmap(
                                D2D_SIZE_U {
                                    width: tw,
                                    height: th,
                                },
                                Some(thumbs.pixels[i].as_ptr() as *const _),
                                tw * 4,
                                &D2D1_BITMAP_PROPERTIES1 {
                                    pixelFormat: D2D1_PIXEL_FORMAT {
                                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                                    },
                                    dpiX: 96.0,
                                    dpiY: 96.0,
                                    ..Default::default()
                                },
                            )?;
                            made.insert(i, b.clone());
                            b
                        }
                    };
                    dc.DrawBitmap(
                        &bitmap,
                        Some(&rect(x, 0.0, x + tile, THUMB_H)),
                        1.0,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    );
                }
                x += tile + TILE_GAP;
            }
        }
        if let Some(peaks) = &pictures.peaks {
            let count = ((w - 2.0 * 4.0) / BAR_STEP).floor().max(0.0) as usize;
            let alpha = if f.muted { MUTED_WAVE } else { 1.0 };
            let ink = solid(BAR, alpha)?;
            let room = BLOCK_H - THUMB_H - 6.0;
            let middle = THUMB_H + (BLOCK_H - THUMB_H) / 2.0;
            for (i, v) in strip::bars(peaks, f.from, f.to, count).iter().enumerate() {
                let h = 2.0 + v.clamp(0.0, 1.0) * (room - 2.0);
                let x = 4.0 + i as f32 * BAR_STEP;
                dc.FillRectangle(
                    &rect(x, middle - h / 2.0, x + BAR_W, middle + h / 2.0),
                    &ink,
                );
            }
        }
        Ok(())
    }
}

fn make_badges(comp: &Comp, k: f32, f: &Fragment) -> Result<ContainerVisual> {
    let row = comp.container()?;
    let mut labels = Vec::new();
    if f.speed != 1.0 {
        labels.push(rate_label(f.speed));
    }
    if f.zoom.is_some() {
        labels.push(tr!("editor.badge_zoom").into_owned());
    }
    if f.muted {
        labels.push(tr!("editor.badge_muted").into_owned());
    }
    let style = TextStyle {
        size: 11.0,
        bold: true,
        color: color(TEXT, 1.0),
        tabular: true,
    };
    let c = &comp.compositor;
    let mut x = 5.0;
    for label in labels {
        let text = comp.text(&label, style, k)?;
        let w = text.width + 14.0;
        let back = c.CreateShapeVisual()?;
        back.SetSize(v2(w, BADGE_H))?;
        let shape = c.CreateRoundedRectangleGeometry()?;
        shape.SetSize(v2(w, BADGE_H))?;
        shape.SetCornerRadius(v2(BADGE_H / 2.0, BADGE_H / 2.0))?;
        let sprite = c.CreateSpriteShapeWithGeometry(&shape)?;
        sprite.SetFillBrush(&comp.brush(color("#000000", 0.62))?)?;
        back.Shapes()?.Append(&sprite)?;
        back.SetOffset(v3(x, 5.0))?;
        text.visual.SetOffset(v3(
            snap(x + 7.0, k),
            snap(5.0 + BADGE_H / 2.0 - comp.cap_middle(style), k),
        ))?;
        row.Children()?.InsertAtTop(&back)?;
        row.Children()?.InsertAtTop(&text.visual)?;
        x += w + 4.0;
    }
    Ok(row)
}

/// The transition's round button, as it sits on a join and as the tool's chip being dragged.
pub fn chip(comp: &Comp, k: f32) -> Result<ContainerVisual> {
    let c = &comp.compositor;
    let button = comp.container()?;
    let back = c.CreateShapeVisual()?;
    back.SetSize(v2(JOIN_BUTTON, JOIN_BUTTON))?;
    let circle = c.CreateEllipseGeometry()?;
    circle.SetCenter(v2(JOIN_BUTTON / 2.0, JOIN_BUTTON / 2.0))?;
    circle.SetRadius(v2(JOIN_BUTTON / 2.0, JOIN_BUTTON / 2.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&circle)?;
    sprite.SetFillBrush(&comp.brush(color(JOIN_FILL, 1.0))?)?;
    back.Shapes()?.Append(&sprite)?;
    button.Children()?.InsertAtTop(&back)?;
    let icon = comp.glyph(Glyph::Dissolve, 12.0, color(TEXT, 1.0), k)?;
    icon.SetOffset(v3((JOIN_BUTTON - 12.0) / 2.0, (JOIN_BUTTON - 12.0) / 2.0))?;
    button.Children()?.InsertAtTop(&icon)?;
    button.SetSize(v2(JOIN_BUTTON, JOIN_BUTTON))?;
    button.SetCenterPoint(v3(JOIN_BUTTON / 2.0, JOIN_BUTTON / 2.0))?;
    Ok(button)
}

pub const CHIP: f32 = JOIN_BUTTON;

fn make_join(comp: &Comp, k: f32) -> Result<Join> {
    let c = &comp.compositor;
    let root = comp.container()?;
    let line = c.CreateSpriteVisual()?;
    line.SetBrush(&comp.brush(color(JOIN_LINE.0, JOIN_LINE.1))?)?;
    line.SetSize(v2(2.0, BLOCK_H - 16.0))?;
    line.SetOffset(v3(JOIN / 2.0 - 1.0, 8.0))?;
    root.Children()?.InsertAtTop(&line)?;
    let button = chip(comp, k)?;
    button.SetOffset(v3(
        (JOIN - JOIN_BUTTON) / 2.0,
        (BLOCK_H - JOIN_BUTTON) / 2.0,
    ))?;
    button.SetOpacity(0.0)?;
    root.Children()?.InsertAtTop(&button)?;
    Ok(Join { root, button })
}

fn make_caret(comp: &Comp, k: f32) -> Result<Caret> {
    let c = &comp.compositor;
    let root = comp.container()?;
    let line = comp.brush(color(TEXT, 1.0))?;
    let bar = c.CreateSpriteVisual()?;
    bar.SetBrush(&line)?;
    bar.SetSize(v2(2.0, CARET_BOTTOM - CARET_TOP))?;
    root.Children()?.InsertAtTop(&bar)?;
    let knob = c.CreateShapeVisual()?;
    knob.SetSize(v2(KNOB, KNOB))?;
    let dot = c.CreateEllipseGeometry()?;
    dot.SetCenter(v2(KNOB / 2.0, KNOB / 2.0))?;
    dot.SetRadius(v2(KNOB / 2.0, KNOB / 2.0))?;
    let fill = c.CreateSpriteShapeWithGeometry(&dot)?;
    fill.SetFillBrush(&comp.brush(color(TEXT, 1.0))?)?;
    knob.Shapes()?.Append(&fill)?;
    knob.SetOffset(v3(1.0 - KNOB / 2.0, -KNOB / 2.0))?;
    root.Children()?.InsertAtTop(&knob)?;
    let head = comp.container()?;
    let back = c.CreateShapeVisual()?;
    back.SetSize(v2(HEAD.0, HEAD.1))?;
    let shape = c.CreateRoundedRectangleGeometry()?;
    shape.SetSize(v2(HEAD.0, HEAD.1))?;
    shape.SetCornerRadius(v2(6.0, 6.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&shape)?;
    sprite.SetFillBrush(&comp.brush(color(YELLOW, 1.0))?)?;
    back.Shapes()?.Append(&sprite)?;
    head.Children()?.InsertAtTop(&back)?;
    let icon = comp.glyph(Glyph::Scissors, 12.0, color(STRIP, 1.0), k)?;
    icon.SetOffset(v3((HEAD.0 - 12.0) / 2.0, (HEAD.1 - 12.0) / 2.0))?;
    head.Children()?.InsertAtTop(&icon)?;
    head.SetOffset(v3(1.0 - HEAD.0 / 2.0, -HEAD.1 + 4.0))?;
    head.SetCenterPoint(v3(HEAD.0 / 2.0, HEAD.1))?;
    head.SetOpacity(0.0)?;
    head.SetScale(scale(0.6, 0.6))?;
    root.Children()?.InsertAtTop(&head)?;
    Ok(Caret {
        root,
        line,
        knob,
        head,
        lit: false,
    })
}

fn make_trash(comp: &Comp, k: f32) -> Result<(ContainerVisual, (f32, f32))> {
    let c = &comp.compositor;
    let root = comp.container()?;
    let style = TextStyle {
        size: 13.0,
        bold: false,
        color: color(RED_TEXT, 1.0),
        tabular: false,
    };
    let text = comp.text(&tr!("editor.trash_here"), style, k)?;
    let icon = 18.0;
    let w = (16.0 + icon + 8.0 + text.width + 16.0).ceil();
    let back = c.CreateShapeVisual()?;
    back.SetSize(v2(w, TRASH_H))?;
    let shape = c.CreateRoundedRectangleGeometry()?;
    shape.SetSize(v2(w, TRASH_H))?;
    shape.SetCornerRadius(v2(TRASH_H / 2.0, TRASH_H / 2.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&shape)?;
    sprite.SetFillBrush(&comp.brush(color(STRIP, PANEL_ALPHA))?)?;
    back.Shapes()?.Append(&sprite)?;
    root.Children()?.InsertAtTop(&back)?;
    let glyph = comp.picture(icon, icon, k, |dc, base| {
        glyph::draw(dc, base, Glyph::Trash, icon, color(RED_TEXT, 1.0))
    })?;
    glyph.SetOffset(v3(16.0, (TRASH_H - icon) / 2.0))?;
    root.Children()?.InsertAtTop(&glyph)?;
    text.visual.SetOffset(v3(
        snap(16.0 + icon + 8.0, k),
        snap(TRASH_H / 2.0 - comp.cap_middle(style), k),
    ))?;
    root.Children()?.InsertAtTop(&text.visual)?;
    root.SetCenterPoint(v3(w / 2.0, TRASH_H / 2.0))?;
    root.SetOpacity(0.0)?;
    Ok((root, (w, TRASH_H)))
}
