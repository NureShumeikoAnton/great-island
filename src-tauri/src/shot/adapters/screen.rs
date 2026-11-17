//! Screen capture via GDI.

use crate::capture::{Image, Stamp, adapters::now};
use windows::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
        CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY,
        SelectObject,
    },
};

/// What is on screen in the `area` rectangle (physical pixels) and when it was taken. The cursor isn't in the
/// frame, like Windows' own shots. On an HDR monitor colors come out washed out: GDI sees the screen
/// only in the standard range.
pub fn capture(area: RECT) -> Option<(Image, Stamp)> {
    let (w, h) = (area.right - area.left, area.bottom - area.top);
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative height = rows top to bottom.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let pixels = match CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            Ok(bitmap) => {
                let old = SelectObject(memory, bitmap.into());
                // CAPTUREBLT: includes semi-transparent windows: tooltips, menus.
                let copied = BitBlt(
                    memory,
                    0,
                    0,
                    w,
                    h,
                    Some(screen),
                    area.left,
                    area.top,
                    SRCCOPY | CAPTUREBLT,
                )
                .is_ok();
                let pixels = (copied && !bits.is_null()).then(|| {
                    let mut pixels =
                        std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize)
                            .to_vec();
                    // GDI leaves alpha empty: the shot is opaque.
                    for alpha in pixels.iter_mut().skip(3).step_by(4) {
                        *alpha = 255;
                    }
                    pixels
                });
                SelectObject(memory, old);
                let _ = DeleteObject(bitmap.into());
                pixels
            }
            Err(_) => None,
        };
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        let image = Image {
            width: w as u32,
            height: h as u32,
            pixels: pixels?,
        };
        Some((image, now()))
    }
}
