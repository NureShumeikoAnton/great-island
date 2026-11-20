//! The small window the viewer becomes when the selection in Explorer moves onto something it
//! doesn't read (a GIF, a document, a folder): the file's sheet with its extension, as the
//! island draws a file without a picture, its name and two lines under it (the size, the
//! creation date). It is dragged by any point; Esc closes it.

use super::super::comp::{Comp, TextStyle, color, scale, v2, v3};
use super::super::view::sheet;
use super::super::window;
use super::{MUTED, STRIP, Spot, TEXT, show_quietly};
use windows::{
    UI::Composition::{ContainerVisual, Desktop::DesktopWindowTarget},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::VK_ESCAPE,
            WindowsAndMessaging::{
                DestroyWindow, HTCAPTION, SW_HIDE, ShowWindow, WM_CLOSE, WM_KEYDOWN, WM_NCHITTEST,
                WM_NCLBUTTONDBLCLK,
            },
        },
    },
    core::Result,
};

const SIZE: (f32, f32) = (400.0, 112.0);
const SHEET: f32 = 72.0;
const PAD: f32 = 20.0;
const GAP: f32 = 16.0;

pub struct FileCard {
    hwnd: HWND,
    target: Option<DesktopWindowTarget>,
    _root: ContainerVisual,
}

impl FileCard {
    /// A hidden window at `spot`. `lines` go under the name, the first one brighter.
    pub fn new(comp: &Comp, spot: Spot, k: f32, name: &str, lines: [&str; 2]) -> Result<Self> {
        let px = ((SIZE.0 * k).round() as i32, (SIZE.1 * k).round() as i32);
        let hwnd = window::create_viewer(spot.rect(px), name)?;
        Self::build(comp, hwnd, k, name, lines).inspect_err(|_| unsafe {
            let _ = DestroyWindow(hwnd);
        })
    }

    fn build(comp: &Comp, hwnd: HWND, k: f32, name: &str, lines: [&str; 2]) -> Result<Self> {
        let (target, root) = comp.window_root(hwnd)?;
        root.SetScale(scale(k, k))?;
        let c = &comp.compositor;
        let backdrop = c.CreateSpriteVisual()?;
        backdrop.SetBrush(&comp.brush(color(STRIP, 1.0))?)?;
        backdrop.SetSize(v2(SIZE.0, SIZE.1))?;
        root.Children()?.InsertAtTop(&backdrop)?;

        let icon = sheet(comp, k, name, SHEET)?;
        icon.SetOffset(v3(PAD - 6.0, (SIZE.1 - SHEET) / 2.0))?;
        root.Children()?.InsertAtTop(&icon)?;

        let left = PAD - 6.0 + SHEET + GAP;
        let room = SIZE.0 - left - PAD;
        let styles = [
            text_style(15.0, true, TEXT),
            text_style(13.0, false, TEXT),
            text_style(13.0, false, MUTED),
        ];
        let contents = [name, lines[0], lines[1]];
        let mut texts = Vec::new();
        for (content, style) in contents.iter().zip(styles) {
            if content.is_empty() {
                continue;
            }
            let fitted = comp.fit(content, style, room);
            texts.push(comp.text(&fitted, style, k)?);
        }
        let gap = 4.0;
        let total: f32 =
            texts.iter().map(|t| t.height).sum::<f32>() + gap * (texts.len().max(1) - 1) as f32;
        let mut y = (SIZE.1 - total) / 2.0;
        let snap = |v: f32| (v * k).round() / k;
        for text in &texts {
            text.visual.SetOffset(v3(snap(left), snap(y)))?;
            root.Children()?.InsertAtTop(&text.visual)?;
            y += text.height + gap;
        }
        Ok(Self {
            hwnd,
            target: Some(target),
            _root: root,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn show_quietly(&self, animated: bool) {
        show_quietly(self.hwnd, animated);
    }

    /// Windows' answer (`None` = the default handler) and whether the card asks to close.
    pub fn handle(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> (Option<LRESULT>, bool) {
        match msg {
            WM_NCHITTEST => (Some(LRESULT(HTCAPTION as isize)), false),
            WM_NCLBUTTONDBLCLK => (Some(LRESULT(0)), false),
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => (Some(LRESULT(0)), true),
            WM_CLOSE => (Some(LRESULT(0)), true),
            _ => (None, false),
        }
    }
}

fn text_style(size: f32, bold: bool, hex: &str) -> TextStyle {
    TextStyle {
        size,
        bold,
        color: color(hex, 1.0),
        tabular: false,
    }
}

impl Drop for FileCard {
    fn drop(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.target = None;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
