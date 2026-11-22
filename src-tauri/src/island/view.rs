//! The island scene: a black capsule and the view inside it. This file is the frame every view
//! shares: the capsule, swapping one view for another, hover and press, hit testing. A view
//! itself (its size, its parts, what changes in place) is a file in `view/`.
//!
//! The capsule is a rounded rectangle whose size the compositor animates. Rounding and centering are bound to
//! the size by expressions, so the system recomputes them every frame, not us.

mod card;
mod device;
mod device_art;
mod download;
mod file_icon;
pub(super) use file_icon::sheet;
mod menu;
mod microphones;
mod panel;
mod player;
mod problem;
mod pull;
mod record;
mod scan;
mod shot;
mod status;
mod strip;
mod swipe;
mod system;
mod timers;
mod update;
mod viewing;

pub use card::{CardModel, CardPhase, Video};
pub use device::DeviceModel;
pub use download::{DownloadModel, FileModel};
pub use menu::button_center;
pub use microphones::{MAX_MICROPHONES, TALLEST as PANEL_TALLEST};
pub use panel::PanelModel;
pub use player::SAMPLE_MS;
pub use problem::ProblemModel;
pub use record::RecordModel;
pub use scan::{CopyLook, ScanHit, ScanLine, ScanModel};
pub use shot::SHUTTER_MS;
pub use status::Modes;
pub use strip::StripModel;
pub use system::{AppModel, SystemModel, has_list};
pub use update::{UpdateLook, UpdateModel};
pub use viewing::HOLD_MS;

use super::comp::{Comp, Motion, Text, TextStyle, color, scale, snap, v2, v3};
use crate::capture::Image;
use crate::control::{ControlId, Item};
use crate::gesture::Dir;
use crate::lang::tr;
use crate::media::Session;
use crate::record::panel::{Page, Press};
use crate::scheduler::Snapshot;
use std::sync::Arc;
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionColorBrush, CompositionEllipseGeometry, CompositionObject,
            CompositionPropertySet, CompositionRoundedRectangleGeometry, CompositionStrokeCap,
            ContainerVisual, ShapeVisual, Visual,
        },
    },
    core::{Interface, Result},
};

const MORPH: Motion = Motion::curve(420, 0.32, 1.28, 0.5, 1.0);
const FADE_IN: Motion = Motion::curve(220, 0.0, 0.0, 0.2, 1.0);
const FADE_OUT: Motion = Motion::curve(120, 0.4, 0.0, 1.0, 1.0);
const DIM: Motion = Motion::curve(600, 0.25, 0.1, 0.25, 1.0);
const UNDIM: Motion = Motion::curve(200, 0.25, 0.1, 0.25, 1.0);
const HOVER: Motion = Motion::curve(150, 0.25, 0.1, 0.25, 1.0);
const LATCH: Motion = Motion::curve(120, 0.22, 1.0, 0.36, 1.0);
const DIM_OPACITY: f32 = 0.45;
const DOTS_TOP: f32 = 5.0;
const DOTS_SPACE: f32 = 6.0;

const TEXT: &str = "#F5F5F7";
const MUTED: &str = "#8E8E93";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Collapsed,
    Expanded,
    Alert,
    Player(PlayerForm),
    Menu,
    /// The pre-start panel of the recording: a page of the control center.
    RecordPanel(Page),
    /// What was toggled and to which state (true = muted).
    Indicator(ControlId, bool),
    Shutter,
    /// The capture card: a screenshot or a finished recording.
    Card,
    Viewing,
    Countdown,
    Recording,
    Discarded,
    Problem {
        open: bool,
    },
    /// A check for an update, its result or the answer to the update window.
    Update,
    /// A system notice (`crate::notify`); `open` only for one with a list of apps, under the cursor.
    System {
        open: bool,
    },
    Downloading,
    /// Files have arrived; `open` is the card under the cursor.
    Download {
        open: bool,
    },
    /// A device notice (`crate::devices`).
    Device,
    /// The text scanner is open; `list` = its languages are listed.
    Scan {
        list: bool,
    },
}

