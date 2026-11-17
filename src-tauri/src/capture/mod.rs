//! Screen capture: what a screenshot and a recording have in common, with no Windows in it.
//!
//! - [`Source`] is what to capture: a monitor and a rectangle on it. The whole monitor is just
//!   a rectangle of its full size; area selection will hand over a smaller one, and both the
//!   screenshot and the recording take it as it comes;
//! - [`Image`] is a picture in memory: the shot itself, a thumbnail, a recording's last frame;
//! - [`Stamp`] is when it was taken, for the file name;
//! - [`Status`] is what was decided about the file while its card is on the island.
//!
//! What each of them does with these is in `shot` and `record`. The Windows side both use
//! (clipboard, file names, the shell, the screen) is in `adapters/`.

pub mod adapters;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    pub fn right(&self) -> u32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// x, y, width and height in logical pixels of a monitor at `scale`.
    pub fn logical(&self, scale: f32) -> (f32, f32, f32, f32) {
        (
            self.x as f32 / scale,
            self.y as f32 / scale,
            self.width as f32 / scale,
            self.height as f32 / scale,
        )
    }
}

/// `area` is in the monitor's own pixels, from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub monitor: isize,
    pub area: Area,
}

impl Source {
    pub fn whole(monitor: isize, width: u32, height: u32) -> Self {
        Self {
            monitor,
            area: Area {
                x: 0,
                y: 0,
                width,
                height,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

impl Stamp {
    /// File name without extension, like Windows' own shots: "<prefix> 2026-11-11 201530".
    pub fn named(&self, prefix: &str) -> String {
        format!(
            "{prefix} {:04}-{:02}-{:02} {:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// BGRA picture, rows top to bottom, opaque (alpha 255 everywhere).
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    /// The part under `area`, clipped to the picture.
    pub fn crop(&self, area: Area) -> Image {
        let x = area.x.min(self.width);
        let y = area.y.min(self.height);
        let width = area.width.min(self.width - x);
        let height = area.height.min(self.height - y);
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in y..y + height {
            let start = ((row * self.width + x) * 4) as usize;
            pixels.extend_from_slice(&self.pixels[start..start + (width * 4) as usize]);
        }
        Image {
            width,
            height,
            pixels,
        }
    }

    /// A reduced copy no wider than `max_width`, for the card and the flight animation. Shrinks by an integer
    /// factor, averaging squares of pixels: fast and without jaggies.
    pub fn preview(&self, max_width: u32) -> Image {
        let k = self.width.div_ceil(max_width.max(1)).max(1);
        if k == 1 || self.height < k {
            return self.clone();
        }
        let (w, h) = (self.width / k, self.height / k);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                for dy in 0..k {
                    let row = (((y * k + dy) * self.width + x * k) * 4) as usize;
                    for px in self.pixels[row..row + (k * 4) as usize].as_chunks::<4>().0 {
                        for (s, &v) in sum.iter_mut().zip(px) {
                            *s += v as u32;
                        }
                    }
                }
                let out = ((y * w + x) * 4) as usize;
                for (o, s) in pixels[out..out + 4].iter_mut().zip(sum) {
                    *o = (s / (k * k)) as u8;
                }
            }
        }
        Image {
            width: w,
            height: h,
            pixels,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Fresh,
    Copied,
    ClipboardOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_prefix_date_and_time() {
        let stamp = Stamp {
            year: 2026,
            month: 11,
            day: 1,
            hour: 9,
            minute: 5,
            second: 7,
        };
        assert_eq!(stamp.named("Shot"), "Shot 2026-11-01 090507");
    }

    #[test]
    fn whole_monitor_is_a_rectangle_from_its_corner() {
        let source = Source::whole(7, 1920, 1080);
        assert_eq!(
            source.area,
            Area {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn crop_cuts_the_area_and_stays_inside() {
        let img = Image {
            width: 3,
            height: 2,
            pixels: (0..24).collect(),
        };
        let part = img.crop(Area {
            x: 1,
            y: 1,
            width: 5,
            height: 5,
        });
        assert_eq!((part.width, part.height), (2, 1));
        assert_eq!(part.pixels, (16..24).collect::<Vec<u8>>());
    }

    #[test]
    fn an_area_in_logical_pixels_is_divided_by_the_scale() {
        let area = Area {
            x: 300,
            y: 150,
            width: 1200,
            height: 600,
        };
        assert_eq!(area.logical(1.5), (200.0, 100.0, 800.0, 400.0));
    }

    #[test]
    fn preview_averages_blocks() {
        let mut img = Image {
            width: 4,
            height: 2,
            pixels: vec![255; 4 * 2 * 4],
        };
        img.pixels[..4].copy_from_slice(&[0, 0, 0, 255]);
        let small = img.preview(2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.pixels[0], (255 * 3 / 4) as u8);
        assert_eq!(small.pixels[4], 255);
        assert_eq!(img.preview(10).width, 4, "a small one is not enlarged");
    }
}
