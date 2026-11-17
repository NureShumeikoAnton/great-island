//! The windows on a monitor, top first, as the selection sees them.

use crate::{capture::Area, fault::Failed};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongW, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TRANSPARENT,
        },
    },
    core::BOOL,
};

/// The desktop itself is a window too; a click on it must take nothing.
const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];

/// Visible windows of other programs on `monitor` (physical pixels), clipped to it, in its
/// own pixels. EnumWindows goes from the top of the Z order down.
pub fn windows_on(monitor: RECT) -> Result<Vec<Area>, Failed> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the vector below, alive for the whole EnumWindows call.
        let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        list.push(hwnd);
        true.into()
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize))? };
    let own = unsafe { GetCurrentProcessId() };
    Ok(all
        .into_iter()
        .filter(|&hwnd| pickable(hwnd, own))
        .filter_map(|hwnd| clip(bounds(hwnd)?, monitor))
        .collect())
}

fn pickable(hwnd: HWND, own: u32) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == own {
            return false;
        }
        // Click-through overlays (a game bar, a video card's) lie over everything unseen.
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0 {
            return false;
        }
        // Store apps and other desktops' windows are "visible" but cloaked.
        let mut cloaked = 0u32;
        let read = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        );
        if read.is_ok() && cloaked != 0 {
            return false;
        }
        let mut class = [0u16; 32];
        let len = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        !DESKTOP_CLASSES.contains(&class.as_str())
    }
}

/// What is seen of the window: Windows 10 and 11 add invisible resize borders around it,
/// which GetWindowRect counts and the frame bounds don't.
fn bounds(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe {
        let frame = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            size_of::<RECT>() as u32,
        );
        if frame.is_err() {
            GetWindowRect(hwnd, &mut rect).ok()?;
        }
    }
    Some(rect)
}

fn clip(rect: RECT, monitor: RECT) -> Option<Area> {
    let left = rect.left.max(monitor.left);
    let top = rect.top.max(monitor.top);
    let right = rect.right.min(monitor.right);
    let bottom = rect.bottom.min(monitor.bottom);
    (right > left && bottom > top).then(|| Area {
        x: (left - monitor.left) as u32,
        y: (top - monitor.top) as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}
