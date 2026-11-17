//! Video viewer window: a recording plays in it, in the recording's proportions, about 80% of the
//! work area and never larger than pixel for pixel. It opens by the last frame's flight from the
//! island card; playback starts when the flight lands ([`VideoViewer::start`]).
//!
//! - on top the screenshot viewer's panel (`bar.rs`): Show in folder, Copy (the file), a pale Edit,
//!   Delete, the file name; at the bottom the video's own (`transport.rs`). With the strip look in
//!   a window they are dark strips above and below the picture; in full screen the strips lie over
//!   the picture's edges; without the strip they are capsules on the picture;
//! - while playing, panels that lie on the picture go a couple of seconds after the mouse stops:
//!   the strips slide out up and down, the capsules fade. Paused or ended, they stay. In full
//!   screen the cursor goes with them;
//! - a click on the picture pauses, a drag moves the window, a press held still runs the video at
//!   twice the speed until it is let go; the wheel zooms as in the screenshot viewer (`lens.rs`), a
//!   zoomed picture is dragged instead. A pause and a play from the picture, the keyboard or the
//!   island flash in its middle, the fast run shows at its top (`signs.rs`);
//! - full screen grows the picture from the window to the whole monitor and shrinks it back;
//! - keys: Space pause, ←/→ 5 s, `,`/`.` a frame while paused, F full screen, Esc leaves full screen
//!   or closes, Ctrl+C copies the file, Ctrl+Shift+C makes the frame a screenshot;
//! - minimized, the video pauses and goes on when shown again.
//!
//! Playback is [`Player`] (`crate::play`) over Media Engine, which presents into its own swap chain
//! shown here as a surface: nothing runs per frame in our process. The window decides nothing about
//! the file: it answers with a request ([`VideoRequest`]), the island carries it out (`watch.rs`).

use super::super::comp::{Comp, Motion, color, scale, v2, v3};
use super::super::flight::Place;
use super::super::window::{self, WM_DECK, WM_STRIP, WM_VIDEO};
use super::super::{log, point, wheel_delta};
use super::bar::{Bar, Item};
use super::lens::{Lens, Room};
use super::montage::{self, EDIT_MOTION, Effect, MontageEditor, Pictures, Setup};
use super::signs::Signs;
use super::transport::{Button, Show, Spot, Transport};
use super::{
    FEEDBACK_MS, Metrics, SCROLL_PX, SHARE, STRIP, TIMER_COPIED, TIMER_PATH, TOUCHPAD_STREAM,
    WHEEL_STEP, WM_MOUSELEAVE, ZOOM, raise, show,
};
use crate::capture::Image;
use crate::fault::{Cause, Failed};
use crate::lang::tr;
use crate::platform;
use crate::play::{
    Phase, Player, Problem,
    adapters::{Note, Video},
    clock,
};
use crate::settings::ViewerBar;
use std::path::{Path, PathBuf};
use std::time::Instant;
use windows::{
    UI::Composition::{ContainerVisual, Desktop::DesktopWindowTarget, SpriteVisual},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        System::SystemServices::MK_CONTROL,
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
                TrackMouseEvent, VK_CONTROL, VK_ESCAPE, VK_LBUTTON, VK_LEFT, VK_OEM_COMMA,
                VK_OEM_PERIOD, VK_RIGHT, VK_SHIFT, VK_SPACE,
            },
            WindowsAndMessaging::{
                DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics, GetWindowRect,
                HTCAPTION, HTCLIENT, IDC_HAND, IDC_SIZEALL, KillTimer, LoadCursorW, MINMAXINFO,
                PostMessageW, SIZE_MINIMIZED, SM_CXDRAG, SM_CYDRAG, SW_HIDE, SWP_NOACTIVATE,
                SWP_NOZORDER, SetCursor, SetTimer, SetWindowPos, ShowWindow, WM_CAPTURECHANGED,
                WM_CLOSE, WM_DPICHANGED, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDBLCLK,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
                WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_MOVING, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
                WM_NCLBUTTONDOWN, WM_NCMOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
                WM_SIZE, WM_TIMER,
            },
        },
    },
    core::Result,
};

/// The bottom panel needs more room than the screenshot's buttons.
const MIN_HEIGHT: f32 = 240.0;
const BACKDROP: &str = "#000000";
/// The panels go this long after the mouse stops, while playing.
const IDLE_MS: u32 = 2500;
const PANELS: Motion = Motion::curve(220, 0.25, 0.1, 0.25, 1.0);
const TIMER_SHOT: usize = 3;
const TIMER_SECOND: usize = 4;
const TIMER_IDLE: usize = 5;
const TIMER_START: usize = 6;
const TIMER_TURN: usize = 7;
const TIMER_ROW: usize = 8;
/// The speed row folds this long after the cursor leaves it.
const ROW_MS: u32 = 500;
const TIMER_HOLD: usize = 9;
/// The editor's panels have left: the window takes its place after the editor.
const TIMER_EDIT: usize = 10;
/// The bottom panel and the timeline travel this far when they swap.
const BOTTOM_TRAVEL: f32 = 28.0;
/// A press on the picture held still this long runs the video fast; let go earlier, it is a click.
const HOLD_MS: u32 = 350;
/// After the opening flight lands: the window's own appearance settles first.
const START_MS: u32 = 250;
/// With the editor's timeline the window grows up to this share of the work area's height.
const EDIT_SHARE: f32 = 0.9;
/// Into full screen and out of it.
const TURN: Motion = Motion::curve(280, 0.3, 0.0, 0.2, 1.0);

/// Full screen on its way in, or out to this window place.
enum Turn {
    In,
    Out(RECT),
}

pub enum VideoRequest {
    Close,
    Copy,
    Delete,
    Folder,
    CopyPath,
    /// The frame on screen, for a screenshot.
    Frame(std::result::Result<Image, Failed>),
    /// The editor's Save or Save copy.
    Save(montage::Order),
    Trouble(Trouble),
}

/// What went wrong, for the island to report. `Open` and `Stopped` end the window.
pub enum Trouble {
    Open(Failed),
    Stopped(Failed),
    Sound,
    /// A press reached the engine and it refused.
    Control(Failed),
}

pub struct VideoViewer {
    hwnd: HWND,
    target: Option<DesktopWindowTarget>,
    root: ContainerVisual,
    backdrop: SpriteVisual,
    /// The video in its own pixels: scale = zoom.
    canvas: ContainerVisual,
    /// The last frame under the video until the video draws itself.
    still: Option<SpriteVisual>,
    picture: Option<SpriteVisual>,
    signs: Signs,
    top: ContainerVisual,
    top_strip: SpriteVisual,
    bar: Bar,
    bottom: ContainerVisual,
    bottom_strip: SpriteVisual,
    transport: Transport,
    player: Player<Video>,
    /// The take's length until the engine says its own.
    duration: f64,
    metrics: Metrics,
    scale: f32,
    size: (f32, f32),
    full: (u32, u32),
    lens: Lens,
    began: Instant,
    panels_shown: bool,
    /// The pointer is over a panel: the panels stay.
    over_panel: bool,
    /// The length shows the time left.
    remaining: bool,
    scrub: Option<f64>,
    volume_drag: bool,
    pan: Option<(f32, f32)>,
    tracking: bool,
    touchpad_until: Option<Instant>,
    /// The window's place before full screen.
    windowed: Option<RECT>,
    turn: Option<Turn>,
    minimized: bool,
    /// Window messages carry the cursor even when it doesn't move: only a real move wakes the panels.
    last_cursor: Option<(i32, i32)>,
    /// The left button is down on the picture, not yet a drag or a hold: where (screen pixels).
    held: Option<(i32, i32)>,
    /// The press is in Windows' move loop: it ends in [`VideoViewer::pressed`].
    in_move: bool,
    /// The cursor left the open speed row: it folds on `TIMER_ROW`.
    row_leaving: bool,
    path: PathBuf,
    editor: Option<MontageEditor>,
    /// The closing editor, kept while its panels leave, and the window's place after it.
    leaving: Option<(MontageEditor, RECT)>,
    /// The editor shows the edit over the viewer's own picture, which has faded.
    canvas_hidden: bool,
    /// The timeline's thumbnails and wave, kept between editor sessions.
    pictures: Pictures,
    /// What entering the editor added to the window (physical pixels): left, top, width,
    /// height; leaving takes it back.
    grown: Option<(i32, i32, i32, i32)>,
}

pub struct Opening<'a> {
    pub path: &'a Path,
    pub name: String,
    pub still: &'a Image,
    /// The video's own size.
    pub size: (u32, u32),
    pub duration_ms: u64,
    pub volume: (f64, bool),
    /// The length shows the time left.
    pub remaining: bool,
}

