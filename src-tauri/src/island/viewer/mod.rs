//! Screenshot viewer window: a dark strip with buttons on top, the shot under it. By setting there may be no
//! strip: then dark capsules lie right on the shot ([`Metrics`]).
//!
//! - the window is about 80% of the work area, in the shot's proportions; it opens by the shot's flight from
//!   the island card or from the middle of the screen (the flight itself is `flight.rs`); a photo from the
//!   disk appears in place ([`Spot`]). Windows draws the shadow and rounding. The window can be dragged by
//!   the strip and by the whole shot;
//! - the wheel zooms around the cursor, a double click toggles "fit" and "pixel for pixel"; a touchpad pinch
//!   zooms too, and a two-finger scroll moves the zoomed shot. With the mouse it is dragged with the left
//!   button, and in the editor (where the left one draws) with the middle or right one, or the left with
//!   Space held;
//! - viewing: the `bar.rs` panel, Esc closes, Ctrl+C copies, Ctrl+X moves the shot to the clipboard only;
//!   Edit starts the editor (`editor.rs`).
//!
//! The shot lies on a "canvas" in its own pixels: the canvas scales and shifts as a whole, together with
//! the editor's marks and the crop.
//!
//! The window decides nothing about the shot: for clicks and keys it answers with a request ([`Request`]),
//! and the island carries it out (`screenshot.rs`). The window holds memory only while open: the shot at
//! its own resolution is one Composition surface (in the editor, plus a marks sheet of the same size).

mod bar;
mod capsule;
mod editor;
mod lens;
mod montage;
mod paint;
mod signs;
mod transport;
mod video;

mod file_card;

pub use file_card::FileCard;
pub use montage::Order;
pub use video::{Opening, Trouble, VideoRequest, VideoViewer};

use super::comp::{Comp, Motion, color, scale, v2, v3};
use super::flight::Place;
use super::{log, point, wheel_delta, window};
use crate::capture::Image;
use crate::lang::tr;
use crate::platform;
use crate::settings::ViewerBar;
use crate::shot::{Target, edit::Area};
use bar::{Bar, Item};
use capsule::Look;
use editor::{Editor, Effect, Pointer};
use lens::{Lens, Room};
use std::time::{Duration, Instant};
use windows::{
    UI::Composition::{ContainerVisual, Desktop::DesktopWindowTarget, InsetClip, SpriteVisual},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        System::SystemServices::MK_CONTROL,
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
                TrackMouseEvent, VK_CONTROL, VK_ESCAPE, VK_SHIFT, VK_SPACE,
            },
            WindowsAndMessaging::{
                DestroyWindow, GetClientRect, GetCursorPos, GetWindowRect, HTCAPTION, HTCLIENT,
                IDC_CROSS, IDC_HAND, IDC_SIZEALL, IsIconic, KillTimer, LoadCursorW, MINMAXINFO,
                SIZE_MINIMIZED, SW_HIDE, SW_RESTORE, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
                SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetForegroundWindow, SetTimer,
                SetWindowPos, ShowWindow, WM_CAPTURECHANGED, WM_CLOSE, WM_DPICHANGED,
                WM_GETMINMAXINFO, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
                WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCMOUSEMOVE,
                WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE, WM_TIMER,
            },
        },
    },
    core::Result,
};

/// The window takes at most this share of the work area and never enlarges the shot beyond pixel for pixel.
const SHARE: f32 = 0.8;
const MIN_SIZE: (f32, f32) = (360.0, 220.0);
/// Windows 11 window rounding: the shot flies into a window with the same corners.
pub const CORNER_RADIUS: f32 = 8.0;
const WHEEL_STEP: f32 = 1.2;
const SCROLL_PX: f32 = 100.0 / 120.0;
/// How long after a small scroll fraction the whole stream counts as a touchpad.
const TOUCHPAD_STREAM: Duration = Duration::from_millis(300);

