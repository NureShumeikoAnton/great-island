//! A dialog window: a [`Sheet`] (`crate::dialog`) drawn the way WebView2 drew the update window: a
//! black card with round corners on a window of its own, in the middle of the island's monitor,
//! above other windows, opening without taking the keyboard.
//!
//! - It is dragged by any empty spot; the buttons, the × and the notes keep their presses.
//! - A button answers when it is pressed and let go on it; after that the buttons are inert until
//!   the owner gives a new sheet or closes the window: an answer is given once.
//! - Esc and the × answer with the sheet's dismiss action, Alt+F4 with [`Request::Closed`].
//! - The wheel scrolls the notes under the pointer; the thumb is dragged, the track pages.
//! - A new sheet redraws the card in place; one where only the progress moved moves the bar.
//!   The card never gets lower while the window is open.
//!
//! It decides nothing: the island (`dialogs.rs`) carries out what it answers.

mod icon;
mod scene;

use super::comp::{Comp, Motion, scale, v3};
use super::{log, point, wheel_delta, window};
use crate::dialog::{
    Action, Block, Sheet,
    layout::{self, Layout, Measure, Placed, SCROLLBAR, Style},
};
use scene::{Scene, Spot, para, shown};
use windows::{
    UI::Composition::{ContainerVisual, Desktop::DesktopWindowTarget},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        UI::{
            Input::KeyboardAndMouse::{
                ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
            },
            WindowsAndMessaging::{
                DestroyWindow, GetCursorPos, HTCAPTION, HTCLIENT, IDC_HAND, LoadCursorW, SW_HIDE,
                SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetCursor,
                SetWindowPos, ShowWindow, WM_CAPTURECHANGED, WM_CLOSE, WM_DPICHANGED, WM_KEYDOWN,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST,
                WM_NCLBUTTONDBLCLK, WM_NCMOUSEMOVE, WM_SETCURSOR,
            },
        },
    },
    core::Result,
};

/// Not in the windows crate: the cursor left the window (after TrackMouseEvent).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The update window's CSS: it rises 16 px and grows from 96% as it fades in, and shrinks to 97% as
/// it fades out.
const RISE: f32 = 16.0;
const START_SCALE: f32 = 0.96;
const END_SCALE: f32 = 0.97;
const FADE_IN: Motion = Motion::curve(260, 0.25, 0.1, 0.25, 1.0);
const GROW_IN: Motion = Motion::curve(420, 0.2, 0.9, 0.25, 1.0);
const FADE_OUT: Motion = Motion::curve(160, 0.25, 0.1, 0.25, 1.0);
const SHRINK_OUT: Motion = Motion::curve(180, 0.25, 0.1, 0.25, 1.0);
/// After this the window can go: its fade-out is over.
pub const LEAVE_MS: u32 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Action(Action),
    /// Closed by Windows' own means (Alt+F4), with no answer.
    Closed,
}

struct Typeset<'a>(&'a Comp);

impl Measure for Typeset<'_> {
    fn size(&self, text: &str, style: Style, width: Option<f32>) -> (f32, f32) {
        self.0
            .paragraph_size(&shown(text, style), &para(style), width)
    }
}

pub struct Dialog {
    hwnd: HWND,
    target: Option<DesktopWindowTarget>,
    root: ContainerVisual,
    scene: Scene,
    sheet: Sheet,
    layout: Layout,
    card: f32,
    scale: f32,
    decor: bool,
    pressed: Option<Spot>,
    /// The thumb held: its block, the pointer's height and the scroll when it was taken.
    drag: Option<(usize, f32, f32)>,
    inert: bool,
    tracking: bool,
}

