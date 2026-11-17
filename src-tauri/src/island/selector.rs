//! The area selection window over the whole monitor: the dimming with a clear hole, the lit
//! window, the drawn frame with its size, and the Alt hint at the top while dragging. Over a
//! frozen picture for a screenshot, over the live screen for a recording. Opened from the
//! island, it starts with a wave of light running out of the island over the screen.
//!
//! It knows nothing about the island: it turns window messages into [`Input`] and shows a
//! [`Look`]. The hole is two corners in a property set; the dimming and both outlines follow
//! them in the compositor, so a move sets two values and a glide onto a window is one animation.

use super::comp::{Comp, Motion, Text, TextStyle, color, snap, v2, v3};
use super::overlay::Overlay;
use super::window;
use crate::{capture::Image, lang::tr, select::Look};
use windows::{
    UI::Composition::{
        CompositionColorBrush, CompositionMappingMode, CompositionObject, CompositionPropertySet,
        CompositionRoundedRectangleGeometry, ContainerVisual, ShapeVisual, SpriteVisual,
    },
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, ReleaseCapture, SetCapture, VK_ESCAPE, VK_MENU,
            },
            WindowsAndMessaging::{
                GetCursorPos, SW_SHOW, SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE,
                WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
                WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    },
    core::{HSTRING, Interface, Result},
};
use windows_numerics::Vector2;

/// Points in the monitor's physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Move { x: i32, y: i32, free: bool },
    Press(i32, i32),
    Release(i32, i32),
    Free(bool),
    Cancel,
}

const DIM: f32 = 0.45;
const LIT: &str = "#4ADE80";
const LIT_STROKE: f32 = 3.0;
const LIT_FILL: f32 = 0.12;
const FRAME_STROKE: f32 = 1.0;
const PILL: &str = "#1C1C1E";
const PILL_FREE: &str = "#48484A";
const PILL_ALPHA: f32 = 0.92;
const PILL_H: f32 = 24.0;
const PILL_PAD: f32 = 10.0;
const LABEL_GAP: f32 = 8.0;
const HINT_TOP: f32 = 16.0;
const TEXT: &str = "#F5F5F7";

const APPEAR: Motion = Motion::curve(180, 0.25, 0.1, 0.25, 1.0);
const SWITCH: Motion = Motion::curve(140, 0.25, 0.1, 0.25, 1.0);
const GLIDE: Motion = Motion::curve(160, 0.2, 0.0, 0.0, 1.0);

/// The wave runs out to the farthest corner, then fades.
const WAVE_RUN: Motion = Motion::curve(450, 0.25, 0.0, 0.2, 1.0);
const WAVE_FADE_MS: u32 = 300;
/// White across the wave, from the island outwards: faint inside, a brighter rim, then nothing.
const WAVE_STOPS: [(f32, f32); 4] = [(0.0, 0.05), (0.8, 0.12), (0.94, 0.24), (1.0, 0.0)];

/// SC_KEYMENU: a lone Alt asks the window for its menu. There is none, and the default
/// handling would hold the keyboard until the next key.
const SC_KEYMENU: usize = 0xF100;

struct Pill {
    body: ContainerVisual,
    shape: ShapeVisual,
    fill: CompositionRoundedRectangleGeometry,
    brush: CompositionColorBrush,
    text: Text,
}

pub struct Selector {
    window: Overlay,
    /// The monitor's top left corner on the desktop, physical pixels.
    origin: (i32, i32),
    k: f32,
    /// `Min` and `Max`: the hole's corners in logical pixels; both zero — no hole.
    hole: CompositionPropertySet,
    lit: ShapeVisual,
    frame: ShapeVisual,
    /// White for a drawn rectangle, green once it has gone onto a window.
    frame_color: CompositionColorBrush,
    label: Pill,
    hint: Pill,
    shown: Look,
}

