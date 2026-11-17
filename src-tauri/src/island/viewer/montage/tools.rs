//! The editor's top panel, in three groups as in the screenshot editor: Back, Undo, Redo on the
//! left; the tools in the middle (scissors, the fragment's sound, speed, zoom, the transition,
//! remove); compression, Save copy and Save on the right. The tools act on the fragment under
//! the caret; the panel shows that fragment's sound and speed.
//!
//! [`Menu`] is a tool's popover: the speed steps in a row, the zoom's choices, the transitions or
//! the compression modes with their size estimates in a list.

use super::super::super::comp::{Comp, Motion, TextStyle, color, v2, v3};
use super::super::super::glyph::Glyph;
use super::super::super::log;
use super::super::capsule::{Capsule, Face, Look, Panel};
use super::super::{MUTED, Metrics, RED_ICON, TEXT};
use crate::lang::tr;
use crate::montage::{SPEEDS, curve::Transition, squeeze::Squeeze};
use crate::play::rate_label;
use windows::{UI::Composition::ContainerVisual, core::Result};

const FADE: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);
const MENU_BACK: (&str, f32) = ("#2C2C2E", 0.97);
const MENU_PAD: f32 = 6.0;
const MENU_GAP: f32 = 8.0;
const ROW_H: f32 = 34.0;
const TALL_ROW_H: f32 = 44.0;
const CHEVRON: f32 = 11.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Back,
    Undo,
    Redo,
    Full,
    Scissors,
    Sound,
    Speed,
    Zoom,
    Transition,
    Squeeze,
    SaveCopy,
    Save,
}

const LEFT: [Key; 4] = [Key::Back, Key::Undo, Key::Redo, Key::Full];
const CENTER: [Key; 5] = [
    Key::Scissors,
    Key::Sound,
    Key::Speed,
    Key::Zoom,
    Key::Transition,
];
const RIGHT: [Key; 3] = [Key::Squeeze, Key::SaveCopy, Key::Save];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub scissors: bool,
    pub muted: bool,
    pub speed: f64,
    pub zooming: bool,
    pub undo: bool,
    pub redo: bool,
    /// More than one fragment: there are joins for a transition.
    pub several: bool,
    pub full: bool,
    pub squeeze: Squeeze,
    /// The edit is being saved: the edits wait, watching goes on.
    pub saving: bool,
}

pub struct Tools {
    pub panel: Panel<Key>,
    m: Metrics,
    k: f32,
    width: f32,
    state: State,
}

impl Tools {
    pub fn new(comp: &Comp, k: f32, m: Metrics, width: f32, state: State) -> Result<Self> {
        let mut panel = Panel::new(comp)?;
        for key in LEFT.into_iter().chain(CENTER).chain(RIGHT) {
            panel.add(key, capsule(comp, k, m, key, &state)?)?;
        }
        let mut tools = Self {
            panel,
            m,
            k,
            width,
            state,
        };
        tools.enable();
        tools.layout();
        Ok(tools)
    }

    fn enable(&mut self) {
        let s = self.state;
        let edits = !s.saving;
        let on = [
            (Key::Undo, s.undo && edits),
            (Key::Redo, s.redo && edits),
            (Key::Scissors, edits),
            (Key::Sound, edits),
            (Key::Speed, edits),
            (Key::Zoom, edits),
            (Key::Transition, s.several && edits),
            (Key::Squeeze, edits),
            (Key::SaveCopy, edits),
        ];
        for (key, enabled) in on {
            if let Some(c) = self.panel.get_mut(key) {
                c.set_enabled(enabled);
            }
        }
    }

    pub fn set_state(&mut self, comp: &Comp, state: State) {
        let old = self.state;
        self.state = state;
        let changed = [
            (Key::Sound, old.muted != state.muted),
            (Key::Speed, old.speed != state.speed),
            (Key::Squeeze, old.squeeze != state.squeeze),
            (Key::Full, old.full != state.full),
        ];
        let mut moved = false;
        for (key, change) in changed {
            if change {
                match capsule(comp, self.k, self.m, key, &state) {
                    Ok(fresh) => {
                        moved |= self.panel.get(key).map(|c| c.rect().2) != Some(fresh.rect().2);
                        log(self.panel.replace(comp, key, fresh));
                    }
                    Err(e) => log(Err(e)),
                }
            }
        }
        let m = self.m;
        for (key, on) in [(Key::Scissors, state.scissors), (Key::Zoom, state.zooming)] {
            if let Some(c) = self.panel.get_mut(key) {
                c.set_look(comp, if on { m.selected() } else { m.plain() });
            }
        }
        self.enable();
        if moved {
            self.layout();
        }
    }

