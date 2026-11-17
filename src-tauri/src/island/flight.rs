//! A shot's flight in a separate transparent window over the monitor. The window doesn't catch clicks and
//! lives only while the flight runs; the compositor computes the whole animation.
//!
//! - **shot** ([`Flight::shot`]): appears full-monitor with a flash, shrinks to the center, hangs there
//!   to be seen and flies up into the middle of the island. By then the island meets it with a capsule
//!   sized for the thumbnail; the shot fades out over it, and only then does the island widen into a card.
//!   While the shot is at the center it can be clicked: an invisible catcher window lies over it (it catches
//!   only clicks on the shot; the rest of the screen is clickable as usual). Then the flight to the island
//!   doesn't happen: the shot expands in place into the viewer window ([`Flight::expand`]);
//! - **area** ([`Flight::area`]): the selected area with a flash flies straight to the island, with a
//!   thin white edge that sets it apart from the screen around it. The user has just looked at it,
//!   so there is no stop at the center;
//! - **glide** ([`Flight::glide`]): from one place to another, fading out over the target: from the card
//!   thumbnail into the viewer window, and from the window back into the island (Ctrl+X).
//!
//! Cheap for the compositor: at most three full-screen layers (dimming, shot, flash) with simple
//! animations; no shapes that would be redrawn every frame.

use super::comp::{Comp, Motion, color, v2, v3};
use super::overlay::Overlay;
use super::window;
use crate::lang::tr;
use crate::{capture::Image, platform};
use windows::{
    UI::Composition::{
        CompositionObject, CompositionRoundedRectangleGeometry, ContainerVisual, SpriteVisual,
    },
    Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::DestroyWindow,
    },
    core::{Interface, Result},
};

/// Flight stages, ms: the shot stands still while the window is first shown (otherwise the first shrink
/// frames jump), shrink to the center, the shot hangs, flight to the island. The flash runs from the start.
const FLASH_MS: u32 = 300;
const START_MS: u32 = 60;
const SHRINK_MS: u32 = 340;
const HOLD_MS: u32 = 900;
const GLIDE_MS: u32 = 460;
/// Until this moment the shot at the center can be clicked; after it the shot flies to the island.
pub const CATCH_UNTIL_MS: u32 = START_MS + SHRINK_MS + HOLD_MS;
pub const LANDING_MS: u32 = CATCH_UNTIL_MS + GLIDE_MS;
const FADE_MS: u32 = 120;
pub const FLIGHT_MS: u32 = LANDING_MS + FADE_MS;
pub const GLIDE_START_MS: u32 = START_MS;
pub const GLIDE_LANDING_MS: u32 = START_MS + GLIDE_MS;
pub const GLIDE_FLIGHT_MS: u32 = GLIDE_LANDING_MS + FADE_MS;
pub const EXPAND_MS: u32 = 280;
pub const EXPAND_FLIGHT_MS: u32 = EXPAND_MS + FADE_MS;

const FLASH: f32 = 0.5;
const MIDDLE: f32 = 0.42;
const MIDDLE_LIFT: f32 = 0.04;
const MIDDLE_RADIUS: f32 = 14.0;
pub const SLOT_RADIUS: f32 = 6.0;
const BACKDROP: f32 = 0.22;
const EDGE: f32 = 1.0;
const EDGE_ALPHA: f32 = 0.9;

const SPRING: Motion = Motion::curve(0, 0.32, 1.2, 0.5, 1.0);
const STILL: Motion = Motion::linear(0);
const GLIDE: Motion = Motion::curve(0, 0.45, 0.0, 0.2, 1.0);
const EXPAND: Motion = Motion::curve(EXPAND_MS, 0.2, 0.0, 0.0, 1.0);

pub type Place = (f32, f32, f32, f32);

struct Stop {
    at: u32,
    place: Place,
    radius: f32,
    motion: Motion,
}

struct Photo {
    visual: SpriteVisual,
    corners: CompositionRoundedRectangleGeometry,
}