impl Selector {
    /// `monitor` in physical pixels, `k` its scale. `frozen` — the screen as it was at the
    /// press: it covers the live one, so the selection is made on that moment. `wave` — where
    /// the island is, in the monitor's logical pixels.
    pub fn open(
        comp: &Comp,
        monitor: RECT,
        k: f32,
        frozen: Option<&Image>,
        wave: Option<(f32, f32)>,
    ) -> Result<Self> {
        let window = Overlay::open(comp, window::create_selector, monitor, k)?;
        let screen = window.screen;
        let children = window.root.Children()?;
        if let Some(image) = frozen {
            let picture = comp.image_rounded(
                &image.pixels,
                (image.width, image.height),
                (image.width as f32, image.height as f32),
                0.0,
                1.0,
            )?;
            picture.SetSize(v2(screen.0, screen.1))?;
            children.InsertAtTop(&picture)?;
        }
        // Everything over the picture fades in; the picture itself is there at once, or the
        // live screen would show through between the press and the freeze.
        let layer = comp.container()?;
        layer.SetSize(v2(screen.0, screen.1))?;
        children.InsertAtTop(&layer)?;
        let over = layer.Children()?;
        let hole = comp.compositor.CreatePropertySet()?;
        for corner in ["Min", "Max"] {
            hole.InsertVector2(&HSTRING::from(corner), v2(0.0, 0.0))?;
        }
        // Names in an expression ignore case: a reference called `h` would be the height `H`.
        let refs = [("hole", &hole.cast::<CompositionObject>()?)];
        let sizes = [("W", screen.0), ("H", screen.1)];
        let shade = comp.brush(color("#000000", DIM))?;
        // Above, below, left and right of the hole.
        let bands = [
            ("Vector3(0, 0, 0)", "Vector2(W, hole.Min.Y)"),
            ("Vector3(0, hole.Max.Y, 0)", "Vector2(W, H - hole.Max.Y)"),
            (
                "Vector3(0, hole.Min.Y, 0)",
                "Vector2(hole.Min.X, hole.Max.Y - hole.Min.Y)",
            ),
            (
                "Vector3(hole.Max.X, hole.Min.Y, 0)",
                "Vector2(W - hole.Max.X, hole.Max.Y - hole.Min.Y)",
            ),
        ];
        for (offset, size) in bands {
            let band = comp.compositor.CreateSpriteVisual()?;
            band.SetBrush(&shade)?;
            comp.bind(&band, "Offset", offset, &refs, &sizes)?;
            comp.bind(&band, "Size", size, &refs, &sizes)?;
            over.InsertAtTop(&band)?;
        }
        let (lit, _) = outline(comp, screen, &hole, LIT, LIT_STROKE, Some(LIT_FILL))?;
        let (frame, frame_color) = outline(comp, screen, &hole, TEXT, FRAME_STROKE, None)?;
        let label = pill(comp, k, "")?;
        let hint = pill(comp, k, &tr!("select.free_hint"))?;
        for visual in [&lit, &frame] {
            visual.SetOpacity(0.0)?;
            over.InsertAtTop(visual)?;
        }
        for body in [&label.body, &hint.body] {
            body.SetOpacity(0.0)?;
            over.InsertAtTop(body)?;
        }
        let hint_w = hint.body.Size()?.X;
        hint.body
            .SetOffset(v3(snap((screen.0 - hint_w) / 2.0, k), HINT_TOP))?;
        comp.animate_scalar_from(&layer, "Opacity", 0.0, 1.0, APPEAR)?;
        if let Some(from) = wave {
            children.InsertAtTop(&run_wave(comp, screen, from)?)?;
        }

        let selector = Self {
            window,
            origin: (monitor.left, monitor.top),
            k,
            hole,
            lit,
            frame,
            frame_color,
            label,
            hint,
            shown: Look::default(),
        };
        unsafe {
            let _ = ShowWindow(selector.window.hwnd(), SW_SHOW);
            let _ = SetForegroundWindow(selector.window.hwnd());
        }
        Ok(selector)
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    /// Where the pointer is, in the monitor's pixels: the window under it lights up before
    /// the first move.
    pub fn cursor(&self) -> Option<(i32, i32)> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.ok()?;
        Some((point.x - self.origin.0, point.y - self.origin.1))
    }