    fn span(&self, keys: &[Key]) -> f32 {
        keys.iter()
            .enumerate()
            .filter_map(|(i, &key)| {
                let gap = if i > 0 { self.m.gap } else { 0.0 };
                self.panel.get(key).map(|c| gap + c.rect().2)
            })
            .sum()
    }

    fn group_gap(&self) -> f32 {
        self.m.gap * 3.0
    }

    pub fn min_width(&self) -> f32 {
        let groups = self.span(&LEFT) + self.span(&CENTER) + self.span(&RIGHT);
        (groups + self.group_gap() * 2.0 + self.m.margin * 2.0).ceil()
    }

    pub fn resize(&mut self, width: f32) {
        self.width = width;
        self.layout();
    }

    /// The middle group is centered in the window but doesn't run into the outer ones.
    fn layout(&mut self) {
        let m = self.m;
        let y = m.button_top();
        let gap = m.gap;
        let group = self.group_gap();
        let right = self.span(&RIGHT);
        let center = self.span(&CENTER);
        let row = |panel: &mut Panel<Key>, keys: &[Key], mut x: f32| {
            for (i, &key) in keys.iter().enumerate() {
                if i > 0 {
                    x += gap;
                }
                if let Some(c) = panel.get_mut(key) {
                    c.place(x, y);
                    x += c.rect().2;
                }
            }
            x
        };
        let left_end = row(&mut self.panel, &LEFT, m.margin);
        let right_start = self.width - m.margin - right;
        row(&mut self.panel, &RIGHT, right_start);
        let x = ((self.width - center) / 2.0)
            .min(right_start - group - center)
            .max(left_end + group);
        row(&mut self.panel, &CENTER, x);
    }

    pub fn anchor(&self, key: Key) -> Option<(f32, f32, f32, f32)> {
        self.panel.get(key).map(Capsule::rect)
    }
}

fn label_style(m: Metrics, ink: &str, bold: bool) -> TextStyle {
    TextStyle {
        size: m.text - 0.5,
        bold,
        color: color(ink, 1.0),
        tabular: true,
    }
}

pub fn squeeze_name(squeeze: Squeeze) -> String {
    match squeeze {
        Squeeze::AsIs => tr!("editor.squeeze_as_is"),
        Squeeze::Chat => tr!("editor.squeeze_chat"),
        Squeeze::Feather => tr!("editor.squeeze_feather"),
    }
    .into_owned()
}

fn squeeze_note(squeeze: Squeeze) -> String {
    match squeeze {
        Squeeze::AsIs => tr!("editor.squeeze_as_is_note"),
        Squeeze::Chat => tr!("editor.squeeze_chat_note"),
        Squeeze::Feather => tr!("editor.squeeze_feather_note"),
    }
    .into_owned()
}

/// The tools in the middle are all as wide as the widest of them, the longest speed label.
fn tool_width(comp: &Comp, m: Metrics) -> f32 {
    let style = label_style(m, TEXT, true);
    SPEEDS
        .iter()
        .map(|&s| Capsule::text_width(comp, &rate_label(s), style, m.button.1))
        .fold(m.button.0, f32::max)
}

