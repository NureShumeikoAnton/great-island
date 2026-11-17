//! Screen capture: GDI, and Windows.Graphics.Capture on a monitor in HDR mode.

use crate::{
    capture::{
        Image, Source, Stamp,
        adapters::{graphics, now, screen_rect},
    },
    gpu, journal,
    lang::tr,
};
use std::{sync::mpsc, time::Duration};
use windows::{
    Win32::{
        Foundation::{E_FAIL, ERROR_TIMEOUT, RECT},
        Graphics::{
            Direct3D11::{D3D11_BOX, ID3D11Device},
            Dxgi::{
                Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, CreateDXGIFactory1,
                IDXGIFactory1, IDXGIOutput6,
            },
            Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
                CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC,
                SRCCOPY, SelectObject,
            },
        },
    },
    core::{Error, Interface, Result},
};

/// A frame that has not come in this long means the capture doesn't work here.
const FRAME_WAIT: Duration = Duration::from_secs(1);

/// What is on screen in the `source` and when it was taken. The cursor isn't in the frame, like Windows' own
/// shots. GDI sees an HDR monitor only in the standard range and the colors come out washed out, so
/// a monitor in HDR mode goes through Graphics Capture. Everything else stays on GDI: Graphics Capture
/// is slower to start and draws a yellow frame around the monitor before Windows 11.
pub fn capture(source: Source) -> Option<(Image, Stamp)> {
    if is_hdr(source.monitor) {
        match grab(source) {
            Ok(image) => return Some((image, now())),
            Err(e) => journal::warn(tr!("journal.shot_hdr_failed", error = e)),
        }
    }
    gdi(screen_rect(source))
}

/// Whether the monitor shows HDR now (the HDR switch in Windows is on for it).
fn is_hdr(monitor: isize) -> bool {
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return false;
        };
        let mut adapters = 0;
        while let Ok(adapter) = factory.EnumAdapters1(adapters) {
            let mut outputs = 0;
            while let Ok(output) = adapter.EnumOutputs(outputs) {
                if let Ok(output) = output.cast::<IDXGIOutput6>()
                    && let Ok(desc) = output.GetDesc1()
                    && desc.Monitor.0 as isize == monitor
                {
                    return desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
                }
                outputs += 1;
            }
            adapters += 1;
        }
    }
    false
}

fn grab(source: Source) -> Result<Image> {
    let device = gpu::video_device()?;
    let item = graphics::item(source.monitor)?;
    let size = item.Size()?;
    let (sent, arrived) = mpsc::channel();
    let (pool, session) = graphics::open(
        &device,
        &item,
        size,
        move || {
            let _ = sent.send(());
        },
        || {},
    )?;
    let image = (|| {
        session.SetIsCursorCaptureEnabled(false)?;
        graphics::borderless(&session);
        session.StartCapture()?;
        arrived
            .recv_timeout(FRAME_WAIT)
            .map_err(|_| Error::from(ERROR_TIMEOUT.to_hresult()))?;
        let frame = pool.TryGetNextFrame()?;
        let (width, height) = (size.Width as u32, size.Height as u32);
        let area = source.area;
        // The resolution changed after the area was chosen.
        if frame.ContentSize()? != size || area.right() > width || area.bottom() > height {
            return Err(Error::from(E_FAIL));
        }
        crop(&device, &graphics::texture(&frame)?, source)
    })();
    // The session and the pool are closed in any case, otherwise the monitor keeps being captured.
    let _ = session.Close();
    let _ = pool.Close();
    image
}

fn crop(
    device: &ID3D11Device,
    frame: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    source: Source,
) -> Result<Image> {
    let area = source.area;
    let cropped = gpu::texture(device, (area.width, area.height))?;
    let region = D3D11_BOX {
        left: area.x,
        top: area.y,
        front: 0,
        right: area.right(),
        bottom: area.bottom(),
        back: 1,
    };
    unsafe {
        let context = device.GetImmediateContext()?;
        context.CopySubresourceRegion(&cropped, 0, 0, 0, 0, frame, 0, Some(&region));
    }
    Ok(Image {
        width: area.width,
        height: area.height,
        pixels: gpu::read_back(device, &cropped, (area.width, area.height))?,
    })
}

/// The `area` rectangle in physical pixels of the desktop.
fn gdi(area: RECT) -> Option<(Image, Stamp)> {
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