impl Dialog {
    /// Made and shown in the middle of `work` (physical pixels) without taking the keyboard.
    /// `decor`: with the appear and leave animations.
    pub fn open(
        comp: &Comp,
        work: RECT,
        k: f32,
        title: &str,
        sheet: Sheet,
        decor: bool,
    ) -> Result<Self> {
        let layout = layout::layout(&sheet, &Typeset(comp));
        let card = layout.height;
        let (w, h) = size_px(card, k);
        let left = work.left + (work.right - work.left - w) / 2;
        let top = work.top + (work.bottom - work.top - h) / 2;
        let hwnd = window::create_dialog(
            RECT {
                left,
                top,
                right: left + w,
                bottom: top + h,
            },
            title,
        )?;
        let built = (|| -> Result<(DesktopWindowTarget, ContainerVisual, Scene)> {
            let (target, root) = comp.window_root(hwnd)?;
            root.SetScale(scale(k, k))?;
            let scene = Scene::build(comp, &sheet, &layout, card, k, &[])?;
            root.Children()?.InsertAtTop(&scene.card)?;
            Ok((target, root, scene))
        })();
        let (target, root, scene) = built.inspect_err(|_| unsafe {
            let _ = DestroyWindow(hwnd);
        })?;
        let dialog = Self {
            hwnd,
            target: Some(target),
            root,
            scene,
            sheet,
            layout,
            card,
            scale: k,
            decor,
            pressed: None,
            drag: None,
            inert: false,
            tracking: false,
        };
        dialog.appear(comp)?;
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(dialog)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn appear(&self, comp: &Comp) -> Result<()> {
        let card = &self.scene.card;
        card.SetCenterPoint(v3(layout::WIDTH / 2.0, self.card / 2.0))?;
        if !self.decor {
            return Ok(());
        }
        card.SetOpacity(0.0)?;
        card.SetOffset(v3(0.0, RISE))?;
        card.SetScale(scale(START_SCALE, START_SCALE))?;
        comp.animate_scalar(card, "Opacity", 1.0, FADE_IN)?;
        comp.animate_vector3(card, "Offset", v3(0.0, 0.0), GROW_IN)?;
        comp.animate_vector3(card, "Scale", scale(1.0, 1.0), GROW_IN)
    }

    /// The fade-out; the window can be dropped [`LEAVE_MS`] later.
    pub fn leave(&self, comp: &Comp) {
        if !self.decor {
            return;
        }
        let card = &self.scene.card;
        log(comp.animate_scalar(card, "Opacity", 0.0, FADE_OUT));
        log(comp.animate_vector3(card, "Scale", scale(END_SCALE, END_SCALE), SHRINK_OUT));
    }

    pub fn decor(&self) -> bool {
        self.decor
    }

    /// The owner's new word: the card is redrawn (or only its bars move) and the buttons take
    /// presses again.
    pub fn set(&mut self, comp: &Comp, sheet: Sheet) {
        self.inert = false;
        if sheet == self.sheet {
            return;
        }
        let layout = layout::layout(&sheet, &Typeset(comp));
        let card = layout::card_height(layout.height, self.card);
        let kinds_kept = sheet.blocks.iter().enumerate().all(|(i, b)| match b {
            Block::Progress { share, .. } => self.scene.same_progress_kind(i, *share),
            _ => true,
        });
        if self.sheet.moves_only(&sheet) && kinds_kept {
            for (i, block) in sheet.blocks.iter().enumerate() {
                if let Block::Progress { share, left, right } = block {
                    log(self.scene.set_progress(comp, i, *share, left, right));
                }
            }
        } else {
            let scrolls = self.scene.scrolls();
            match Scene::build(comp, &sheet, &layout, card, self.scale, &scrolls) {
                Ok(scene) => {
                    if let Ok(children) = self.root.Children() {
                        let _ = children.Remove(&self.scene.card);
                        let _ = children.InsertAtTop(&scene.card);
                    }
                    self.scene = scene;
                }
                Err(e) => log(Err(e)),
            }
        }
        self.sheet = sheet;
        self.layout = layout;
        self.drag = None;
        self.pressed = None;
        self.resize(card);
        self.refresh_hover();
    }

    fn resize(&mut self, card: f32) {
        self.card = card;
        log(self.scene.set_card(card));
        log(self
            .scene
            .card
            .SetCenterPoint(v3(layout::WIDTH / 2.0, card / 2.0)));
        let (w, h) = size_px(card, self.scale);
        // The top stays: the window may have been dragged.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                w,
                h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Returns Windows' answer (`None` — the default handler's) and what the owner is asked.
    pub fn handle(
        &mut self,
        comp: &Comp,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (Option<LRESULT>, Option<Request>) {
        let done = |result: isize| (Some(LRESULT(result)), None);
        match msg {
            WM_NCHITTEST => {
                let p = self.at_screen(point(lparam));
                done(if self.spot(p).is_some() {
                    HTCLIENT
                } else {
                    HTCAPTION
                } as isize)
            }
            // A double click on the card would maximize the window.
            WM_NCLBUTTONDBLCLK => done(0),
            WM_SETCURSOR => {
                let pressable = matches!(
                    self.spot(self.cursor()),
                    Some(Spot::Close | Spot::Button(..))
                );
                if !pressable {
                    return (None, None);
                }
                unsafe {
                    if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
                        SetCursor(Some(hand));
                    }
                }
                done(1)
            }
            WM_MOUSEMOVE => {
                self.track_leave();
                let p = self.logical(point(lparam));
                if let Some((block, y0, from)) = self.drag {
                    if let Some(mut scroll) = self.scene.scroll(block)
                        && scroll.drag(from, p.1 - y0)
                    {
                        log(self.scene.set_scroll(comp, block, scroll, false));
                    }
                } else {
                    self.hover_at(Some(p));
                }
                done(0)
            }
            WM_NCMOUSEMOVE | WM_MOUSELEAVE => {
                if msg == WM_MOUSELEAVE {
                    self.tracking = false;
                }
                if self.drag.is_none() {
                    self.hover_at(None);
                }
                (None, None)
            }
            WM_LBUTTONDOWN => {
                let p = self.logical(point(lparam));
                match self.spot(p) {
                    Some(spot @ (Spot::Close | Spot::Button(..))) => {
                        self.pressed = Some(spot);
                        unsafe {
                            SetCapture(self.hwnd);
                        }
                    }
                    Some(Spot::Thumb(block)) => {
                        if let Some(scroll) = self.scene.scroll(block) {
                            self.drag = Some((block, p.1, scroll.offset));
                            self.hover_at(Some(p));
                            unsafe {
                                SetCapture(self.hwnd);
                            }
                        }
                    }
                    Some(Spot::Track(block)) => {
                        if let (Some(mut scroll), Some(frame)) =
                            (self.scene.scroll(block), self.notes_frame(block))
                            && scroll.page(p.1 - frame.y)
                        {
                            log(self.scene.set_scroll(comp, block, scroll, true));
                        }
                    }
                    Some(Spot::Notes(_)) | None => {}
                }
                done(0)
            }
            WM_LBUTTONUP => {
                let p = self.logical(point(lparam));
                let pressed = self.pressed.take();
                let request = match pressed {
                    Some(spot) if self.spot(p) == Some(spot) => self.answer(spot),
                    _ => None,
                };
                let dragged = self.drag.take().is_some();
                // Capture is released last: its message comes at once, into the state as it is now.
                unsafe {
                    let _ = ReleaseCapture();
                }
                if dragged {
                    self.hover_at(Some(p));
                }
                (Some(LRESULT(0)), request)
            }
            WM_CAPTURECHANGED => {
                self.pressed = None;
                if self.drag.take().is_some() {
                    self.refresh_hover();
                }
                done(0)
            }
            WM_MOUSEWHEEL => {
                let p = self.at_screen(point(lparam));
                if let Some(Spot::Notes(block) | Spot::Thumb(block) | Spot::Track(block)) =
                    self.spot(p)
                    && let Some(mut scroll) = self.scene.scroll(block)
                    && scroll.wheel(wheel_delta(wparam) as f32)
                {
                    log(self.scene.set_scroll(comp, block, scroll, true));
                }
                done(0)
            }
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                let request = self.sheet.dismiss.and_then(|_| self.answer(Spot::Close));
                (Some(LRESULT(0)), request)
            }
            WM_CLOSE => (Some(LRESULT(0)), Some(Request::Closed)),
            WM_DPICHANGED => {
                self.rescale(comp, (wparam.0 >> 16) as u16 as f32 / 96.0, lparam);
                done(0)
            }
            _ => (None, None),
        }
    }

