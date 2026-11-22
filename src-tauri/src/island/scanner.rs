//! The text scanner's window over the whole monitor: the frozen screen, the glint that runs over
//! the found text once, the selection, the links' underlines, the QR codes' frames and the Alt
//! rectangle. The cursor is an I-beam over text and a hand over what opens or copies.
//!
//! It knows nothing about the island: it turns window messages into [`Input`] and draws the
//! rectangles it is given. Rectangles come in the frozen picture's pixels, which are the
//! monitor's physical pixels; the window draws in logical ones.

use super::comp::{Comp, color, v2, v3};
use super::overlay::Overlay;
use super::window;
use crate::{
    capture::Image,
    text::{
        Rect,
        session::{Cursor, Keys},
    },
};
use windows::{
    UI::Composition::{CompositionColorBrush, ContainerVisual, ShapeVisual, SpriteVisual},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, ReleaseCapture, SetCapture, VK_A, VK_C, VK_CONTROL, VK_ESCAPE,
                VK_INSERT, VK_MENU, VK_SHIFT,
            },
            WindowsAndMessaging::{
                IDC_ARROW, IDC_HAND, IDC_IBEAM, LoadCursorW, SW_SHOW, SetCursor,
                SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_KEYDOWN, WM_KEYUP,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_SETCURSOR,
                WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    },
    core::Result,
};

/// Points in the monitor's physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Move(f32, f32),
    Press(f32, f32, Keys),
    Release(f32, f32),
    /// Ctrl or Alt went down or up: a link under the cursor may now open.
    Keys,
    /// The cursor is asked for: the window answers with the one `cursor` gives.
    Cursor,
    Copy,
    SelectAll,
    Cancel,
}

const SELECTION: &str = "#0A84FF";
const SELECTION_ALPHA: f32 = 0.34;
/// The highlight reaches a little above and below the reader's tight box.
const SELECTION_PAD: f32 = 2.0;
const UNDERLINE: &str = "#3B9CFF";
const UNDERLINE_H: f32 = 1.5;
const CODE_PAD: f32 = 6.0;
const CODE_RADIUS: f32 = 10.0;
const AREA_STROKE: f32 = 1.0;
const AREA_FILL: f32 = 0.10;

/// The glint: each line flashes in turn, from the top left to the bottom right.
const GLINT_PEAK: f32 = 0.38;
const GLINT_LINE_MS: f32 = 260.0;
/// The whole sweep, whatever the screen holds.
const GLINT_SWEEP_MS: f32 = 520.0;

/// SC_KEYMENU: a lone Alt asks the window for its menu. There is none, and the default
/// handling would hold the keyboard until the next key.
const SC_KEYMENU: usize = 0xF100;

pub struct Scanner {
    window: Overlay,
    k: f32,
    glint: ContainerVisual,
    marks: ContainerVisual,
    highlight: ContainerVisual,
    /// Reused between drags: one per selected line.
    spans: Vec<SpriteVisual>,
    selection: CompositionColorBrush,
    area: ShapeVisual,
    area_box: windows::UI::Composition::CompositionRoundedRectangleGeometry,
}

