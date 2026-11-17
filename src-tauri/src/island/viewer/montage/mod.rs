//! Video editor: a mode of the video window. Edit swaps both panels: the editor's tools come in on
//! top (`tools.rs`), the timeline at the bottom (`timeline.rs`); the picture shows the edit as it
//! will be saved (`screen.rs`).
//!
//! - the caret is shared by everything: a click on the timeline or the ruler puts it there, the
//!   tools act on the fragment under it;
//! - scissors: a tool with its own yellow line; near the caret it snaps and the caret lights up;
//!   S cuts at the caret. The two edges part after the cut;
//! - a fragment is dragged to the start, the end or any join; dropped on the trash target it goes,
//!   as with Delete;
//! - the fragment's sound, speed (a row of steps) and zoom (a frame on the picture, smooth or at
//!   once; taken away apart). Without the strip the picture moves up between the panels while its
//!   zoom is set;
//! - a transition: the tool unfolds its kinds, one is dragged onto a join (the tool itself drags a
//!   dissolve); a click on a join's button picks its kind or none;
//! - full screen, as in the viewer;
//! - compression: a list of modes with their size estimates; saving is the next step of the editor;
//! - unsaved edits raise no dialog: the first Esc, Back or closing the window makes Save glint,
//!   the second discards them, as in the screenshot editor.
//!
//! Watching the edit is [`Program`] (`montage::watch`) over two Media Engines in turns; every edit
//! is a whole [`Montage`] kept for undo. The panels come at once: the engines open a moment later
//! (a spinner on the picture until the first frame), the thumbnails and the wave in the background
//! (a glint over the timeline until they come).

mod pictures;
mod pointer;
mod screen;
mod timeline;
mod tools;

pub use pictures::Pictures;

use super::super::comp::{Comp, Motion, color, v3};
use super::super::log;
use super::super::window::WM_DECK;
use super::Metrics;
use super::lens::Room;
use crate::capture::Image;
use crate::fault::Failed;
use crate::lang::tr;
use crate::montage::{
    Montage, SPEEDS,
    curve::{Frame, Transition, Zoom},
    squeeze::{self, FEATHER_BYTES, Squeeze},
    watch::Program,
};
use crate::play::{
    Engine, Event,
    adapters::{Note, Video},
    clock, clock_tenths,
};
use crate::shot::edit::History;
use pointer::Pointer;
use screen::{Framer, Grip, Screen};
use std::path::Path;
use std::time::Instant;
use timeline::{Hit, SPRING, Timeline};
use tools::{Anchor, Key, Menu, Owner, Pick, State, Tools};
use windows::{
    UI::Composition::ContainerVisual,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{
                VK_DELETE, VK_ESCAPE, VK_LEFT, VK_OEM_COMMA, VK_OEM_PERIOD, VK_RIGHT, VK_SPACE,
            },
            WindowsAndMessaging::{
                HCURSOR, IDC_CROSS, IDC_HAND, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENWSE, KillTimer,
                LoadCursorW, PostMessageW, SetTimer,
            },
        },
    },
    core::PCWSTR,
};

/// The editor's timers on the video window (the window's own are below 20).
pub const TIMER_TICK: usize = 20;
pub const TIMER_CLOCK: usize = 21;
pub const TIMER_DISARM: usize = 22;
pub const TIMER_RETIRE: usize = 23;
pub const TIMER_HINT: usize = 24;
pub const TIMER_OPEN: usize = 25;
pub const TIMER_READY: usize = 26;
/// The engines open once the editor's panels are on their way: their start doesn't hold them.
const OPEN_MS: u32 = 30;
/// After the first frame's seek: the engine presents it a moment later.
const READY_MS: u32 = 120;
/// Until the recording's own is read on the thumbnails' thread.
const DEFAULT_FPS: u32 = 30;
/// The editor's pictures fade in over the viewer's this long.
pub const REVEAL_MS: u32 = screen::REVEAL.ms;
/// A click on the transition tool shows the joins it can be dragged to this long.
const HINT_MS: u32 = 1200;
/// The editor's panels and picture coming and going.
pub const EDIT_MOTION: Motion = Motion::curve(320, 0.3, 0.0, 0.2, 1.0);
/// While playing the time (to a tenth) changes this often.
const CLOCK_MS: u32 = 100;
/// "Save" glints this long; a second Esc within it discards the edits.
const DISARM_MS: u32 = 4000;
/// The scissors snap to the caret this close (logical pixels).
const SNAP: f32 = 8.0;
/// A press on a block that moves this far becomes a drag.
const DRAG_START: f32 = 4.0;
const SKIP: f64 = 5.0;
/// A new zoom frame takes this share of the picture's side.
const FIRST_FRAME: f64 = 0.5;

/// An engine of the program: not open in the editor's first moment.
enum Deck {
    Waiting,
    Open(Video),
}

impl Deck {
    fn video(&self) -> Option<&Video> {
        match self {
            Deck::Open(video) => Some(video),
            Deck::Waiting => None,
        }
    }
}

impl Engine for Deck {
    fn play(&mut self) -> Result<(), Failed> {
        match self {
            Deck::Open(v) => v.play(),
            Deck::Waiting => Ok(()),
        }
    }
    fn pause(&mut self) -> Result<(), Failed> {
        match self {
            Deck::Open(v) => v.pause(),
            Deck::Waiting => Ok(()),
        }
    }
    fn seek(&mut self, to: f64, exact: bool) -> Result<(), Failed> {
        match self {
            Deck::Open(v) => v.seek(to, exact),
            Deck::Waiting => Ok(()),
        }
    }
    fn set_rate(&mut self, rate: f64) -> Result<(), Failed> {
        match self {
            Deck::Open(v) => v.set_rate(rate),
            Deck::Waiting => Ok(()),
        }
    }
    fn set_volume(&mut self, volume: f64, muted: bool) -> Result<(), Failed> {
        match self {
            Deck::Open(v) => v.set_volume(volume, muted),
            Deck::Waiting => Ok(()),
        }
    }
    fn step(&mut self, forward: bool) -> Result<(), Failed> {
        match self {
            Deck::Open(v) => v.step(forward),
            Deck::Waiting => Ok(()),
        }
    }
}

pub enum Effect {
    Exit,
    Full,
    Save(Order),
    Failed(Failed),
}