    /// Once per sheet: the buttons are inert until the owner answers.
    fn answer(&mut self, spot: Spot) -> Option<Request> {
        if self.inert {
            return None;
        }
        let action = match spot {
            Spot::Close => self.sheet.dismiss?,
            Spot::Button(block, i) => match self.sheet.blocks.get(block)? {
                Block::Buttons(buttons) => buttons.get(i)?.action,
                _ => return None,
            },
            _ => return None,
        };
        self.inert = true;
        Some(Request::Action(action))
    }

    fn notes_frame(&self, block: usize) -> Option<layout::Rect> {
        match self.layout.blocks.get(block)? {
            Placed::Notes(n) => Some(n.frame),
            _ => None,
        }
    }

    fn spot(&self, p: (f32, f32)) -> Option<Spot> {
        if self.layout.close.is_some_and(|r| r.contains(p)) {
            return Some(Spot::Close);
        }
        for (block, placed) in self.layout.blocks.iter().enumerate() {
            match placed {
                Placed::Buttons(rects) => {
                    if let Some(i) = rects.iter().position(|r| r.contains(p)) {
                        return Some(Spot::Button(block, i));
                    }
                }
                Placed::Notes(n) if n.frame.contains(p) => {
                    let f = n.frame;
                    if !n.scrolls() || p.0 < f.x + f.w - SCROLLBAR {
                        return Some(Spot::Notes(block));
                    }
                    let (top, len) = self.scene.scroll(block)?.thumb();
                    let y = p.1 - f.y;
                    return Some(if y >= top && y < top + len {
                        Spot::Thumb(block)
                    } else {
                        Spot::Track(block)
                    });
                }
                _ => {}
            }
        }
        None
    }