    /// What a window message means for the selection; `Some(result)` — it is answered here.
    pub fn handle(
        &self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (Option<LRESULT>, Option<Input>) {
        let (x, y) = (
            (lparam.0 & 0xFFFF) as u16 as i16 as i32,
            ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        );
        let done = Some(LRESULT(0));
        let key = wparam.0 as u16;
        match msg {
            WM_MOUSEMOVE => (
                done,
                Some(Input::Move {
                    x,
                    y,
                    free: alt_down(),
                }),
            ),
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.window.hwnd());
                }
                (done, Some(Input::Press(x, y)))
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                (done, Some(Input::Release(x, y)))
            }
            WM_RBUTTONDOWN => (done, Some(Input::Cancel)),
            WM_KEYDOWN if key == VK_ESCAPE.0 => (done, Some(Input::Cancel)),
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP if key == VK_MENU.0 => {
                (done, Some(Input::Free(alt_down())))
            }
            WM_SYSCOMMAND if wparam.0 & 0xFFF0 == SC_KEYMENU => (done, None),
            // Alt+Tab, the Start menu, a window brought to the front: the selection is left.
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => (None, Some(Input::Cancel)),
            _ => (None, None),
        }
    }

    pub fn show(&mut self, comp: &Comp, look: &Look) -> Result<()> {
        // Most moves of the pointer change nothing: it is still over the same window.
        if *look == self.shown {
            return Ok(());
        }
        let k = self.k;
        let (w, h) = self.window.screen;
        let was = self.shown;
        // Onto a window the hole glides; a drawn rectangle follows the hand at once.
        let glide = look.window && was.hole.is_some();
        if look.hole != was.hole {
            let (min, max) = match look.hole.map(|a| a.logical(k)) {
                Some((x, y, hw, hh)) => (v2(x, y), v2(x + hw, y + hh)),
                None => (v2(0.0, 0.0), v2(0.0, 0.0)),
            };
            for (corner, to) in [("Min", min), ("Max", max)] {
                if glide {
                    comp.animate_vector2(&self.hole, corner, to, GLIDE)?;
                } else {
                    set_corner(comp, &self.hole, corner, to)?;
                }
            }
        }

        let lit = |l: &Look| l.hole.is_some() && !l.dragging;
        if lit(look) != lit(&was) {
            if look.dragging {
                comp.stop(&self.lit, "Opacity")?;
                self.lit.SetOpacity(0.0)?;
            } else {
                let to = if lit(look) { 1.0 } else { 0.0 };
                comp.animate_scalar(&self.lit, "Opacity", to, SWITCH)?;
            }
        }
        let framed = look.hole.is_some() && look.dragging;
        self.frame.SetOpacity(if framed { 1.0 } else { 0.0 })?;
        // The frame turns green as it goes onto a window: it went there on purpose.
        let caught = |l: &Look| l.dragging && l.window;
        if caught(look) != caught(&was) {
            let hex = if caught(look) { LIT } else { TEXT };
            comp.animate_color(&self.frame_color, color(hex, 1.0), SWITCH)?;
        }

        match look.hole {
            Some(area) => {
                let text = tr!("select.size", width = area.width, height = area.height);
                set_pill(comp, &mut self.label, k, &text)?;
                let (x, y, _, ah) = area.logical(k);
                let size = self.label.body.Size()?;
                let below = y + ah + LABEL_GAP;
                let above = y - LABEL_GAP - size.Y;
                let top = if below + size.Y <= h {
                    below
                } else if above >= 0.0 {
                    above
                } else {
                    y + LABEL_GAP
                };
                let left = x.clamp(0.0, (w - size.X).max(0.0));
                let at = v3(snap(left, k), snap(top, k));
                if glide {
                    comp.animate_vector3(&self.label.body, "Offset", at, GLIDE)?;
                } else {
                    comp.stop(&self.label.body, "Offset")?;
                    self.label.body.SetOffset(at)?;
                }
                self.label.body.SetOpacity(1.0)?;
            }
            None => self.label.body.SetOpacity(0.0)?,
        }

        if look.dragging != was.dragging {
            let to = if look.dragging { 1.0 } else { 0.0 };
            comp.animate_scalar(&self.hint.body, "Opacity", to, SWITCH)?;
        }
        if look.free != was.free {
            let fill = if look.free { PILL_FREE } else { PILL };
            comp.animate_color(&self.hint.brush, color(fill, PILL_ALPHA), SWITCH)?;
        }
        self.shown = *look;
        Ok(())
    }
}

/// Read from the keyboard itself, not from the window's messages: a key-up the window misses
/// would leave the frame free until the next move.
pub fn alt_down() -> bool {
    unsafe { GetAsyncKeyState(i32::from(VK_MENU.0)) < 0 }
}

/// A glide still running would win over the value set: it is stopped first.
fn set_corner(comp: &Comp, hole: &CompositionPropertySet, corner: &str, to: Vector2) -> Result<()> {
    comp.stop(hole, corner)?;
    hole.InsertVector2(&HSTRING::from(corner), to)
}

