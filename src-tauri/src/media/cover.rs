//! Cover art: decoded once per track, straight into a square of the needed size, and the dominant
//! color is computed to tint the bars and the card.

use windows::{
    Graphics::Imaging::{
        BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat,
        BitmapTransform, ColorManagementMode, ExifOrientationMode,
    },
    Storage::Streams::IRandomAccessStreamReference,
    core::Result,
};

/// Square side in pixels: a 52 px cover at up to 250% scale without losing sharpness.
pub const COVER_SIZE: u32 = 128;
pub const NEUTRAL_TINT: [u8; 3] = [0xD1, 0xD1, 0xD6];

pub struct Cover {
    pub size: u32,
    /// BGRA with alpha premultiplied: what Direct2D expects.
    pub bgra: Vec<u8>,
    pub tint: [u8; 3],
}

impl std::fmt::Debug for Cover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cover")
            .field("size", &self.size)
            .field("tint", &self.tint)
            .finish()
    }
}

pub fn decode(reference: &IRandomAccessStreamReference) -> Result<Cover> {
    let stream = reference.OpenReadAsync()?.join()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (w, h) = (
        decoder.OrientedPixelWidth()?,
        decoder.OrientedPixelHeight()?,
    );
    let short = w.min(h).max(1) as f32;
    let scaled_w = ((w as f32 * COVER_SIZE as f32 / short).round() as u32).max(COVER_SIZE);
    let scaled_h = ((h as f32 * COVER_SIZE as f32 / short).round() as u32).max(COVER_SIZE);

    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(scaled_w)?;
    transform.SetScaledHeight(scaled_h)?;
    transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    transform.SetBounds(BitmapBounds {
        X: (scaled_w - COVER_SIZE) / 2,
        Y: (scaled_h - COVER_SIZE) / 2,
        Width: COVER_SIZE,
        Height: COVER_SIZE,
    })?;
    let pixels = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &transform,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::ColorManageToSRgb,
        )?
        .join()?
        .DetachPixelData()?;

    let bgra = pixels.to_vec();
    let tint = tint(&bgra).unwrap_or(NEUTRAL_TINT);
    Ok(Cover {
        size: COVER_SIZE,
        bgra,
        tint,
    })
}

/// The cover's dominant color: the heaviest hue among saturated pixels. Brought to saturation ≤ 85%
/// and lightness 55–75% to be readable on black. `None` if the cover is almost gray.
pub fn tint(bgra: &[u8]) -> Option<[u8; 3]> {
    const BUCKETS: usize = 12;
    let mut weight = [0f32; BUCKETS];
    let mut sum = [[0f32; 3]; BUCKETS];
    let mut total = 0f32;

    for px in bgra.as_chunks::<4>().0.iter().step_by(2) {
        let a = px[3] as f32 / 255.0;
        if a < 0.5 {
            continue;
        }
        let rgb = [px[2], px[1], px[0]].map(|c| c as f32 / 255.0 / a);
        let (h, s, l) = to_hsl(rgb);
        total += 1.0;
        // Bright saturated pixels weigh more; almost black and almost white weigh almost nothing.
        let w = s * (1.0 - (2.0 * l - 1.0).abs());
        if w <= 0.05 {
            continue;
        }
        let bucket = ((h / 360.0 * BUCKETS as f32) as usize).min(BUCKETS - 1);
        weight[bucket] += w;
        for (acc, c) in sum[bucket].iter_mut().zip(rgb) {
            *acc += c * w;
        }
    }

    let (best, &w) = weight
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    if total == 0.0 || w / total < 0.04 {
        return None;
    }
    let avg = sum[best].map(|c| c / w);
    let (h, s, l) = to_hsl(avg);
    if s < 0.12 {
        return None;
    }
    let rgb = from_hsl(h, s.min(0.85), l.clamp(0.55, 0.75));
    Some(rgb.map(|c| (c * 255.0).round() as u8))
}

fn to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s.min(1.0), l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(pixels: &[[u8; 3]]) -> Vec<u8> {
        pixels
            .iter()
            .cycle()
            .take(64 * 64)
            .flat_map(|&[r, g, b]| [b, g, r, 255])
            .collect()
    }

    #[test]
    fn saturated_cover_gives_its_hue() {
        let [r, g, b] = tint(&image(&[[200, 20, 20]])).unwrap();
        assert!(r > g && r > b);
        let (_, _, l) = to_hsl([r, g, b].map(|c| c as f32 / 255.0));
        assert!((0.54..=0.76).contains(&l));
    }

    #[test]
    fn dark_cover_is_lifted_to_readable_lightness() {
        let rgb = tint(&image(&[[10, 20, 90]])).unwrap();
        let (h, _, l) = to_hsl(rgb.map(|c| c as f32 / 255.0));
        assert!((200.0..260.0).contains(&h));
        assert!(l >= 0.54);
    }

    #[test]
    fn gray_cover_has_no_tint() {
        assert_eq!(
            tint(&image(&[[128, 128, 128], [30, 30, 30], [240, 240, 240]])),
            None
        );
    }

    #[test]
    fn dominant_color_wins_over_small_accent() {
        let mut pixels = vec![[30, 160, 60]; 9];
        pixels.insert(0, [220, 30, 30]);
        let [r, g, _] = tint(&image(&pixels)).unwrap();
        assert!(g > r);
    }

    #[test]
    fn hsl_round_trip() {
        for rgb in [[0.8, 0.2, 0.1], [0.1, 0.5, 0.9], [0.3, 0.9, 0.4]] {
            let (h, s, l) = to_hsl(rgb);
            let back = from_hsl(h, s, l);
            for (a, b) in rgb.iter().zip(back) {
                assert!((a - b).abs() < 0.01);
            }
        }
    }
}