impl Mode {
    /// Whether the island dims in this view when untouched. Only collapsed activities dim; notices and states
    /// (the viewer window is open, a download is coming) never do: they are shown to be seen.
    pub fn dims(self) -> bool {
        matches!(self, Self::Collapsed | Self::Player(PlayerForm::Collapsed))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerForm {
    Collapsed,
    Announce,
    /// Player card on hover. Without a position there is no progress row.
    Expanded {
        timeline: bool,
    },
}

impl PlayerForm {
    pub fn size(self) -> (f32, f32) {
        match self {
            Self::Collapsed => (62.0, 30.0),
            Self::Announce => (300.0, 52.0),
            Self::Expanded { timeline: true } => (340.0, 150.0),
            Self::Expanded { timeline: false } => (340.0, 124.0),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Hit {
    Timer(String),
    Pause,
    Done,
    Player(PlayerHit),
    Control(ControlId),
    /// The card's thumbnail: open the file.
    Card,
    CardCopy,
    Viewer,
    ViewerPlay,
    ToIsland,
    RecordMic,
    RecordPause,
    RecordStop,
    /// Held: the recording is dropped without a file.
    RecordCancel,
    /// Countdown circle: start right away.
    RecordNow,
    Panel(Press),
    Problem,
    /// The one arrived file's picture: open it.
    DownloadOpen,
    /// The text of the downloads card, or its pile: show the files in Explorer.
    DownloadFolder,
    Scan(ScanHit),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerHit {
    Open,
    /// The title and artist: copied as text.
    Copy,
    Previous,
    PlayPause,
    Next,
    Seek,
}

/// How an element behaves on a frozen island (`Model::frozen`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Liveness {
    #[allow(dead_code)]
    Static,
    /// Exists only in motion: the track time. Hidden on a frozen island.
    Live,
    /// Has a live and a frozen form: the bars, the timer clocks (seconds or minutes).
    Freezable,
}

impl Liveness {
    fn visible(self, frozen: bool) -> bool {
        !(frozen && self == Self::Live)
    }

    fn moving(self, frozen: bool) -> bool {
        self != Self::Static && !frozen
    }
}

pub struct Model<'a> {
    pub snapshot: &'a Snapshot,
    pub mode: Mode,
    /// Static shapes instead of live ones: a dimmed island without live animation.
    /// Timers show minutes, the bars freeze, the track time is hidden.
    pub frozen: bool,
    pub skipping: Option<&'a str>,
    pub player: Option<PlayerModel<'a>>,
    /// `None` = there is one activity, no dots needed.
    pub dots: Option<(usize, usize)>,
    pub controls: &'a [Item],
    /// Island ms: the recording time is computed from it.
    pub now_ms: u64,
    pub modes: Modes,
    pub card: Option<CardModel<'a>>,
    pub viewing: Option<&'a Arc<Image>>,
    /// The status belongs to a video: whether it runs.
    pub viewing_play: Option<bool>,
    pub record: Option<RecordModel>,
    pub countdown: Option<u32>,
    pub problem: Option<ProblemModel<'a>>,
    pub update: Option<UpdateModel<'a>>,
    pub panel: Option<PanelModel<'a>>,
    pub system: Option<SystemModel<'a>>,
    pub strip: Option<StripModel>,
    pub download: Option<DownloadModel<'a>>,
    pub device: Option<DeviceModel<'a>>,
    pub scan: Option<ScanModel<'a>>,
}

pub struct PlayerModel<'a> {
    pub session: &'a Session,
    /// Unix ms: the track position is computed from it.
    pub now_unix_ms: u64,
    pub headphones: bool,
    pub decorative: bool,
    pub copied: bool,
}

#[derive(Clone, Copy, Default)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

struct Ring {
    visual: ShapeVisual,
    arc: CompositionEllipseGeometry,
}

struct Button {
    fill: CompositionColorBrush,
    rect: Rect,
    hit: Hit,
    base: Color,
    hover: Color,
}

struct IconButton {
    visual: Visual,
    hit: Hit,
}

struct Clock {
    text: Text,
    timer: Option<String>,
    right: Option<f32>,
    y: f32,
}

struct Content {
    root: ContainerVisual,
    /// Structure description: if unchanged, the view updates in place.
    key: String,
    width: f32,
    height: f32,
    rings: Vec<(Option<String>, Ring)>,
    clocks: Vec<Clock>,
    buttons: Vec<Button>,
    icon_buttons: Vec<IconButton>,
    hits: Vec<(Rect, Hit)>,
    /// For the collapsed view: the width without the clock; the clock is added on the right.
    clock_left: f32,
    anchor_left: bool,
    slide: CompositionPropertySet,
    player: Option<player::Parts>,
    menu: Option<menu::Parts>,
    panel: Option<panel::Parts>,
    card: Option<card::Parts>,
    viewing: Option<viewing::Parts>,
    record: Option<record::Parts>,
    download: Option<download::Parts>,
    scan: Option<scan::Parts>,
    /// Kept alive because an expression holds it only weakly; dropped, the gradient falls back to a one-DIP silver stripe.
    _light: Option<CompositionPropertySet>,
}

pub struct Scene {
    pub comp: Comp,
    island: ContainerVisual,
    geometry: CompositionRoundedRectangleGeometry,
    frame_width: f32,
    scale: f32,
    content: Option<Content>,
    dying: Vec<ContainerVisual>,
    dimmed: bool,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    incoming: Option<(Dir, Content)>,
    pulled: Option<Content>,
    dragging: bool,
    shudder: CompositionPropertySet,
}

fn style(size: f32, bold: bool, hex: &str) -> TextStyle {
    TextStyle {
        size,
        bold,
        color: color(hex, 1.0),
        tabular: false,
    }
}

fn clock_style(size: f32, hex: &str) -> TextStyle {
    TextStyle {
        size,
        bold: true,
        color: color(hex, 1.0),
        tabular: true,
    }
}

pub fn format_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1000.0 {
        tr!("size.gb", n = format!("{:.1}", mb / 1024.0)).into_owned()
    } else if mb < 10.0 {
        tr!("size.mb", n = format!("{mb:.1}")).into_owned()
    } else {
        tr!("size.mb", n = mb.round() as u64).into_owned()
    }
}

pub fn format_clock(ms: i64) -> String {
    let total = (ms.max(0) + 999) / 1000;
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl Scene {
    pub fn new(comp: Comp) -> Result<Self> {
        let c = &comp.compositor;
        let island = c.CreateContainerVisual()?;
        comp.root.Children()?.InsertAtTop(&island)?;

        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(0.0, 30.0))?;
        // Rounding is half the height but at most 24, like border-radius in the CSS version.
        comp.bind(
            &geometry,
            "CornerRadius",
            "Vector2(Min(this.Target.Size.Y / 2, 24), Min(this.Target.Size.Y / 2, 24))",
            &[],
            &[],
        )?;

        let background = c.CreateShapeVisual()?;
        background.SetSize(v2(2000.0, 2000.0))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&comp.brush(color("#000000", 1.0))?)?;
        shape.SetStrokeBrush(&comp.brush(color("#FFFFFF", 0.08))?)?;
        shape.SetStrokeThickness(2.0)?;
        background.Shapes()?.Append(&shape)?;
        island.Children()?.InsertAtBottom(&background)?;

        let clip_geometry = c.CreateRoundedRectangleGeometry()?;
        let g: CompositionObject = geometry.cast()?;
        comp.bind(&clip_geometry, "Size", "g.Size", &[("g", &g)], &[])?;
        comp.bind(
            &clip_geometry,
            "CornerRadius",
            "g.CornerRadius",
            &[("g", &g)],
            &[],
        )?;
        island.SetClip(&c.CreateGeometricClipWithGeometry(&clip_geometry)?)?;
        let shudder = c.CreatePropertySet()?;
        shudder.InsertScalar(&windows::core::HSTRING::from("X"), 0.0)?;

        let scene = Self {
            comp,
            island,
            geometry,
            frame_width: 400.0,
            scale: 1.0,
            content: None,
            dying: Vec::new(),
            dimmed: false,
            hovered: None,
            pressed: None,
            incoming: None,
            pulled: None,
            dragging: false,
            shudder,
        };
        scene.bind_island_offset()?;
        Ok(scene)
    }

    fn bind_island_offset(&self) -> Result<()> {
        let g: CompositionObject = self.geometry.cast()?;
        let k: CompositionObject = self.shudder.cast()?;
        self.comp.bind(
            &self.island,
            "Offset",
            "Vector3((W - g.Size.X) / 2 + k.X, 0, 0)",
            &[("g", &g), ("k", &k)],
            &[("W", self.frame_width)],
        )
    }

    pub fn set_frame_width(&mut self, width: f32) -> Result<()> {
        self.frame_width = width;
        self.bind_island_offset()
    }

    pub fn set_scale(&mut self, scale: f32) -> Result<()> {
        if (scale - self.scale).abs() < f32::EPSILON {
            return Ok(());
        }
        self.scale = scale;
        self.comp.root.SetScale(windows_numerics::Vector3 {
            X: scale,
            Y: scale,
            Z: 1.0,
        })?;
        if let Some(content) = &mut self.content {
            content.key.clear();
        }
        Ok(())
    }

    pub fn measure(&self, model: &Model) -> (f32, f32) {
        match model.mode {
            Mode::Collapsed => (self.collapsed_width(model), 30.0),
            Mode::Expanded => self.expanded_size(model),
            Mode::Alert => (self.alert_width(model), 66.0),
            Mode::Player(form) => form.size(),
            Mode::Menu => menu::menu_size(model),
            Mode::RecordPanel(_) => self.panel_size(model),
            Mode::Indicator(id, off) => (self.indicator_width(id, off).round(), 30.0),
            Mode::Shutter => (30.0, 30.0),
            Mode::Card => self.card_size(model),
            Mode::Viewing => self.viewing_size(model),
            Mode::Countdown => (30.0, 30.0),
            Mode::Recording => self.recording_size(model),
            Mode::Discarded => (self.discarded_width().round(), 30.0),
            Mode::Problem { open } => self.problem_size(model, open),
            Mode::Update => self.update_size(model),
            Mode::System { open } => self.system_size(model, open),
            Mode::Downloading => self.downloading_size(model),
            Mode::Download { open } => self.download_size(model, open),
            Mode::Device => self.device_size(model),
            Mode::Scan { .. } => self.scan_size(model),
        }
    }

    /// Draws the island for `model`. If the view structure is the same, updates the labels in place;
    /// otherwise builds a new view and cross-fades to it. Returns the capsule size and whether the view
    /// was rebuilt.
    pub fn render(&mut self, model: &Model) -> Result<((f32, f32), bool)> {
        let key = content_key(model);
        let rebuild = self.content.as_ref().is_none_or(|c| c.key != key);
        // Mid-swipe the view isn't swapped and the capsule doesn't resize: the finger controls them.
        if self.dragging
            && let Some(content) = &self.content
        {
            let size = (content.width, content.height);
            if !rebuild {
                let mut content = self.content.take().expect("the island has a view");
                let result = self.update_content(model, &mut content);
                self.content = Some(content);
                result?;
            }
            return Ok((size, false));
        }
        if rebuild {
            let content = self.build(model, key)?;
            self.swap(content)?;
        } else {
            self.update(model)?;
        }
        let content = self.content.as_ref().expect("the view was just built");
        Ok(((content.width, content.height), rebuild))
    }

    fn swap(&mut self, content: Content) -> Result<()> {
        if let Some(old) = self.content.take() {
            self.comp
                .animate_scalar(&old.root, "Opacity", 0.0, FADE_OUT)?;
            self.dying.push(old.root);
        }
        content.root.SetOpacity(0.0)?;
        self.island.Children()?.InsertAtTop(&content.root)?;
        self.comp
            .animate_scalar(&content.root, "Opacity", 1.0, FADE_IN)?;
        self.content = Some(content);
        self.hovered = None;
        self.morph()
    }

    pub fn purge(&mut self) -> Result<()> {
        let children = self.island.Children()?;
        for visual in self.dying.drain(..) {
            children.Remove(&visual)?;
        }
        Ok(())
    }

    fn morph(&self) -> Result<()> {
        let Some(content) = &self.content else {
            return Ok(());
        };
        self.comp.animate_vector2(
            &self.geometry,
            "Size",
            v2(content.width, content.height),
            MORPH,
        )?;
        self.bind_offset(content)
    }

    /// The view is held in the capsule's middle (or at its left edge) plus the gesture offset:
    /// X is a ribbon swipe, Y a menu pull.
    fn bind_offset(&self, content: &Content) -> Result<()> {
        let g: CompositionObject = self.geometry.cast()?;
        let s: CompositionObject = content.slide.cast()?;
        if content.anchor_left {
            return self.comp.bind(
                &content.root,
                "Offset",
                "Vector3(s.X, s.Y, 0)",
                &[("s", &s)],
                &[],
            );
        }
        self.comp.bind(
            &content.root,
            "Offset",
            "Vector3((g.Size.X - w) / 2 + s.X, s.Y, 0)",
            &[("g", &g), ("s", &s)],
            &[("w", content.width)],
        )
    }

    pub fn set_dimmed(&mut self, dimmed: bool) -> Result<()> {
        if dimmed == self.dimmed {
            return Ok(());
        }
        self.dimmed = dimmed;
        let (opacity, motion) = if dimmed {
            (DIM_OPACITY, DIM)
        } else {
            (1.0, UNDIM)
        };
        self.comp
            .animate_scalar(&self.island, "Opacity", opacity, motion)
    }

    pub fn set_hovered(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.hovered {
            return Ok(());
        }
        if let Some(content) = &self.content {
            let changed = |h: &Hit| (self.hovered.as_ref() == Some(h)) != (hit.as_ref() == Some(h));
            let now = |h: &Hit| hit.as_ref() == Some(h);
            for button in content.buttons.iter().filter(|b| changed(&b.hit)) {
                let target = if now(&button.hit) {
                    button.hover
                } else {
                    button.base
                };
                self.comp.animate_color(&button.fill, target, HOVER)?;
            }
            for button in content.icon_buttons.iter().filter(|b| changed(&b.hit)) {
                let opacity = if now(&button.hit) { 0.82 } else { 1.0 };
                self.comp
                    .animate_scalar(&button.visual, "Opacity", opacity, HOVER)?;
            }
            let seek = Hit::Player(PlayerHit::Seek);
            if changed(&seek)
                && let Some(parts) = &content.player
            {
                self.hover_progress(parts, now(&seek))?;
            }
            if let Some(parts) = &content.menu {
                self.hover_menu(parts, self.hovered.as_ref(), hit.as_ref())?;
            }
        }
        self.hovered = hit;
        Ok(())
    }

    pub fn set_pressed(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.pressed {
            return Ok(());
        }
        if let Some(content) = &self.content {
            for button in &content.icon_buttons {
                let was = self.pressed.as_ref() == Some(&button.hit);
                let now = hit.as_ref() == Some(&button.hit);
                if was != now {
                    let k = if now { 0.88 } else { 1.0 };
                    self.comp
                        .animate_vector3(&button.visual, "Scale", scale(k, k), HOVER)?;
                }
            }
            if let Some(parts) = &content.menu {
                self.press_menu(parts, self.pressed.as_ref(), hit.as_ref())?;
            }
        }
        self.pressed = hit;
        Ok(())
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<Hit> {
        let content = self.content.as_ref()?;
        let local_x = x - (self.frame_width - content.width) / 2.0;
        content
            .hits
            .iter()
            .find(|(rect, _)| rect.contains(local_x, y))
            .map(|(_, hit)| hit.clone())
    }

    fn build(&self, model: &Model, key: String) -> Result<Content> {
        let root = self.comp.container()?;
        let mut content = Content {
            root,
            key,
            width: 0.0,
            height: 0.0,
            rings: Vec::new(),
            clocks: Vec::new(),
            buttons: Vec::new(),
            icon_buttons: Vec::new(),
            hits: Vec::new(),
            clock_left: 0.0,
            anchor_left: false,
            slide: {
                let slide = self.comp.compositor.CreatePropertySet()?;
                slide.InsertScalar(&windows::core::HSTRING::from("X"), 0.0)?;
                slide.InsertScalar(&windows::core::HSTRING::from("Y"), 0.0)?;
                slide
            },
            player: None,
            menu: None,
            panel: None,
            card: None,
            viewing: None,
            record: None,
            download: None,
            scan: None,
            _light: None,
        };
        match model.mode {
            Mode::Collapsed => self.build_collapsed(model, &mut content)?,
            Mode::Expanded => self.build_expanded(model, &mut content)?,
            Mode::Alert => self.build_alert(model, &mut content)?,
            Mode::Player(form) => self.build_player(model, form, &mut content)?,
            Mode::Menu => self.build_menu(model, &mut content)?,
            Mode::RecordPanel(_) => self.build_panel(model, &mut content)?,
            Mode::Indicator(id, off) => self.build_indicator(id, off, &mut content)?,
            Mode::Shutter => self.build_shutter(&mut content)?,
            Mode::Card => self.build_card(model, &mut content)?,
            Mode::Viewing => self.build_viewing(model, &mut content)?,
            Mode::Countdown => self.build_countdown(model, &mut content)?,
            Mode::Recording => self.build_recording(&mut content)?,
            Mode::Discarded => self.build_discarded(&mut content)?,
            Mode::Problem { open } => self.build_problem(model, open, &mut content)?,
            Mode::Update => self.build_update(model, &mut content)?,
            Mode::System { open } => self.build_system(model, open, &mut content)?,
            Mode::Downloading => self.build_downloading(model, &mut content)?,
            Mode::Download { open } => self.build_download(model, open, &mut content)?,
            Mode::Device => self.build_device(model, &mut content)?,
            Mode::Scan { .. } => self.build_scan(model, &mut content)?,
        }
        self.update_content(model, &mut content)?;
        Ok(content)
    }

    fn update(&mut self, model: &Model) -> Result<()> {
        let mut content = self.content.take().expect("the island has a view");
        let before = (content.width, content.height);
        let result = self.update_content(model, &mut content);
        let resized = before != (content.width, content.height);
        self.content = Some(content);
        result?;
        if resized {
            self.morph()?;
        }
        Ok(())
    }

    fn update_content(&self, model: &Model, content: &mut Content) -> Result<()> {
        let s = self.scale;
        let own_clocks = model.mode == Mode::Countdown;
        for clock in content.clocks.iter_mut().filter(|_| !own_clocks) {
            let (label, hex) = timers::clock_label(model, clock.timer.as_deref());
            self.comp.set_text(&mut clock.text, &label, s)?;
            if let Some(hex) = hex {
                self.comp
                    .set_text_color(&mut clock.text, color(&hex, 1.0), s)?;
            }
        }
        match model.mode {
            Mode::Collapsed | Mode::Expanded | Mode::Alert => {
                self.update_timers(model.mode, content)?;
            }
            Mode::Player(_) => self.update_player(model, content)?,
            Mode::Menu => self.update_menu(model, content)?,
            Mode::RecordPanel(_) => self.update_panel(model, content)?,
            Mode::Card => self.update_card(model, content)?,
            Mode::Recording => self.update_recording(model, content)?,
            Mode::Countdown => self.update_countdown(model, content)?,
            Mode::Viewing => self.update_viewing(model, content)?,
            Mode::Downloading => self.update_downloading(model, content)?,
            Mode::Scan { .. } => self.update_scan(model, content)?,
            Mode::Indicator(..)
            | Mode::Shutter
            | Mode::Discarded
            | Mode::Problem { .. }
            | Mode::Update
            | Mode::System { .. }
            | Mode::Download { .. }
            | Mode::Device => {}
        }
        Ok(())
    }

    fn dots(&self, root: &ContainerVisual, width: f32, model: &Model) -> Result<()> {
        let Some((count, current)) = model.dots.filter(|(n, _)| *n > 1) else {
            return Ok(());
        };
        const SIZE: f32 = 5.0;
        const GAP: f32 = 6.0;
        let c = &self.comp.compositor;
        let total = count as f32 * SIZE + (count - 1) as f32 * GAP;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(total, SIZE))?;
        for i in 0..count {
            let geometry = c.CreateEllipseGeometry()?;
            let x = i as f32 * (SIZE + GAP) + SIZE / 2.0;
            geometry.SetCenter(v2(x, SIZE / 2.0))?;
            geometry.SetRadius(v2(SIZE / 2.0, SIZE / 2.0))?;
            let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
            let alpha = if i == current { 1.0 } else { 0.3 };
            shape.SetFillBrush(&self.comp.brush(color(TEXT, alpha))?)?;
            visual.Shapes()?.Append(&shape)?;
        }
        self.place(root, &visual, (width - total) / 2.0, DOTS_TOP)
    }

    fn place(
        &self,
        parent: &ContainerVisual,
        visual: &impl Interface,
        x: f32,
        y: f32,
    ) -> Result<()> {
        let visual: Visual = visual.cast()?;
        visual.SetOffset(v3(snap(x, self.scale), snap(y, self.scale)))?;
        parent.Children()?.InsertAtTop(&visual)
    }

    fn label(
        &self,
        parent: &ContainerVisual,
        content: &str,
        st: TextStyle,
        x: f32,
        y: f32,
    ) -> Result<Text> {
        let text = self.comp.text(content, st, self.scale)?;
        self.place(parent, &text.visual, x, y)?;
        Ok(text)
    }

    fn ring(&self, hex: &str, size: f32, stroke: f32) -> Result<Ring> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(size, size))?;
        visual.SetCenterPoint(v3(size / 2.0, size / 2.0))?;
        let center = v2(size / 2.0, size / 2.0);
        let radius = (size - stroke) / 2.0;

