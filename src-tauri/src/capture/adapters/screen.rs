//! The screen: where a source lies on it, the local time of a capture, hiding a window from captures.

use super::super::{Source, Stamp};
use crate::{
    lang::tr,
    platform::{self, MonitorId},
};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Dwm::DwmFlush,
    System::SystemInformation::GetLocalTime,
    UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE},
};

pub fn whole_monitor(monitor: MonitorId) -> Source {
    let rect = platform::monitor_rect(monitor);
    Source::whole(
        monitor,
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
    )
}

/// The source in physical pixels of the whole desktop, where GDI and window positions live.
pub fn screen_rect(source: Source) -> RECT {
    let monitor = platform::monitor_rect(source.monitor);
    let left = monitor.left + source.area.x as i32;
    let top = monitor.top + source.area.y as i32;
    RECT {
        left,
        top,
        right: left + source.area.width as i32,
        bottom: top + source.area.height as i32,
    }
}

pub fn now() -> Stamp {
    let t = unsafe { GetLocalTime() };
    Stamp {
        year: t.wYear,
        month: t.wMonth,
        day: t.wDay,
        hour: t.wHour,
        minute: t.wMinute,
        second: t.wSecond,
    }
}

/// Hiding takes effect with the next frame the desktop composes: this waits for it.
pub fn wait_for_hiding() {
    unsafe {
        let _ = DwmFlush();
    }
}

/// The window disappears from screenshots and recordings but stays visible on the monitor (Windows 10 2004+).
pub fn hide_from_capture(hwnd: HWND, hidden: bool) {
    let affinity = if hidden {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe {
        if let Err(e) = SetWindowDisplayAffinity(hwnd, affinity) {
            crate::journal::warn(tr!("journal.hide_from_captures_failed", error = e));
        }
    }
}