/// What to save: the edit, the file it becomes, and whether it is a copy beside the recording.
pub struct Order {
    pub montage: Montage,
    pub output: squeeze::Output,
    pub copy: bool,
}

pub struct Setup<'a> {
    pub path: &'a Path,
    pub full: (u32, u32),
    pub duration: f64,
    pub volume: (f64, bool),
    pub pictures: Pictures,
    /// Where the viewer stood: the edit starts there.
    pub at: f64,
    pub full_screen: bool,
}

enum Press {
    Menu {
        start: (f32, f32),
    },
    Tool,
    Play,
    Scrub {
        resume: bool,
    },
    Block {
        index: usize,
        start: (f32, f32),
        /// The pointer from the block's corner.
        grab: (f32, f32),
        dragging: bool,
    },
    Frame {
        grip: Grip,
        from: Frame,
        start: (f32, f32),
    },
    /// A transition, dragged onto a join once it moved.
    Chip {
        kind: Transition,
        start: (f32, f32),
        chip: Option<ContainerVisual>,
        hot: Option<usize>,
    },
}

pub struct MontageEditor {
    hwnd: HWND,
    /// The window root: the timeline and the popovers; the tools go into the top panel's place.
    host: ContainerVisual,
    top: ContainerVisual,
    m: Metrics,
    k: f32,
    size: (f32, f32),
    tools: Tools,
    timeline: Timeline,
    screen: Screen,
    framer: Framer,
    menu: Option<Menu>,
    program: Program<Deck>,
    path: std::path::PathBuf,
    base: Montage,
    history: History<Montage>,
    pictures: Pictures,
    source: squeeze::Source,
    squeeze: Squeeze,
    began: Instant,
    scissors: bool,
    /// The fragment whose zoom frame is being set.
    zooming: Option<usize>,
    press: Option<Press>,
    current: usize,
    armed: bool,
    full: bool,
    /// The decks show the edit: the viewer's own picture under it can go.
    ready: bool,
    scissors_pointer: Option<Pointer>,
    /// The share of the save done; `None` = not saving.
    saving: Option<f64>,
}

impl MontageEditor {
    /// The editor over the window's root `host`, its screen under `top` (the top panel, where its
    /// tools go).
    pub fn new(
        comp: &Comp,
        hwnd: HWND,
        (host, top): (&ContainerVisual, &ContainerVisual),
        (k, m): (f32, Metrics),
        size: (f32, f32),
        setup: Setup,
    ) -> Result<Self, Failed> {
        let base = Montage::new(setup.duration);
        let program = Program::new(
            [Deck::Waiting, Deck::Waiting],
            base.clone(),
            setup.at,
            setup.volume.0,
            setup.volume.1,
        );
        let bytes = std::fs::metadata(setup.path).map_or(0, |m| m.len());
        let source = squeeze::Source {
            width: setup.full.0,
            height: setup.full.1,
            fps: DEFAULT_FPS,
            bytes,
            duration: setup.duration,
        };

        let screen = Screen::new(comp, setup.full)?;
        let framer = Framer::new(comp, k)?;
        let children = host.Children()?;
        children.InsertBelow(&screen.root, top)?;
        children.InsertBelow(&framer.root, top)?;
        let timeline = Timeline::new(comp, k, m)?;
        children.InsertAtTop(&timeline.root)?;
        let state = State {
            scissors: false,
            muted: false,
            speed: 1.0,
            zooming: false,
            undo: false,
            redo: false,
            several: false,
            squeeze: Squeeze::AsIs,
            full: setup.full_screen,
            saving: false,
        };
        let tools = Tools::new(comp, k, m, size.0, state)?;
        top.Children()?.InsertAtTop(&tools.panel.root)?;
        tools.panel.set_shown(comp, false, m.slide_distance());
        tools.panel.slide(comp, true, m.slide_distance());

        let mut editor = Self {
            hwnd,
            host: host.clone(),
            top: top.clone(),
            m,
            k,
            size,
            tools,
            timeline,
            screen,
            framer,
            menu: None,
            program,
            base,
            history: History::default(),
            pictures: setup.pictures,
            path: setup.path.to_path_buf(),
            source,
            squeeze: Squeeze::AsIs,
            began: Instant::now(),
            scissors: false,
            zooming: None,
            press: None,
            current: 0,
            armed: false,
            full: setup.full_screen,
            ready: false,
            scissors_pointer: scissors_pointer(k),
            saving: None,
        };
        editor.resize(comp, size);
        let loading = editor.pictures.thumbs.is_none();
        editor.timeline.set_loading(comp, loading);
        editor.start_timer(TIMER_OPEN, OPEN_MS);
        Ok(editor)
    }

