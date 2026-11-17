//! A smaller video than the screen ("Medium", "Low" quality): Direct2D draws each frame
//! reduced into a texture of the video's size, on the recording's video card. The encoder
//! itself only takes frames of its own size.

use super::fade::{bitmap, context};
use crate::gpu::texture;
use windows::{
    Win32::Graphics::{
        Direct2D::{
            Common::D2D_RECT_F, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE,
            D2D1_BITMAP_OPTIONS_TARGET, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, ID2D1Bitmap1,
            ID2D1DeviceContext,
        },
        Direct3D11::{ID3D11Device, ID3D11Texture2D},
    },
    core::Result,
};

pub struct Scaler {
    dc: ID2D1DeviceContext,
    out: ID3D11Texture2D,
    out_bitmap: ID2D1Bitmap1,
    size: (u32, u32),
    /// The frames come from one or two textures (the live one, the dissolve's): their
    /// bitmaps are made once.
    sources: Vec<(ID3D11Texture2D, ID2D1Bitmap1)>,
}

impl Scaler {
    pub fn new(device: &ID3D11Device, size: (u32, u32)) -> Result<Self> {
        let dc = context(device)?;
        let out = texture(device, size)?;
        Ok(Self {
            out_bitmap: bitmap(
                &dc,
                &out,
                D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            )?,
            dc,
            out,
            size,
            sources: Vec::new(),
        })
    }

    pub fn scale(&mut self, frame: &ID3D11Texture2D) -> Result<&ID3D11Texture2D> {
        let source = match self.sources.iter().position(|(t, _)| t == frame) {
            Some(i) => self.sources[i].1.clone(),
            None => {
                let made = bitmap(&self.dc, frame, D2D1_BITMAP_OPTIONS_NONE)?;
                self.sources.push((frame.clone(), made.clone()));
                made
            }
        };
        let whole = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: self.size.0 as f32,
            bottom: self.size.1 as f32,
        };
        unsafe {
            self.dc.SetTarget(&self.out_bitmap);
            self.dc.BeginDraw();
            self.dc.DrawBitmap(
                &source,
                Some(&whole),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
            let drawn = self.dc.EndDraw(None, None);
            // Don't keep the output bound: the encoder copies it next.
            self.dc.SetTarget(None);
            drawn?;
        }
        Ok(&self.out)
    }
}