        let track_geometry = c.CreateEllipseGeometry()?;
        track_geometry.SetCenter(center)?;
        track_geometry.SetRadius(v2(radius, radius))?;
        let track = c.CreateSpriteShapeWithGeometry(&track_geometry)?;
        track.SetStrokeBrush(&self.comp.brush(color(hex, 0.25))?)?;
        track.SetStrokeThickness(stroke)?;

        let arc = c.CreateEllipseGeometry()?;
        arc.SetCenter(center)?;
        arc.SetRadius(v2(radius, radius))?;
        let arc_shape = c.CreateSpriteShapeWithGeometry(&arc)?;
        arc_shape.SetStrokeBrush(&self.comp.brush(color(hex, 1.0))?)?;
        arc_shape.SetStrokeThickness(stroke)?;
        arc_shape.SetStrokeStartCap(CompositionStrokeCap::Round)?;
        arc_shape.SetStrokeEndCap(CompositionStrokeCap::Round)?;
        // An ellipse path in Composition starts at 12 o'clock and goes clockwise: the remainder is an arc from 12
        // clockwise, its end moves 12 → 9 → 6 → 3.

        let shapes = visual.Shapes()?;
        shapes.Append(&track)?;
        shapes.Append(&arc_shape)?;
        Ok(Ring { visual, arc })
    }

    #[allow(clippy::too_many_arguments)]
    fn button(
        &self,
        parent: &ContainerVisual,
        label: &str,
        rect: Rect,
        hit: Hit,
        text_hex: &str,
        base: Color,
        hover: Color,
    ) -> Result<Button> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(rect.w, rect.h))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(rect.w, rect.h))?;
        geometry.SetCornerRadius(v2(rect.h / 2.0, rect.h / 2.0))?;
        let fill = self.comp.brush(base)?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetFillBrush(&fill)?;
        visual.Shapes()?.Append(&shape)?;
        self.place(parent, &visual, rect.x, rect.y)?;

        let st = style(12.0, true, text_hex);
        let width = self.comp.measure(label, st);
        let text = self.comp.text(label, st, self.scale)?;
        self.place(
            parent,
            &text.visual,
            rect.x + (rect.w - width) / 2.0,
            rect.y + (rect.h - text.height) / 2.0,
        )?;
        Ok(Button {
            fill,
            rect,
            hit,
            base,
            hover,
        })
    }
}

