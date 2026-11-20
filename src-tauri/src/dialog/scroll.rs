//! The notes' scroll: where the content stands in its box, the thumb, and whether the edges fade.
//! It behaves like the WebView2 box it replaces: a wheel notch moves 100 px, a press on the track
//! pages by 87.5% of the box, the thumb is at least 30 px long.

/// The distance of one wheel notch (`WHEEL_DELTA` = 120).
const NOTCH: f32 = 100.0;
const THUMB_MIN: f32 = 30.0;
const PAGE: f32 = 0.875;
/// An edge fades while more than this is hidden past it.
const FADE_PAST: f32 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scroll {
    pub offset: f32,
    view: f32,
    content: f32,
}

impl Scroll {
    pub fn new(view: f32, content: f32) -> Self {
        Self {
            offset: 0.0,
            view,
            content,
        }
    }

    fn max(&self) -> f32 {
        (self.content - self.view).max(0.0)
    }

    /// False: it stays where it was.
    pub fn set(&mut self, offset: f32) -> bool {
        let offset = offset.clamp(0.0, self.max());
        let moved = offset != self.offset;
        self.offset = offset;
        moved
    }

    /// `delta` in wheel units, positive away from the user (up).
    pub fn wheel(&mut self, delta: f32) -> bool {
        self.set(self.offset - delta / 120.0 * NOTCH)
    }

    /// The thumb's top and length along the track, which is the box's height.
    pub fn thumb(&self) -> (f32, f32) {
        if self.content <= 0.0 {
            return (0.0, self.view);
        }
        let len = (self.view * self.view / self.content)
            .max(THUMB_MIN)
            .min(self.view);
        let max = self.max();
        let top = if max > 0.0 {
            self.offset / max * (self.view - len)
        } else {
            0.0
        };
        (top, len)
    }

    /// The thumb grabbed when the scroll stood at `from` and moved by `dy` since.
    pub fn drag(&mut self, from: f32, dy: f32) -> bool {
        let (_, len) = self.thumb();
        let room = self.view - len;
        if room <= 0.0 {
            return false;
        }
        self.set(from + dy / room * self.max())
    }

    /// A press on the track at `at` off the thumb: a page towards it.
    pub fn page(&mut self, at: f32) -> bool {
        let (top, len) = self.thumb();
        let page = self.view * PAGE;
        if at < top {
            self.set(self.offset - page)
        } else if at >= top + len {
            self.set(self.offset + page)
        } else {
            false
        }
    }

    pub fn fades(&self) -> (bool, bool) {
        (
            self.offset > FADE_PAST,
            self.offset + self.view < self.content - FADE_PAST,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_moves_within_the_content() {
        let mut s = Scroll::new(170.0, 400.0);
        assert!(s.wheel(-120.0));
        assert_eq!(s.offset, 100.0);
        s.wheel(-360.0);
        assert_eq!(s.offset, 230.0);
        assert!(!s.wheel(-120.0), "at the end it stays");
        s.wheel(60.0);
        assert_eq!(s.offset, 180.0);
    }

    #[test]
    fn the_thumb_is_as_long_as_the_view_is_of_the_content() {
        let mut s = Scroll::new(170.0, 340.0);
        assert_eq!(s.thumb(), (0.0, 85.0));
        s.set(170.0);
        assert_eq!(s.thumb(), (85.0, 85.0));
        assert_eq!(Scroll::new(170.0, 5000.0).thumb().1, 30.0);
    }

    #[test]
    fn dragging_the_thumb_moves_the_content_in_proportion() {
        let mut s = Scroll::new(170.0, 340.0);
        assert!(s.drag(0.0, 42.5));
        assert_eq!(s.offset, 85.0);
        s.drag(0.0, 1000.0);
        assert_eq!(s.offset, 170.0);
    }

    #[test]
    fn a_press_on_the_track_pages_towards_it() {
        let mut s = Scroll::new(160.0, 1000.0);
        assert!(s.page(150.0));
        assert_eq!(s.offset, 140.0);
        let (top, _) = s.thumb();
        assert!(!s.page(top + 1.0), "on the thumb");
        assert!(s.page(0.0));
        assert_eq!(s.offset, 0.0);
    }

    #[test]
    fn the_edges_fade_where_there_is_more() {
        let mut s = Scroll::new(170.0, 400.0);
        assert_eq!(s.fades(), (false, true));
        s.set(100.0);
        assert_eq!(s.fades(), (true, true));
        s.set(230.0);
        assert_eq!(s.fades(), (true, false));
        assert_eq!(Scroll::new(170.0, 120.0).fades(), (false, false));
    }
}