pub struct Flight {
    window: Overlay,
    photo: Option<Photo>,
    backdrop: Option<SpriteVisual>,
    catcher: Option<HWND>,
}

impl Flight {
    /// `monitor` is the whole monitor in physical pixels, `scale` its scale; `slot` is the thumbnail on
    /// the future card.
    pub fn shot(comp: &Comp, monitor: RECT, k: f32, preview: &Image, slot: Place) -> Result<Self> {
        let mut flight = Self::open(comp, monitor, k)?;
        let (width, height) = flight.window.screen;
        let start = (0.0, 0.0, width, height);
        let middle = middle_of((width, height));
        let stops = [
            stop(0, start, 0.0, STILL),
            stop(START_MS, start, 0.0, STILL),
            stop(START_MS + SHRINK_MS, middle, MIDDLE_RADIUS, SPRING),
            stop(CATCH_UNTIL_MS, middle, MIDDLE_RADIUS, STILL),
            stop(LANDING_MS, slot, SLOT_RADIUS, GLIDE),
        ];
        flight.draw_shot(comp, (width, height), preview, &stops)?;
        let (x, y, w, h) = middle;
        flight.catcher = window::create_catcher(RECT {
            left: monitor.left + (x * k).round() as i32,
            top: monitor.top + (y * k).round() as i32,
            right: monitor.left + ((x + w) * k).round() as i32,
            bottom: monitor.top + ((y + h) * k).round() as i32,
        })
        .inspect_err(|e| crate::journal::warn(tr!("journal.flight_click_off", error = e)))
        .ok();
        flight.show();
        Ok(flight)
    }

    /// `area` — where the selected area is on the monitor; it lands like a glide.
    pub fn area(
        comp: &Comp,
        monitor: RECT,
        k: f32,
        preview: &Image,
        area: Place,
        slot: Place,
    ) -> Result<Self> {
        let mut flight = Self::open(comp, monitor, k)?;
        let stops = glide_stops((area, slot), (0.0, SLOT_RADIUS));
        let root = flight.window.root.clone();
        let photo = photo(comp, &root, preview, &stops, GLIDE_FLIGHT_MS)?;
        edge(comp, &root, &photo)?;
        flight.photo = Some(photo);
        flash(comp, &root, area, GLIDE_FLIGHT_MS)?;
        flight.show();
        Ok(flight)
    }

    pub fn glide(
        comp: &Comp,
        monitor: RECT,
        k: f32,
        preview: &Image,
        path: (Place, Place),
        radius: (f32, f32),
    ) -> Result<Self> {
        let mut flight = Self::open(comp, monitor, k)?;
        let stops = glide_stops(path, radius);
        let root = flight.window.root.clone();
        flight.photo = Some(photo(comp, &root, preview, &stops, GLIDE_FLIGHT_MS)?);
        flight.show();
        Ok(flight)
    }

    pub fn is_catcher(&self, hwnd: HWND) -> bool {
        self.catcher == Some(hwnd)
    }

    pub fn stop_catching(&mut self) {
        if let Some(catcher) = self.catcher.take() {
            unsafe {
                let _ = DestroyWindow(catcher);
            }
        }
    }

    /// The shot, wherever it is now, expands into `to` and fades out over it; the dimming goes away.
    /// The flight to the island is cancelled.
    pub fn expand(&mut self, comp: &Comp, to: Place, radius: f32) -> Result<()> {
        self.stop_catching();
        if let Some(backdrop) = &self.backdrop {
            comp.animate_scalar(backdrop, "Opacity", 0.0, EXPAND)?;
        }
        let Some(photo) = &self.photo else {
            return Ok(());
        };
        // Without a starting point an animation starts from what is on screen now.
        comp.animate_vector2(&photo.visual, "Size", v2(to.2, to.3), EXPAND)?;
        comp.animate_vector3(&photo.visual, "Offset", v3(to.0, to.1), EXPAND)?;
        comp.animate_vector2(&photo.corners, "CornerRadius", v2(radius, radius), EXPAND)?;
        let landed = EXPAND_MS as f32 / EXPAND_FLIGHT_MS as f32;
        comp.keyframes(
            &photo.visual,
            "Opacity",
            &[(0.0, 1.0), (landed, 1.0), (1.0, 0.0)],
            EXPAND_FLIGHT_MS,
        )
    }