fn dots_space(model: &Model) -> f32 {
    if model.dots.is_some_and(|(n, _)| n > 1) {
        DOTS_SPACE
    } else {
        0.0
    }
}

fn content_key(model: &Model) -> String {
    match (model.mode, &model.card) {
        (Mode::Shutter, _) => return "Shutter".into(),
        // Everything in these changes in place: time, pause, hover, the countdown number.
        (Mode::Countdown | Mode::Recording | Mode::Discarded, _) => {
            return format!("{:?}", model.mode);
        }
        // What was decided about the file isn't in the key: the labels change in place.
        (Mode::Card, Some(card)) => return format!("Card|{:?}", Arc::as_ptr(card.thumb)),
        // Play or pause changes in place; a video's status and a screenshot's are built apart.
        (Mode::Viewing, _) => {
            return format!(
                "Viewing|{:?}|{}",
                model.viewing.map(Arc::as_ptr),
                model.viewing_play.is_some()
            );
        }
        (Mode::Update, _) => {
            let (look, title, reason) = model
                .update
                .as_ref()
                .map_or((UpdateLook::Checking, "", ""), |u| {
                    (u.look, u.title, u.reason)
                });
            return format!("Update|{look:?}|{title}|{reason}");
        }
        // A new notice, even the same kind with another percentage, plays its animation again.
        (Mode::System { .. }, _) => {
            let apps: Vec<&str> = model
                .system
                .as_ref()
                .map(|s| s.apps.iter().map(|a| a.name).collect())
                .unwrap_or_default();
            let (notice, decor) = model.system.as_ref().map_or((String::new(), false), |s| {
                (format!("{:?}", s.notice), s.decor)
            });
            return format!("{:?}|{notice}|{decor}|{}", model.mode, apps.join(","));
        }
        // How much has come changes in place; the name or the count is a new view.
        (Mode::Downloading, _) => {
            let coming = model.download.as_ref().and_then(|d| d.coming);
            let (files, name) = coming.map_or((0, None), |c| (c.files, c.name.as_deref()));
            return format!("Downloading|{files}|{name:?}");
        }
        // One more file is a new view: the count and the stack change.
        (Mode::Download { open }, _) => {
            let (names, decor): (Vec<&str>, bool) =
                model.download.as_ref().map_or((Vec::new(), false), |d| {
                    (d.files.iter().map(|f| f.name).collect(), d.decor)
                });
            return format!("Download|{open}|{decor}|{}", names.join("|"));
        }
        // A new notice plays its animation again.
        (Mode::Device, _) => {
            let (notice, decor) = model.device.as_ref().map_or((String::new(), false), |d| {
                (format!("{:?}", d.notice), d.decor)
            });
            return format!("Device|{notice}|{decor}");
        }
        // The text and the circles change in place; another language or the list is a new view.
        (Mode::Scan { list }, _) => {
            let language = model.scan.as_ref().map_or("", |s| s.language);
            let rows: Vec<String> = model.scan.as_ref().map_or(Vec::new(), |s| {
                s.list
                    .iter()
                    .map(|(name, on)| format!("{name}{on}"))
                    .collect()
            });
            return format!("Scan|{list}|{language}|{}", rows.join(","));
        }
        (Mode::Problem { .. }, _) => {
            let (critical, title, reason) = model
                .problem
                .as_ref()
                .map_or((false, "", ""), |p| (p.critical, p.title, p.reason));
            return format!("{:?}|{critical}|{title}|{reason}", model.mode);
        }
        _ => {}
    }
    if let Mode::RecordPanel(page) = model.mode {
        // Choices change in place; a new list of microphones or another area is a new page.
        let (devices, sound): (Vec<&str>, _) = model
            .panel
            .as_ref()
            .filter(|_| page == Page::Microphones)
            .map(|p| {
                (
                    p.devices.iter().map(|d| d.id.as_str()).collect(),
                    Some(p.choice.sound),
                )
            })
            .unwrap_or_default();
        let area = model.panel.as_ref().and_then(|p| p.area);
        return format!("{page:?}|{}|{sound:?}|{area:?}", devices.join(","));
    }
    if model.mode == Mode::Menu {
        // Button positions aren't in the key: a button flows in place.
        let ids: Vec<String> = model
            .controls
            .iter()
            .map(|i| format!("{:?}", i.id))
            .collect();
        let strip = model.strip.as_ref().map(strip::key).unwrap_or_default();
        return format!("Menu|{}|{strip}", ids.join(","));
    }
    if let (Mode::Player(_), Some(player)) = (model.mode, &model.player) {
        let s = player.session;
        return format!(
            "{:?}|{:?}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{}",
            model.mode,
            model.modes,
            s.key,
            s.title,
            s.artist,
            s.album,
            s.video,
            s.cover.as_ref().map(Arc::as_ptr),
            s.icon.as_ref().map(Arc::as_ptr),
            s.controls,
            model.dots,
            player.headphones,
        );
    }
    let snapshot = model.snapshot;
    let timers: Vec<String> = snapshot
        .timers
        .iter()
        .map(|t| format!("{}|{}|{}", t.id, t.name, t.color))
        .collect();
    let alert = snapshot
        .alert
        .as_ref()
        .map(|a| a.ids.join(","))
        .unwrap_or_default();
    format!(
        "{:?}|{:?}|{}|{}|{}|{:?}",
        model.mode,
        model.modes,
        snapshot.paused,
        timers.join(";"),
        alert,
        model.dots
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_explorer() {
        crate::lang::with(crate::lang::Lang::Ru, || {
            assert_eq!(format_size(0), "0.0 МБ");
            assert_eq!(format_size(4_404_019), "4.2 МБ");
            assert_eq!(format_size(14 * 1024 * 1024 + 300_000), "14 МБ");
            assert_eq!(format_size(1_288_490_189), "1.2 ГБ");
        });
        crate::lang::with(crate::lang::Lang::En, || {
            assert_eq!(format_size(0), "0.0 MB");
            assert_eq!(format_size(4_404_019), "4.2 MB");
            assert_eq!(format_size(14 * 1024 * 1024 + 300_000), "14 MB");
            assert_eq!(format_size(1_288_490_189), "1.2 GB");
        });
    }
}