fn capsule(comp: &Comp, k: f32, m: Metrics, key: Key, state: &State) -> Result<Capsule> {
    let ink = color(TEXT, 1.0);
    let h = m.button.1;
    let size = if CENTER.contains(&key) {
        (tool_width(comp, m), h)
    } else {
        m.button
    };
    let glyph = |g: Glyph| Capsule::new(comp, k, size, Face::Glyph(g, m.icon, ink), m.plain());
    let text = |label: &str, bold: bool, look: Look| -> Result<Capsule> {
        let style = label_style(m, TEXT, bold);
        let w = Capsule::text_width(comp, label, style, h).max(m.button.0);
        Capsule::new(comp, k, (w, h), Face::Text(label, style), look)
    };
    match key {
        Key::Back => glyph(Glyph::Back),
        Key::Undo => glyph(Glyph::Undo),
        Key::Redo => glyph(Glyph::Redo),
        Key::Full if state.full => glyph(Glyph::ExitFullScreen),
        Key::Full => glyph(Glyph::FullScreen),
        Key::Scissors => glyph(Glyph::Scissors),
        Key::Sound if state.muted => glyph(Glyph::Muted),
        Key::Sound => glyph(Glyph::Volume),
        Key::Speed => {
            let style = label_style(m, TEXT, true);
            let label = rate_label(state.speed);
            Capsule::new(comp, k, size, Face::Text(&label, style), m.plain())
        }
        Key::Zoom => glyph(Glyph::ZoomIn),
        Key::Transition => glyph(Glyph::Dissolve),
        Key::Squeeze => {
            let name = squeeze_name(state.squeeze);
            let style = label_style(m, TEXT, false);
            let w = Capsule::text_width(comp, &name, style, h) + CHEVRON + 4.0;
            let capsule = Capsule::new(comp, k, (w, h), Face::Text(&name, style), m.plain())?;
            let face = capsule.face.Offset()?;
            capsule
                .face
                .SetOffset(v3(face.X - (CHEVRON + 4.0) / 2.0, face.Y))?;
            let chevron = comp.glyph(Glyph::ChevronDown, CHEVRON, ink, k)?;
            chevron.SetOffset(v3(w - h * 0.5 - CHEVRON, (h - CHEVRON) / 2.0))?;
            capsule.body.Children()?.InsertAtTop(&chevron)?;
            Ok(capsule)
        }
        Key::SaveCopy => text(&tr!("viewer.save_copy"), false, m.plain()),
        Key::Save => text(&tr!("viewer.save"), true, m.accent()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Speed(usize),
    Smooth,
    AtOnce,
    Unzoom,
    Transition(Option<Transition>),
    /// A kind in the transition tool's row: dragged onto a join.
    Kind(Transition),
    Squeeze(Squeeze),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Tool(Key),
    Join(usize),
}

pub struct Menu {
    pub owner: Owner,
    pub panel: Panel<Pick>,
    rect: (f32, f32, f32, f32),
}

/// Where a popover opens: under a button of the top panel, or above a point of the timeline.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    Below((f32, f32, f32, f32)),
    Above((f32, f32)),
}

/// A join's list: "no transition" first, it is how a transition is taken away.
const TRANSITIONS: [Option<Transition>; 5] = [
    None,
    Some(Transition::Dissolve),
    Some(Transition::Black),
    Some(Transition::Slide),
    Some(Transition::PushIn),
];

fn transition_name(t: Option<Transition>) -> String {
    match t {
        None => tr!("editor.transition_none"),
        Some(Transition::Dissolve) => tr!("editor.transition_dissolve"),
        Some(Transition::Black) => tr!("editor.transition_black"),
        Some(Transition::Slide) => tr!("editor.transition_slide"),
        Some(Transition::PushIn) => tr!("editor.transition_push"),
    }
    .into_owned()
}

impl Menu {
    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        let (left, top, w, h) = self.rect;
        x >= left && x <= left + w && y >= top && y <= top + h
    }

    pub fn remove(&self, host: &ContainerVisual) {
        if let Ok(children) = host.Children() {
            let _ = children.Remove(&self.panel.root);
        }
    }

    pub fn speeds(
        comp: &Comp,
        k: f32,
        m: Metrics,
        anchor: Anchor,
        width: f32,
        speed: f64,
    ) -> Result<Self> {
        let h = m.button.1;
        let items: Vec<(Pick, String, bool)> = SPEEDS
            .iter()
            .enumerate()
            .map(|(i, &s)| (Pick::Speed(i), rate_label(s), s == speed))
            .collect();
        Self::row(
            comp,
            (k, m),
            (Owner::Tool(Key::Speed), anchor),
            width,
            h,
            (items, None),
        )
    }

    /// The transition tool's row: the kinds, each to be dragged onto a join.
    pub fn kinds(comp: &Comp, k: f32, m: Metrics, anchor: Anchor, width: f32) -> Result<Self> {
        let items = TRANSITIONS
            .iter()
            .flatten()
            .map(|&t| (Pick::Kind(t), transition_name(Some(t)), false))
            .collect();
        Self::row(
            comp,
            (k, m),
            (Owner::Tool(Key::Transition), anchor),
            width,
            m.button.1,
            (items, None),
        )
    }

    pub fn zoom(
        comp: &Comp,
        k: f32,
        m: Metrics,
        anchor: Anchor,
        width: f32,
        smooth: Option<bool>,
    ) -> Result<Self> {
        let h = m.button.1;
        let items = vec![
            (
                Pick::Smooth,
                tr!("editor.zoom_smooth").into_owned(),
                smooth == Some(true),
            ),
            (
                Pick::AtOnce,
                tr!("editor.zoom_at_once").into_owned(),
                smooth == Some(false),
            ),
        ];
        // Taking the zoom away is not a third kind of zoom: a capsule of its own, apart.
        let unzoom = smooth.map(|_| (Pick::Unzoom, tr!("editor.zoom_remove").into_owned()));
        Self::row(
            comp,
            (k, m),
            (Owner::Tool(Key::Zoom), anchor),
            width,
            h,
            (items, unzoom),
        )
    }

    #[allow(clippy::type_complexity)]
    fn row(
        comp: &Comp,
        (k, m): (f32, Metrics),
        (owner, anchor): (Owner, Anchor),
        width: f32,
        h: f32,
        (items, apart): (Vec<(Pick, String, bool)>, Option<(Pick, String)>),
    ) -> Result<Self> {
        let mut capsules = Vec::new();
        for (pick, label, chosen) in items {
            let (ink, look) = if chosen {
                ("#1C1C1E", Look::new(TEXT, 1.0, TEXT, 1.0))
            } else {
                (TEXT, m.clear())
            };
            let style = label_style(m, ink, true);
            let w = Capsule::text_width(comp, &label, style, h).max(h * 1.4);
            capsules.push((
                pick,
                Capsule::new(comp, k, (w, h), Face::Text(&label, style), look)?,
            ));
        }
        let inner: f32 = capsules.iter().map(|(_, c)| c.rect().2).sum::<f32>()
            + 4.0 * (capsules.len() as f32 - 1.0);
        let pill = inner + 2.0 * MENU_PAD;
        let apart = match apart {
            Some((pick, label)) => {
                let style = label_style(m, RED_ICON, false);
                let w = Capsule::text_width(comp, &label, style, h);
                let capsule = Capsule::new(comp, k, (w, h), Face::Text(&label, style), m.danger())?;
                Some((pick, capsule))
            }
            None => None,
        };
        let extra = apart
            .as_ref()
            .map_or(0.0, |(_, c)| MENU_GAP + c.rect().2 + 2.0 * MENU_PAD);
        let size = (pill + extra, h + 2.0 * MENU_PAD);
        let rect = place(anchor, size, width, m);
        let mut panel = Panel::new(comp)?;
        backing(comp, &panel, (rect.0, rect.1, pill, rect.3))?;
        let mut x = rect.0 + MENU_PAD;
        for (pick, mut capsule) in capsules {
            capsule.place(x, rect.1 + MENU_PAD);
            x += capsule.rect().2 + 4.0;
            panel.add(pick, capsule)?;
        }
        if let Some((pick, mut capsule)) = apart {
            let left = rect.0 + pill + MENU_GAP;
            backing(comp, &panel, (left, rect.1, extra - MENU_GAP, rect.3))?;
            capsule.place(left + MENU_PAD, rect.1 + MENU_PAD);
            panel.add(pick, capsule)?;
        }
        Ok(Self { owner, panel, rect })
    }

    pub fn transitions(
        comp: &Comp,
        k: f32,
        m: Metrics,
        (owner, anchor): (Owner, Anchor),
        width: f32,
        current: Option<Transition>,
    ) -> Result<Self> {
        let rows = TRANSITIONS
            .iter()
            .map(|&t| {
                (
                    Pick::Transition(t),
                    transition_name(t),
                    None,
                    None,
                    t == current,
                )
            })
            .collect();
        Self::list(comp, (k, m), owner, anchor, width, (220.0, ROW_H), rows)
    }

    pub fn squeezes(
        comp: &Comp,
        k: f32,
        m: Metrics,
        anchor: Anchor,
        width: f32,
        (current, sizes): (Squeeze, [Option<String>; 3]),
    ) -> Result<Self> {
        let rows = Squeeze::ALL
            .iter()
            .zip(sizes)
            .map(|(&s, size)| {
                (
                    Pick::Squeeze(s),
                    squeeze_name(s),
                    Some(squeeze_note(s)),
                    size,
                    s == current,
                )
            })
            .collect();
        Self::list(
            comp,
            (k, m),
            Owner::Tool(Key::Squeeze),
            anchor,
            width,
            (300.0, TALL_ROW_H),
            rows,
        )
    }

    #[allow(clippy::type_complexity)]
    fn list(
        comp: &Comp,
        (k, m): (f32, Metrics),
        owner: Owner,
        anchor: Anchor,
        width: f32,
        (w, h): (f32, f32),
        rows: Vec<(Pick, String, Option<String>, Option<String>, bool)>,
    ) -> Result<Self> {
        let size = (
            w + 2.0 * MENU_PAD,
            rows.len() as f32 * (h + 2.0) - 2.0 + 2.0 * MENU_PAD,
        );
        let rect = place(anchor, size, width, m);
        let mut panel = Panel::new(comp)?;
        backing(comp, &panel, rect)?;
        let title = label_style(m, TEXT, false);
        let small = TextStyle {
            size: m.text - 1.5,
            bold: false,
            color: color(MUTED, 1.0),
            tabular: true,
        };
        for (i, (pick, name, note, right, chosen)) in rows.into_iter().enumerate() {
            let look = if chosen { m.selected() } else { m.clear() };
            let mut capsule = Capsule::new(comp, k, (w, h), Face::Text(&name, title), look)?;
            let middle = h / 2.0 - comp.cap_middle(title);
            let y = if note.is_some() { middle - 7.0 } else { middle };
            capsule.face.SetOffset(v3(snap(12.0, k), snap(y, k)))?;
            if let Some(note) = note {
                let text = comp.text(&note, small, k)?;
                text.visual
                    .SetOffset(v3(snap(12.0, k), snap(middle + 8.0, k)))?;
                capsule.body.Children()?.InsertAtTop(&text.visual)?;
            }
            if let Some(right) = right {
                let text = comp.text(&right, small, k)?;
                text.visual.SetOffset(v3(
                    snap(w - 12.0 - text.width, k),
                    snap(h / 2.0 - comp.cap_middle(small), k),
                ))?;
                capsule.body.Children()?.InsertAtTop(&text.visual)?;
            }
            capsule.place(rect.0 + MENU_PAD, rect.1 + MENU_PAD + i as f32 * (h + 2.0));
            panel.add(pick, capsule)?;
        }
        Ok(Self { owner, panel, rect })
    }

    pub fn show(&self, comp: &Comp, host: &ContainerVisual) -> Result<()> {
        self.panel.root.SetOpacity(0.0)?;
        host.Children()?.InsertAtTop(&self.panel.root)?;
        comp.animate_scalar(&self.panel.root, "Opacity", 1.0, FADE)
    }
}

