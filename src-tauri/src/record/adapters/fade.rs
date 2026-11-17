//! The dissolve after a pause, drawn by Direct2D on the recording's video card:
//! the live picture with the one from before the pause over it, fading out.
//! Works for a few frames per pause; the rest of the time it does nothing.

use windows::{
    Win32::Graphics::{
        Direct2D::{
            Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT},
            D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE,
            D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
            D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            D2D1_UNIT_MODE_PIXELS, D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext,
            ID2D1Factory1,
        },
        Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D},
        Dxgi::{Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGIDevice, IDXGISurface},
    },
    core::{Interface, Result},
};

pub struct Mixer {
    dc: ID2D1DeviceContext,
    held: ID3D11Texture2D,
    held_bitmap: ID2D1Bitmap1,
    live: ID2D1Bitmap1,
    out: ID3D11Texture2D,
    out_bitmap: ID2D1Bitmap1,
}

impl Mixer {
    pub fn new(device: &ID3D11Device, live: &ID3D11Texture2D, size: (u32, u32)) -> Result<Self> {
        let dc = context(device)?;
        let held = crate::gpu::texture(device, size)?;
        let out = crate::gpu::texture(device, size)?;
        Ok(Self {
            held_bitmap: bitmap(&dc, &held, D2D1_BITMAP_OPTIONS_NONE)?,
            live: bitmap(&dc, live, D2D1_BITMAP_OPTIONS_NONE)?,
            out_bitmap: bitmap(
                &dc,
                &out,
                D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            )?,
            dc,
            held,
            out,
        })
    }

    pub fn hold(&self, context: &ID3D11DeviceContext, live: &ID3D11Texture2D) {
        unsafe { context.CopyResource(&self.held, live) };
    }

    pub fn mix(&self, old: f32) -> Result<&ID3D11Texture2D> {
        unsafe {
            self.dc.SetTarget(&self.out_bitmap);
            self.dc.BeginDraw();
            for (bitmap, opacity) in [(&self.live, 1.0), (&self.held_bitmap, old)] {
                self.dc.DrawBitmap(
                    bitmap,
                    None,
                    opacity,
                    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                    None,
                );
            }
            self.dc.EndDraw(None, None)?;
            // Don't keep the output bound: the encoder copies it next.
            self.dc.SetTarget(None);
        }
        Ok(&self.out)
    }
}

/// Direct2D on the recording's device, in pixels.
pub(super) fn context(device: &ID3D11Device) -> Result<ID2D1DeviceContext> {
    // Multithreaded: Media Foundation uses the same device from its own threads.
    let factory: ID2D1Factory1 =
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)? };
    let dc = unsafe {
        factory
            .CreateDevice(&device.cast::<IDXGIDevice>()?)?
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?
    };
    unsafe { dc.SetUnitMode(D2D1_UNIT_MODE_PIXELS) };
    Ok(dc)
}

pub(super) fn bitmap(
    dc: &ID2D1DeviceContext,
    texture: &ID3D11Texture2D,
    options: D2D1_BITMAP_OPTIONS,
) -> Result<ID2D1Bitmap1> {
    unsafe {
        dc.CreateBitmapFromDxgiSurface(
            &texture.cast::<IDXGISurface>()?,
            Some(&D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: options,
                ..Default::default()
            }),
        )
    }
}
