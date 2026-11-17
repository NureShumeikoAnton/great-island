//! App icon for the expanded player card: the real colored one, like in the Start menu. The Windows
//! shell provides it, both for ordinary programs and for Store apps.

use super::app;
use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
            GetDIBits, GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
        },
        UI::Shell::{
            IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
            SIIGBF_ICONONLY,
        },
    },
    core::{HSTRING, Interface},
};

/// Side in pixels: 16 logical at up to 200% scale.
pub const ICON_SIZE: i32 = 32;

pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Icon({}×{})", self.width, self.height)
    }
}

pub fn load(app_id: &str) -> Option<Icon> {
    let by_id = shell_item(&format!("shell:AppsFolder\\{app_id}"));
    let item = by_id.or_else(|| app::exe_path(app_id).and_then(|path| shell_item(&path)))?;
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let size = SIZE {
        cx: ICON_SIZE,
        cy: ICON_SIZE,
    };
    let bitmap = unsafe { factory.GetImage(size, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK) }.ok()?;
    let icon = read_bitmap(bitmap);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    icon
}

fn shell_item(path: &str) -> Option<IShellItem> {
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None).ok() }
}

fn read_bitmap(bitmap: HBITMAP) -> Option<Icon> {
    unsafe {
        let mut info = BITMAP::default();
        let read = GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        );
        if read == 0 || info.bmWidth <= 0 || info.bmHeight == 0 {
            return None;
        }
        let (width, height) = (info.bmWidth, info.bmHeight.abs());
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height = rows top to bottom.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        if lines == 0 {
            return None;
        }
        premultiply(&mut bgra);
        Some(Icon {
            width: width as u32,
            height: height as u32,
            bgra,
        })
    }
}

/// The shell usually returns premultiplied alpha, but not always: if some color is brighter than its
/// alpha, the alpha is straight. No alpha at all = the picture is opaque.
fn premultiply(bgra: &mut [u8]) {
    let pixels = bgra.as_chunks_mut::<4>().0;
    if pixels.iter().all(|p| p[3] == 0) {
        pixels.iter_mut().for_each(|p| p[3] = 255);
        return;
    }
    let straight = pixels.iter().any(|p| p[0].max(p[1]).max(p[2]) > p[3]);
    if straight {
        for p in pixels {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::premultiply;

    #[test]
    fn opaque_bitmap_without_alpha_gets_full_alpha() {
        let mut px = vec![10, 20, 30, 0, 40, 50, 60, 0];
        premultiply(&mut px);
        assert_eq!(px, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn straight_alpha_is_multiplied_and_premultiplied_is_kept() {
        let mut straight = vec![200, 200, 200, 128];
        premultiply(&mut straight);
        assert_eq!(straight, vec![100, 100, 100, 128]);

        let mut already = vec![100, 100, 100, 128];
        premultiply(&mut already);
        assert_eq!(already, vec![100, 100, 100, 128]);
    }
}