fn snap(v: f32, k: f32) -> f32 {
    (v * k).round() / k
}

fn place(anchor: Anchor, (w, h): (f32, f32), width: f32, m: Metrics) -> (f32, f32, f32, f32) {
    let (cx, y) = match anchor {
        Anchor::Below((x, y, aw, ah)) => (x + aw / 2.0, y + ah + MENU_GAP),
        Anchor::Above((x, y)) => (x, y - MENU_GAP - h),
    };
    let x = (cx - w / 2.0).min(width - m.margin - w).max(m.margin);
    (x, y, w, h)
}

fn backing<K: Copy + PartialEq>(
    comp: &Comp,
    panel: &Panel<K>,
    rect: (f32, f32, f32, f32),
) -> Result<()> {
    let c = &comp.compositor;
    let back = c.CreateShapeVisual()?;
    back.SetSize(v2(rect.2, rect.3))?;
    back.SetOffset(v3(rect.0, rect.1))?;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetSize(v2(rect.2, rect.3))?;
    geometry.SetCornerRadius(v2(14.0, 14.0))?;
    let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
    shape.SetFillBrush(&comp.brush(color(MENU_BACK.0, MENU_BACK.1))?)?;
    back.Shapes()?.Append(&shape)?;
    panel.root.Children()?.InsertAtBottom(&back)?;
    Ok(())
}