const STRIP: &str = "#1C1C1E";
const TEXT: &str = "#F5F5F7";
const MUTED: &str = "#8E8E93";
const CHECK: &str = "#30D158";
const FILL: (f32, f32) = (0.08, 0.16);
const SELECTED: (f32, f32) = (0.26, 0.32);
const RED: &str = "#FF453A";
const RED_FILL: (f32, f32) = (0.24, 0.34);
const RED_ICON: &str = "#FF6961";
const FLOAT_ALPHA: f32 = 0.86;
const FLOAT_FILL: (&str, &str) = ("#1C1C1E", "#3A3A3C");
const FLOAT_SELECTED: (&str, &str) = ("#4A4A4E", "#58585C");
const FLOAT_RED: (&str, &str) = ("#3D110E", "#5A1914");
const ACCENT: (&str, &str) = ("#0A84FF", "#3D9BFF");

const ZOOM: Motion = Motion::curve(200, 0.2, 0.0, 0.0, 1.0);

const FEEDBACK_MS: u32 = 1200;
/// Viewer timers (on its own window; the editor has its own, 3 and 4).
const TIMER_COPIED: usize = 1;
const TIMER_PATH: usize = 2;
/// Not in the windows crate: the cursor left the window (after TrackMouseEvent).
const WM_MOUSELEAVE: u32 = 0x02A3;

/// How the buttons stand, by setting: on a dark strip on top (buttons in the strip's center, the file name
/// in the middle) or right on the shot, without strip or name.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    /// Strip height; 0 = no strip, the buttons lie on the shot.
    strip: f32,
    button: (f32, f32),
    icon: f32,
    margin: f32,
    gap: f32,
    text: f32,
}

impl Metrics {
    fn of(bar: ViewerBar) -> Self {
        match bar {
            ViewerBar::Strip => Self {
                strip: 40.0,
                button: (38.0, 24.0),
                icon: 15.0,
                margin: 8.0,
                gap: 6.0,
                text: 13.0,
            },
            ViewerBar::Floating => Self {
                strip: 0.0,
                button: (38.0, 28.0),
                icon: 16.0,
                margin: 12.0,
                gap: 6.0,
                text: 13.0,
            },
        }
    }

    fn floating(self) -> bool {
        self.strip == 0.0
    }

    fn button_top(self) -> f32 {
        if self.floating() {
            self.margin
        } else {
            (self.strip - self.button.1) / 2.0
        }
    }

    fn slide_distance(self) -> f32 {
        self.button_top() + self.button.1 + 4.0
    }

    fn plain(self) -> Look {
        if self.floating() {
            Look::new(FLOAT_FILL.0, FLOAT_ALPHA, FLOAT_FILL.1, FLOAT_ALPHA)
        } else {
            Look::new("#FFFFFF", FILL.0, "#FFFFFF", FILL.1)
        }
    }

    fn selected(self) -> Look {
        if self.floating() {
            Look::new(FLOAT_SELECTED.0, 0.95, FLOAT_SELECTED.1, 0.95)
        } else {
            Look::new("#FFFFFF", SELECTED.0, "#FFFFFF", SELECTED.1)
        }
    }

    fn clear(self) -> Look {
        Look::new("#FFFFFF", 0.0, "#FFFFFF", FILL.1)
    }

    fn danger(self) -> Look {
        if self.floating() {
            Look::new(FLOAT_RED.0, FLOAT_ALPHA, FLOAT_RED.1, FLOAT_ALPHA)
        } else {
            Look::new(RED, RED_FILL.0, RED, RED_FILL.1)
        }
    }