fn down(key: u16) -> bool {
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

pub fn keys() -> Keys {
    Keys {
        shift: down(VK_SHIFT.0),
        ctrl: down(VK_CONTROL.0),
        alt: down(VK_MENU.0),
    }
}

impl Scanner {
    /// `monitor` in physical pixels, `k` its scale; `frozen` covers the live screen at once.
    pub fn open(comp: &Comp, monitor: RECT, k: f32, frozen: &Image) -> Result<Self> {
        let window = Overlay::open(comp, window::create_scanner, monitor, k)?;
        window.freeze(comp, frozen)?;
        let screen = window.screen;
        let children = window.root.Children()?;
        let layer =
            |children: &windows::UI::Composition::VisualCollection| -> Result<ContainerVisual> {
                let layer = comp.container()?;
                layer.SetSize(v2(screen.0, screen.1))?;
                children.InsertAtTop(&layer)?;
                Ok(layer)
            };
        let glint = layer(&children)?;
        let marks = layer(&children)?;
        let highlight = layer(&children)?;

        let c = &comp.compositor;
        let area = c.CreateShapeVisual()?;
        area.SetSize(v2(screen.0, screen.1))?;
        let area_box = c.CreateRoundedRectangleGeometry()?;
        let shape = c.CreateSpriteShapeWithGeometry(&area_box)?;
        shape.SetStrokeBrush(&comp.brush(color(SELECTION, 1.0))?)?;
        shape.SetStrokeThickness(AREA_STROKE)?;
        shape.SetFillBrush(&comp.brush(color(SELECTION, AREA_FILL))?)?;
        area.Shapes()?.Append(&shape)?;
        area.SetOpacity(0.0)?;
        children.InsertAtTop(&area)?;

        let scanner = Self {
            window,
            k,
            glint,
            marks,
            highlight,
            spans: Vec::new(),
            selection: comp.brush(color(SELECTION, SELECTION_ALPHA))?,
            area,
            area_box,
        };
        unsafe {
            let _ = ShowWindow(scanner.window.hwnd(), SW_SHOW);
            let _ = SetForegroundWindow(scanner.window.hwnd());
        }
        Ok(scanner)
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    fn logical(&self, r: &Rect) -> (f32, f32, f32, f32) {
        (
            r.x / self.k,
            r.y / self.k,
            r.width / self.k,
            r.height / self.k,
        )
    }

    /// What a window message means; `Some(result)` — it is answered here.
    pub fn handle(
        &self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (Option<LRESULT>, Option<Input>) {
        let (x, y) = (
            (lparam.0 & 0xFFFF) as u16 as i16 as f32,
            ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32,
        );
        let done = Some(LRESULT(0));
        let key = wparam.0 as u16;
        match msg {
            WM_MOUSEMOVE => (done, Some(Input::Move(x, y))),
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.window.hwnd());
                }
                (done, Some(Input::Press(x, y, keys())))
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                (done, Some(Input::Release(x, y)))
            }
            WM_RBUTTONDOWN => (done, Some(Input::Cancel)),
            WM_SETCURSOR => (Some(LRESULT(1)), Some(Input::Cursor)),
            WM_KEYDOWN if key == VK_ESCAPE.0 => (done, Some(Input::Cancel)),
            WM_KEYDOWN if (key == VK_C.0 || key == VK_INSERT.0) && down(VK_CONTROL.0) => {
                (done, Some(Input::Copy))
            }
            WM_KEYDOWN if key == VK_A.0 && down(VK_CONTROL.0) => (done, Some(Input::SelectAll)),
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
                if key == VK_CONTROL.0 || key == VK_MENU.0 =>
            {
                (done, Some(Input::Keys))
            }
            WM_SYSCOMMAND if wparam.0 & 0xFFF0 == SC_KEYMENU => (done, None),
            // Alt+Tab, the Start menu, a window brought to the front: the scanner is left.
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => (None, Some(Input::Cancel)),
            _ => (None, None),
        }
    }

    pub fn set_cursor(cursor: Cursor) {
        let id = match cursor {
            Cursor::Arrow => IDC_ARROW,
            Cursor::Text => IDC_IBEAM,
            Cursor::Hand => IDC_HAND,
        };
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, id) {
                SetCursor(Some(cursor));
            }
        }
    }

    /// The text is found: the links get their underline, the codes their frame, and `glint`
    /// (one box per line or code) flashes once across the screen.
    pub fn show_found(
        &self,
        comp: &Comp,
        underlines: &[Rect],
        codes: &[Rect],
        glint: Option<&[Rect]>,
    ) -> Result<()> {
        let c = &comp.compositor;
        let marks = self.marks.Children()?;
        marks.RemoveAll()?;
        let line = comp.brush(color(UNDERLINE, 0.9))?;
        for r in underlines {
            let (x, y, w, h) = self.logical(r);
            let sprite = c.CreateSpriteVisual()?;
            sprite.SetBrush(&line)?;
            sprite.SetSize(v2(w, UNDERLINE_H))?;
            sprite.SetOffset(v3(x, y + h + 1.0))?;
            marks.InsertAtTop(&sprite)?;
        }
        for r in codes {
            let (x, y, w, h) = self.logical(r);
            let visual = c.CreateShapeVisual()?;
            visual.SetSize(v2(w + 2.0 * CODE_PAD + 4.0, h + 2.0 * CODE_PAD + 4.0))?;
            visual.SetOffset(v3(x - CODE_PAD - 2.0, y - CODE_PAD - 2.0))?;
            for (stroke, hex, alpha, inset) in
                [(3.0, "#000000", 0.25, 0.5), (2.0, "#FFFFFF", 1.0, 2.0)]
            {
                let geometry = c.CreateRoundedRectangleGeometry()?;
                geometry.SetOffset(v2(inset, inset))?;
                geometry.SetSize(v2(
                    w + 2.0 * CODE_PAD + 4.0 - 2.0 * inset,
                    h + 2.0 * CODE_PAD + 4.0 - 2.0 * inset,
                ))?;
                geometry.SetCornerRadius(v2(CODE_RADIUS, CODE_RADIUS))?;
                let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
                shape.SetStrokeBrush(&comp.brush(color(hex, alpha))?)?;
                shape.SetStrokeThickness(stroke)?;
                visual.Shapes()?.Append(&shape)?;
            }
            marks.InsertAtTop(&visual)?;
        }

        let flashes = self.glint.Children()?;
        flashes.RemoveAll()?;
        let Some(boxes) = glint else {
            return Ok(());
        };
        let (sw, sh) = self.window.screen;
        let white = comp.brush(color("#FFFFFF", GLINT_PEAK))?;
        let total = GLINT_SWEEP_MS + GLINT_LINE_MS;
        for r in boxes {
            let (x, y, w, h) = self.logical(r);
            let sprite = c.CreateSpriteVisual()?;
            sprite.SetBrush(&white)?;
            sprite.SetSize(v2(w + 4.0, h + 2.0 * SELECTION_PAD))?;
            sprite.SetOffset(v3(x - 2.0, y - SELECTION_PAD))?;
            sprite.SetOpacity(0.0)?;
            flashes.InsertAtTop(&sprite)?;
            // Diagonal: a line's turn comes by where it starts on the screen.
            let at = ((x / sw.max(1.0)) * 0.6 + (y / sh.max(1.0)) * 0.4) * GLINT_SWEEP_MS;
            let (start, peak, end) = (
                at / total,
                (at + GLINT_LINE_MS * 0.35) / total,
                (at + GLINT_LINE_MS) / total,
            );
            comp.keyframes(
                &sprite,
                "Opacity",
                &[
                    (0.0, 0.0),
                    (start, 0.0),
                    (peak, 1.0),
                    (end, 0.0),
                    (1.0, 0.0),
                ],
                total as u32,
            )?;
        }
        Ok(())
    }

    /// The selection, one box per line, and the Alt rectangle while it is drawn.
    pub fn show_selection(
        &mut self,
        comp: &Comp,
        spans: &[Rect],
        area: Option<Rect>,
    ) -> Result<()> {
        let children = self.highlight.Children()?;
        while self.spans.len() < spans.len() {
            let sprite = comp.compositor.CreateSpriteVisual()?;
            sprite.SetBrush(&self.selection)?;
            children.InsertAtTop(&sprite)?;
            self.spans.push(sprite);
        }
        for (i, sprite) in self.spans.iter().enumerate() {
            match spans.get(i) {
                Some(r) => {
                    let (x, y, w, h) = self.logical(r);
                    sprite.SetOffset(v3(x, y - SELECTION_PAD))?;
                    sprite.SetSize(v2(w, h + 2.0 * SELECTION_PAD))?;
                    sprite.SetOpacity(1.0)?;
                }
                None => sprite.SetOpacity(0.0)?,
            }
        }
        match area {
            Some(r) => {
                let (x, y, w, h) = self.logical(&r);
                self.area_box.SetOffset(v2(x, y))?;
                self.area_box.SetSize(v2(w, h))?;
                self.area.SetOpacity(1.0)
            }
            None => self.area.SetOpacity(0.0),
        }
    }
}
