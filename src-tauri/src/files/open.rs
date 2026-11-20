//! A file opened from outside the island (Open with, Space in Explorer): which window takes
//! it, the photo formats Windows can read and write, how a camera turned the picture, and
//! where an edit of such a file is saved. Pure rules.

use super::extension;
use crate::capture::Image;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Photo,
    Video,
}

/// What Windows decodes with its own codecs (WebP since Windows 10 1809; HEIC and AVIF only
/// with their Store extensions). GIF is left out: the viewer shows no animation yet.
pub const PHOTOS: [&str; 13] = [
    "png", "jpg", "jpeg", "jpe", "jfif", "bmp", "dib", "tif", "tiff", "webp", "heic", "heif",
    "avif",
];
/// Media Foundation's own readers. WebM is left out: VP9 needs a Store extension and the
/// editor can't read it.
pub const VIDEOS: [&str; 6] = ["mp4", "m4v", "mov", "mkv", "avi", "wmv"];

impl Window {
    pub fn of(name: &str) -> Option<Self> {
        let ext = extension(name)?;
        let any = |list: &[&str]| list.iter().any(|e| e.eq_ignore_ascii_case(ext));
        if any(&PHOTOS) {
            Some(Self::Photo)
        } else if any(&VIDEOS) {
            Some(Self::Video)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Bmp,
    Tiff,
    Webp,
    Heif,
    Avif,
}

impl Format {
    pub fn of(name: &str) -> Option<Self> {
        let ext = extension(name)?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "png" => Self::Png,
            "jpg" | "jpeg" | "jpe" | "jfif" => Self::Jpeg,
            "bmp" | "dib" => Self::Bmp,
            "tif" | "tiff" => Self::Tiff,
            "webp" => Self::Webp,
            "heic" | "heif" => Self::Heif,
            "avif" => Self::Avif,
            _ => return None,
        })
    }