    fn accent(self) -> Look {
        Look::new(ACCENT.0, 1.0, ACCENT.1, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Close,
    Copy,
    Cut,
    Delete,
    Folder,
    CopyPath,
    Save(Target),
}

pub struct Viewer {
    hwnd: HWND,
    target: Option<DesktopWindowTarget>,
    root: ContainerVisual,
    backdrop: SpriteVisual,
    /// The shot in its own pixels with marks; scale = zoom, crop = its frame.
    canvas: ContainerVisual,
    clip: InsetClip,
    photo: SpriteVisual,
    strip: SpriteVisual,
    bar: Bar,
    editor: Option<Editor>,
    retired: Option<ContainerVisual>,
    metrics: Metrics,
    scale: f32,
    size: (f32, f32),
    full: (u32, u32),
    /// The visible part of the shot (all of it, or the editor's crop) in the window.
    lens: Lens,
    pan: Option<(f32, f32)>,
    tracking: bool,
    space: bool,
    touchpad_until: Option<Instant>,
}

impl Viewer {
    /// A hidden window with the shot at `spot`. `has_file` = the shot will have a file: "Show
    /// in folder" is available. `title` is for the taskbar and Alt+Tab.
    pub fn new(
        comp: &Comp,
        spot: Spot,
        k: f32,
        image: &Image,
        has_file: bool,
        bar: ViewerBar,
        title: &str,
    ) -> Result<Self> {
        let metrics = Metrics::of(bar);
        let work = spot.work;
        let (iw, ih) = (image.width as f32 / k, image.height as f32 / k);
        let room = (
            (work.right - work.left) as f32 / k * SHARE,
            (work.bottom - work.top) as f32 / k * SHARE - metrics.strip,
        );
        let fit = (room.0 / iw).min(room.1 / ih).min(1.0);
        let size = (
            (iw * fit).round().max(MIN_SIZE.0),
            (ih * fit).round().max(MIN_SIZE.1) + metrics.strip,
        );
        let px = ((size.0 * k).round() as i32, (size.1 * k).round() as i32);
        let hwnd = window::create_viewer(spot.rect(px), title)?;
        Self::build(comp, hwnd, (k, metrics), size, image, has_file).inspect_err(|_| unsafe {
            let _ = DestroyWindow(hwnd);
        })
    }

    fn build(
        comp: &Comp,
        hwnd: HWND,
        (k, metrics): (f32, Metrics),
        size: (f32, f32),
        image: &Image,
        has_file: bool,
    ) -> Result<Self> {
        let (target, root) = comp.window_root(hwnd)?;
        root.SetScale(scale(k, k))?;
        let c = &comp.compositor;
        let backdrop = c.CreateSpriteVisual()?;
        backdrop.SetBrush(&comp.brush(color(STRIP, 1.0))?)?;
        root.Children()?.InsertAtTop(&backdrop)?;
        let canvas = comp.container()?;
        let clip = c.CreateInsetClip()?;
        canvas.SetClip(&clip)?;
        root.Children()?.InsertAtTop(&canvas)?;
        let photo = photo(comp, image)?;
        canvas.Children()?.InsertAtTop(&photo)?;
        let strip = c.CreateSpriteVisual()?;
        strip.SetBrush(&comp.brush(color(STRIP, 1.0))?)?;
        root.Children()?.InsertAtTop(&strip)?;
        let bar = Bar::new(comp, k, metrics, None, has_file)?;
        root.Children()?.InsertAtTop(&bar.panel.root)?;

        let mut viewer = Self {
            hwnd,
            target: Some(target),
            root,
            backdrop,
            canvas,
            clip,
            photo,
            strip,
            bar,
            editor: None,
            retired: None,
            metrics,
            scale: k,
            size,
            full: (image.width, image.height),
            lens: Lens::new(),
            pan: None,
            tracking: false,
            space: false,
            touchpad_until: None,
        };
        viewer.set_full(image)?;
        viewer.fit();
        viewer.resize(comp, size)?;
        Ok(viewer)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Shows the window and gives it focus. `animated` = with Windows' appear animation;
    /// without it the window stands under the arriving shot.
    pub fn show(&self, animated: bool) {
        show(self.hwnd, animated);
    }

    /// Shows the window over the others without taking the focus from the one in front.
    pub fn show_quietly(&self, animated: bool) {
        show_quietly(self.hwnd, animated);
    }

    /// The window disappears without animation: the shot flies out of it to the island.
    pub fn vanish(&self) {
        window::set_transitions(self.hwnd, false);
    }

    pub fn raise(&self) {
        raise(self.hwnd);
    }

    pub fn photo_place(&self, monitor: RECT) -> Place {
        let mut window = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut window);
        }
        let k = self.scale;
        let v = self.view();
        (
            (window.left - monitor.left) as f32 / k + self.lens.offset.0,
            (window.top - monitor.top) as f32 / k + self.lens.offset.1,
            v.w * self.lens.zoom,
            v.h * self.lens.zoom,
        )
    }

    pub fn set_file(&mut self, comp: &Comp, name: Option<String>) {
        log(self.bar.set_file(comp, name));
    }

    pub fn show_copied(&self, comp: &Comp) {
        self.bar.show_copied(comp, true);
        self.start_timer(TIMER_COPIED);
    }

    pub fn show_path_copied(&mut self, comp: &Comp) {
        log(self.bar.show_path_copied(comp, true));
        self.start_timer(TIMER_PATH);
    }

    fn start_timer(&self, id: usize) {
        unsafe {
            SetTimer(Some(self.hwnd), id, FEEDBACK_MS, None);
        }
    }

    pub fn set_bar(&mut self, comp: &Comp, bar: ViewerBar) {
        let metrics = Metrics::of(bar);
        if metrics != self.metrics {
            self.metrics = metrics;
            self.restyle(comp, self.size);
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    pub fn has_edits(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::is_dirty)
    }

    pub fn flatten(&self, image: &Image) -> Option<Image> {
        let editor = self.editor.as_ref()?;
        editor
            .flatten(image)
            .inspect_err(|e| crate::journal::warn(tr!("journal.viewer_build_failed", error = e)))
            .ok()
    }

    fn enter_edit(&mut self, comp: &Comp) {
        let editor = Editor::new(
            comp,
            self.hwnd,
            &self.root,
            &self.canvas,
            (self.scale, self.metrics),
            self.size.0,
            self.full,
        );
        match editor {
            Ok(editor) => {
                self.bar.cancel(comp);
                self.bar.hover(comp, None);
                self.bar
                    .panel
                    .slide(comp, false, self.metrics.slide_distance());
                self.editor = Some(editor);
                self.make_room(comp);
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_editor_failed", error = e)),
        }
    }

    fn min_size(&self) -> (f32, f32) {
        let panel = self.editor.as_ref().map_or(0.0, Editor::min_width);
        (MIN_SIZE.0.max(panel), MIN_SIZE.1 + self.metrics.strip)
    }

    fn make_room(&mut self, comp: &Comp) {
        let need = self.min_size().0;
        if self.size.0 >= need {
            return;
        }
        let mut rect = RECT::default();
        unsafe {
            if GetWindowRect(self.hwnd, &mut rect).is_err() {
                return;
            }
        }
        let work = platform::work_rect(platform::window_monitor(self.hwnd));
        let width = ((need * self.scale).ceil() as i32).min(work.right - work.left);
        let left = (rect.left - (width - (rect.right - rect.left)) / 2)
            .clamp(work.left, work.right - width);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                left,
                rect.top,
                width,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        // The WM_SIZE of the call above arrives while the state is busy and is dropped: take the size here.
        let mut client = RECT::default();
        unsafe {
            if GetClientRect(self.hwnd, &mut client).is_err() {
                return;
            }
        }
        let size = self.logical((client.right, client.bottom));
        log(self.resize(comp, size));
    }

    fn leave_edit(&mut self, comp: &Comp) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let panel = editor.retire(comp);
        drop(editor);
        if let Some(old) = self.retired.replace(panel)
            && let Ok(children) = self.root.Children()
        {
            let _ = children.Remove(&old);
        }
        self.bar
            .panel
            .slide(comp, true, self.metrics.slide_distance());
        self.fit();
        log(self.layout(comp, true));
    }

    pub fn finish_edit(&mut self, comp: &Comp, image: &Image, name: Option<String>) {
        match photo(comp, image) {
            Ok(fresh) => {
                if let Ok(children) = self.canvas.Children() {
                    let _ = children.Remove(&self.photo);
                    let _ = children.InsertAtBottom(&fresh);
                }
                self.photo = fresh;
                log(self.set_full(image));
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_show_failed", error = e)),
        }
        self.leave_edit(comp);
        self.set_file(comp, name);
    }

    pub fn close_editor(&mut self, comp: &Comp) {
        self.leave_edit(comp);
    }

    pub fn save_failed(&mut self, comp: &Comp) {
        if let Some(editor) = &mut self.editor {
            editor.save_failed(comp);
        }
    }

    fn apply(&mut self, comp: &Comp, effect: Option<Effect>) -> Option<Request> {
        match effect? {
            Effect::Refit => {
                self.fit();
                log(self.layout(comp, true));
                None
            }
            Effect::Exit => {
                self.leave_edit(comp);
                None
            }
            Effect::Save(target) => Some(Request::Save(target)),
        }
    }

    /// `image` is the shot without edits (the editor takes its pixels from it).
    /// Returns Windows' answer (`None` = default handler) and a request to the island.
    pub fn handle(
        &mut self,
        comp: &Comp,
        image: &Image,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (Option<LRESULT>, Option<Request>) {
        let done = |result: isize| (Some(LRESULT(result)), None);
        match msg {
            WM_NCHITTEST => {
                let p = self.at_screen(point(lparam));
                done(if self.is_client(p) {
                    HTCLIENT
                } else {
                    HTCAPTION
                } as isize)
            }
            WM_SETCURSOR => match self.cursor_shape(self.cursor()) {
                Some(shape) => {
                    unsafe {
                        if let Ok(cursor) = LoadCursorW(None, shape) {
                            SetCursor(Some(cursor));
                        }
                    }
                    done(1)
                }
                None => (None, None),
            },
            WM_MOUSEMOVE => {
                self.track_leave();
                let p = self.logical(point(lparam));
                if let Some(last) = self.pan {
                    self.lens
                        .pan((p.0 - last.0, p.1 - last.1), self.area(), self.content());
                    self.pan = Some(p);
                    log(self.layout(comp, false));
                } else if self.editor.is_some() {
                    let at = self.to_image(p);
                    if let Some(editor) = &mut self.editor {
                        editor.moved(comp, p, at, self.lens.zoom);
                    }
                } else {
                    self.bar.hover(comp, Some(p));
                }
                done(0)
            }
            // The cursor is on the window's "title" (the strip without buttons) or left the window.
            WM_NCMOUSEMOVE | WM_MOUSELEAVE => {
                if msg == WM_MOUSELEAVE {
                    self.tracking = false;
                }
                let p = (msg == WM_NCMOUSEMOVE).then(|| self.at_screen(point(lparam)));
                match &mut self.editor {
                    Some(editor) => editor.left(comp, p),
                    None => self.bar.hover(comp, None),
                }
                (None, None)
            }
            WM_LBUTTONDOWN => {
                let p = self.logical(point(lparam));
                self.press(comp, p);
                done(0)
            }
            WM_LBUTTONDBLCLK => {
                let p = self.logical(point(lparam));
                // In the editor and on buttons a second click is just another click.
                if self.editor.is_some() || self.bar.at(p).is_some() {
                    self.press(comp, p);
                } else {
                    self.toggle_zoom(comp, p);
                }
                done(0)
            }
            WM_LBUTTONUP => {
                let p = self.logical(point(lparam));
                (Some(LRESULT(0)), self.release(comp, image, p))
            }
            WM_MBUTTONDOWN | WM_RBUTTONDOWN => {
                let p = self.logical(point(lparam));
                if self.zoomed() && self.on_view(p) {
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
                if let Some(editor) = &mut self.editor {
                    editor.cancel_press(comp);
                }
                done(0)
            }
            // A double click on the whole shot or the strip (they are the window's "title"):
            // enlarge the shot instead of maximizing the window.
            WM_NCLBUTTONDBLCLK => {
                let p = self.at_screen(point(lparam));
                self.toggle_zoom(comp, p);
                done(0)
            }
            // Mouse wheel and touchpad pinch (it arrives with Ctrl) zoom; a two-finger scroll moves the zoomed shot.
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let p = self.at_screen(point(lparam));
                let delta = wheel_delta(wparam) as f32;
                let ctrl = wparam.0 & MK_CONTROL.0 as usize != 0;
                if msg == WM_MOUSEHWHEEL {
                    self.scroll(comp, (-delta * SCROLL_PX, 0.0));
                } else if !ctrl && self.touchpad_scroll(delta) {
                    self.scroll(comp, (0.0, delta * SCROLL_PX));
                } else {
                    self.zoom_at(comp, self.lens.zoom * WHEEL_STEP.powf(delta / 120.0), p);
                }
                done(0)
            }
            WM_KEYDOWN => {
                let request = self.key(comp, wparam.0 as u16);
                (Some(LRESULT(0)), request)
            }
            WM_KEYUP if wparam.0 as u16 == VK_SPACE.0 => {
                self.space = false;
                self.refresh_cursor();
                done(0)
            }
            WM_KILLFOCUS => {
                self.space = false;
                (None, None)
            }
            WM_CLOSE => {
                let close = match &mut self.editor {
                    Some(editor) => editor.may_close(comp),
                    None => true,
                };
                (Some(LRESULT(0)), close.then_some(Request::Close))
            }
            WM_SIZE => {
                if wparam.0 as u32 != SIZE_MINIMIZED {
                    let (w, h) = point(lparam);
                    log(self.resize(comp, self.logical((w, h))));
                }
                (None, None)
            }
            // A window the buttons don't fit in can no longer be made.
            WM_GETMINMAXINFO => {
                let (w, h) = self.min_size();
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
            WM_TIMER => {
                match wparam.0 {
                    TIMER_COPIED | TIMER_PATH => {
                        unsafe {
                            let _ = KillTimer(Some(self.hwnd), wparam.0);
                        }
                        if wparam.0 == TIMER_COPIED {
                            self.bar.show_copied(comp, false);
                        } else {
                            log(self.bar.show_path_copied(comp, false));
                        }
                    }
                    id => {
                        if let Some(editor) = &mut self.editor {
                            editor.on_timer(comp, id);
                        }
                    }
                }
                done(0)
            }
            _ => (None, None),
        }
    }

    /// Keys. There is no Ctrl+X in the editor: an edit can't be interrupted that way.
    fn key(&mut self, comp: &Comp, key: u16) -> Option<Request> {
        let held = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
        let (ctrl, shift) = (held(VK_CONTROL.0), held(VK_SHIFT.0));
        let letter = |c: u8| ctrl && key == u16::from(c);
        if key == VK_SPACE.0 && self.editor.is_some() {
            if !self.space {
                self.space = true;
                self.refresh_cursor();
            }
            return None;
        }
        let Some(editor) = &mut self.editor else {
            return match key {
                k if k == VK_ESCAPE.0 => Some(Request::Close),
                _ if letter(b'C') => Some(Request::Copy),
                _ if letter(b'X') => Some(Request::Cut),
                _ => None,
            };
        };
        let effect = match key {
            k if k == VK_ESCAPE.0 => editor.escape(comp),
            _ if letter(b'Z') && shift => editor.redo(comp),
            _ if letter(b'Z') => editor.undo(comp),
            _ if letter(b'Y') => editor.redo(comp),
            _ if letter(b'S') => Some(editor.save_key(shift)),
            _ if letter(b'C') => return Some(Request::Copy),
            _ => None,
        };
        self.apply(comp, effect)
    }

    fn press(&mut self, comp: &Comp, p: (f32, f32)) {
        let on_view = self.on_view(p);
        let at = on_view.then(|| self.to_image(p));
        let zoomed = self.zoomed();
        let taken = match &mut self.editor {
            Some(_) if self.space => {
                if zoomed && on_view {
                    self.pan = Some(p);
                }
                true
            }
            Some(editor) => editor.press(comp, p, at),
            None if self.bar.press(comp, p) => true,
            None if zoomed && on_view => {
                self.pan = Some(p);
                true
            }
            None => false,
        };
        if taken {
            unsafe {
                SetCapture(self.hwnd);
            }
        }
    }

    fn release(&mut self, comp: &Comp, image: &Image, p: (f32, f32)) -> Option<Request> {
        self.pan = None;
        let request = match &mut self.editor {
            Some(editor) => {
                let effect = editor.release(comp, p, image);
                self.apply(comp, effect)
            }
            None => match self.bar.release(comp, p)? {
                Item::Folder => Some(Request::Folder),
                Item::Copy => Some(Request::Copy),
                Item::Delete => Some(Request::Delete),
                Item::Name => Some(Request::CopyPath),
                Item::Edit => {
                    self.enter_edit(comp);
                    None
                }
            },
        };
        // Capture is released last: this message arrives at once, into the already torn-down state.
        unsafe {
            let _ = ReleaseCapture();
        }
        request
    }

    /// The pressable part of the window; the rest is the "title", the window is dragged by it.
    fn is_client(&self, p: (f32, f32)) -> bool {
        match &self.editor {
            Some(editor) => editor.wants(p) || self.on_view(p),
            None => self.bar.at(p).is_some() || (self.zoomed() && self.on_view(p)),
        }
    }

    fn cursor_shape(&self, p: (f32, f32)) -> Option<windows::core::PCWSTR> {
        if self.pan.is_some() || (self.space && self.on_view(p)) {
            return Some(IDC_SIZEALL);
        }
        if let Some(editor) = &self.editor {
            return editor.cursor(p, self.on_view(p)).map(|c| match c {
                Pointer::Hand => IDC_HAND,
                Pointer::Cross => IDC_CROSS,
            });
        }
        if self.bar.enabled_at(p).is_some() {
            Some(IDC_HAND)
        } else if self.bar.at(p).is_none() && self.zoomed() && self.on_view(p) {
            Some(IDC_SIZEALL)
        } else {
            None
        }
    }

    fn refresh_cursor(&self) {
        if let Some(shape) = self.cursor_shape(self.cursor()) {
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, shape) {
                    SetCursor(Some(cursor));
                }
            }
        }
    }

    /// The scroll came from a touchpad. A mouse wheel steps exactly by 120, a touchpad in small fractions;
    /// a touchpad scroll stream counts as touchpad as a whole, even if a round 120 turns up in it.
    fn touchpad_scroll(&mut self, delta: f32) -> bool {
        let now = Instant::now();
        if delta % 120.0 != 0.0 {
            self.touchpad_until = Some(now + TOUCHPAD_STREAM);
            return true;
        }
        self.touchpad_until.is_some_and(|until| now < until)
    }

    fn scroll(&mut self, comp: &Comp, (dx, dy): (f32, f32)) {
        if !self.zoomed() {
            return;
        }
        self.lens.pan((dx, dy), self.area(), self.content());
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

    fn view(&self) -> Area {
        self.editor.as_ref().map_or(
            Area {
                x: 0.0,
                y: 0.0,
                w: self.full.0 as f32,
                h: self.full.1 as f32,
            },
            Editor::visible,
        )
    }

    fn set_full(&mut self, image: &Image) -> Result<()> {
        self.full = (image.width, image.height);
        self.canvas
            .SetSize(v2(image.width as f32, image.height as f32))
    }

    fn to_image(&self, p: (f32, f32)) -> (f32, f32) {
        let v = self.view();
        let (x, y) = self.lens.point_at(p);
        (x + v.x, y + v.y)
    }

    fn on_view(&self, p: (f32, f32)) -> bool {
        p.1 >= self.metrics.strip && self.lens.covers(p, self.content())
    }

    fn area(&self) -> Room {
        let strip = self.metrics.strip;
        (0.0, strip, self.size.0, (self.size.1 - strip).max(1.0))
    }

    fn content(&self) -> (f32, f32) {
        let v = self.view();
        (v.w, v.h)
    }

    fn zoomed(&self) -> bool {
        self.lens.zoomed(self.area(), self.content())
    }

    fn fit(&mut self) {
        self.lens.fit(self.area(), self.content());
    }

    fn clamp(&mut self) {
        self.lens.clamp(self.area(), self.content());
    }

    fn zoom_at(&mut self, comp: &Comp, zoom: f32, p: (f32, f32)) {
        self.lens
            .zoom_at(zoom, p, self.area(), self.content(), self.scale);
        log(self.layout(comp, true));
    }

    /// Double click: fit ↔ pixel for pixel (and if the shot is already smaller than the window, twice that).
    fn toggle_zoom(&mut self, comp: &Comp, p: (f32, f32)) {
        if self.zoomed() {
            self.fit();
            log(self.layout(comp, true));
            return;
        }
        if p.1 < self.metrics.strip {
            return;
        }
        let target = Lens::closer(self.area(), self.content(), self.scale);
        self.zoom_at(comp, target, p);
    }

    fn layout(&self, comp: &Comp, animate: bool) -> Result<()> {
        let v = self.view();
        let (w, h) = (self.full.0 as f32, self.full.1 as f32);
        self.clip.SetLeftInset(v.x)?;
        self.clip.SetTopInset(v.y)?;
        self.clip.SetRightInset(w - v.right())?;
        self.clip.SetBottomInset(h - v.bottom())?;
        let size = scale(self.lens.zoom, self.lens.zoom);
        let at = v3(
            self.lens.offset.0 - v.x * self.lens.zoom,
            self.lens.offset.1 - v.y * self.lens.zoom,
        );
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
        self.backdrop.SetSize(v2(size.0, size.1))?;
        self.strip.SetSize(v2(size.0, self.metrics.strip))?;
        if was_fit {
            self.fit();
        } else {
            self.clamp();
        }
        self.layout(comp, false)?;
        if let Some(editor) = &mut self.editor {
            editor.resize(size.0);
        }
        self.bar.layout(comp, size.0)
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

    fn restyle(&mut self, comp: &Comp, size: (f32, f32)) {
        let m = self.metrics;
        let file = self.bar.file().cloned();
        match Bar::new(comp, self.scale, m, file, false) {
            Ok(bar) => {
                if let Ok(children) = self.root.Children() {
                    let _ = children.Remove(&self.bar.panel.root);
                    let _ = children.InsertAtTop(&bar.panel.root);
                }
                self.bar = bar;
                self.bar
                    .panel
                    .set_shown(comp, self.editor.is_none(), m.slide_distance());
            }
            Err(e) => crate::journal::warn(tr!("journal.viewer_panel_failed", error = e)),
        }
        if let Some(editor) = &mut self.editor {
            editor.restyle(comp, self.scale, m, size.0);
        }
        log(self.resize(comp, size));
    }
}

/// Where a viewer window opens (physical pixels): the middle of the work area, or around the
/// middle of the window it takes the place of.
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    pub work: RECT,
    pub center: Option<(i32, i32)>,
}

impl Spot {
    pub fn middle(work: RECT) -> Self {
        Self { work, center: None }
    }

    /// The place of a window `(w, h)` big, inside the work area.
    fn rect(self, (w, h): (i32, i32)) -> RECT {
        let work = self.work;
        let (cx, cy) = self
            .center
            .unwrap_or(((work.left + work.right) / 2, (work.top + work.bottom) / 2));
        let left = (cx - w / 2).clamp(work.left, (work.right - w).max(work.left));
        let top = (cy - h / 2).clamp(work.top, (work.bottom - h).max(work.top));
        RECT {
            left,
            top,
            right: left + w,
            bottom: top + h,
        }
    }
}

/// The middle of a window on the screen (physical pixels).
pub fn center_of(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

fn show(hwnd: HWND, animated: bool) {
    window::set_transitions(hwnd, animated);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
    }
    // Minimizing and closing already have Windows animations.
    window::set_transitions(hwnd, true);
}

fn show_quietly(hwnd: HWND, animated: bool) {
    window::set_transitions(hwnd, animated);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    window::set_transitions(hwnd, true);
}

fn raise(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

fn photo(comp: &Comp, image: &Image) -> Result<SpriteVisual> {
    comp.image_rounded(
        &image.pixels,
        (image.width, image.height),
        (image.width as f32, image.height as f32),
        0.0,
        1.0,
    )
}

impl Drop for Viewer {
    fn drop(&mut self) {
        // The editor drops its timers and pictures while the window is alive.
        self.editor = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        // First disconnect the window from Composition, then close it.
        self.target = None;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