impl VideoViewer {
    /// A hidden window in the middle of the work area `work` (physical pixels); the video starts
    /// loading at once and plays from [`VideoViewer::start`].
    pub fn new(
        comp: &Comp,
        work: RECT,
        k: f32,
        opening: Opening,
        bar: ViewerBar,
    ) -> std::result::Result<Self, Failed> {
        let metrics = Metrics::of(bar);
        let full = if opening.size.0 > 0 && opening.size.1 > 0 {
            opening.size
        } else {
            (opening.still.width, opening.still.height)
        };
        let (iw, ih) = (full.0 as f32 / k, full.1 as f32 / k);
        let room = (
            (work.right - work.left) as f32 / k * SHARE,
            (work.bottom - work.top) as f32 / k * SHARE,
        );
        let strips = 2.0 * strip_room(metrics, false);
        let fit = (room.0 / iw).min((room.1 - strips) / ih).min(1.0);
        let duration = opening.duration_ms as f64 / 1000.0;
        let probe = Transport::new(comp, k, metrics, duration, show_of(None, opening.volume))?;
        let min_w = probe.min_width(comp);
        drop(probe);
        let size = (
            (iw * fit).round().max(min_w),
            ((ih * fit).round() + strips).max(MIN_HEIGHT),
        );
        let px = ((size.0 * k).round() as i32, (size.1 * k).round() as i32);
        let left = work.left + (work.right - work.left - px.0) / 2;
        let top = work.top + (work.bottom - work.top - px.1) / 2;
        let rect = RECT {
            left,
            top,
            right: left + px.0,
            bottom: top + px.1,
        };
        let hwnd = window::create_viewer(rect, &tr!("window.video"))?;
        Self::build(comp, hwnd, (k, metrics), size, full, duration, opening).inspect_err(
            |_| unsafe {
                let _ = DestroyWindow(hwnd);
            },
        )
    }