    /// The flight window, still empty and hidden.
    fn open(comp: &Comp, monitor: RECT, k: f32) -> Result<Self> {
        Ok(Self {
            window: Overlay::open(comp, window::create_overlay, monitor, k)?,
            photo: None,
            backdrop: None,
            catcher: None,
        })
    }

    fn show(&self) {
        platform::show_without_focus(self.window.hwnd());
        if let Some(catcher) = self.catcher {
            platform::show_without_focus(catcher);
        }
    }

    fn draw_shot(
        &mut self,
        comp: &Comp,
        (width, height): (f32, f32),
        preview: &Image,
        stops: &[Stop],
    ) -> Result<()> {
        let c = &comp.compositor;
        let root = self.window.root.clone();
        let ms = FLIGHT_MS;
        let size = v2(width, height);
        let shrunk = at(START_MS + SHRINK_MS, ms);
        let held = at(CATCH_UNTIL_MS, ms);
        let landed = at(LANDING_MS, ms);
        let children = root.Children()?;

        let backdrop = c.CreateSpriteVisual()?;
        backdrop.SetSize(size)?;
        backdrop.SetBrush(&comp.brush(color("#000000", 1.0))?)?;
        backdrop.SetOpacity(0.0)?;
        comp.keyframes(
            &backdrop,
            "Opacity",
            &[
                (0.0, 0.0),
                (shrunk, BACKDROP),
                (held, BACKDROP),
                (landed, 0.0),
                (1.0, 0.0),
            ],
            ms,
        )?;
        children.InsertAtTop(&backdrop)?;
        self.backdrop = Some(backdrop);
        self.photo = Some(photo(comp, &root, preview, stops, ms)?);
        flash(comp, &root, (0.0, 0.0, width, height), ms)
    }
}

/// The flash over `lit`, the captured part of the screen; `total` is the flight's length.
fn flash(comp: &Comp, root: &ContainerVisual, lit: Place, total: u32) -> Result<()> {
    let flash = comp.compositor.CreateSpriteVisual()?;
    flash.SetOffset(v3(lit.0, lit.1))?;
    flash.SetSize(v2(lit.2, lit.3))?;
    flash.SetBrush(&comp.brush(color("#FFFFFF", 1.0))?)?;
    flash.SetOpacity(0.0)?;
    comp.keyframes(
        &flash,
        "Opacity",
        &[(0.0, FLASH), (at(FLASH_MS, total), 0.0), (1.0, 0.0)],
        total,
    )?;
    root.Children()?.InsertAtTop(&flash)
}

/// A white layer under the shot, one edge wider on each side and rounded with it: only the edge
/// shows. It follows the shot by expressions, so it costs as little as the shot's own clip.
fn edge(comp: &Comp, root: &ContainerVisual, photo: &Photo) -> Result<()> {
    let c = &comp.compositor;
    let visual = c.CreateSpriteVisual()?;
    visual.SetBrush(&comp.brush(color("#FFFFFF", EDGE_ALPHA))?)?;
    let shot: CompositionObject = photo.visual.cast()?;
    let round: CompositionObject = photo.corners.cast()?;
    let refs = [("shot", &shot), ("round", &round)];
    let width = [("e", EDGE)];
    let grown = "shot.Size + Vector2(2 * e, 2 * e)";
    comp.bind(
        &visual,
        "Offset",
        "shot.Offset - Vector3(e, e, 0)",
        &refs,
        &width,
    )?;
    comp.bind(&visual, "Size", grown, &refs, &width)?;
    comp.bind(&visual, "Opacity", "shot.Opacity", &refs, &width)?;
    let corners = c.CreateRoundedRectangleGeometry()?;
    comp.bind(&corners, "Size", grown, &refs, &width)?;
    comp.bind(
        &corners,
        "CornerRadius",
        "round.CornerRadius + Vector2(e, e)",
        &refs,
        &width,
    )?;
    visual.SetClip(&c.CreateGeometricClipWithGeometry(&corners)?)?;
    root.Children()?.InsertBelow(&visual, &photo.visual)
}

