//! Screenshot editing: marks over the shot and the edit history.
//!
//! Pure logic without Windows: what is drawn, where, and how to undo it. The island draws the marks
//! (`island/viewer/paint.rs`) with the same code on screen and in the file, so what is seen in the
//! editor is what gets saved.
//!
//! All coordinates are in pixels of the original shot.

use crate::capture::Image;

pub type Point = (f32, f32);

pub type Rgb = [u8; 3];

pub const MARKER_ALPHA: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    pub fn of(image: &Image) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: image.width as f32,
            h: image.height as f32,
        }
    }

    pub fn between(a: Point, b: Point) -> Self {
        Self {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (a.0 - b.0).abs(),
            h: (a.1 - b.1).abs(),
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn inflate(self, by: f32) -> Self {
        Self {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }

    pub fn union(self, other: Self) -> Self {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Self {
            x,
            y,
            w: self.right().max(other.right()) - x,
            h: self.bottom().max(other.bottom()) - y,
        }
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (r, b) = (
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        (r > x && b > y).then_some(Self {
            x,
            y,
            w: r - x,
            h: b - y,
        })
    }

    /// Whole pixels inside a `width`×`height` image: x, y, width, height.
    /// `None` = nothing of the rectangle is left inside the image.
    pub fn pixels(self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let x0 = self.x.floor().clamp(0.0, width as f32) as u32;
        let y0 = self.y.floor().clamp(0.0, height as f32) as u32;
        let x1 = self.right().ceil().clamp(0.0, width as f32) as u32;
        let y1 = self.bottom().ceil().clamp(0.0, height as f32) as u32;
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
    }
}

#[derive(Clone)]
pub enum Mark {
    Stroke {
        points: Vec<Point>,
        color: Rgb,
        width: f32,
        marker: bool,
    },
    Arrow {
        from: Point,
        to: Point,
        color: Rgb,
        width: f32,
    },
    Frame {
        area: Area,
        color: Rgb,
        width: f32,
    },
    /// An area covered with large pixels: `patch` is the ready picture of that area.
    Pixels {
        area: Area,
        patch: Image,
    },
    /// Crop: only this part of the shot stays visible.
    Crop(Area),
}

impl Mark {
    /// What the mark covers on the shot, with room for line thickness and the arrow head.
    /// A crop has no extent: it draws nothing itself.
    pub fn bounds(&self) -> Option<Area> {
        match self {
            Self::Stroke { points, width, .. } => {
                let first = *points.first()?;
                let area = points.iter().fold(Area::between(first, first), |area, &p| {
                    area.union(Area::between(p, p))
                });
                Some(area.inflate(width / 2.0 + 1.0))
            }
            Self::Arrow {
                from, to, width, ..
            } => {
                let head = arrow_head(*from, *to, *width).map_or(0.0, |h| h.size);
                Some(Area::between(*from, *to).inflate(width / 2.0 + head + 1.0))
            }
            Self::Frame { area, width, .. } => Some(area.inflate(width / 2.0 + 1.0)),
            Self::Pixels { area, .. } => Some(*area),
            Self::Crop(_) => None,
        }
    }
}

/// Arrow head: a triangle and where the shaft ends, so that its round end doesn't stick out of the tip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Head {
    pub points: [Point; 3],
    pub shaft_end: Point,
    pub size: f32,
}

pub fn arrow_head(from: Point, to: Point, width: f32) -> Option<Head> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0 {
        return None;
    }
    let (ux, uy) = (dx / length, dy / length);
    // The head grows with thickness but is no longer than half the arrow.
    let size = (width * 3.0 + 8.0).min(length * 0.5).max(width * 1.5);
    let half = size * 0.6;
    let base = (to.0 - ux * size, to.1 - uy * size);
    Some(Head {
        points: [
            to,
            (base.0 - uy * half, base.1 + ux * half),
            (base.0 + uy * half, base.1 - ux * half),
        ],
        shaft_end: (to.0 - ux * size * 0.6, to.1 - uy * size * 0.6),
        size,
    })
}

pub fn visible<'a>(marks: impl IntoIterator<Item = &'a Mark>, full: Area) -> Area {
    marks
        .into_iter()
        .filter_map(|m| match m {
            Mark::Crop(area) => Some(*area),
            _ => None,
        })
        .last()
        .unwrap_or(full)
}

pub fn cut_out(image: &Image, (x, y, w, h): (u32, u32, u32, u32)) -> Image {
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * image.width + x) * 4) as usize;
        pixels.extend_from_slice(&image.pixels[start..start + (w * 4) as usize]);
    }
    Image {
        width: w,
        height: h,
        pixels,
    }
}