    fn build(
        comp: &Comp,
        hwnd: HWND,
        (k, metrics): (f32, Metrics),
        size: (f32, f32),
        full: (u32, u32),
        duration: f64,
        opening: Opening,
    ) -> std::result::Result<Self, Failed> {
        let (target, root) = comp.window_root(hwnd)?;
        root.SetScale(scale(k, k))?;
        let c = &comp.compositor;
        let backdrop = c.CreateSpriteVisual()?;
        backdrop.SetBrush(&comp.brush(color(BACKDROP, 1.0))?)?;
        root.Children()?.InsertAtTop(&backdrop)?;
        let canvas = comp.container()?;
        canvas.SetSize(v2(full.0 as f32, full.1 as f32))?;
        root.Children()?.InsertAtTop(&canvas)?;
        let still = comp.image_rounded(
            &opening.still.pixels,
            (opening.still.width, opening.still.height),
            (full.0 as f32, full.1 as f32),
            0.0,
            1.0,
        )?;
        canvas.Children()?.InsertAtTop(&still)?;
        let signs = Signs::new(comp, k, metrics)?;
        root.Children()?.InsertAtTop(&signs.root)?;

        let top = comp.container()?;
        let top_strip = c.CreateSpriteVisual()?;
        top_strip.SetBrush(&comp.brush(color(STRIP, 1.0))?)?;
        top.Children()?.InsertAtTop(&top_strip)?;
        let mut bar = Bar::new(comp, k, metrics, Some(opening.name), false)?;
        bar.set_editable(false);
        top.Children()?.InsertAtTop(&bar.panel.root)?;
        root.Children()?.InsertAtTop(&top)?;

        let bottom = comp.container()?;
        let bottom_strip = c.CreateSpriteVisual()?;
        bottom_strip.SetBrush(&comp.brush(color(STRIP, 1.0))?)?;
        bottom.Children()?.InsertAtTop(&bottom_strip)?;
        let transport = Transport::new(comp, k, metrics, duration, show_of(None, opening.volume))?;
        bottom.Children()?.InsertAtTop(&transport.root)?;
        root.Children()?.InsertAtTop(&bottom)?;

        // A HWND can't cross threads, a number can: the engine's notes come back as messages.
        let address = hwnd.0 as isize;
        let video = Video::open(opening.path, move |note: Note| {
            let (wparam, lparam) = note.pack();
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(address as *mut _)),
                    WM_VIDEO,
                    WPARAM(wparam),
                    LPARAM(lparam),
                );
            }
        })?;
        let player = Player::new(video, opening.volume.0, opening.volume.1);

        let mut viewer = Self {
            hwnd,
            target: Some(target),
            root,
            backdrop,
            canvas,
            still: Some(still),
            picture: None,
            signs,
            top,
            top_strip,
            bar,
            bottom,
            bottom_strip,
            transport,
            player,
            duration,
            metrics,
            scale: k,
            size,
            full,
            lens: Lens::new(),
            began: Instant::now(),
            panels_shown: true,
            over_panel: false,
            remaining: opening.remaining,
            scrub: None,
            volume_drag: false,
            pan: None,
            tracking: false,
            touchpad_until: None,
            windowed: None,
            turn: None,
            minimized: false,
            last_cursor: None,
            held: None,
            in_move: false,
            row_leaving: false,
            path: opening.path.to_path_buf(),
            editor: None,
            leaving: None,
            canvas_hidden: false,
            pictures: Pictures::default(),
            grown: None,
        };
        viewer.fit();
        viewer.resize(comp, size)?;
        viewer.sync(comp);
        Ok(viewer)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn show(&self, animated: bool) {
        show(self.hwnd, animated);
    }

    /// The window disappears without animation: the frame flies out of it to the island.
    pub fn vanish(&self) {
        window::set_transitions(self.hwnd, false);
    }

    pub fn raise(&self) {
        raise(self.hwnd);
    }

    /// The opening flight has landed: the video plays a moment later (once loaded).
    pub fn start(&self) {
        self.start_timer(TIMER_START, START_MS);
    }

    pub fn pause(&mut self, comp: &Comp) {
        if let Some(editor) = &mut self.editor {
            editor.pause(comp);
            return;
        }
        let now = self.now();
        let _ = self.player.pause(now);
        self.sync(comp);
    }

    pub fn toggle(&mut self, comp: &Comp) -> Option<VideoRequest> {
        if let Some(editor) = &mut self.editor {
            let effect = editor.toggle(comp);
            return self.on_effect(comp, effect);
        }
        let now = self.now();
        let done = self.player.toggle(now);
        self.sync(comp);
        self.flash(comp);
        refused(done)
    }

    fn flash(&self, comp: &Comp) {
        self.signs.flash(comp, self.player.running());
    }

    pub fn running(&self) -> bool {
        match &self.editor {
            Some(editor) => editor.running(),
            None => self.player.running(),
        }
    }

    /// The island's arrow would close the editor: with unsaved edits Save glints first, as for
    /// closing the window. True = the window stays.
    pub fn keeps_edits(&mut self, comp: &Comp) -> bool {
        match &mut self.editor {
            Some(editor) => !editor.may_close(comp),
            None => false,
        }
    }

    /// Edit: the editor's panels come in, the window grows down by the timeline (up to 90% of the
    /// work area's height; the picture gives up the rest). The picture glides to its new place and
    /// stays under the editor's own until the editor shows the edit; the bottom panel sinks away
    /// and the timeline comes up in its place.
    fn enter_edit(&mut self, comp: &Comp) -> Option<VideoRequest> {
        if self.editor.is_some() || self.turn.is_some() {
            return None;
        }
        self.finish_leaving(comp);
        let info = self.player.info()?;
        let now = self.now();
        let _ = self.player.pause(now);
        let at = self.player.position(now);
        self.sync(comp);
        self.pictures
            .load(self.hwnd, &self.path, info.duration, self.full);
        let setup = Setup {
            path: &self.path,
            full: self.full,
            duration: self.duration(),
            volume: self.volume(),
            pictures: std::mem::take(&mut self.pictures),
            at,
            full_screen: self.windowed.is_some(),
        };
        let editor = match MontageEditor::new(
            comp,
            self.hwnd,
            (&self.root, &self.top),
            (self.scale, self.metrics),
            self.size,
            setup,
        ) {
            Ok(editor) => editor,
            Err(failed) => return Some(VideoRequest::Trouble(Trouble::Control(failed))),
        };
        self.bar.cancel(comp);
        self.bar.hover(comp, None);
        self.bar
            .panel
            .slide(comp, false, self.metrics.slide_distance());
        self.stop_timer(TIMER_IDLE);
        self.set_panels(comp, true, false);
        let min_width = editor.min_width();
        let (before, lens) = (window_rect(self.hwnd), self.lens);
        let bottom = self.size.1 - self.band();
        self.editor = Some(editor);
        if self.windowed.is_none() {
            self.grow_for_edit(comp, min_width);
        } else {
            log(self.resize(comp, self.size));
        }
        self.fit();
        log(self.layout(comp, false));
        let shift = self.shift(before, window_rect(self.hwnd));
        log(self.glide_canvas(comp, shifted(lens, shift)));
        log(self.move_bottom(
            comp,
            (shift.0, bottom + shift.1),
            (0.0, BOTTOM_TRAVEL),
            false,
        ));
        if let Some(editor) = &self.editor {
            editor.arrive(comp);
        }
        None
    }

    /// The window grows down; up only as far as the work area makes it.
    fn grow_for_edit(&mut self, comp: &Comp, min_width: f32) {
        let k = self.scale;
        let rect = window_rect(self.hwnd);
        let work = platform::work_rect(platform::window_monitor(self.hwnd));
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        // Without the strip the timeline lies on the picture: the window keeps its height.
        let grow = if self.metrics.floating() {
            0.0
        } else {
            (MontageEditor::band(self.metrics) - self.metrics.strip).max(0.0)
        };
        let max_h = ((work.bottom - work.top) as f32 * EDIT_SHARE) as i32;
        let new_h = (h + (grow * k).round() as i32).min(max_h.max(h));
        let new_w = w
            .max((min_width * k).ceil() as i32)
            .min(work.right - work.left);
        let left =
            (rect.left - (new_w - w) / 2).clamp(work.left, (work.right - new_w).max(work.left));
        let top = rect
            .top
            .clamp(work.top, (work.bottom - new_h).max(work.top));
        self.grown = Some((left - rect.left, top - rect.top, new_w - w, new_h - h));
        self.place_window(
            comp,
            RECT {
                left,
                top,
                right: left + new_w,
                bottom: top + new_h,
            },
        );
    }

    /// The picture jumps to `from` and glides to where it is now.
    fn glide_canvas(&self, comp: &Comp, from: Lens) -> Result<()> {
        comp.stop(&self.canvas, "Scale")?;
        comp.stop(&self.canvas, "Offset")?;
        self.canvas.SetScale(scale(from.zoom, from.zoom))?;
        self.canvas.SetOffset(v3(from.offset.0, from.offset.1))?;
        self.glide_canvas_to(comp, self.lens)
    }

    fn glide_canvas_to(&self, comp: &Comp, to: Lens) -> Result<()> {
        comp.animate_vector3(&self.canvas, "Scale", scale(to.zoom, to.zoom), EDIT_MOTION)?;
        comp.animate_vector3(
            &self.canvas,
            "Offset",
            v3(to.offset.0, to.offset.1),
            EDIT_MOTION,
        )
    }

    /// The bottom panel moves by `travel` from `at`, fading out, or (`shown`) the other way in.
    fn move_bottom(
        &self,
        comp: &Comp,
        at: (f32, f32),
        travel: (f32, f32),
        shown: bool,
    ) -> Result<()> {
        let near = v3(at.0, at.1);
        let far = v3(at.0 + travel.0, at.1 + travel.1);
        let (from, to, opacity) = if shown {
            (far, near, (0.0, 1.0))
        } else {
            (near, far, (1.0, 0.0))
        };
        let path = [(0.0, from, Motion::linear(0)), (1.0, to, EDIT_MOTION)];
        comp.path_vector3(&self.bottom, "Offset", &path, EDIT_MOTION.ms)?;
        comp.animate_scalar_from(&self.bottom, "Opacity", opacity.0, opacity.1, EDIT_MOTION)
    }

    /// The editor goes: its panels leave and its picture fades, the viewer's come back and the
    /// picture glides to where it will be; the window takes back what the editor added when they
    /// are there (`TIMER_EDIT`). The viewer stands where the caret was.
    fn leave_edit(&mut self, comp: &Comp) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let (pictures, source) = editor.retire(comp);
        self.pictures = pictures;
        let d = self.duration();
        if d > 0.0 {
            let f = (source / d).clamp(0.0, 1.0);
            let now = self.now();
            let _ = self.player.grab(f, now);
            let _ = self.player.release(f, now);
        }
        self.bar
            .panel
            .slide(comp, true, self.metrics.slide_distance());
        self.canvas_hidden = false;
        log(comp.stop(&self.canvas, "Opacity"));
        log(self.canvas.SetOpacity(1.0));
        log(self.signs.root.SetOpacity(1.0));
        let now_rect = window_rect(self.hwnd);
        let target = match self.grown.take() {
            Some((dl, dt, dw, dh)) => {
                let back = |r: RECT| RECT {
                    left: r.left - dl,
                    top: r.top - dt,
                    right: r.left - dl + (r.right - r.left - dw).max(1),
                    bottom: r.top - dt + (r.bottom - r.top - dh).max(1),
                };
                match &mut self.windowed {
                    // In full screen the window's place for later takes it back.
                    Some(rect) => {
                        *rect = back(*rect);
                        now_rect
                    }
                    None => back(now_rect),
                }
            }
            None => now_rect,
        };
        let size = self.logical((target.right - target.left, target.bottom - target.top));
        let shift = self.shift(target, now_rect);
        let full = self.windowed.is_some();
        let mut lens = Lens::new();
        lens.fit(area_of(self.metrics, full, size), self.content());
        log(self.glide_canvas_to(comp, shifted(lens, shift)));
        let bottom = size.1 - Transport::band(self.metrics) + shift.1;
        log(self.move_bottom(comp, (shift.0, bottom), (0.0, BOTTOM_TRAVEL), true));
        self.leaving = Some((editor, target));
        self.start_timer(TIMER_EDIT, EDIT_MOTION.ms);
    }

    /// The closing editor's panels are gone: it goes, the window takes its place.
    fn finish_leaving(&mut self, comp: &Comp) {
        self.stop_timer(TIMER_EDIT);
        let Some((editor, rect)) = self.leaving.take() else {
            return;
        };
        drop(editor);
        if self.windowed.is_none() {
            self.place_window(comp, rect);
        }
        self.fit();
        log(self.layout(comp, false));
        log(self.place_panels(comp, false));
        self.sync(comp);
        self.wake(comp);
    }

    fn on_effect(&mut self, comp: &Comp, effect: Option<Effect>) -> Option<VideoRequest> {
        match effect? {
            Effect::Exit => {
                self.leave_edit(comp);
                None
            }
            Effect::Full => {
                self.toggle_full(comp);
                None
            }
            Effect::Save(order) => Some(VideoRequest::Save(order)),
            Effect::Failed(failed) => Some(VideoRequest::Trouble(Trouble::Control(failed))),
        }
    }

    /// The save moved on (`None`: it ended); the editor shows it on Save.
    pub fn set_saving(&mut self, comp: &Comp, share: Option<f64>) {
        if let Some(editor) = &mut self.editor {
            editor.set_saving(comp, share);
        }
    }

    pub fn save_failed(&mut self, comp: &Comp) {
        if let Some(editor) = &mut self.editor {
            editor.save_failed(comp);
        }
    }

    /// Every engine of the window lets the recording go: the saved edit takes its place.
    pub fn let_go(&self) {
        if let Some(editor) = &self.editor {
            editor.close_engines();
        }
        if let Some((editor, _)) = &self.leaving {
            editor.close_engines();
        }
        self.player.engine().close();
    }

    /// The edit is saved as `path`: the editor closes and the window plays the new file, from its
    /// start, the edit's frame standing in until it plays.
    pub fn saved(&mut self, comp: &Comp, path: &Path, name: String) -> Option<VideoRequest> {
        let frame = self.editor.as_ref().and_then(MontageEditor::frame);
        if let Some(editor) = &mut self.editor {
            editor.set_saving(comp, None);
        }
        self.leave_edit(comp);
        self.path = path.to_path_buf();
        log(self.bar.set_file(comp, Some(name)));
        // A HWND can't cross threads, a number can: the engine's notes come back as messages.
        let address = self.hwnd.0 as isize;
        let video = Video::open(path, move |note: Note| {
            let (wparam, lparam) = note.pack();
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(address as *mut _)),
                    WM_VIDEO,
                    WPARAM(wparam),
                    LPARAM(lparam),
                );
            }
        });
        let video = match video {
            Ok(video) => video,
            Err(e) => return Some(VideoRequest::Trouble(Trouble::Open(e.into()))),
        };
        if let Ok(children) = self.canvas.Children() {
            if let Some(picture) = self.picture.take() {
                let _ = children.Remove(&picture);
            }
            if let Some(old) = self.still.take() {
                let _ = children.Remove(&old);
            }
            if let Some(frame) = frame
                && let Ok(still) = comp.image_rounded(
                    &frame.pixels,
                    (frame.width, frame.height),
                    (self.full.0 as f32, self.full.1 as f32),
                    0.0,
                    1.0,
                )
            {
                let _ = children.InsertAtBottom(&still);
                self.still = Some(still);
            }
        }
        let (volume, muted) = self.volume();
        self.player = Player::new(video, volume, muted);
        self.sync(comp);
        None
    }

    /// The editor shows the edit: the viewer's own picture under it fades.
    fn hide_canvas(&mut self, comp: &Comp) {
        if self.canvas_hidden || !self.editor.as_ref().is_some_and(MontageEditor::is_ready) {
            return;
        }
        self.canvas_hidden = true;
        // Once the editor's pictures have faded in over it: no dip to black between them.
        log(comp.animate_scalar_after(
            &self.canvas,
            "Opacity",
            0.0,
            Motion::linear(1),
            montage::REVEAL_MS,
        ));
        log(self.signs.root.SetOpacity(0.0));
    }

    /// The editor's share of the window's messages; `None` = the window's own handling.
    fn handle_editing(
        &mut self,
        comp: &Comp,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<(Option<LRESULT>, Option<VideoRequest>)> {
        self.editor.as_ref()?;
        let done = |result: isize| Some((Some(LRESULT(result)), None));
        let p = self.logical(point(lparam));
        let screen = self.at_screen(point(lparam));
        let cursor = self.cursor();
        if msg == WM_MOUSEMOVE {
            self.track_leave();
        }
        if msg == WM_MOUSELEAVE {
            self.tracking = false;
        }
        let editor = self.editor.as_mut()?;
        let effect = match msg {
            m if m == WM_DECK || m == WM_DECK + 1 => {
                editor.on_deck(comp, (m - WM_DECK) as usize, wparam, lparam)
            }
            WM_STRIP => {
                editor.on_pictures(comp);
                None
            }
            WM_NCHITTEST => {
                return done(if editor.wants(screen) {
                    HTCLIENT
                } else {
                    HTCAPTION
                } as isize);
            }
            WM_SETCURSOR => {
                return match editor.cursor(cursor) {
                    Some(shape) => {
                        unsafe {
                            SetCursor(Some(shape));
                        }
                        done(1)
                    }
                    None => Some((None, None)),
                };
            }
            WM_MOUSEMOVE => editor.moved(comp, p),
            WM_NCMOUSEMOVE | WM_MOUSELEAVE => {
                editor.left(comp);
                return Some((None, None));
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (taken, effect) = editor.press(comp, p);
                if taken {
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                }
                effect
            }
            WM_LBUTTONUP => {
                let effect = editor.release(comp, p);
                // Capture is released last: this message arrives at once, into the finished state.
                unsafe {
                    let _ = ReleaseCapture();
                }
                effect
            }
            WM_CAPTURECHANGED => {
                editor.cancel(comp);
                return done(0);
            }
            // The picture moves the window in Windows' own loop; no maximize on a double click.
            WM_NCLBUTTONDOWN | WM_MOVING => return Some((None, None)),
            WM_NCLBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP
            | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => return done(0),
            WM_KEYDOWN => {
                let held = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
                let (ctrl, shift) = (held(VK_CONTROL.0), held(VK_SHIFT.0));
                editor.key(comp, wparam.0 as u16, ctrl, shift)
            }
            WM_CLOSE => {
                let close = editor.may_close(comp);
                return Some((Some(LRESULT(0)), close.then_some(VideoRequest::Close)));
            }
            WM_TIMER if wparam.0 >= montage::TIMER_TICK => editor.on_timer(comp, wparam.0),
            _ => return None,
        };
        let request = self.on_effect(comp, effect);
        self.hide_canvas(comp);
        Some((Some(LRESULT(0)), request))
    }

    /// Volume and mute, to be remembered for the next video.
    pub fn volume(&self) -> (f64, bool) {
        (self.player.volume(), self.player.muted())
    }

    /// The length or the time left, to be remembered for the next video.
    pub fn remaining(&self) -> bool {
        self.remaining
    }

    /// The frame on screen; `None` before the video drew one.
    pub fn frame(&self) -> Option<Image> {
        if let Some(editor) = &self.editor {
            return editor.frame();
        }
        self.picture.as_ref()?;
        self.player.engine().frame().ok()
    }

    pub fn place(&self, monitor: RECT) -> Place {
        let window = window_rect(self.hwnd);
        let k = self.scale;
        (
            (window.left - monitor.left) as f32 / k + self.lens.offset.0,
            (window.top - monitor.top) as f32 / k + self.lens.offset.1,
            self.full.0 as f32 * self.lens.zoom,
            self.full.1 as f32 * self.lens.zoom,
        )
    }

    pub fn show_copied(&self, comp: &Comp) {
        self.bar.show_copied(comp, true);
        self.start_timer(TIMER_COPIED, FEEDBACK_MS);
    }

    pub fn show_path_copied(&mut self, comp: &Comp) {
        log(self.bar.show_path_copied(comp, true));
        self.start_timer(TIMER_PATH, FEEDBACK_MS);
    }

    pub fn show_shot_done(&self, comp: &Comp) {
        self.transport.show_shot_done(comp, true);
        self.start_timer(TIMER_SHOT, FEEDBACK_MS);
    }

    /// In a window the strips' rows come and go with the look: the window grows or shrinks by them
    /// and the picture keeps its size.
    pub fn set_bar(&mut self, comp: &Comp, bar: ViewerBar) {
        let metrics = Metrics::of(bar);
        if metrics == self.metrics {
            return;
        }
        let grow = 2.0 * (strip_room(metrics, false) - strip_room(self.metrics, false));
        self.metrics = metrics;
        if self.windowed.is_none() && self.turn.is_none() && grow != 0.0 {
            let mut rect = window_rect(self.hwnd);
            let px = (grow * self.scale).round() as i32;
            rect.top -= px / 2;
            rect.bottom += px - px / 2;
            self.place_window(comp, rect);
        }
        self.restyle(comp, self.size);
        self.wake(comp);
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

    fn now(&self) -> u64 {
        self.began.elapsed().as_millis() as u64
    }

    fn show_state(&self) -> Show {
        show_of(Some(&self.player), self.volume()).with_full(self.windowed.is_some())
    }

    /// Everything the player's state shows: the bar's run, the times, the buttons, the panels.
    fn sync(&mut self, comp: &Comp) {
        let now = self.now();
        // The buttons first: a new speed label moves the bar, and its run is measured on the new one.
        log(self.transport.set_show(comp, self.show_state()));
        self.transport.set_bar(comp, self.player.bar(now));
        self.draw_times(comp);
        match self.player.next_second(now) {
            Some(at) => self.start_timer(TIMER_SECOND, (at - now).max(1) as u32),
            None => self.stop_timer(TIMER_SECOND),
        }
        if self.player.phase() != Phase::Playing {
            self.wake(comp);
        }
    }

    fn duration(&self) -> f64 {
        match self.player.info() {
            Some(info) if info.duration > 0.0 => info.duration,
            _ => self.duration,
        }
    }

    fn draw_times(&mut self, comp: &Comp) {
        let now = self.now();
        let d = self.duration();
        let at = self.scrub.map_or(self.player.position(now), |f| f * d);
        let time = clock(at, d);
        let length = if self.remaining {
            format!("\u{2212}{}", clock((d - at).max(0.0), d))
        } else {
            clock(d, d)
        };
        log(self.transport.set_times(comp, &time, &length));
    }

    fn on_note(&mut self, comp: &Comp, note: Note) -> Option<VideoRequest> {
        let event = self.player.engine().event(note)?;
        // The engine plays: the video draws itself, the still under it is no longer needed.
        let drawing = matches!(event, crate::play::Event::Playing(_));
        let now = self.now();
        let was = self.player.phase();
        let update = self.player.on_event(event, now);
        let phase = self.player.phase();
        if was == Phase::Loading && phase != Phase::Loading && phase != Phase::Broken {
            self.loaded(comp);
        }
        if drawing
            && let Some(still) = self.still.take()
            && self.picture.is_some()
            && let Ok(children) = self.canvas.Children()
        {
            let _ = children.Remove(&still);
        }
        if update.bar || was != phase {
            self.sync(comp);
        }
        let problem = update.problem?;
        let trouble = match problem {
            Problem::NoPicture => Trouble::Open(Failed::because(Cause::Unsupported, "no picture")),
            Problem::NoSound => Trouble::Sound,
            Problem::Failed(failed) if phase != Phase::Broken => Trouble::Control(failed),
            Problem::Failed(failed) if was == Phase::Loading => Trouble::Open(failed),
            Problem::Failed(failed) => Trouble::Stopped(failed),
        };
        Some(VideoRequest::Trouble(trouble))
    }

    /// The engine knows the video: its swap chain goes over the still, the panel takes its length.
    fn loaded(&mut self, comp: &Comp) {
        if let Some(info) = self.player.info()
            && info.size.0 > 0
            && info.size.1 > 0
        {
            self.full = info.size;
            log(self
                .canvas
                .SetSize(v2(info.size.0 as f32, info.size.1 as f32)));
        }
        let duration = self.duration();
        self.transport.set_duration(comp, duration);
        self.bar.set_editable(true);
        let picture =
            self.player.engine().swap_chain().and_then(|handle| {
                comp.swap_chain(handle, (self.full.0 as f32, self.full.1 as f32))
            });
        match picture {
            Ok(picture) => {
                if let Ok(children) = self.canvas.Children() {
                    let _ = children.InsertAtTop(&picture);
                }
                self.picture = Some(picture);
            }
            Err(e) => crate::journal::warn(tr!("journal.video_surface_failed", error = e)),
        }
        log(self.resize(comp, self.size));
    }

    /// Returns Windows' answer (`None` = default handler) and a request to the island.
    pub fn handle(
        &mut self,
        comp: &Comp,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (Option<LRESULT>, Option<VideoRequest>) {
        if let Some(answer) = self.handle_editing(comp, msg, wparam, lparam) {
            return answer;
        }
        let done = |result: isize| (Some(LRESULT(result)), None);
        match msg {
            // The timeline's pictures came after the editor closed: kept for the next time.
            WM_STRIP => {
                for failed in self.pictures.take().1 {
                    crate::journal::warn(tr!(
                        "journal.timeline_pictures_failed",
                        error = failed.detail
                    ));
                }
                done(0)
            }
            WM_VIDEO => {
                let request = self.on_note(comp, Note::unpack(wparam.0, lparam.0));
                (Some(LRESULT(0)), request)
            }
            WM_NCHITTEST => {
                let p = self.at_screen(point(lparam));
                done(if self.is_client(p) {
                    HTCLIENT
                } else {
                    HTCAPTION
                } as isize)
            }
            WM_SETCURSOR => {
                let p = self.cursor();
                let idle = !self.panels_shown && self.turn.is_none() && self.pan.is_none();
                if self.windowed.is_some() && idle {
                    unsafe {
                        SetCursor(None);
                    }
                    return done(1);
                }
                match self.cursor_shape(p) {
                    Some(shape) => {
                        unsafe {
                            if let Ok(cursor) = LoadCursorW(None, shape) {
                                SetCursor(Some(cursor));
                            }
                        }
                        done(1)
                    }
                    None => (None, None),
                }
            }
            WM_MOUSEMOVE => {
                self.track_leave();
                let p = self.logical(point(lparam));
                self.moved(comp);
                self.drag_held();
                (Some(LRESULT(0)), self.pointer(comp, p))
            }
            WM_NCMOUSEMOVE | WM_MOUSELEAVE => {
                if msg == WM_MOUSELEAVE {
                    self.tracking = false;
                }
                if msg == WM_NCMOUSEMOVE {
                    self.moved(comp);
                }
                if self.scrub.is_none() && !self.volume_drag {
                    self.bar.hover(comp, None);
                    log(self.transport.hover(comp, None, false));
                    self.refresh_bar(comp);
                    self.watch_row(None);
                    self.over_panel = false;
                }
                (None, None)
            }
            // On the panels a second click is just another click.
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let p = self.logical(point(lparam));
                (Some(LRESULT(0)), self.press(comp, p))
            }
            WM_LBUTTONUP => {
                let p = self.logical(point(lparam));
                let click = self.click_held(comp);
                let fast = self.end_fast(comp);
                let request = self.release(comp, p);
                (Some(LRESULT(0)), click.or(fast).or(request))
            }
            // Windowed, Windows' move loop takes the press (our timers still run inside it); in full screen and on a double click it stays ours (no maximize).
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 == HTCAPTION as usize => {
                let (x, y) = point(lparam);
                if self.on_picture(self.at_screen((x, y))) {
                    self.held = Some((x, y));
                    self.start_timer(TIMER_HOLD, HOLD_MS);
                    if msg == WM_NCLBUTTONDOWN && self.windowed.is_none() {
                        self.in_move = true;
                        return (None, None);
                    }
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    return done(0);
                }
                // The strips' empty parts move the window in the default handler.
                if msg == WM_NCLBUTTONDBLCLK || self.windowed.is_some() {
                    done(0)
                } else {
                    (None, None)
                }
            }
            WM_NCLBUTTONDBLCLK => done(0),
            WM_MOVING => {
                if self.in_move && self.held.take().is_some() {
                    self.stop_timer(TIMER_HOLD);
                }
                (None, None)
            }
            // The move loop's own capture ends before the loop returns.
            WM_CAPTURECHANGED if self.in_move => (None, None),
            WM_MBUTTONDOWN | WM_RBUTTONDOWN => {
                let p = self.logical(point(lparam));
                if self.zoomed() && self.on_picture(p) {
                    self.pan = Some(p);
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                }
                done(0)
            }
            WM_MBUTTONUP | WM_RBUTTONUP => {
                if self.pan.take().is_some() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                done(0)
            }
            WM_CAPTURECHANGED => {
                self.pan = None;
                self.bar.cancel(comp);
                self.transport.cancel(comp);
                self.volume_drag = false;
                if self.held.take().is_some() {
                    self.stop_timer(TIMER_HOLD);
                }
                let fast = self.end_fast(comp);
                let request = self.scrub.take().and_then(|f| self.end_scrub(comp, f));
                (Some(LRESULT(0)), fast.or(request))
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL if self.turn.is_none() => {
                let p = self.at_screen(point(lparam));
                let delta = wheel_delta(wparam) as f32;
                let ctrl = wparam.0 & MK_CONTROL.0 as usize != 0;
                if msg == WM_MOUSEHWHEEL {
                    self.scroll(comp, (-delta * SCROLL_PX, 0.0));
                } else if !ctrl && self.touchpad_scroll(delta) {
                    self.scroll(comp, (0.0, delta * SCROLL_PX));
                } else {
                    let zoom = self.lens.zoom * WHEEL_STEP.powf(delta / 120.0);
                    self.zoom_at(comp, zoom, p);
                }
                done(0)
            }
            WM_KEYDOWN => {
                let request = self.key(comp, wparam.0 as u16);
                (Some(LRESULT(0)), request)
            }
            WM_CLOSE => (Some(LRESULT(0)), Some(VideoRequest::Close)),
            WM_SIZE => {
                let now = self.now();
                let request = if wparam.0 as u32 == SIZE_MINIMIZED {
                    self.minimized = true;
                    refused(self.player.hide(now))
                } else {
                    let (w, h) = point(lparam);
                    log(self.resize(comp, self.logical((w, h))));
                    std::mem::take(&mut self.minimized)
                        .then(|| self.player.show(now))
                        .and_then(refused)
                };
                self.sync(comp);
                (None, request)
            }
            WM_GETMINMAXINFO => {
                let (mut w, mut h) = (self.transport.min_width(comp), MIN_HEIGHT);
                if let Some(editor) = &self.editor {
                    w = w.max(editor.min_width());
                    h = h.max(MontageEditor::band(self.metrics) + 2.0 * self.top_band());
                }
                unsafe {
                    let info = &mut *(lparam.0 as *mut MINMAXINFO);
                    info.ptMinTrackSize = POINT {
                        x: (w * self.scale).ceil() as i32,
                        y: (h * self.scale).ceil() as i32,
                    };
                }
                done(0)
            }
            WM_DPICHANGED => {
                self.rescale(comp, (wparam.0 >> 16) as u16 as f32 / 96.0, lparam);
                done(0)
            }
            WM_TIMER => (Some(LRESULT(0)), self.on_timer(comp, wparam.0)),
            _ => (None, None),
        }
    }

    fn on_timer(&mut self, comp: &Comp, id: usize) -> Option<VideoRequest> {
        match id {
            TIMER_START => {
                self.stop_timer(id);
                let now = self.now();
                let done = self.player.start(now);
                self.sync(comp);
                return refused(done);
            }
            TIMER_COPIED => {
                self.stop_timer(id);
                self.bar.show_copied(comp, false);
            }
            TIMER_PATH => {
                self.stop_timer(id);
                log(self.bar.show_path_copied(comp, false));
            }
            TIMER_SHOT => {
                self.stop_timer(id);
                self.transport.show_shot_done(comp, false);
            }
            TIMER_TURN => {
                self.stop_timer(id);
                self.end_turn(comp);
            }
            TIMER_EDIT => self.finish_leaving(comp),
            TIMER_ROW => {
                self.fold_row(comp);
                self.start_timer(TIMER_IDLE, IDLE_MS);
            }
            TIMER_HOLD => {
                self.stop_timer(id);
                if self.held.take().is_some() {
                    return self.start_fast(comp);
                }
            }
            TIMER_SECOND => {
                self.stop_timer(id);
                self.draw_times(comp);
                let now = self.now();
                if let Some(at) = self.player.next_second(now) {
                    self.start_timer(TIMER_SECOND, (at - now).max(1) as u32);
                }
            }
            TIMER_IDLE => {
                self.stop_timer(id);
                if self.may_hide() {
                    self.set_panels(comp, false, true);
                }
            }
            _ => {}
        }
        None
    }

    /// The panels may go: they lie on the picture, the video plays and nothing on them is in use.
    fn may_hide(&self) -> bool {
        self.editor.is_none()
            && self.panels_on_picture()
            && self.player.phase() == Phase::Playing
            && self.scrub.is_none()
            && !self.volume_drag
            && !self.over_panel
            && !self.transport.row_open()
            && !self.transport.slider_open()
    }

    /// The mouse moved (for real): the panels come back and the idle count starts over.
    fn moved(&mut self, comp: &Comp) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        if self.last_cursor == Some((p.x, p.y)) {
            return;
        }
        self.last_cursor = Some((p.x, p.y));
        self.wake(comp);
    }

    /// While full screen comes or goes the panels stay away: they come back when it is done.
    fn wake(&mut self, comp: &Comp) {
        if self.turn.is_some() {
            return;
        }
        self.set_panels(comp, true, true);
        self.start_timer(TIMER_IDLE, IDLE_MS);
    }

    fn panels_on_picture(&self) -> bool {
        self.metrics.floating() || self.windowed.is_some()
    }

    fn set_panels(&mut self, comp: &Comp, shown: bool, animate: bool) {
        if shown == self.panels_shown {
            return;
        }
        self.panels_shown = shown;
        if !shown {
            self.bar.hover(comp, None);
            log(self.transport.hover(comp, None, false));
            self.refresh_bar(comp);
            log(self.transport.set_tip(comp, None));
            self.over_panel = false;
        }
        log(self.place_panels(comp, animate));
    }

    fn band(&self) -> f32 {
        Transport::band(self.metrics)
    }

    fn top_band(&self) -> f32 {
        let m = self.metrics;
        if m.floating() {
            m.margin + m.button.1
        } else {
            m.strip
        }
    }

    fn place_panels(&self, comp: &Comp, animate: bool) -> Result<()> {
        let (_, h) = self.size;
        let shown = self.panels_shown;
        let (top_y, bottom_y, opacity) = if self.metrics.floating() {
            (0.0, h - self.band(), if shown { 1.0 } else { 0.0 })
        } else if shown {
            (0.0, h - self.band(), 1.0)
        } else {
            (-self.top_band(), h, 1.0)
        };
        // The editor's timeline takes the bottom panel's place.
        let editing = self.editor.is_some();
        for (visual, y) in [(&self.top, top_y), (&self.bottom, bottom_y)] {
            let opacity = if editing && visual == &self.bottom {
                0.0
            } else {
                opacity
            };
            if animate {
                comp.animate_vector3(visual, "Offset", v3(0.0, y), PANELS)?;
                comp.animate_scalar(visual, "Opacity", opacity, PANELS)?;
            } else {
                comp.stop(visual, "Offset")?;
                comp.stop(visual, "Opacity")?;
                visual.SetOffset(v3(0.0, y))?;
                visual.SetOpacity(opacity)?;
            }
        }
        Ok(())
    }

    /// `p` in the bottom panel's coordinates.
    fn in_bottom(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0, p.1 - (self.size.1 - self.band()))
    }

    fn on_panel(&self, p: (f32, f32)) -> bool {
        self.panels_shown
            && (self.bar.at(p).is_some() || self.transport.at(self.in_bottom(p)).is_some())
    }

    /// On the picture, not on a strip (the strips' empty parts move the window, they don't pause).
    fn on_picture(&self, p: (f32, f32)) -> bool {
        let strips = self.panels_shown && !self.metrics.floating();
        let in_strip = strips && (p.1 < self.top_band() || p.1 > self.size.1 - self.band());
        !in_strip && !self.on_panel(p)
    }

    fn is_client(&self, p: (f32, f32)) -> bool {
        self.on_panel(p)
            || self.scrub.is_some()
            || self.volume_drag
            || (self.zoomed() && self.on_picture(p))
    }

    fn cursor_shape(&self, p: (f32, f32)) -> Option<windows::core::PCWSTR> {
        if self.pan.is_some() {
            return Some(IDC_SIZEALL);
        }
        if self.panels_shown {
            if self.bar.enabled_at(p).is_some() {
                return Some(IDC_HAND);
            }
            if self.transport.at(self.in_bottom(p)).is_some() || self.scrub.is_some() {
                return Some(IDC_HAND);
            }
        }
        (self.zoomed() && self.on_picture(p)).then_some(IDC_SIZEALL)
    }

    /// The cursor moved to `p` (window coordinates) with the left button up or held.
    fn pointer(&mut self, comp: &Comp, p: (f32, f32)) -> Option<VideoRequest> {
        if let Some(last) = self.pan {
            self.lens
                .pan((p.0 - last.0, p.1 - last.1), self.area(), self.content());
            self.pan = Some(p);
            log(self.layout(comp, false));
            return None;
        }
        let at = self.in_bottom(p);
        if self.scrub.is_some() {
            let f = self.transport.fraction_at(at.0);
            self.scrub = Some(f);
            log(self.transport.set_tip(comp, Some(f)));
            self.draw_times(comp);
            return refused(self.player.drag(f));
        }
        if self.volume_drag {
            let v = self.transport.volume_at(at.0);
            let done = self.player.set_volume(v);
            log(self.transport.set_show(comp, self.show_state()));
            return refused(done);
        }
        if !self.panels_shown {
            return None;
        }
        self.over_panel = self.bar.at(p).is_some() || self.transport.covers(at);
        self.bar.hover(comp, Some(p));
        log(self.transport.hover(comp, Some(at), false));
        self.refresh_bar(comp);
        self.watch_row(Some(at));
        None
    }

    /// The bottom panel moved its bar (the volume slider came or went): the bar's run is retimed.
    fn refresh_bar(&mut self, comp: &Comp) {
        if self.transport.bar_moved() {
            let now = self.now();
            self.transport.set_bar(comp, self.player.bar(now));
        }
    }

    /// `p` in the bottom panel's coordinates; `None` = the cursor left the window. Coming back to
    /// the row before it folds keeps it open.
    fn watch_row(&mut self, p: Option<(f32, f32)>) {
        if !self.transport.row_open() {
            return;
        }
        let on = p.is_some_and(|p| self.transport.on_row(p));
        if on && self.row_leaving {
            self.stop_timer(TIMER_ROW);
            self.row_leaving = false;
        } else if !on && !self.row_leaving {
            self.start_timer(TIMER_ROW, ROW_MS);
            self.row_leaving = true;
        }
    }

    fn fold_row(&mut self, comp: &Comp) {
        self.stop_timer(TIMER_ROW);
        self.row_leaving = false;
        log(self.transport.close_row(comp));
        let now = self.now();
        self.transport.set_bar(comp, self.player.bar(now));
    }

    fn press(&mut self, comp: &Comp, p: (f32, f32)) -> Option<VideoRequest> {
        let mut taken = false;
        let mut request = None;
        if self.panels_shown {
            let at = self.in_bottom(p);
            if self.bar.press(comp, p) {
                taken = true;
            } else {
                match self.transport.at(at) {
                    Some(Spot::Bar(f)) => {
                        let now = self.now();
                        self.scrub = Some(f);
                        self.transport.hold_bar(comp, true);
                        log(self.transport.set_tip(comp, Some(f)));
                        request = refused(self.player.grab(f, now));
                        self.sync(comp);
                        taken = true;
                    }
                    Some(Spot::Volume(v)) => {
                        self.volume_drag = true;
                        request = refused(self.player.set_volume(v));
                        log(self.transport.set_show(comp, self.show_state()));
                        taken = true;
                    }
                    Some(Spot::Length) => {
                        self.remaining = !self.remaining;
                        self.draw_times(comp);
                    }
                    Some(Spot::Button(_) | Spot::Step(_)) => {
                        taken = self.transport.press(comp, at);
                    }
                    None => {}
                }
            }
        }
        if !taken && self.zoomed() && self.on_picture(p) {
            self.pan = Some(p);
            taken = true;
        }
        if taken {
            unsafe {
                SetCapture(self.hwnd);
            }
        }
        request
    }

    fn release(&mut self, comp: &Comp, p: (f32, f32)) -> Option<VideoRequest> {
        self.pan = None;
        let at = self.in_bottom(p);
        let request = if let Some(f) = self.scrub.take() {
            self.end_scrub(comp, f)
        } else if std::mem::take(&mut self.volume_drag) {
            log(self.transport.hover(comp, Some(at), false));
            None
        } else if let Some(item) = self.bar.release(comp, p) {
            match item {
                Item::Folder => Some(VideoRequest::Folder),
                Item::Copy => Some(VideoRequest::Copy),
                Item::Delete => Some(VideoRequest::Delete),
                Item::Name => Some(VideoRequest::CopyPath),
                Item::Edit => self.enter_edit(comp),
            }
        } else {
            match self.transport.release(comp, at) {
                Some(Spot::Button(button)) => self.button(comp, button),
                Some(Spot::Step(step)) => {
                    let now = self.now();
                    let done = self.player.set_speed(step, now);
                    self.fold_row(comp);
                    self.sync(comp);
                    refused(done)
                }
                _ => None,
            }
        };
        // Capture is released last: this message arrives at once, into the already torn-down state.
        unsafe {
            let _ = ReleaseCapture();
        }
        request
    }

    fn end_scrub(&mut self, comp: &Comp, f: f64) -> Option<VideoRequest> {
        let now = self.now();
        let done = self.player.release(f, now);
        self.transport.hold_bar(comp, false);
        log(self.transport.set_tip(comp, None));
        self.sync(comp);
        refused(done)
    }

    /// Windows' move loop for a press on the picture has returned, which it does when the button
    /// is let go. Should it ever return with the button still down, the press goes on as ours.
    pub fn pressed(&mut self, comp: &Comp) -> Option<VideoRequest> {
        if !std::mem::take(&mut self.in_move) {
            return None;
        }
        if unsafe { GetKeyState(VK_LBUTTON.0 as i32) } < 0 {
            unsafe {
                SetCapture(self.hwnd);
            }
            return None;
        }
        self.click_held(comp).or_else(|| self.end_fast(comp))
    }

    /// A press of ours on the picture that left the drag rectangle before it became a hold is
    /// neither a click nor a hold.
    fn drag_held(&mut self) {
        let Some((x0, y0)) = self.held else {
            return;
        };
        let mut p = POINT::default();
        let (w, h) = unsafe {
            let _ = GetCursorPos(&mut p);
            (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG))
        };
        if (p.x - x0).abs() <= w && (p.y - y0).abs() <= h {
            return;
        }
        self.held = None;
        self.stop_timer(TIMER_HOLD);
        unsafe {
            let _ = ReleaseCapture();
        }
    }

    /// Let go before it became a drag or a hold: a click, pause or play.
    fn click_held(&mut self, comp: &Comp) -> Option<VideoRequest> {
        self.held.take()?;
        self.stop_timer(TIMER_HOLD);
        let now = self.now();
        let done = self.player.toggle(now);
        self.sync(comp);
        self.flash(comp);
        refused(done)
    }

    fn start_fast(&mut self, comp: &Comp) -> Option<VideoRequest> {
        let now = self.now();
        let done = self.player.boost(now);
        self.sync(comp);
        self.signs.set_fast(comp, self.player.boosted());
        refused(done)
    }

    fn end_fast(&mut self, comp: &Comp) -> Option<VideoRequest> {
        if !self.player.boosted() {
            return None;
        }
        let now = self.now();
        let done = self.player.unboost(now);
        self.sync(comp);
        self.signs.set_fast(comp, false);
        refused(done)
    }

    fn button(&mut self, comp: &Comp, button: Button) -> Option<VideoRequest> {
        let now = self.now();
        let done = match button {
            Button::Play => self.player.toggle(now),
            Button::Back => self.player.skip(false, now),
            Button::Forward => self.player.skip(true, now),
            Button::Sound => self.player.toggle_mute(),
            Button::Speed => {
                log(self.transport.open_row(comp));
                self.transport.set_bar(comp, self.player.bar(now));
                Ok(())
            }
            Button::Shot => return Some(self.shot()),
            Button::Full => {
                self.toggle_full(comp);
                Ok(())
            }
        };
        self.sync(comp);
        refused(done)
    }

    fn shot(&self) -> VideoRequest {
        let frame = self.player.engine().frame().map_err(Failed::from);
        VideoRequest::Frame(frame)
    }

    fn key(&mut self, comp: &Comp, key: u16) -> Option<VideoRequest> {
        let held = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
        let (ctrl, shift) = (held(VK_CONTROL.0), held(VK_SHIFT.0));
        let now = self.now();
        let done = match key {
            k if k == VK_ESCAPE.0 => {
                if self.windowed.is_none() {
                    return Some(VideoRequest::Close);
                }
                self.toggle_full(comp);
                Ok(())
            }
            k if ctrl && shift && k == u16::from(b'C') => return Some(self.shot()),
            k if ctrl && k == u16::from(b'C') => return Some(VideoRequest::Copy),
            _ if ctrl => return None,
            k if k == VK_SPACE.0 => self.player.toggle(now),
            k if k == VK_LEFT.0 => self.player.skip(false, now),
            k if k == VK_RIGHT.0 => self.player.skip(true, now),
            k if k == VK_OEM_COMMA.0 => self.player.step(false),
            k if k == VK_OEM_PERIOD.0 => self.player.step(true),
            k if k == u16::from(b'F') => {
                self.toggle_full(comp);
                Ok(())
            }
            _ => return None,
        };
        self.wake(comp);
        self.sync(comp);
        if key == VK_SPACE.0 {
            self.flash(comp);
        }
        refused(done)
    }

    /// The whole monitor without the window's corners and border, and back to where the window was.
    fn toggle_full(&mut self, comp: &Comp) {
        if self.turn.is_some() || self.leaving.is_some() {
            return;
        }
        match self.windowed {
            Some(rect) => self.leave_full(comp, rect),
            None => self.enter_full(comp),
        }
    }

    /// The window takes the monitor at once; the picture and its black grow there from where the
    /// window was. The panels come back when it is done.
    fn enter_full(&mut self, comp: &Comp) {
        let window = window_rect(self.hwnd);
        let monitor = platform::monitor_rect(platform::window_monitor(self.hwnd));
        let shift = self.shift(window, monitor);
        let lens = shifted(self.lens, shift);
        let back = (shift.0, shift.1, self.size.0, self.size.1);
        let picture = self.editor.as_ref().map(|e| moved(e.picture(), shift));
        self.windowed = Some(window);
        window::set_framed(self.hwnd, false);
        self.set_panels(comp, false, false);
        self.place_window(comp, monitor);
        self.fit();
        log(self.layout(comp, false));
        log(self.turn_from(comp, lens, back));
        if let (Some(editor), Some(from)) = (&self.editor, picture) {
            editor.set_panels(comp, false);
            editor.glide_from(comp, from, TURN);
        }
        self.turn = Some(Turn::In);
        self.start_timer(TIMER_TURN, TURN.ms);
    }

    /// The picture and its black shrink to where the window will be; the window takes that place
    /// when they get there.
    fn leave_full(&mut self, comp: &Comp, rect: RECT) {
        let monitor = window_rect(self.hwnd);
        let shift = self.shift(rect, monitor);
        let size = self.logical((rect.right - rect.left, rect.bottom - rect.top));
        let mut lens = Lens::new();
        lens.fit(area_of(self.metrics, false, size), self.content());
        self.set_panels(comp, false, false);
        log(self.turn_to(
            comp,
            shifted(lens, shift),
            (shift.0, shift.1, size.0, size.1),
        ));
        if let Some(editor) = &self.editor {
            editor.set_panels(comp, false);
            editor.glide_to(comp, moved(editor.picture_in(size), shift), TURN);
        }
        self.turn = Some(Turn::Out(rect));
        self.start_timer(TIMER_TURN, TURN.ms);
    }

    fn end_turn(&mut self, comp: &Comp) {
        if let Some(Turn::Out(rect)) = self.turn.take() {
            self.windowed = None;
            window::set_framed(self.hwnd, true);
            self.place_window(comp, rect);
            self.fit();
            log(self.layout(comp, false));
        }
        let full = self.windowed.is_some();
        if let Some(editor) = &mut self.editor {
            editor.set_full(comp, full);
            editor.set_panels(comp, true);
        }
        self.sync(comp);
        self.wake(comp);
    }

    /// Logical pixels from `outer`'s corner to `inner`'s.
    fn shift(&self, inner: RECT, outer: RECT) -> (f32, f32) {
        self.logical((inner.left - outer.left, inner.top - outer.top))
    }

    /// The picture and the backdrop jump to `lens` and `back`, then go to where they are now.
    fn turn_from(&self, comp: &Comp, lens: Lens, back: Room) -> Result<()> {
        let (to_lens, to_back) = (self.lens, (0.0, 0.0, self.size.0, self.size.1));
        comp.stop(&self.canvas, "Scale")?;
        comp.stop(&self.canvas, "Offset")?;
        comp.stop(&self.backdrop, "Offset")?;
        comp.stop(&self.backdrop, "Size")?;
        self.canvas.SetScale(scale(lens.zoom, lens.zoom))?;
        self.canvas.SetOffset(v3(lens.offset.0, lens.offset.1))?;
        self.backdrop.SetOffset(v3(back.0, back.1))?;
        self.backdrop.SetSize(v2(back.2, back.3))?;
        self.turn_to(comp, to_lens, to_back)
    }

    fn turn_to(&self, comp: &Comp, lens: Lens, back: Room) -> Result<()> {
        comp.animate_vector3(&self.canvas, "Scale", scale(lens.zoom, lens.zoom), TURN)?;
        comp.animate_vector3(
            &self.canvas,
            "Offset",
            v3(lens.offset.0, lens.offset.1),
            TURN,
        )?;
        comp.animate_vector3(&self.backdrop, "Offset", v3(back.0, back.1), TURN)?;
        comp.animate_vector2(&self.backdrop, "Size", v2(back.2, back.3), TURN)
    }

    fn place_window(&mut self, comp: &Comp, rect: RECT) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        // The size message arrives inside this call, while the island is busy with it: the new
        // size is taken here.
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        log(self.resize(comp, self.logical((client.right, client.bottom))));
    }

    fn touchpad_scroll(&mut self, delta: f32) -> bool {
        let now = Instant::now();
        if delta % 120.0 != 0.0 {
            self.touchpad_until = Some(now + TOUCHPAD_STREAM);
            return true;
        }
        self.touchpad_until.is_some_and(|until| now < until)
    }

    fn scroll(&mut self, comp: &Comp, delta: (f32, f32)) {
        if !self.zoomed() {
            return;
        }
        self.lens.pan(delta, self.area(), self.content());
        log(self.layout(comp, false));
    }

    fn track_leave(&mut self) {
        if self.tracking {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        self.tracking = unsafe { TrackMouseEvent(&mut track) }.is_ok();
    }

    fn logical(&self, (x, y): (i32, i32)) -> (f32, f32) {
        (x as f32 / self.scale, y as f32 / self.scale)
    }

    fn at_screen(&self, (x, y): (i32, i32)) -> (f32, f32) {
        let mut p = POINT { x, y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        self.logical((p.x, p.y))
    }

    fn cursor(&self) -> (f32, f32) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        self.at_screen((p.x, p.y))
    }

    fn area(&self) -> Room {
        if self.editor.is_some() {
            return MontageEditor::area(self.metrics, self.size);
        }
        area_of(self.metrics, self.windowed.is_some(), self.size)
    }

    fn content(&self) -> (f32, f32) {
        (self.full.0 as f32, self.full.1 as f32)
    }

    fn zoomed(&self) -> bool {
        self.lens.zoomed(self.area(), self.content())
    }

    fn fit(&mut self) {
        self.lens.fit(self.area(), self.content());
    }

    fn zoom_at(&mut self, comp: &Comp, zoom: f32, p: (f32, f32)) {
        self.lens
            .zoom_at(zoom, p, self.area(), self.content(), self.scale);
        log(self.layout(comp, true));
    }

    fn layout(&self, comp: &Comp, animate: bool) -> Result<()> {
        let size = scale(self.lens.zoom, self.lens.zoom);
        let at = v3(self.lens.offset.0, self.lens.offset.1);
        if animate {
            comp.animate_vector3(&self.canvas, "Scale", size, ZOOM)?;
            comp.animate_vector3(&self.canvas, "Offset", at, ZOOM)
        } else {
            comp.stop(&self.canvas, "Scale")?;
            comp.stop(&self.canvas, "Offset")?;
            self.canvas.SetScale(size)?;
            self.canvas.SetOffset(at)
        }
    }

    fn resize(&mut self, comp: &Comp, size: (f32, f32)) -> Result<()> {
        let was_fit = !self.zoomed();
        self.size = size;
        comp.stop(&self.backdrop, "Offset")?;
        comp.stop(&self.backdrop, "Size")?;
        self.backdrop.SetOffset(v3(0.0, 0.0))?;
        self.backdrop.SetSize(v2(size.0, size.1))?;
        let strip = if self.metrics.floating() {
            0.0
        } else {
            self.metrics.strip
        };
        self.top_strip.SetSize(v2(size.0, strip))?;
        self.bottom_strip.SetSize(v2(size.0, strip))?;
        if was_fit {
            self.fit();
        } else {
            self.lens.clamp(self.area(), self.content());
        }
        self.layout(comp, false)?;
        self.place_signs()?;
        self.bar.layout(comp, size.0)?;
        self.transport.layout(comp, size.0, false)?;
        self.place_panels(comp, false)?;
        let now = self.now();
        self.transport.set_bar(comp, self.player.bar(now));
        if let Some(editor) = &mut self.editor {
            editor.resize(comp, size);
        }
        Ok(())
    }

    /// The 2× sign under the top panel when the panel lies on the picture.
    fn place_signs(&self) -> Result<()> {
        let area = self.area();
        let panel = if self.panels_on_picture() {
            self.top_band()
        } else {
            0.0
        };
        self.signs
            .layout(area, area.1 + panel + self.metrics.margin)
    }

    fn rescale(&mut self, comp: &Comp, k: f32, lparam: LPARAM) {
        self.scale = k;
        log(self.root.SetScale(scale(k, k)));
        unsafe {
            // Windows suggests a new window place: take it.
            let rect = *(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let size = self.logical((client.right, client.bottom));
        self.restyle(comp, size);
    }

    /// New metrics or a new scale: both panels are built again.
    fn restyle(&mut self, comp: &Comp, size: (f32, f32)) {
        let m = self.metrics;
        let file = self.bar.file().cloned();
        match Bar::new(comp, self.scale, m, file, false) {
            Ok(mut bar) => {
                bar.set_editable(self.player.info().is_some());
                bar.panel
                    .set_shown(comp, self.editor.is_none(), m.slide_distance());
                if let Ok(children) = self.top.Children() {
                    let _ = children.Remove(&self.bar.panel.root);
                    let _ = children.InsertAtTop(&bar.panel.root);
                }
                self.bar = bar;
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)),
        }
        match Signs::new(comp, self.scale, m) {
            Ok(signs) => {
                if let Ok(children) = self.root.Children() {
                    let _ = children.InsertAbove(&signs.root, &self.signs.root);
                    let _ = children.Remove(&self.signs.root);
                }
                self.signs = signs;
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)),
        }
        match Transport::new(comp, self.scale, m, self.duration(), self.show_state()) {
            Ok(transport) => {
                if let Ok(children) = self.bottom.Children() {
                    let _ = children.Remove(&self.transport.root);
                    let _ = children.InsertAtTop(&transport.root);
                }
                self.transport = transport;
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)),
        }
        if let Some(editor) = &mut self.editor {
            editor.restyle(comp, self.scale, m);
        }
        log(self.resize(comp, size));
        self.sync(comp);
    }
}