    /// Windows has an encoder for it in any edition. WebP, HEIF and AVIF are read only (HEIF's
    /// encoder comes with a paid extension, which can't be counted on).
    pub fn writable(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::Bmp | Self::Tiff)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Bmp => "bmp",
            Self::Tiff => "tif",
            Self::Webp => "webp",
            Self::Heif => "heic",
            Self::Avif => "avif",
        }
    }

    /// Where an edit of a photo in this format goes. A format Windows can't write gets a JPEG
    /// beside it, the original untouched.
    pub fn edited(self, copy: bool) -> Destination {
        match (self.writable(), copy) {
            (true, false) => Destination::Replace(self),
            (true, true) => Destination::Beside(self),
            (false, _) => Destination::Beside(Self::Jpeg),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Replace(Format),
    /// A free name beside it ("<name>.jpg", else "<name> (2).jpg"); from then on the window
    /// shows that file.
    Beside(Format),
}

/// Where an edited video goes: the editor writes MP4 only, so only an MP4 is replaced; any
/// other one, and any copy, is a free name beside it with `.mp4`.
pub fn video_replaced(source: &Path, copy: bool) -> bool {
    !copy
        && source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
}

pub fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn folder(path: &Path) -> PathBuf {
    path.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

/// How the picture has to be turned to stand upright: the EXIF orientation a camera writes
/// instead of turning the pixels (1–8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    /// Clockwise, in quarter turns, applied first.
    pub quarters: u8,
    pub mirror: bool,
}

impl Turn {
    pub fn of_exif(orientation: u16) -> Self {
        let (quarters, mirror) = match orientation {
            2 => (0, true),
            3 => (2, false),
            4 => (2, true),
            5 => (1, true),
            6 => (1, false),
            7 => (3, true),
            8 => (3, false),
            _ => (0, false),
        };
        Self { quarters, mirror }
    }

    pub fn upright(self) -> bool {
        self == Self::default()
    }

    pub fn size(self, (w, h): (u32, u32)) -> (u32, u32) {
        if self.quarters % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    pub fn apply(self, image: Image) -> Image {
        if self.upright() {
            return image;
        }
        let (w, h) = (image.width as usize, image.height as usize);
        let (nw, nh) = self.size((image.width, image.height));
        let (nw, nh) = (nw as usize, nh as usize);
        let mut pixels = vec![0u8; image.pixels.len()];
        let src = image.pixels.as_chunks::<4>().0;
        let dst = pixels.as_chunks_mut::<4>().0;
        for y in 0..h {
            for x in 0..w {
                let (mut tx, ty) = match self.quarters % 4 {
                    1 => (h - 1 - y, x),
                    2 => (w - 1 - x, h - 1 - y),
                    3 => (y, w - 1 - x),
                    _ => (x, y),
                };
                if self.mirror {
                    tx = nw - 1 - tx;
                }
                dst[ty * nw + tx] = src[y * w + x];
            }
        }
        Image {
            width: nw as u32,
            height: nh as u32,
            pixels,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_and_videos_go_to_their_windows_and_the_rest_to_none() {
        assert_eq!(Window::of("IMG_0042.JPG"), Some(Window::Photo));
        assert_eq!(Window::of("Снимок.heic"), Some(Window::Photo));
        assert_eq!(Window::of("clip.MOV"), Some(Window::Video));
        assert_eq!(Window::of("anim.gif"), None);
        assert_eq!(Window::of("talk.webm"), None);
        assert_eq!(Window::of("notes.txt"), None);
        assert_eq!(Window::of("README"), None);
    }

    #[test]
    fn every_photo_extension_has_a_format() {
        for ext in PHOTOS {
            assert!(Format::of(&format!("a.{ext}")).is_some(), "{ext}");
        }
        assert_eq!(Format::of("a.JPEG"), Some(Format::Jpeg));
        assert_eq!(Format::of("a.gif"), None);
    }

    #[test]
    fn an_edit_replaces_a_writable_photo_and_goes_beside_a_read_only_one() {
        assert_eq!(
            Format::Jpeg.edited(false),
            Destination::Replace(Format::Jpeg)
        );
        assert_eq!(Format::Png.edited(true), Destination::Beside(Format::Png));
        for read_only in [Format::Webp, Format::Heif, Format::Avif] {
            for copy in [false, true] {
                assert_eq!(
                    read_only.edited(copy),
                    Destination::Beside(Format::Jpeg),
                    "{read_only:?}"
                );
            }
        }
    }

    #[test]
    fn only_an_mp4_is_replaced_by_its_edit() {
        assert!(video_replaced(Path::new(r"C:\v\a.MP4"), false));
        assert!(!video_replaced(Path::new(r"C:\v\a.mp4"), true));
        assert!(!video_replaced(Path::new(r"C:\v\a.mov"), false));
        assert!(!video_replaced(Path::new(r"C:\v\a.mkv"), false));
    }

    #[test]
    fn a_phone_held_upright_turns_a_quarter_clockwise() {
        let portrait = Turn::of_exif(6);
        assert_eq!(portrait.quarters, 1);
        assert!(!portrait.mirror);
        assert_eq!(portrait.size((4032, 3024)), (3024, 4032));
        assert!(Turn::of_exif(1).upright());
        assert!(Turn::of_exif(0).upright());
        assert!(Turn::of_exif(42).upright());
        assert_eq!(Turn::of_exif(3).size((4, 2)), (4, 2));
        assert!(Turn::of_exif(5).mirror && Turn::of_exif(7).mirror);
    }

    /// A 2×1 picture: pixel 1 on the left, 2 on the right.
    fn pair() -> Image {
        Image {
            width: 2,
            height: 1,
            pixels: vec![1, 0, 0, 255, 2, 0, 0, 255],
        }
    }

    fn firsts(image: &Image) -> Vec<u8> {
        image.pixels.chunks(4).map(|p| p[0]).collect()
    }

    #[test]
    fn turning_moves_the_pixels_the_way_the_camera_meant() {
        let quarter = Turn::of_exif(6).apply(pair());
        assert_eq!((quarter.width, quarter.height), (1, 2));
        // A quarter clockwise: the left pixel goes to the top.
        assert_eq!(firsts(&quarter), [1, 2]);
        let back = Turn::of_exif(8).apply(pair());
        assert_eq!(firsts(&back), [2, 1]);
        assert_eq!(firsts(&Turn::of_exif(3).apply(pair())), [2, 1]);
        assert_eq!(firsts(&Turn::of_exif(2).apply(pair())), [2, 1]);
        // Transposed: the left pixel stays on top, as the main diagonal is kept.
        assert_eq!(firsts(&Turn::of_exif(5).apply(pair())), [1, 2]);
        assert_eq!(firsts(&Turn::of_exif(7).apply(pair())), [2, 1]);
        assert!(Turn::of_exif(1).apply(pair()) == pair());
    }
}