    fn hover_at(&self, p: Option<(f32, f32)>) {
        let spot = p.and_then(|p| self.spot(p));
        let notes = match spot {
            Some(Spot::Notes(b) | Spot::Thumb(b) | Spot::Track(b)) => Some(b),
            _ => None,
        };
        let lit = spot.filter(|s| matches!(s, Spot::Close | Spot::Button(..)));
        let held = self.drag.map(|(block, _, _)| block);
        log(self.scene.hover(lit, notes.or(held), held));
    }

    fn refresh_hover(&self) {
        let p = self.cursor();
        let inside = p.0 >= 0.0 && p.1 >= 0.0 && p.0 < layout::WIDTH && p.1 < self.card;
        self.hover_at(inside.then_some(p));
    }

    fn rescale(&mut self, comp: &Comp, k: f32, lparam: LPARAM) {
        self.scale = k;
        log(self.root.SetScale(scale(k, k)));
        let scrolls = self.scene.scrolls();
        match Scene::build(comp, &self.sheet, &self.layout, self.card, k, &scrolls) {
            Ok(scene) => {
                if let Ok(children) = self.root.Children() {
                    let _ = children.Remove(&self.scene.card);
                    let _ = children.InsertAtTop(&scene.card);
                }
                log(scene
                    .card
                    .SetCenterPoint(v3(layout::WIDTH / 2.0, self.card / 2.0)));
                self.scene = scene;
            }
            Err(e) => log(Err(e)),
        }
        let (w, h) = size_px(self.card, k);
        unsafe {
            // Windows suggests where the window goes on the new monitor: its corner is taken.
            let rect = *(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                rect.left,
                rect.top,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
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
}

fn size_px(card: f32, k: f32) -> (i32, i32) {
    (
        (layout::WIDTH * k).round() as i32,
        (layout::window_height(card) * k).round() as i32,
    )
}

impl Drop for Dialog {
    fn drop(&mut self) {
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
