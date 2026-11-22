//! QR codes on a picture: where each one is and what it says.

use crate::{capture::Image, text::Rect};

#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub rect: Rect,
    pub content: String,
}

/// Every QR code that decodes. A code that is found but doesn't decode (cut by the screen's
/// edge, too small) is left out: there is nothing to copy from it.
pub fn find(image: &Image) -> Vec<Code> {
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| {
        let p = (y * w + x) * 4;
        let px = &image.pixels[p..p + 3];
        // Rec. 601 luma from BGR.
        ((px[0] as u32 * 29 + px[1] as u32 * 150 + px[2] as u32 * 77) >> 8) as u8
    });
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|grid| {
            let (_, content) = grid.decode().ok()?;
            let xs = grid.bounds.map(|p| p.x as f32);
            let ys = grid.bounds.map(|p| p.y as f32);
            let x = xs.iter().copied().fold(f32::MAX, f32::min);
            let y = ys.iter().copied().fold(f32::MAX, f32::min);
            // rqrr maps the far corners at `size + 1` modules, one past the code's edge.
            let modules = rqrr::BitGrid::size(&grid.grid) as f32;
            let fit = modules / (modules + 1.0);
            Some(Code {
                rect: Rect {
                    x,
                    y,
                    width: (xs.iter().copied().fold(f32::MIN, f32::max) - x) * fit,
                    height: (ys.iter().copied().fold(f32::MIN, f32::max) - y) * fit,
                },
                content,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code drawn black on white with a quiet zone, `scale` pixels per module, at (`at`, `at`)
    /// on a larger white picture.
    fn picture(content: &str, scale: u32, at: u32) -> Image {
        let code = qrcode::QrCode::new(content).unwrap();
        let modules = code.width() as u32;
        let colors = code.to_colors();
        let side = at * 2 + modules * scale;
        let mut pixels = vec![255u8; (side * side * 4) as usize];
        for (i, color) in colors.iter().enumerate() {
            if *color != qrcode::Color::Dark {
                continue;
            }
            let (mx, my) = (i as u32 % modules, i as u32 / modules);
            for y in 0..scale {
                for x in 0..scale {
                    let p = (((at + my * scale + y) * side + at + mx * scale + x) * 4) as usize;
                    pixels[p..p + 3].fill(0);
                }
            }
        }
        Image {
            width: side,
            height: side,
            pixels,
        }
    }

    #[test]
    fn a_code_is_found_with_its_place_and_content() {
        let image = picture("https://example.com/a?b=1", 4, 60);
        let side = qrcode::QrCode::new("https://example.com/a?b=1")
            .unwrap()
            .width() as f32
            * 4.0;
        let codes = find(&image);
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].content, "https://example.com/a?b=1");
        let r = codes[0].rect;
        let near = |a: f32, b: f32| (a - b).abs() <= 2.0;
        assert!(
            near(r.x, 60.0)
                && near(r.y, 60.0)
                && near(r.right(), 60.0 + side)
                && near(r.bottom(), 60.0 + side),
            "the box fits the code on every side: {r:?}"
        );
    }

    #[test]
    fn a_picture_without_codes_has_none() {
        let image = Image {
            width: 64,
            height: 64,
            pixels: vec![255; 64 * 64 * 4],
        };
        assert!(find(&image).is_empty());
    }
}
