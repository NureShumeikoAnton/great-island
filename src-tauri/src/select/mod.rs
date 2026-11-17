//! Area selection: what the pointer over a monitor picks, with no Windows in it.
//!
//! The screen is one monitor in its own pixels; the windows on it come top first, already
//! clipped to it. The island feeds the pointer in and draws [`Look`]; a release gives the
//! [`Area`] that the screenshot or the recording takes.
//!
//! - Before the press the window under the pointer is lit, and a click takes it. A click on
//!   the empty desktop takes nothing.
//! - A drag draws a rectangle. When it holds a window with a little room around it, the
//!   frame goes onto the window and the release takes the window exactly. A rectangle
//!   inside a window is taken as drawn.
//! - Alt held: no window, the rectangle is exactly as drawn.

pub mod adapters;

use crate::capture::Area;

/// Distances in the monitor's pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// A press and release closer than this is a click.
    pub click: u32,
    /// How far outside a window's edge the drawn edge may be for the window to be meant.
    pub around: u32,
    /// How far inside: a drag from the very edge lands a few pixels in.
    pub inside: u32,
}

impl Tuning {
    /// The same distances for the hand at any monitor scale.
    pub fn at(scale: f32) -> Self {
        let px = |v: f32| (v * scale).round() as u32;
        Self {
            click: px(4.0),
            around: px(64.0),
            inside: px(8.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// The clear part of the dimming, and what the release would take.
    pub hole: Option<Area>,
    /// The hole is a window, not a drawn rectangle.
    pub window: bool,
    pub dragging: bool,
    /// Alt is held.
    pub free: bool,
}

struct Drag {
    from: (u32, u32),
    moved: bool,
}

pub struct Selection {
    screen: (u32, u32),
    windows: Vec<Area>,
    tuning: Tuning,
    at: (u32, u32),
    free: bool,
    drag: Option<Drag>,
}

impl Selection {
    pub fn new(screen: (u32, u32), windows: Vec<Area>, tuning: Tuning) -> Self {
        Self {
            screen,
            windows,
            tuning,
            at: (u32::MAX, u32::MAX),
            free: false,
            drag: None,
        }
    }

    /// Outside the screen counts as its edge.
    fn clamp(&self, x: i32, y: i32) -> (u32, u32) {
        (
            x.clamp(0, self.screen.0 as i32) as u32,
            y.clamp(0, self.screen.1 as i32) as u32,
        )
    }

    pub fn point(&mut self, x: i32, y: i32) {
        self.at = self.clamp(x, y);
        let (at, click) = (self.at, self.tuning.click);
        if let Some(drag) = &mut self.drag {
            drag.moved |= distance(drag.from, at) > click;
        }
    }

    pub fn press(&mut self, x: i32, y: i32) {
        self.at = self.clamp(x, y);
        self.drag = Some(Drag {
            from: self.at,
            moved: false,
        });
    }

    /// The picked area; `None` — nothing (a click on the desktop, a drag with no size).
    /// Selection goes on after `None`.
    pub fn release(&mut self, x: i32, y: i32) -> Option<Area> {
        self.point(x, y);
        let target = self.look().hole.filter(|_| self.drag.is_some());
        self.drag = None;
        target
    }

    pub fn set_free(&mut self, free: bool) {
        self.free = free;
    }

    pub fn look(&self) -> Look {
        let (hole, window, dragging) = match &self.drag {
            Some(drag) if drag.moved => match self.held() {
                Some(w) => (Some(w), true, true),
                None => (self.drawn(), false, true),
            },
            _ => {
                let lit = self.window_at(self.at);
                (lit, lit.is_some(), false)
            }
        };
        Look {
            hole,
            window,
            dragging,
            free: self.free,
        }
    }

    fn window_at(&self, (x, y): (u32, u32)) -> Option<Area> {
        self.windows.iter().copied().find(|w| w.contains(x, y))
    }

    fn drawn(&self) -> Option<Area> {
        let drag = self.drag.as_ref()?;
        let area = corners(drag.from, self.at);
        (area.width > 0 && area.height > 0).then_some(area)
    }

    /// The window the drawn rectangle holds with a little room: the snuggest one, the top one
    /// on a tie.
    fn held(&self) -> Option<Area> {
        if self.free {
            return None;
        }
        let drawn = self.drawn()?;
        let t = self.tuning;
        // How far each drawn edge is outside the window's; below zero — inside.
        let room = |w: &Area| {
            [
                i64::from(w.x) - i64::from(drawn.x),
                i64::from(w.y) - i64::from(drawn.y),
                i64::from(drawn.right()) - i64::from(w.right()),
                i64::from(drawn.bottom()) - i64::from(w.bottom()),
            ]
        };
        let fits = |r: &i64| (-i64::from(t.inside)..=i64::from(t.around)).contains(r);
        self.windows
            .iter()
            .filter_map(|w| {
                let room = room(w);
                room.iter()
                    .all(fits)
                    .then(|| (room.iter().map(|r| r.unsigned_abs()).sum::<u64>(), *w))
            })
            .min_by_key(|(snug, _)| *snug)
            .map(|(_, w)| w)
    }
}

fn distance(a: (u32, u32), b: (u32, u32)) -> u32 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}

fn corners(a: (u32, u32), b: (u32, u32)) -> Area {
    let (x, y) = (a.0.min(b.0), a.1.min(b.1));
    Area {
        x,
        y,
        width: a.0.max(b.0) - x,
        height: a.1.max(b.1) - y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: u32, y: u32, width: u32, height: u32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    const EDITOR: Area = Area {
        x: 200,
        y: 100,
        width: 800,
        height: 600,
    };
    const DIALOG: Area = Area {
        x: 400,
        y: 300,
        width: 300,
        height: 200,
    };

    fn screen() -> Selection {
        Selection::new((1920, 1080), vec![DIALOG, EDITOR], Tuning::at(1.0))
    }

    fn drag(s: &mut Selection, from: (i32, i32), to: (i32, i32)) {
        s.press(from.0, from.1);
        s.point((from.0 + to.0) / 2, (from.1 + to.1) / 2);
        s.point(to.0, to.1);
    }

    #[test]
    fn the_top_window_under_the_pointer_is_lit() {
        let mut s = screen();
        s.point(450, 350);
        assert_eq!(s.look().hole, Some(DIALOG));
        assert!(s.look().window);
        s.point(250, 150);
        assert_eq!(s.look().hole, Some(EDITOR));
        s.point(1500, 900);
        assert_eq!(s.look().hole, None, "the desktop is not lit");
    }

    #[test]
    fn a_click_takes_the_window_and_the_desktop_nothing() {
        let mut s = screen();
        s.press(250, 150);
        assert_eq!(
            s.release(252, 151),
            Some(EDITOR),
            "a shake is still a click"
        );
        s.press(1500, 900);
        assert_eq!(s.release(1500, 900), None);
    }

    #[test]
    fn a_rough_drag_around_a_window_takes_it_exactly() {
        let mut s = screen();
        drag(&mut s, (150, 50), (1050, 760));
        let look = s.look();
        assert_eq!((look.hole, look.window), (Some(EDITOR), true));
        assert_eq!(s.release(1050, 760), Some(EDITOR));
        drag(&mut s, (205, 104), (995, 696));
        assert_eq!(
            s.release(995, 696),
            Some(EDITOR),
            "a hair inside still counts"
        );
    }

    #[test]
    fn the_snuggest_window_wins() {
        let mut s = screen();
        drag(&mut s, (390, 290), (710, 510));
        assert_eq!(s.release(710, 510), Some(DIALOG));
    }

    #[test]
    fn inside_a_window_the_drawn_rectangle_is_taken() {
        let mut s = screen();
        drag(&mut s, (250, 150), (900, 650));
        let look = s.look();
        assert_eq!(
            (look.hole, look.window),
            (Some(area(250, 150, 650, 500)), false)
        );
        assert_eq!(s.release(900, 650), Some(area(250, 150, 650, 500)));
    }

    #[test]
    fn too_much_room_is_a_drawn_rectangle() {
        let mut s = screen();
        drag(&mut s, (100, 20), (1100, 800));
        assert!(!s.look().window);
        assert_eq!(s.release(1100, 800), Some(area(100, 20, 1000, 780)));
    }

    #[test]
    fn holding_still_keeps_the_window() {
        let mut s = screen();
        drag(&mut s, (150, 50), (1050, 760));
        for _ in 0..50 {
            s.point(1050, 760);
        }
        assert_eq!(s.release(1050, 760), Some(EDITOR));
    }

    #[test]
    fn far_from_any_window_the_drawn_rectangle_is_taken() {
        let mut s = screen();
        drag(&mut s, (1200, 200), (1500, 400));
        assert_eq!(s.look().hole, Some(area(1200, 200, 300, 200)));
        assert_eq!(s.release(1500, 400), Some(area(1200, 200, 300, 200)));
    }

    #[test]
    fn alt_draws_exactly() {
        let mut s = screen();
        drag(&mut s, (150, 50), (1050, 760));
        s.set_free(true);
        let look = s.look();
        assert!(look.free && look.dragging && !look.window);
        assert_eq!(s.release(1050, 760), Some(area(150, 50, 900, 710)));
    }

    #[test]
    fn letting_alt_go_brings_the_window_back() {
        let mut s = screen();
        drag(&mut s, (150, 50), (1050, 760));
        s.set_free(true);
        s.set_free(false);
        assert_eq!(s.look().hole, Some(EDITOR));
    }

    #[test]
    fn a_line_without_size_is_nothing() {
        let mut s = screen();
        drag(&mut s, (1200, 200), (1500, 200));
        assert_eq!(s.release(1500, 200), None);
    }

    #[test]
    fn distances_grow_with_the_scale() {
        let t = Tuning::at(1.5);
        assert_eq!((t.click, t.around, t.inside), (6, 96, 12));
    }
}
