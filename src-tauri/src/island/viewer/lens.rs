//! Where a picture lies in a viewer window: its zoom and offset inside the room it is shown in.
//! The screenshot and the video windows zoom and move their picture the same way.
//!
//! Units: the room and the offset are logical window pixels, the content is the picture's own
//! pixels, `zoom` is window pixels per picture pixel.

/// x, y, width, height.
pub type Room = (f32, f32, f32, f32);

const MAX_ZOOM: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub zoom: f32,
    pub offset: (f32, f32),
}

impl Lens {
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            offset: (0.0, 0.0),
        }
    }

    pub fn fit_zoom(room: Room, content: (f32, f32)) -> f32 {
        (room.2 / content.0).min(room.3 / content.1)
    }

    /// `k` = physical pixels per logical one: at most 8 screen pixels per picture pixel.
    pub fn max_zoom(room: Room, content: (f32, f32), k: f32) -> f32 {
        (MAX_ZOOM / k).max(Self::fit_zoom(room, content))
    }

    pub fn zoomed(&self, room: Room, content: (f32, f32)) -> bool {
        self.zoom > Self::fit_zoom(room, content) * 1.001
    }

    pub fn fit(&mut self, room: Room, content: (f32, f32)) {
        self.zoom = Self::fit_zoom(room, content);
        self.clamp(room, content);
    }

    /// A picture smaller than the room is centered, a larger one has no empty margins.
    pub fn clamp(&mut self, (x, y, w, h): Room, content: (f32, f32)) {
        let axis = |offset: f32, start: f32, room: f32, size: f32| {
            if size <= room {
                start + (room - size) / 2.0
            } else {
                offset.clamp(start + room - size, start)
            }
        };
        self.offset = (
            axis(self.offset.0, x, w, content.0 * self.zoom),
            axis(self.offset.1, y, h, content.1 * self.zoom),
        );
    }

    /// Zooms keeping the picture point under `p` in place.
    pub fn zoom_at(&mut self, zoom: f32, p: (f32, f32), room: Room, content: (f32, f32), k: f32) {
        let zoom = zoom.clamp(
            Self::fit_zoom(room, content),
            Self::max_zoom(room, content, k),
        );
        let ratio = zoom / self.zoom;
        self.offset = (
            p.0 - (p.0 - self.offset.0) * ratio,
            p.1 - (p.1 - self.offset.1) * ratio,
        );
        self.zoom = zoom;
        self.clamp(room, content);
    }

    /// The double click's target from "fit": pixel for pixel, or twice the fit when the picture
    /// is already smaller than that.
    pub fn closer(room: Room, content: (f32, f32), k: f32) -> f32 {
        let one = 1.0 / k;
        let fit = Self::fit_zoom(room, content);
        if one > fit * 1.05 { one } else { fit * 2.0 }
    }

    pub fn pan(&mut self, (dx, dy): (f32, f32), room: Room, content: (f32, f32)) {
        self.offset.0 += dx;
        self.offset.1 += dy;
        self.clamp(room, content);
    }

    pub fn point_at(&self, p: (f32, f32)) -> (f32, f32) {
        (
            (p.0 - self.offset.0) / self.zoom,
            (p.1 - self.offset.1) / self.zoom,
        )
    }

    pub fn covers(&self, p: (f32, f32), content: (f32, f32)) -> bool {
        let (w, h) = (content.0 * self.zoom, content.1 * self.zoom);
        p.0 >= self.offset.0
            && p.0 <= self.offset.0 + w
            && p.1 >= self.offset.1
            && p.1 <= self.offset.1 + h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: Room = (0.0, 40.0, 800.0, 450.0);

    #[test]
    fn a_fitted_picture_is_centered() {
        let mut lens = Lens::new();
        lens.fit(ROOM, (1920.0, 1080.0));
        assert!((lens.zoom - 800.0 / 1920.0).abs() < 1e-6);
        assert_eq!(lens.offset, (0.0, 40.0));
        let mut small = Lens::new();
        small.fit(ROOM, (400.0, 100.0));
        assert_eq!(small.zoom, 2.0);
        assert_eq!(small.offset, (0.0, 40.0 + (450.0 - 200.0) / 2.0));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor_and_leaves_no_margins() {
        let content = (1920.0, 1080.0);
        let mut lens = Lens::new();
        lens.fit(ROOM, content);
        let p = (400.0, 265.0);
        let before = lens.point_at(p);
        lens.zoom_at(1.0, p, ROOM, content, 1.0);
        let after = lens.point_at(p);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
        lens.zoom_at(1.0, (0.0, 40.0), ROOM, content, 1.0);
        lens.pan((10_000.0, 10_000.0), ROOM, content);
        assert_eq!(lens.offset, (0.0, 40.0));
        assert!(lens.zoomed(ROOM, content));
    }

    #[test]
    fn zoom_stays_between_fit_and_eight_screen_pixels() {
        let content = (1920.0, 1080.0);
        let mut lens = Lens::new();
        lens.fit(ROOM, content);
        lens.zoom_at(100.0, (0.0, 0.0), ROOM, content, 2.0);
        assert_eq!(lens.zoom, 4.0);
        lens.zoom_at(0.0, (0.0, 0.0), ROOM, content, 2.0);
        assert!(!lens.zoomed(ROOM, content));
    }

    #[test]
    fn a_double_click_goes_to_pixel_for_pixel_or_twice_the_fit() {
        assert_eq!(Lens::closer(ROOM, (1920.0, 1080.0), 1.0), 1.0);
        assert_eq!(Lens::closer(ROOM, (400.0, 100.0), 1.0), 4.0);
    }
}
