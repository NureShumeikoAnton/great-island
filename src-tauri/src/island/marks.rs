//! Corner marks around the area being recorded, in a click-through window over the monitor.
//! They come in from the screen's corners and leave back to them.
//! They sit just outside the area, so they never cover what is recorded; the recording
//! feature also hides the window from captures.

use super::comp::{Comp, Motion, color, v2, v3};
use super::overlay::Overlay;
use super::window;
use crate::{capture::Area, platform};
use windows::{
    UI::Composition::ContainerVisual,
    Win32::Foundation::{HWND, RECT},
    core::Result,
};
use windows_numerics::Vector3;

const ARM: f32 = 18.0;
const THICK: f32 = 3.0;
const GAP: f32 = 2.0;
const MARK: &str = "#FF375F";
/// A dark edge under the marks: they stay seen on a white page too.
const EDGE: f32 = 1.0;
const EDGE_ALPHA: f32 = 0.5;
const CLOSE_IN: Motion = Motion::curve(500, 0.2, 0.0, 0.0, 1.0);
const FLY_OFF: Motion = Motion::curve(LEAVE_MS, 0.5, 0.0, 0.9, 0.6);
pub const LEAVE_MS: u32 = 400;

pub struct Marks {
    window: Overlay,
    area: Area,
    /// Each corner and the screen corner it came from.
    corners: Vec<(ContainerVisual, Vector3)>,
}

impl Marks {
    /// Created hidden: `show` once the window is kept out of captures. The corners set off
    /// from the screen's corners at once and close in on the area.
    pub fn new(comp: &Comp, monitor: RECT, k: f32, area: Area) -> Result<Self> {
        let mut marks = Self {
            window: Overlay::open(comp, window::create_overlay, monitor, k)?,
            area,
            corners: Vec::with_capacity(4),
        };
        let screen = marks.window.screen;
        let (x, y, w, h) = area.logical(k);
        let (left, top) = (x - GAP - THICK, y - GAP - THICK);
        let (right, bottom) = (x + w + GAP + THICK, y + h + GAP + THICK);
        // Each corner's outer point, the screen corner it comes from, and where its arms go.
        let corners = [
            ((left, top), (0.0, 0.0), (1.0, 1.0)),
            ((right, top), (screen.0, 0.0), (-1.0, 1.0)),
            ((left, bottom), (0.0, screen.1), (1.0, -1.0)),
            ((right, bottom), screen, (-1.0, -1.0)),
        ];
        let children = marks.window.root.Children()?;
        let edge = comp.brush(color("#000000", EDGE_ALPHA))?;
        let fill = comp.brush(color(MARK, 1.0))?;
        for ((cx, cy), from, (sx, sy)) in corners {
            let corner = comp.container()?;
            corner.SetOffset(v3(from.0, from.1))?;
            let arms = [(ARM, THICK), (THICK, ARM)];
            let parts = corner.Children()?;
            for (brush, grow) in [(&edge, EDGE), (&fill, 0.0)] {
                for (aw, ah) in arms {
                    let arm = comp.compositor.CreateSpriteVisual()?;
                    arm.SetBrush(brush)?;
                    let ax = if sx < 0.0 { -aw } else { 0.0 };
                    let ay = if sy < 0.0 { -ah } else { 0.0 };
                    arm.SetOffset(v3(ax - grow, ay - grow))?;
                    arm.SetSize(v2(aw + 2.0 * grow, ah + 2.0 * grow))?;
                    parts.InsertAtTop(&arm)?;
                }
            }
            children.InsertAtTop(&corner)?;
            comp.animate_vector3(&corner, "Offset", v3(cx, cy), CLOSE_IN)?;
            marks.corners.push((corner, v3(from.0, from.1)));
        }
        Ok(marks)
    }

    /// The corners fly back to the screen's corners and fade on the way; the window is closed
    /// by dropping it after [`LEAVE_MS`].
    pub fn leave(&self, comp: &Comp) -> Result<()> {
        for (corner, from) in &self.corners {
            comp.animate_vector3(corner, "Offset", *from, FLY_OFF)?;
        }
        comp.keyframes(
            &self.window.root,
            "Opacity",
            &[(0.0, 1.0), (0.5, 1.0), (1.0, 0.0)],
            LEAVE_MS,
        )
    }

    pub fn area(&self) -> Area {
        self.area
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    pub fn show(&self) {
        platform::show_without_focus(self.window.hwnd());
    }
}