/// A player call the engine refused: a press that did nothing.
fn refused(done: std::result::Result<(), Failed>) -> Option<VideoRequest> {
    done.err()
        .map(|failed| VideoRequest::Trouble(Trouble::Control(failed)))
}

/// The strip look in a window gives each strip a row of its own above or below the picture; in
/// full screen and without the strip the panels lie on the picture.
fn strip_room(metrics: Metrics, full: bool) -> f32 {
    if metrics.floating() || full {
        0.0
    } else {
        metrics.strip
    }
}

fn area_of(metrics: Metrics, full: bool, (w, h): (f32, f32)) -> Room {
    let strip = strip_room(metrics, full);
    (0.0, strip, w, (h - 2.0 * strip).max(1.0))
}

fn moved((x, y, w, h): Room, (dx, dy): (f32, f32)) -> Room {
    (x + dx, y + dy, w, h)
}

fn shifted(lens: Lens, (dx, dy): (f32, f32)) -> Lens {
    Lens {
        zoom: lens.zoom,
        offset: (lens.offset.0 + dx, lens.offset.1 + dy),
    }
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    rect
}

fn show_of(player: Option<&Player<Video>>, (volume, muted): (f64, bool)) -> Show {
    Show {
        running: player.is_some_and(Player::running),
        ended: player.is_some_and(|p| p.phase() == Phase::Ended),
        muted,
        volume,
        speed: player.map_or(crate::play::NORMAL, Player::speed),
        full: false,
    }
}

impl Show {
    fn with_full(self, full: bool) -> Self {
        Self { full, ..self }
    }
}

impl Drop for VideoViewer {
    fn drop(&mut self) {
        // The editor drops its timers and engines while the window is alive.
        self.editor = None;
        self.leaving = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        // First disconnect the window from Composition, then close it. The engine stops after:
        // its last notes go to a window that is gone, and Windows drops them.
        self.target = None;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