/// Large pixels over a part of the picture: each `block`×`block` square is the average color.
/// Squares count from the corner of the whole picture, not the area, so adjacent areas fall onto one grid.
pub fn pixelate(image: &Image, (x, y, w, h): (u32, u32, u32, u32), block: u32) -> Image {
    let block = block.max(1);
    let mut patch = cut_out(image, (x, y, w, h));
    let first = |start: u32| start / block * block;
    let mut by = first(y);
    while by < y + h {
        let (y0, y1) = (by.max(y), (by + block).min(y + h));
        let mut bx = first(x);
        while bx < x + w {
            let (x0, x1) = (bx.max(x), (bx + block).min(x + w));
            let mut sum = [0u32; 4];
            for row in y0..y1 {
                for col in x0..x1 {
                    let i = ((row * image.width + col) * 4) as usize;
                    for (s, &v) in sum.iter_mut().zip(&image.pixels[i..i + 4]) {
                        *s += u32::from(v);
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            let average = sum.map(|s| (s / n) as u8);
            for row in y0..y1 {
                for col in x0..x1 {
                    let i = (((row - y) * w + (col - x)) * 4) as usize;
                    patch.pixels[i..i + 4].copy_from_slice(&average);
                }
            }
            bx += block;
        }
        by += block;
    }
    patch
}

/// Edit history: undo and redo. Stores anything: the island puts a mark here together with its on-screen picture.
pub struct History<T> {
    done: Vec<T>,
    undone: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self {
            done: Vec::new(),
            undone: Vec::new(),
        }
    }
}

impl<T> History<T> {
    /// A new edit. Undone ones after it can't be redone: they are returned so they can be removed.
    pub fn push(&mut self, item: T) -> Vec<T> {
        self.done.push(item);
        std::mem::take(&mut self.undone)
    }

    pub fn undo(&mut self) -> Option<&T> {
        let item = self.done.pop()?;
        self.undone.push(item);
        self.undone.last()
    }

    pub fn redo(&mut self) -> Option<&T> {
        let item = self.undone.pop()?;
        self.done.push(item);
        self.done.last()
    }

    pub fn done(&self) -> &[T] {
        &self.done
    }

    pub fn is_dirty(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            pixels: (0..width * height)
                .flat_map(|i| [i as u8, 0, 0, 255])
                .collect(),
        }
    }

    #[test]
    fn area_between_any_corners() {
        let a = Area::between((10.0, 5.0), (2.0, 9.0));
        assert_eq!(
            a,
            Area {
                x: 2.0,
                y: 5.0,
                w: 8.0,
                h: 4.0
            }
        );
        assert_eq!(a.pixels(6, 100), Some((2, 5, 4, 4)), "обрезано по картинке");
        assert_eq!(a.pixels(2, 100), None);
    }

    #[test]
    fn history_undo_redo_and_new_edit_drops_redo() {
        let mut h = History::default();
        assert!(!h.is_dirty());
        h.push(1);
        h.push(2);
        assert_eq!(h.undo(), Some(&2));
        assert_eq!(h.done(), &[1]);
        assert_eq!(h.redo(), Some(&2));
        assert_eq!(h.redo(), None);
        h.undo();
        assert_eq!(h.push(3), vec![2], "отменённую уже не вернуть");
        assert_eq!(h.done(), &[1, 3]);
        h.undo();
        h.undo();
        assert_eq!(h.undo(), None);
        assert!(!h.is_dirty());
    }

    #[test]
    fn last_crop_wins() {
        let full = Area::between((0.0, 0.0), (100.0, 100.0));
        let first = Area::between((10.0, 10.0), (90.0, 90.0));
        let second = Area::between((20.0, 20.0), (50.0, 50.0));
        assert_eq!(visible([], full), full);
        let marks = [Mark::Crop(first), Mark::Crop(second)];
        assert_eq!(visible(&marks, full), second);
    }

    #[test]
    fn cut_out_takes_the_rows() {
        let img = image(4, 3);
        let part = cut_out(&img, (1, 1, 2, 2));
        assert_eq!((part.width, part.height), (2, 2));
        assert_eq!(part.pixels[0], 5);
        assert_eq!(part.pixels[4 * 3], 10);
    }

    #[test]
    fn pixelate_averages_blocks_on_a_shared_grid() {
        let img = image(4, 1);
        let patch = pixelate(&img, (1, 0, 3, 1), 2);
        // Grid from the picture corner: pixel 1 is in the second half-block [0, 2), counted alone;
        // pixels 2 and 3 are block [2, 4), average 2.
        assert_eq!(patch.pixels[0], 1);
        assert_eq!(patch.pixels[4], 2);
        assert_eq!(patch.pixels[8], 2);
        assert_eq!(patch.pixels[11], 255, "непрозрачность сохраняется");
    }

    #[test]
    fn arrow_head_points_at_the_target() {
        let head = arrow_head((0.0, 0.0), (100.0, 0.0), 4.0).expect("стрелка есть");
        assert_eq!(head.points[0], (100.0, 0.0));
        assert!(head.shaft_end.0 < 100.0 && head.shaft_end.0 > 100.0 - head.size);
        assert_eq!(head.points[1].0, head.points[2].0);
        assert!(arrow_head((5.0, 5.0), (5.2, 5.0), 4.0).is_none());
    }

    #[test]
    fn stroke_bounds_cover_the_line_width() {
        let mark = Mark::Stroke {
            points: vec![(10.0, 10.0), (20.0, 15.0)],
            color: [255, 0, 0],
            width: 4.0,
            marker: false,
        };
        let b = mark.bounds().expect("есть");
        assert!(b.x <= 8.0 && b.right() >= 22.0 && b.y <= 8.0 && b.bottom() >= 17.0);
        assert!(Mark::Crop(b).bounds().is_none());
    }
}