impl Drop for Flight {
    fn drop(&mut self) {
        self.stop_catching();
    }
}

fn stop(at: u32, place: Place, radius: f32, motion: Motion) -> Stop {
    Stop {
        at,
        place,
        radius,
        motion,
    }
}

/// Straight from one place to another, with the corner radius of each.
fn glide_stops((from, to): (Place, Place), radius: (f32, f32)) -> [Stop; 3] {
    [
        stop(0, from, radius.0, STILL),
        stop(START_MS, from, radius.0, STILL),
        stop(GLIDE_LANDING_MS, to, radius.1, GLIDE),
    ]
}

/// Where the screen's shot hangs to be seen: a smaller copy at the center, a little lifted.
fn middle_of((width, height): (f32, f32)) -> Place {
    let (w, h) = (width * MIDDLE, height * MIDDLE);
    (
        (width - w) / 2.0,
        (height - h) / 2.0 - height * MIDDLE_LIFT,
        w,
        h,
    )
}

fn at(ms: u32, total: u32) -> f32 {
    ms as f32 / total as f32
}

/// The shot with rounded corners follows `stops` and fades out [`FADE_MS`] after the last one.
/// `total` is the length of the whole flight.
fn photo(
    comp: &Comp,
    root: &ContainerVisual,
    preview: &Image,
    stops: &[Stop],
    total: u32,
) -> Result<Photo> {
    let c = &comp.compositor;
    let visual = comp.image_rounded(
        &preview.pixels,
        (preview.width, preview.height),
        (preview.width as f32, preview.height as f32),
        0.0,
        1.0,
    )?;
    let corners = c.CreateRoundedRectangleGeometry()?;
    let p: CompositionObject = visual.cast()?;
    comp.bind(&corners, "Size", "p.Size", &[("p", &p)], &[])?;
    visual.SetClip(&c.CreateGeometricClipWithGeometry(&corners)?)?;

    let first = &stops[0];
    visual.SetSize(v2(first.place.2, first.place.3))?;
    visual.SetOffset(v3(first.place.0, first.place.1))?;
    comp.path_vector2(
        &visual,
        "Size",
        &path(stops, total, |s| v2(s.place.2, s.place.3)),
        total,
    )?;
    comp.path_vector3(
        &visual,
        "Offset",
        &path(stops, total, |s| v3(s.place.0, s.place.1)),
        total,
    )?;
    comp.path_vector2(
        &corners,
        "CornerRadius",
        &path(stops, total, |s| v2(s.radius, s.radius)),
        total,
    )?;
    let landed = stops[stops.len() - 1].at;
    comp.keyframes(
        &visual,
        "Opacity",
        &[
            (0.0, 1.0),
            (at(landed, total), 1.0),
            (at(landed + FADE_MS, total), 0.0),
        ],
        total,
    )?;
    root.Children()?.InsertAtTop(&visual)?;
    Ok(Photo { visual, corners })
}

/// Keyframes of one property of the shot; the last one holds until the end of the flight.
fn path<T: Copy>(stops: &[Stop], total: u32, value: impl Fn(&Stop) -> T) -> Vec<(f32, T, Motion)> {
    let mut keys: Vec<_> = stops
        .iter()
        .map(|s| (at(s.at, total), value(s), s.motion))
        .collect();
    if let Some(last) = stops.last().filter(|s| s.at < total) {
        keys.push((1.0, value(last), STILL));
    }
    keys
}
