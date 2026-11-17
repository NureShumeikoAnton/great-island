//! The scissors' mouse cursor: the scissors icon, white with a dark outline, drawn by Direct2D into
//! a bitmap and made a cursor. Made once when the editor opens, destroyed with it.

use super::super::super::comp::color;
use super::super::super::glyph::{self, Glyph};
use windows::{
    Win32::{
        Graphics::{
            Direct2D::{
                Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT},
                D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
                D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
                D2D1_RENDER_TARGET_USAGE_NONE, D2D1CreateFactory, ID2D1DeviceContext, ID2D1Factory,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection,
                DIB_RGB_COLORS, DeleteObject,
            },
            Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
                WICBitmapCacheOnLoad,
            },
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::WindowsAndMessaging::{CreateIconIndirect, DestroyCursor, HCURSOR, ICONINFO},
    },
    core::{Interface, Result},
};
use windows_numerics::Matrix3x2;

/// The icon's size on screen, logical pixels.
const SIZE: f32 = 26.0;
const OUTLINE: f32 = 1.3;

pub struct Pointer(pub HCURSOR);

impl Pointer {
    /// The scissors at scale `k`; the hot spot is between the blades' tips, where they cut.
    pub fn scissors(k: f32) -> Result<Self> {
        let side = (SIZE * k).round().max(16.0) as u32;
        let s = side as f32;
        let pad = (OUTLINE * 2.0 * k).ceil();
        let icon = s - 2.0 * pad;
        let mut pixels = vec![0u8; (side * side * 4) as usize];
        unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let bitmap = wic.CreateBitmap(
                side,
                side,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let rt = factory.CreateWicBitmapRenderTarget(
                &bitmap,
                &D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                },
            )?;
            let dc: ID2D1DeviceContext = rt.cast()?;
            dc.BeginDraw();
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let d = OUTLINE * k;
            let ring = [
                (-d, 0.0),
                (d, 0.0),
                (0.0, -d),
                (0.0, d),
                (-d * 0.7, -d * 0.7),
                (d * 0.7, -d * 0.7),
                (-d * 0.7, d * 0.7),
                (d * 0.7, d * 0.7),
            ];
            // Drawing is ended in any case, otherwise the target stays busy.
            let drawn = ring
                .iter()
                .map(|&(dx, dy)| (dx, dy, color("#1C1C1E", 0.9)))
                .chain([(0.0, 0.0, color("#FFFFFF", 1.0))])
                .try_for_each(|(dx, dy, ink)| {
                    let base = Matrix3x2::translation(pad + dx, pad + dy);
                    glyph::draw(&dc, &base, Glyph::Scissors, icon, ink)
                });
            dc.EndDraw(None, None)?;
            drawn?;
            bitmap.CopyPixels(std::ptr::null(), side * 4, &mut pixels)?;

            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: side as i32,
                    // Negative: rows top to bottom, as WIC gives them.
                    biHeight: -(side as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            // A clear mask (rows of whole 16-bit words): the colors' alpha decides.
            let mask_bits = vec![0u8; side.div_ceil(16) as usize * 2 * side as usize];
            let mut bits = std::ptr::null_mut();
            let colors = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
            let mask = CreateBitmap(
                side as i32,
                side as i32,
                1,
                1,
                Some(mask_bits.as_ptr() as *const _),
            );
            // The tips are at the top of the icon's middle (viewBox 20: x 10, y 3).
            let unit = icon / 20.0;
            let cursor = CreateIconIndirect(&ICONINFO {
                fIcon: false.into(),
                xHotspot: (pad + 10.0 * unit).round() as u32,
                yHotspot: (pad + 3.0 * unit).round() as u32,
                hbmMask: mask,
                hbmColor: colors,
            });
            let _ = DeleteObject(colors.into());
            let _ = DeleteObject(mask.into());
            Ok(Self(HCURSOR(cursor?.0)))
        }
    }
}

impl Drop for Pointer {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyCursor(self.0);
        }
    }
}