    /// The engines open: the program starts once both have loaded.
    fn open_decks(&mut self) -> Result<(), Failed> {
        let deck = |d: usize| {
            let address = self.hwnd.0 as isize;
            Video::open(&self.path, move |note: Note| {
                let (wparam, lparam) = note.pack();
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(address as *mut _)),
                        WM_DECK + d as u32,
                        WPARAM(wparam),
                        LPARAM(lparam),
                    );
                }
            })
        };
        let decks = [Deck::Open(deck(0)?), Deck::Open(deck(1)?)];
        *self.program.decks_mut() = decks;
        Ok(())
    }

    fn fps(&self) -> u32 {
        self.pictures.fps.unwrap_or(DEFAULT_FPS)
    }

    /// The editor opens: the timeline comes up into its place.
    pub fn arrive(&self, comp: &Comp) {
        self.timeline.arrive(comp, EDIT_MOTION);
    }

    /// The panels leave: the editor closes. The window keeps it until they are gone (its engines
    /// too: the picture fades out instead of going black), then drops it. Returns the timeline's
    /// pictures for the next time and the recording's moment under the caret.
    pub fn retire(&mut self, comp: &Comp) -> (Pictures, f64) {
        self.close_menu();
        self.framer.hide();
        self.tools.panel.slide(comp, false, self.m.slide_distance());
        self.timeline.depart(comp, EDIT_MOTION);
        self.screen.set_shown(comp, false, EDIT_MOTION);
        let now = self.now();
        let _ = self.program.pause(now);
        for id in [
            TIMER_TICK,
            TIMER_CLOCK,
            TIMER_DISARM,
            TIMER_RETIRE,
            TIMER_HINT,
            TIMER_OPEN,
            TIMER_READY,
        ] {
            self.stop_timer(id);
        }
        let source = self.montage().at(self.program.position(now)).1;
        (std::mem::take(&mut self.pictures), source)
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn set_full(&mut self, comp: &Comp, full: bool) {
        self.full = full;
        self.refresh(comp);
    }

    /// The picture's place in the window now, for the window's own animations.
    pub fn picture(&self) -> Room {
        self.screen.picture()
    }

    /// Where the picture will lie in a window of `size`, in that window's coordinates.
    pub fn picture_in(&self, size: (f32, f32)) -> Room {
        self.screen.fitted(self.picture_area(size))
    }

    pub fn glide_from(&self, comp: &Comp, from: Room, motion: Motion) {
        log(self.screen.glide_from(comp, from, motion));
    }

    pub fn glide_to(&self, comp: &Comp, to: Room, motion: Motion) {
        log(self.screen.glide_to(comp, to, motion));
    }

    /// While full screen comes or goes the timeline stays away.
    pub fn set_panels(&self, comp: &Comp, shown: bool) {
        if shown {
            self.timeline.arrive(comp, EDIT_MOTION);
        } else {
            log(self.timeline.root.SetOpacity(0.0));
        }
    }

    pub fn min_width(&self) -> f32 {
        self.tools.min_width()
    }

    fn now(&self) -> u64 {
        self.began.elapsed().as_millis() as u64
    }

    fn montage(&self) -> &Montage {
        self.history.done().last().unwrap_or(&self.base)
    }

    pub fn is_dirty(&self) -> bool {
        self.montage() != &self.base
    }

    pub fn running(&self) -> bool {
        self.program.running()
    }

    pub fn area(m: Metrics, size: (f32, f32)) -> Room {
        if m.floating() {
            (0.0, 0.0, size.0, size.1)
        } else {
            let h = size.1 - m.strip - Timeline::band(m);
            (0.0, m.strip, size.0, h.max(1.0))
        }
    }

    pub fn band(m: Metrics) -> f32 {
        Timeline::band(m)
    }

    /// Without the strip the panels lie on the picture: while a zoom is set the picture moves up
    /// between them, so all of it can be framed.
    fn picture_area(&self, size: (f32, f32)) -> Room {
        let m = self.m;
        if self.zooming.is_none() || !m.floating() {
            return Self::area(m, size);
        }
        let top = m.margin + m.button.1 + m.margin;
        let bottom = Timeline::band(m) + m.margin;
        (0.0, top, size.0, (size.1 - top - bottom).max(1.0))
    }

    pub fn resize(&mut self, comp: &Comp, size: (f32, f32)) {
        self.size = size;
        log(self.screen.layout(comp, self.picture_area(size)));
        log(self.timeline.resize(comp, size));
        self.tools.resize(size.0);
        self.close_menu();
        if let Some(i) = self.zooming {
            let frame = self.framer.frame();
            self.zooming = None;
            self.zoom_on(comp, i, Some(frame));
        }
        self.resync(comp, None);
    }

    /// The window's metrics or scale changed: the panels are built again.
    pub fn restyle(&mut self, comp: &Comp, k: f32, m: Metrics) {
        self.k = k;
        self.m = m;
        self.close_menu();
        self.scissors_pointer = scissors_pointer(k);
        let state = self.state();
        match Tools::new(comp, k, m, self.size.0, state) {
            Ok(tools) => {
                if let Ok(children) = self.top.Children() {
                    let _ = children.Remove(&self.tools.panel.root);
                    let _ = children.InsertAtTop(&tools.panel.root);
                }
                self.tools = tools;
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)),
        }
        match Timeline::new(comp, k, m) {
            Ok(timeline) => {
                if let Ok(children) = self.host.Children() {
                    let _ = children.Remove(&self.timeline.root);
                    let _ = children.InsertAtTop(&timeline.root);
                }
                self.timeline = timeline;
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)),
        }
        if let Ok(framer) = Framer::new(comp, k) {
            if let Ok(children) = self.host.Children() {
                let _ = children.InsertAbove(&framer.root, &self.framer.root);
                let _ = children.Remove(&self.framer.root);
            }
            self.framer = framer;
        }
        self.resize(comp, self.size);
    }

    fn state(&self) -> State {
        let montage = self.montage();
        let f = montage.fragments()[self.current.min(montage.fragments().len() - 1)];
        State {
            scissors: self.scissors,
            muted: f.muted,
            speed: f.speed,
            zooming: self.zooming.is_some(),
            undo: self.history.is_dirty(),
            redo: self.history.can_redo(),
            several: montage.fragments().len() > 1,
            squeeze: self.squeeze,
            full: self.full,
            saving: self.saving.is_some(),
        }
    }

    /// What Save (or Save copy) writes. An edit that changes nothing has nothing to save.
    fn order(&self, copy: bool) -> Option<Effect> {
        if !self.is_dirty() {
            return Some(Effect::Exit);
        }
        let montage = self.montage().clone();
        let source = squeeze::Source {
            fps: self.fps(),
            ..self.source
        };
        let output = squeeze::plan(self.squeeze, &source, montage.duration());
        Some(Effect::Save(Order {
            montage,
            output,
            copy,
        }))
    }

    /// The save moved on: Save fills; `None` = it is over.
    pub fn set_saving(&mut self, comp: &Comp, share: Option<f64>) {
        self.disarm(comp);
        let started = self.saving.is_none() && share.is_some();
        self.saving = share;
        if started {
            self.close_menu();
            self.zoom_off(comp);
            self.scissors = false;
            self.timeline.scissors(comp, None, false);
        }
        if let Some(save) = self.tools.panel.get_mut(Key::Save) {
            save.set_progress(comp, share.map(|s| s as f32), color("#FFFFFF", 0.35));
        }
        self.refresh(comp);
    }

    /// The save failed: the edits are still here, Save glints.
    pub fn save_failed(&mut self, comp: &Comp) {
        self.set_saving(comp, None);
        self.arm(comp);
    }

    /// The engines let the recording go: it is about to be replaced by the saved edit.
    pub fn close_engines(&self) {
        for deck in self.program.decks() {
            if let Some(video) = deck.video() {
                video.close();
            }
        }
    }

    /// The timeline laid out again for the montage shown, then everything that follows it.
    fn resync(&mut self, comp: &Comp, motion: Option<super::super::comp::Motion>) {
        let montage = self.montage().clone();
        log(self
            .timeline
            .sync(comp, &montage, &self.pictures, self.current, motion));
        self.refresh(comp);
    }

    /// Everything the program's state shows: the pictures, the caret, the time, the tools.
    fn refresh(&mut self, comp: &Comp) {
        let now = self.now();
        if self.ready {
            let looks = self.program.looks(now);
            self.screen.show(comp, &looks, self.zooming.is_some());
        }
        let at = self.program.position(now);
        let x = self.timeline.x_of(at);
        let ms = self
            .program
            .next_change(now)
            .map(|when| (when - now).max(1) as u32);
        let run = ms.map(|ms| {
            let then = at + f64::from(ms) / 1000.0;
            (self.timeline.caret_path(at, then), ms)
        });
        self.timeline.caret(comp, x, run);
        match ms {
            Some(ms) => self.start_timer(TIMER_TICK, ms),
            None => self.stop_timer(TIMER_TICK),
        }
        if self.program.running() {
            self.start_timer(TIMER_CLOCK, CLOCK_MS);
        } else {
            self.stop_timer(TIMER_CLOCK);
        }
        self.draw_time(comp);
        let current = self.program.current(now);
        if current != self.current {
            self.current = current;
            self.timeline.set_current(current);
        }
        log(self.timeline.set_running(comp, self.program.running()));
        let state = self.state();
        self.tools.set_state(comp, state);
    }

    fn draw_time(&mut self, comp: &Comp) {
        let now = self.now();
        let d = self.program.duration();
        let at = self.program.position(now);
        log(self
            .timeline
            .set_times(comp, &clock_tenths(at, d), &clock(d, d)));
    }

    fn start_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), id, ms, None);
        }
    }

    fn stop_timer(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
    }

    fn done(&mut self, comp: &Comp, result: Result<(), Failed>) -> Option<Effect> {
        self.refresh(comp);
        result.err().map(Effect::Failed)
    }

    pub fn on_deck(
        &mut self,
        comp: &Comp,
        deck: usize,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<Effect> {
        let note = Note::unpack(wparam.0, lparam.0);
        let event = self.program.decks()[deck].video()?.event(note)?;
        if matches!(event, Event::Loaded(_)) {
            let attached = self.program.decks()[deck]
                .video()?
                .swap_chain()
                .map_err(Failed::from)
                .and_then(|handle| Ok(self.screen.attach(comp, deck, handle)?));
            if let Err(failed) = attached {
                return Some(Effect::Failed(failed));
            }
        }
        let seeked = matches!(event, Event::Seeked(_));
        let result = self.program.on_event(deck, event);
        // The first frame of the caret's fragment is on its deck: the edit can be shown.
        if seeked
            && !self.ready
            && self.program.phase() != crate::montage::watch::Phase::Loading
            && deck == self.program.current(self.now()) % 2
        {
            self.start_timer(TIMER_READY, READY_MS);
        }
        self.done(comp, result)
    }

    /// The timeline's pictures arrived from their threads.
    pub fn on_pictures(&mut self, comp: &Comp) {
        let (changed, failures) = self.pictures.take();
        self.timeline
            .set_loading(comp, self.pictures.thumbs.is_none());
        for failed in failures {
            crate::journal::warn(tr!(
                "journal.timeline_pictures_failed",
                error = failed.detail
            ));
        }
        if changed {
            self.resync(comp, None);
        }
    }

    pub fn toggle(&mut self, comp: &Comp) -> Option<Effect> {
        let now = self.now();
        let result = self.program.toggle(now);
        self.done(comp, result)
    }

    pub fn pause(&mut self, comp: &Comp) {
        let now = self.now();
        let _ = self.program.pause(now);
        self.refresh(comp);
    }

    /// The frame on screen, of the fragment under the caret.
    pub fn frame(&self) -> Option<Image> {
        let deck = self.current % 2;
        self.program.decks()[deck].video()?.frame().ok()
    }

    fn seek(&mut self, comp: &Comp, to: f64, exact: bool) -> Option<Effect> {
        let now = self.now();
        let result = self.program.seek(to, exact, now);
        self.done(comp, result)
    }

    /// A new edit: kept for undo, shown and watched from the same moment.
    fn edit(&mut self, comp: &Comp, montage: Montage) -> Option<Effect> {
        self.disarm(comp);
        self.history.push(montage);
        self.show_edit(comp)
    }

    /// The caret stays on the recording's moment it was on, wherever the edit puts it.
    fn show_edit(&mut self, comp: &Comp) -> Option<Effect> {
        let now = self.now();
        let was = self.program.position(now);
        let (_, source) = self.program.montage().at(was);
        let montage = self.montage().clone();
        let at = montage
            .fragments()
            .iter()
            .position(|f| f.from <= source && source < f.to)
            .map_or(was, |i| montage.result_of(i, source));
        let result = self.program.set_montage(montage, at, now);
        self.current = self.program.current(now);
        self.resync(comp, Some(SPRING));
        result.err().map(Effect::Failed)
    }

    fn undo(&mut self, comp: &Comp) -> Option<Effect> {
        self.disarm(comp);
        self.history.undo()?;
        self.close_menu();
        self.zoom_off(comp);
        self.show_edit(comp)
    }

    fn redo(&mut self, comp: &Comp) -> Option<Effect> {
        self.disarm(comp);
        self.history.redo()?;
        self.close_menu();
        self.zoom_off(comp);
        self.show_edit(comp)
    }

    fn cut(&mut self, comp: &Comp, at: f64) -> Option<Effect> {
        let mut montage = self.montage().clone();
        if !montage.split(at) {
            return None;
        }
        let x = self.timeline.x_of(at);
        log(self.timeline.cut_at(comp, x));
        self.start_timer(TIMER_RETIRE, timeline::RETIRE_MS);
        self.edit(comp, montage)
    }

    fn set_transition(
        &mut self,
        comp: &Comp,
        join: usize,
        transition: Option<Transition>,
    ) -> Option<Effect> {
        let mut montage = self.montage().clone();
        if !montage.set_transition(join, transition) {
            return None;
        }
        self.edit(comp, montage)
    }

    fn remove(&mut self, comp: &Comp, index: usize) -> Option<Effect> {
        let mut montage = self.montage().clone();
        if !montage.remove(index) {
            return None;
        }
        self.timeline.fly_away(comp, index);
        self.start_timer(TIMER_RETIRE, timeline::RETIRE_MS);
        if self.zooming == Some(index) {
            self.zoom_off(comp);
        }
        self.edit(comp, montage)
    }

    fn close_menu(&mut self) {
        if let Some(menu) = self.menu.take() {
            menu.remove(&self.host);
        }
    }

    fn open_menu(&mut self, comp: &Comp, menu: windows::core::Result<Menu>) {
        self.close_menu();
        match menu.and_then(|menu| menu.show(comp, &self.host).map(|_| menu)) {
            Ok(menu) => self.menu = Some(menu),
            Err(e) => log(Err(e)),
        }
    }

    fn anchor(&self, key: Key) -> Anchor {
        Anchor::Below(self.tools.anchor(key).unwrap_or((0.0, 0.0, 0.0, 0.0)))
    }

    /// The zoom frame of fragment `index` on the whole picture, the program paused on it.
    fn zoom_on(&mut self, comp: &Comp, index: usize, frame: Option<Frame>) {
        let now = self.now();
        let _ = self.program.pause(now);
        let f = self.montage().fragments()[index];
        let frame = frame
            .or(f.zoom.map(|z| z.frame))
            .unwrap_or_else(|| Frame::around((0.5, 0.5), FIRST_FRAME));
        self.zooming = Some(index);
        self.move_picture(comp);
        self.framer.show(self.screen.picture(), frame);
        self.refresh(comp);
    }

    /// The picture area changed with the zoom being set: the picture glides to its new place.
    fn move_picture(&mut self, comp: &Comp) {
        let from = self.screen.picture();
        log(self.screen.layout(comp, self.picture_area(self.size)));
        if from != self.screen.picture() {
            log(self.screen.glide_from(comp, from, EDIT_MOTION));
        }
    }

    fn zoom_off(&mut self, comp: &Comp) {
        if self.zooming.take().is_some() {
            self.framer.hide();
            self.move_picture(comp);
            if matches!(
                self.menu.as_ref().map(|m| m.owner),
                Some(Owner::Tool(Key::Zoom))
            ) {
                self.close_menu();
            }
            self.refresh(comp);
        }
    }

    fn zoom_menu(&mut self, comp: &Comp) {
        let smooth = self
            .zooming
            .and_then(|i| self.montage().fragments()[i].zoom)
            .map(|z| z.smooth);
        let menu = Menu::zoom(
            comp,
            self.k,
            self.m,
            self.anchor(Key::Zoom),
            self.size.0,
            smooth,
        );
        self.open_menu(comp, menu);
    }

    /// The zoom being set gets the frame on the picture; `smooth: None` keeps its kind.
    fn set_zoom(&mut self, comp: &Comp, smooth: Option<bool>) -> Option<Effect> {
        let index = self.zooming?;
        let mut montage = self.montage().clone();
        let kind = montage.fragments()[index].zoom.is_none_or(|z| z.smooth);
        let zoom = Zoom {
            frame: self.framer.frame(),
            smooth: smooth.unwrap_or(kind),
        };
        if !montage.set_zoom(index, Some(zoom)) {
            return None;
        }
        self.edit(comp, montage)
    }

    fn transition_menu(&mut self, comp: &Comp, join: usize, anchor: Anchor) {
        let current = self.montage().joins().get(join).copied().flatten();
        let menu = Menu::transitions(
            comp,
            self.k,
            self.m,
            (Owner::Join(join), anchor),
            self.size.0,
            current,
        );
        self.open_menu(comp, menu);
    }

    fn squeeze_menu(&mut self, comp: &Comp) {
        let result = self.montage().duration();
        let sizes = Squeeze::ALL.map(|s| {
            let source = squeeze::Source {
                fps: self.fps(),
                ..self.source
            };
            let out = squeeze::plan(s, &source, result);
            let size = super::super::view::format_size(out.bytes);
            Some(
                if s == Squeeze::Feather && out.fits && out.bytes * 10 >= FEATHER_BYTES * 9 {
                    tr!(
                        "editor.up_to",
                        size = super::super::view::format_size(FEATHER_BYTES)
                    )
                    .into_owned()
                } else {
                    tr!("editor.about", size = size).into_owned()
                },
            )
        });
        let menu = Menu::squeezes(
            comp,
            self.k,
            self.m,
            self.anchor(Key::Squeeze),
            self.size.0,
            (self.squeeze, sizes),
        );
        self.open_menu(comp, menu);
    }

    /// The pressable part of the window; the rest moves it.
    pub fn wants(&self, p: (f32, f32)) -> bool {
        self.menu.as_ref().is_some_and(|m| m.contains(p))
            || self.tools.panel.at(p).is_some()
            || self.timeline.contains(p)
            || self.framer.on_picture(p)
            || self.press.is_some()
    }

    pub fn cursor(&self, p: (f32, f32)) -> Option<HCURSOR> {
        let system = |shape: PCWSTR| unsafe { LoadCursorW(None, shape).ok() };
        if let Some(menu) = &self.menu
            && menu.contains(p)
        {
            return menu.panel.enabled_at(p).and_then(|_| system(IDC_HAND));
        }
        match &self.press {
            Some(Press::Frame { grip, .. }) => return system(grip_cursor(*grip)),
            Some(Press::Block { dragging: true, .. }) => return system(IDC_SIZEALL),
            Some(Press::Chip { chip: Some(_), .. }) => return system(IDC_HAND),
            _ => {}
        }
        if self.tools.panel.at(p).is_some() {
            return self
                .tools
                .panel
                .enabled_at(p)
                .and_then(|_| system(IDC_HAND));
        }
        if let Some(grip) = self.framer.grip_at(p) {
            return system(grip_cursor(grip));
        }
        if !self.timeline.contains(p) {
            return None;
        }
        match self.timeline.hit(self.timeline.local(p)) {
            Some(Hit::Block(_) | Hit::Ruler) if self.scissors => self
                .scissors_pointer
                .as_ref()
                .map(|s| s.0)
                .or_else(|| system(IDC_CROSS)),
            Some(_) => system(IDC_HAND),
            None => None,
        }
    }

    pub fn moved(&mut self, comp: &Comp, p: (f32, f32)) -> Option<Effect> {
        let local = self.timeline.local(p);
        match &mut self.press {
            Some(Press::Scrub { .. }) => {
                // Exact frames: a recording's keyframes are 2 s apart, the nearest one jumps.
                let to = self.timeline.result_at(local.0);
                return self.seek(comp, to, true);
            }
            Some(Press::Block {
                index,
                start,
                grab,
                dragging,
            }) => {
                let far = (local.0 - start.0).abs().max((local.1 - start.1).abs()) > DRAG_START;
                if !*dragging && far {
                    *dragging = true;
                    self.timeline.lift(comp, *index);
                }
                if *dragging {
                    self.timeline.drag(comp, local.0 - grab.0, local.1 - grab.1);
                }
                return None;
            }
            Some(Press::Frame { grip, from, start }) => {
                let delta = (p.0 - start.0, p.1 - start.1);
                let (grip, from) = (*grip, *from);
                self.framer.dragged(grip, from, delta);
                return None;
            }
            Some(Press::Menu { start }) => {
                let far = (p.0 - start.0).abs().max((p.1 - start.1).abs()) > DRAG_START;
                let kind = match self.menu.as_ref().and_then(|m| m.panel.pressed()) {
                    Some(Pick::Kind(kind)) if far => kind,
                    _ => return None,
                };
                // A kind taken out of the row: the row goes, the kind follows the cursor.
                let start = *start;
                self.close_menu();
                self.press = Some(Press::Chip {
                    kind,
                    start,
                    chip: None,
                    hot: None,
                });
                return self.moved(comp, p);
            }
            Some(Press::Chip {
                start, chip, hot, ..
            }) => {
                let far = (p.0 - start.0).abs().max((p.1 - start.1).abs()) > DRAG_START;
                let fresh = chip.is_none() && far;
                if fresh {
                    self.tools.panel.cancel(comp);
                    *chip = timeline::chip(comp, self.k).ok();
                    if let Some(c) = chip.as_ref()
                        && let Ok(children) = self.host.Children()
                    {
                        let _ = children.InsertAtTop(c);
                    }
                }
                let c = chip.as_ref()?;
                let half = timeline::CHIP / 2.0;
                log(c.SetOffset(v3(p.0 - half, p.1 - half)));
                let near = self.timeline.join_near(local);
                if near != *hot || fresh {
                    *hot = near;
                    let montage = self.montage().clone();
                    self.timeline.targets(comp, Some(near), &montage);
                }
                return None;
            }
            Some(_) => return None,
            None => {}
        }
        if let Some(menu) = &mut self.menu {
            menu.panel.hover(comp, Some(p));
        }
        self.tools.panel.hover(comp, Some(p));
        self.timeline.hover(comp, Some(local));
        let hit = self
            .timeline
            .contains(p)
            .then(|| self.timeline.hit(local))
            .flatten();
        let join = match hit {
            Some(Hit::Join(j)) => Some(j),
            _ => None,
        };
        let montage = self.montage().clone();
        self.timeline.hover_join(join, &montage);
        let cutting = self.scissors && matches!(hit, Some(Hit::Block(_) | Hit::Ruler));
        let caret = self.timeline.x_of(self.program.position(self.now()));
        let snapped = (local.0 - caret).abs() <= SNAP;
        self.timeline
            .scissors(comp, cutting.then_some(local.0), snapped);
        None
    }

    /// The cursor left the window or went onto its "title".
    pub fn left(&mut self, comp: &Comp) {
        if self.press.is_some() {
            return;
        }
        if let Some(menu) = &mut self.menu {
            menu.panel.hover(comp, None);
        }
        self.tools.panel.hover(comp, None);
        self.timeline.hover(comp, None);
        self.timeline.scissors(comp, None, false);
        let montage = self.montage().clone();
        self.timeline.hover_join(None, &montage);
    }

    /// A left press at window point `p`. True = the editor took it, the window captures the mouse.
    pub fn press(&mut self, comp: &Comp, p: (f32, f32)) -> (bool, Option<Effect>) {
        self.disarm(comp);
        if let Some(menu) = &mut self.menu {
            if menu.panel.press(comp, p) {
                self.press = Some(Press::Menu { start: p });
                return (true, None);
            }
            if menu.contains(p) {
                return (true, None);
            }
            let owner = menu.owner;
            let on_framer = self.framer.grip_at(p).is_some();
            match owner {
                // The zoom's frame is set with its popover open.
                Owner::Tool(Key::Zoom) if on_framer => {}
                Owner::Tool(key) if self.tools.panel.at(p) == Some(key) => {
                    self.close_menu();
                    if key == Key::Zoom {
                        self.zoom_off(comp);
                    }
                    return (true, None);
                }
                _ => {
                    self.close_menu();
                    if owner == Owner::Tool(Key::Zoom) {
                        self.zoom_off(comp);
                    }
                }
            }
        }
        if self.tools.panel.press(comp, p) {
            self.press = Some(if self.tools.panel.at(p) == Some(Key::Transition) {
                Press::Chip {
                    kind: Transition::Dissolve,
                    start: p,
                    chip: None,
                    hot: None,
                }
            } else {
                Press::Tool
            });
            return (true, None);
        }
        if let Some(grip) = self.framer.grip_at(p) {
            self.press = Some(Press::Frame {
                grip,
                from: self.framer.frame(),
                start: p,
            });
            return (true, None);
        }
        if self.zooming.is_some() && self.framer.on_picture(p) {
            return (true, None);
        }
        if !self.timeline.contains(p) {
            return (false, None);
        }
        let local = self.timeline.local(p);
        let effect = match self.timeline.hit(local) {
            Some(Hit::Play) => {
                self.timeline.press_play(comp, local);
                self.press = Some(Press::Play);
                None
            }
            Some(Hit::Ruler) => {
                let resume = self.program.running();
                let now = self.now();
                let _ = self.program.pause(now);
                self.press = Some(Press::Scrub { resume });
                let to = self.timeline.result_at(local.0);
                self.seek(comp, to, true)
            }
            Some(Hit::Block(_) | Hit::Join(_)) if self.saving.is_some() => {
                let to = self.timeline.result_at(local.0);
                self.seek(comp, to, true)
            }
            Some(Hit::Block(index)) if self.scissors => {
                let now = self.now();
                let caret = self.program.position(now);
                let snapped = (local.0 - self.timeline.x_of(caret)).abs() <= SNAP;
                let at = if snapped {
                    caret
                } else {
                    self.timeline.result_at(local.0)
                };
                let _ = index;
                self.cut(comp, at)
            }
            Some(Hit::Block(index)) => {
                let x = self.timeline.x_of(self.montage().starts()[index]);
                self.press = Some(Press::Block {
                    index,
                    start: local,
                    grab: (local.0 - x, local.1 - timeline::BLOCKS_Y),
                    dragging: false,
                });
                None
            }
            Some(Hit::Join(join)) => {
                let anchor = Anchor::Above(self.timeline.join_anchor(join));
                self.transition_menu(comp, join, anchor);
                None
            }
            None => None,
        };
        (true, effect)
    }

    pub fn release(&mut self, comp: &Comp, p: (f32, f32)) -> Option<Effect> {
        let local = self.timeline.local(p);
        match self.press.take()? {
            Press::Menu { .. } => {
                let pick = self.menu.as_mut()?.panel.release(comp, p)?;
                self.pick(comp, pick)
            }
            Press::Tool => {
                let key = self.tools.panel.release(comp, p)?;
                self.click(comp, key)
            }
            Press::Play => {
                if self.timeline.release_play(comp, local) {
                    self.toggle(comp)
                } else {
                    None
                }
            }
            Press::Scrub { resume } => {
                let to = self.timeline.result_at(local.0);
                let effect = self.seek(comp, to, true);
                if resume {
                    let now = self.now();
                    let result = self.program.play(now);
                    return self.done(comp, result).or(effect);
                }
                effect
            }
            Press::Block {
                index,
                dragging: false,
                ..
            } => {
                let _ = index;
                let to = self.timeline.result_at(local.0);
                self.seek(comp, to, true)
            }
            Press::Block { index, grab, .. } => {
                let left = local.0 - grab.0;
                let span = (left, left + self.block_width(index));
                if self.timeline.over_trash(span, local.1) && self.montage().fragments().len() > 1 {
                    self.timeline.land(comp);
                    return self.remove(comp, index);
                }
                let slot = self.timeline.drag(comp, local.0 - grab.0, local.1 - grab.1);
                self.timeline.land(comp);
                let target = if slot < index { slot } else { slot + 1 };
                let mut montage = self.montage().clone();
                if montage.move_to(index, target) {
                    self.edit(comp, montage)
                } else {
                    self.resync(comp, Some(SPRING));
                    None
                }
            }
            Press::Frame { .. } => {
                let effect = self.set_zoom(comp, None);
                self.zoom_menu(comp);
                effect
            }
            Press::Chip { chip: None, .. } => {
                let key = self.tools.panel.release(comp, p)?;
                self.click(comp, key)
            }
            Press::Chip {
                kind,
                chip: Some(chip),
                hot,
                ..
            } => {
                self.drop_chip(comp, &chip);
                self.set_transition(comp, hot?, Some(kind))
            }
        }
    }

    fn drop_chip(&mut self, comp: &Comp, chip: &ContainerVisual) {
        if let Ok(children) = self.host.Children() {
            let _ = children.Remove(chip);
        }
        let montage = self.montage().clone();
        self.timeline.targets(comp, None, &montage);
    }

    fn block_width(&self, index: usize) -> f32 {
        let montage = self.montage();
        let starts = montage.starts();
        let end = starts[index] + montage.fragments()[index].length();
        self.timeline.x_of(end) - self.timeline.x_of(starts[index])
    }

    pub fn cancel(&mut self, comp: &Comp) {
        self.tools.panel.cancel(comp);
        self.timeline.cancel(comp);
        if let Some(menu) = &mut self.menu {
            menu.panel.cancel(comp);
        }
        match self.press.take() {
            Some(Press::Block { dragging: true, .. }) => {
                self.timeline.land(comp);
                self.resync(comp, Some(SPRING));
            }
            Some(Press::Frame { from, .. }) => {
                if let Some(i) = self.zooming {
                    self.zooming = None;
                    self.zoom_on(comp, i, Some(from));
                }
            }
            Some(Press::Chip {
                chip: Some(chip), ..
            }) => self.drop_chip(comp, &chip),
            _ => {}
        }
    }

    fn click(&mut self, comp: &Comp, key: Key) -> Option<Effect> {
        if self.saving.is_some() && !matches!(key, Key::Back | Key::Full) {
            return None;
        }
        if key != Key::Zoom {
            self.zoom_off(comp);
        }
        self.close_menu();
        let i = self.current;
        match key {
            Key::Back => self.leave(comp),
            Key::Undo => self.undo(comp),
            Key::Redo => self.redo(comp),
            Key::Full => Some(Effect::Full),
            Key::Scissors => {
                self.scissors = !self.scissors;
                if !self.scissors {
                    self.timeline.scissors(comp, None, false);
                }
                self.refresh(comp);
                None
            }
            Key::Sound => {
                let mut montage = self.montage().clone();
                let muted = montage.fragments()[i].muted;
                montage.set_muted(i, !muted);
                self.edit(comp, montage)
            }
            Key::Speed => {
                let speed = self.montage().fragments()[i].speed;
                let menu = Menu::speeds(
                    comp,
                    self.k,
                    self.m,
                    self.anchor(Key::Speed),
                    self.size.0,
                    speed,
                );
                self.open_menu(comp, menu);
                None
            }
            Key::Zoom => {
                if self.zooming.is_some() {
                    self.zoom_off(comp);
                } else {
                    self.zoom_on(comp, i, None);
                    self.zoom_menu(comp);
                }
                None
            }
            Key::Transition => {
                let menu = Menu::kinds(
                    comp,
                    self.k,
                    self.m,
                    self.anchor(Key::Transition),
                    self.size.0,
                );
                self.open_menu(comp, menu);
                None
            }
            Key::Squeeze => {
                self.squeeze_menu(comp);
                None
            }
            Key::Save => self.order(false),
            Key::SaveCopy => self.order(true),
        }
    }

    fn pick(&mut self, comp: &Comp, pick: Pick) -> Option<Effect> {
        match pick {
            Pick::Speed(step) => {
                self.close_menu();
                let mut montage = self.montage().clone();
                montage.set_speed(self.current, SPEEDS[step]);
                self.edit(comp, montage)
            }
            Pick::Smooth | Pick::AtOnce => {
                let effect = self.set_zoom(comp, Some(pick == Pick::Smooth));
                self.zoom_menu(comp);
                effect
            }
            Pick::Unzoom => {
                let index = self.zooming?;
                self.zoom_off(comp);
                let mut montage = self.montage().clone();
                montage.set_zoom(index, None);
                self.edit(comp, montage)
            }
            Pick::Transition(transition) => {
                let Some(Owner::Join(join)) = self.menu.as_ref().map(|m| m.owner) else {
                    return None;
                };
                self.close_menu();
                self.set_transition(comp, join, transition)
            }
            // A click on a kind shows where it can be dragged.
            Pick::Kind(_) => {
                let montage = self.montage().clone();
                self.timeline.targets(comp, Some(None), &montage);
                self.start_timer(TIMER_HINT, HINT_MS);
                None
            }
            Pick::Squeeze(squeeze) => {
                self.close_menu();
                self.squeeze = squeeze;
                self.refresh(comp);
                None
            }
        }
    }

    pub fn key(&mut self, comp: &Comp, key: u16, ctrl: bool, shift: bool) -> Option<Effect> {
        let letter = |c: u8| key == u16::from(c);
        // While saving only watching: the edits wait.
        let watching = [
            VK_ESCAPE.0,
            VK_SPACE.0,
            VK_LEFT.0,
            VK_RIGHT.0,
            VK_OEM_COMMA.0,
            VK_OEM_PERIOD.0,
        ];
        if self.saving.is_some() && (ctrl || !(watching.contains(&key) || letter(b'F'))) {
            return None;
        }
        if ctrl {
            if letter(b'S') {
                return self.order(shift);
            }
            return match key {
                _ if letter(b'Z') && shift => self.redo(comp),
                _ if letter(b'Z') => self.undo(comp),
                _ if letter(b'Y') => self.redo(comp),
                _ => None,
            };
        }
        let now = self.now();
        let at = self.program.position(now);
        match key {
            k if k == VK_ESCAPE.0 => self.escape(comp),
            k if k == VK_SPACE.0 => self.toggle(comp),
            _ if letter(b'S') => self.cut(comp, at),
            _ if letter(b'F') => Some(Effect::Full),
            // With a join's list open Delete takes its transition away, otherwise the fragment.
            k if k == VK_DELETE.0 => match self.menu.as_ref().map(|m| m.owner) {
                Some(Owner::Join(join)) => {
                    self.close_menu();
                    self.set_transition(comp, join, None)
                }
                _ => self.remove(comp, self.current),
            },
            k if k == VK_LEFT.0 => self.seek(comp, at - SKIP, true),
            k if k == VK_RIGHT.0 => self.seek(comp, at + SKIP, true),
            k if k == VK_OEM_COMMA.0 || k == VK_OEM_PERIOD.0 => {
                let frame = 1.0 / f64::from(self.fps());
                let step = if k == VK_OEM_COMMA.0 { -frame } else { frame };
                self.seek(comp, at + step, true)
            }
            _ => None,
        }
    }

    /// Esc: what was started (a popover, the zoom frame, the scissors) ends first, then the editor.
    fn escape(&mut self, comp: &Comp) -> Option<Effect> {
        if self.menu.is_some() {
            self.close_menu();
            self.zoom_off(comp);
            return None;
        }
        if self.zooming.is_some() {
            self.zoom_off(comp);
            return None;
        }
        if self.scissors {
            self.scissors = false;
            self.timeline.scissors(comp, None, false);
            self.refresh(comp);
            return None;
        }
        if self.full {
            return Some(Effect::Full);
        }
        self.leave(comp)
    }

    /// Back or Esc: with unsaved edits, Save glints first.
    fn leave(&mut self, comp: &Comp) -> Option<Effect> {
        if self.is_dirty() && !self.armed {
            self.arm(comp);
            return None;
        }
        Some(Effect::Exit)
    }

    /// A window being saved closes at once: the island finishes the save.
    pub fn may_close(&mut self, comp: &Comp) -> bool {
        if self.saving.is_some() {
            return true;
        }
        if self.is_dirty() && !self.armed {
            self.arm(comp);
            return false;
        }
        true
    }

    fn arm(&mut self, comp: &Comp) {
        self.armed = true;
        if let Some(save) = self.tools.panel.get_mut(Key::Save) {
            save.shine(comp, true);
        }
        self.start_timer(TIMER_DISARM, DISARM_MS);
    }

    fn disarm(&mut self, comp: &Comp) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        if let Some(save) = self.tools.panel.get_mut(Key::Save) {
            save.shine(comp, false);
        }
        self.stop_timer(TIMER_DISARM);
    }

    pub fn on_timer(&mut self, comp: &Comp, id: usize) -> Option<Effect> {
        match id {
            TIMER_TICK => {
                self.stop_timer(id);
                let now = self.now();
                let result = self.program.tick(now);
                self.done(comp, result)
            }
            TIMER_CLOCK => {
                self.draw_time(comp);
                None
            }
            TIMER_DISARM => {
                self.disarm(comp);
                None
            }
            TIMER_RETIRE => {
                self.stop_timer(id);
                self.timeline.clear_retired(comp);
                None
            }
            TIMER_OPEN => {
                self.stop_timer(id);
                self.open_decks().err().map(Effect::Failed)
            }
            TIMER_READY => {
                self.stop_timer(id);
                self.ready = true;
                self.screen.reveal(comp);
                self.refresh(comp);
                None
            }
            TIMER_HINT => {
                self.stop_timer(id);
                let montage = self.montage().clone();
                self.timeline.targets(comp, None, &montage);
                None
            }
            _ => None,
        }
    }
}

impl Drop for MontageEditor {
    fn drop(&mut self) {
        for id in [
            TIMER_TICK,
            TIMER_CLOCK,
            TIMER_DISARM,
            TIMER_RETIRE,
            TIMER_HINT,
            TIMER_OPEN,
            TIMER_READY,
        ] {
            self.stop_timer(id);
        }
        self.close_menu();
        if let Ok(children) = self.top.Children() {
            let _ = children.Remove(&self.tools.panel.root);
        }
        if let Ok(children) = self.host.Children() {
            let _ = children.Remove(&self.screen.root);
            let _ = children.Remove(&self.framer.root);
            let _ = children.Remove(&self.timeline.root);
        }
    }
}

fn scissors_pointer(k: f32) -> Option<Pointer> {
    Pointer::scissors(k)
        .inspect_err(|e| crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)))
        .ok()
}

fn grip_cursor(grip: Grip) -> PCWSTR {
    match grip {
        Grip::Move => IDC_SIZEALL,
        Grip::Corner(0 | 3) => IDC_SIZENWSE,
        Grip::Corner(_) => IDC_SIZENESW,
    }
}
