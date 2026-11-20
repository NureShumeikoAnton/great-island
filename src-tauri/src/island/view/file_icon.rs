//! A file on the island: its own picture where Windows has one, else the placeholder, a sheet
//! with a folded corner and a band in its kind's color (`crate::files::Kind`) carrying the
//! extension. Any view that shows a file takes both from here.

use super::super::comp::{Comp, color, v2, v3};
use super::super::glyph::Glyph;
use super::{Scene, style};
use crate::{
    files::{Kind, extension},
    media::Icon,
};
use windows::{
    UI::Composition::{ContainerVisual, SpriteVisual},
    core::Result,
};

const SHEET: &str = "#3A3A3F";
const FOLD: &str = "#5A5A60";
/// Smaller than this the extension can't be read: the band stays bare.
const LETTERS_FROM: f32 = 24.0;
const LETTERS: usize = 4;

fn tint(kind: Kind) -> &'static str {
    match kind {
        Kind::Image => "#5E5CE6",
        Kind::Video => "#BF5AF2",
        Kind::Audio => "#FF375F",
        Kind::Pdf => "#FF453A",
        Kind::Document => "#0A84FF",
        Kind::Sheet => "#30D158",
        Kind::Slides => "#FF9F0A",
        Kind::Text => "#8E8E93",
        Kind::Archive => "#E0A800",
        Kind::App | Kind::Other => "#636366",
    }
}

/// The placeholder of the file `name` in a square `side` wide; the sheet is narrower and
/// stands in its middle. Also outside the island (the viewer's card of a file).
pub(in crate::island) fn sheet(
    comp: &Comp,
    scale: f32,
    name: &str,
    side: f32,
) -> Result<ContainerVisual> {
    let holder = comp.container()?;
    holder.SetSize(v2(side, side))?;
    let band = tint(Kind::of(name));
    for (glyph, hex) in [
        (Glyph::Sheet, SHEET),
        (Glyph::SheetBand, band),
        (Glyph::SheetFold, FOLD),
    ] {
        let part = comp.glyph(glyph, side, color(hex, 1.0), scale)?;
        holder.Children()?.InsertAtTop(&part)?;
    }
    if side >= LETTERS_FROM
        && let Some(ext) = extension(name)
    {
        let k = side / 24.0;
        let letters = ext.chars().take(LETTERS).collect::<String>().to_uppercase();
        let label = comp.text(&letters, style(5.4 * k, true, "#FFFFFF"), scale)?;
        // The band's middle in the glyph's viewBox.
        let (x, y) = (
            12.25 * k - label.width / 2.0,
            16.75 * k - label.height / 2.0,
        );
        let snap = |v: f32| (v * scale).round() / scale;
        label.visual.SetOffset(v3(snap(x), snap(y)))?;
        holder.Children()?.InsertAtTop(&label.visual)?;
    }
    Ok(holder)
}

impl Scene {
    pub(super) fn file_icon(&self, name: &str, side: f32) -> Result<ContainerVisual> {
        sheet(&self.comp, self.scale, name, side)
    }

    /// The file's own picture filling `size`: cut to it, not stretched.
    pub(super) fn file_picture(
        &self,
        picture: &Icon,
        size: (f32, f32),
        radius: f32,
    ) -> Result<SpriteVisual> {
        let (pixels, width, height) = cover(picture, size.0 / size.1);
        self.comp
            .image_rounded(&pixels, (width, height), size, radius, self.scale)
    }
}

/// The middle of the picture with the proportions `aspect` (width / height).
fn cover(picture: &Icon, aspect: f32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (picture.width.max(1), picture.height.max(1));
    let (cw, ch) = if w as f32 / h as f32 > aspect {
        (((h as f32 * aspect).round() as u32).clamp(1, w), h)
    } else {
        (w, ((w as f32 / aspect).round() as u32).clamp(1, h))
    };
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let mut pixels = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..y0 + ch {
        let start = ((y * w + x0) * 4) as usize;
        pixels.extend_from_slice(&picture.bgra[start..start + (cw * 4) as usize]);
    }
    (pixels, cw, ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32, height: u32) -> Icon {
        let bgra = (0..width * height)
            .flat_map(|i| [i as u8, 0, 0, 255])
            .collect();
        Icon {
            width,
            height,
            bgra,
        }
    }

    #[test]
    fn a_wide_picture_loses_its_sides_and_a_tall_one_its_top_and_bottom() {
        let (pixels, w, h) = cover(&picture(4, 2), 1.0);
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            pixels.chunks(4).map(|p| p[0]).collect::<Vec<_>>(),
            [1, 2, 5, 6]
        );
        let (pixels, w, h) = cover(&picture(2, 4), 1.0);
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            pixels.chunks(4).map(|p| p[0]).collect::<Vec<_>>(),
            [2, 3, 4, 5]
        );
    }
}