/// An outline bound to the hole, and its stroke's brush. The stroke lies inside the hole, never
/// over what is outside the area.
fn outline(
    comp: &Comp,
    screen: (f32, f32),
    hole: &CompositionPropertySet,
    hex: &str,
    stroke: f32,
    fill: Option<f32>,
) -> Result<(ShapeVisual, CompositionColorBrush)> {
    let c = &comp.compositor;
    let visual = c.CreateShapeVisual()?;
    visual.SetSize(v2(screen.0, screen.1))?;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    let refs = [("hole", &hole.cast::<CompositionObject>()?)];
    let widths = [("s", stroke), ("q", stroke / 2.0)];
    comp.bind(
        &geometry,
        "Offset",
        "hole.Min + Vector2(q, q)",
        &refs,
        &widths,
    )?;
    comp.bind(
        &geometry,
        "Size",
        "Vector2(Max(hole.Max.X - hole.Min.X - s, 0), Max(hole.Max.Y - hole.Min.Y - s, 0))",
        &refs,
        &widths,
    )?;
    let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
    let stroke_color = comp.brush(color(hex, 1.0))?;
    shape.SetStrokeBrush(&stroke_color)?;
    shape.SetStrokeThickness(stroke)?;
    if let Some(alpha) = fill {
        shape.SetFillBrush(&comp.brush(color(hex, alpha))?)?;
    }
    visual.Shapes()?.Append(&shape)?;
    Ok((visual, stroke_color))
}

/// One gradient over the screen whose radius grows: the compositor draws it, for less than a
/// second, and it stays at zero opacity afterwards.
fn run_wave(comp: &Comp, screen: (f32, f32), (x, y): (f32, f32)) -> Result<SpriteVisual> {
    let c = &comp.compositor;
    let brush = c.CreateRadialGradientBrush()?;
    brush.SetMappingMode(CompositionMappingMode::Absolute)?;
    brush.SetEllipseCenter(v2(x, y))?;
    brush.SetEllipseRadius(v2(0.0, 0.0))?;
    let stops = brush.ColorStops()?;
    for (at, alpha) in WAVE_STOPS {
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(at, color("#FFFFFF", alpha))?)?;
    }
    let visual = c.CreateSpriteVisual()?;
    visual.SetSize(v2(screen.0, screen.1))?;
    visual.SetBrush(&brush)?;
    // Past the farthest corner, so the rim leaves the screen before the fade.
    let reach = [(0.0, 0.0), (screen.0, 0.0), (0.0, screen.1), screen]
        .iter()
        .map(|(cx, cy)| (cx - x).hypot(cy - y))
        .fold(0.0, f32::max)
        / WAVE_STOPS[2].0;
    comp.animate_vector2(&brush, "EllipseRadius", v2(reach, reach), WAVE_RUN)?;
    let total = WAVE_RUN.ms + WAVE_FADE_MS;
    let ran = WAVE_RUN.ms as f32 / total as f32;
    comp.keyframes(
        &visual,
        "Opacity",
        &[(0.0, 1.0), (ran, 1.0), (1.0, 0.0)],
        total,
    )?;
    Ok(visual)
}

fn pill(comp: &Comp, k: f32, content: &str) -> Result<Pill> {
    let c = &comp.compositor;
    let body = comp.container()?;
    let shape = c.CreateShapeVisual()?;
    let fill = c.CreateRoundedRectangleGeometry()?;
    fill.SetCornerRadius(v2(PILL_H / 2.0, PILL_H / 2.0))?;
    let sprite = c.CreateSpriteShapeWithGeometry(&fill)?;
    let brush = comp.brush(color(PILL, PILL_ALPHA))?;
    sprite.SetFillBrush(&brush)?;
    shape.Shapes()?.Append(&sprite)?;
    body.Children()?.InsertAtTop(&shape)?;
    let style = TextStyle {
        size: 12.0,
        bold: true,
        color: color(TEXT, 1.0),
        tabular: true,
    };
    let text = comp.text(content, style, k)?;
    body.Children()?.InsertAtTop(&text.visual)?;
    let pill = Pill {
        body,
        shape,
        fill,
        brush,
        text,
    };
    fit_pill(&pill, k)?;
    Ok(pill)
}

fn set_pill(comp: &Comp, pill: &mut Pill, k: f32, content: &str) -> Result<()> {
    comp.set_text(&mut pill.text, content, k)?;
    fit_pill(pill, k)
}

fn fit_pill(pill: &Pill, k: f32) -> Result<()> {
    let size = v2((pill.text.width + 2.0 * PILL_PAD).round(), PILL_H);
    pill.body.SetSize(size)?;
    pill.shape.SetSize(size)?;
    pill.fill.SetSize(size)?;
    pill.text.visual.SetOffset(v3(
        snap(PILL_PAD, k),
        snap((PILL_H - pill.text.height) / 2.0, k),
    ))
}
