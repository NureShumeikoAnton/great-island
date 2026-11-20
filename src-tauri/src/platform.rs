//! Windows calls unrelated to drawing: user idle, fullscreen mode, monitors, showing a window
//! without focus, autostart, single instance, hotkeys.

use crate::{
    fault::{Cause, Failed},
    hotkey::{Hotkey, Key},
};
use std::{
    ffi::OsString,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, FILETIME, GetLastError, HINSTANCE,
            HWND, LPARAM, POINT, RECT, SYSTEMTIME, WPARAM,
        },
        Globalization::{
            DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, GetUserPreferredUILanguages,
            MUI_LANGUAGE_NAME, TIME_NOSECONDS,
        },
        Graphics::Gdi::{
            CreateRectRgn, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
            MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MONITORINFOEXW,
            MonitorFromPoint, MonitorFromWindow, SetWindowRgn,
        },
        System::{
            DataExchange::COPYDATASTRUCT,
            Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD,
                RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
            },
            SystemInformation::{GetLocalTime, GetTickCount},
            Threading::CreateMutexW,
            Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
        },
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{
                GetLastInputInfo, HOT_KEY_MODIFIERS, LASTINPUTINFO, MAPVK_VSC_TO_VK, MOD_ALT,
                MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, MapVirtualKeyW, RegisterHotKey,
                UnregisterHotKey,
            },
            Shell::{
                QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
                SHQueryUserNotificationState,
            },
            WindowsAndMessaging::{
                AllowSetForegroundWindow, FindWindowW, GetCursorPos, GetWindowThreadProcessId,
                HWND_BROADCAST, HWND_TOPMOST, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
                PostMessageW, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW, SetWindowPos, ShowWindow, WM_CLOSE,
                WM_COPYDATA, WM_SETTINGCHANGE, WindowFromPoint,
            },
        },
    },
    core::{BOOL, HSTRING, PCWSTR, PWSTR, w},
};

pub type MonitorId = isize;

fn local_system_time(unix_ms: u64) -> SYSTEMTIME {
    // FILETIME counts 100 ns from 1601, Unix time ms from 1970.
    let ticks = unix_ms * 10_000 + 116_444_736_000_000_000;
    let file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        let _ = FileTimeToSystemTime(&file, &mut utc);
        // Without the zone the UTC reading is still a moment, only an hour or two off.
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
    }
    local
}

/// A moment as Explorer writes it: the user's short date and time, in Windows' own format.
pub fn date_time_text(moment: std::time::SystemTime) -> String {
    let unix_ms = moment
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let local = local_system_time(unix_ms);
    let mut date = [0u16; 80];
    let mut time = [0u16; 80];
    let (d, t) = unsafe {
        (
            GetDateFormatEx(
                // The user's own locale (LOCALE_NAME_USER_DEFAULT is null).
                PCWSTR::null(),
                DATE_SHORTDATE,
                Some(&local),
                PCWSTR::null(),
                Some(&mut date),
                PCWSTR::null(),
            ),
            GetTimeFormatEx(
                PCWSTR::null(),
                TIME_NOSECONDS,
                Some(&local),
                PCWSTR::null(),
                Some(&mut time),
            ),
        )
    };
    let text = |buf: &[u16], n: i32| String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize]);
    format!("{} {}", text(&date, d), text(&time, t))
}

/// A Unix moment (ms) on this computer's clock and in its time zone.
pub fn local_time(unix_ms: u64) -> crate::update::LocalTime {
    let local = local_system_time(unix_ms);
    crate::update::LocalTime {
        month: local.wMonth,
        day: local.wDay,
        hour: local.wHour,
        minute: local.wMinute,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallClock {
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

pub fn wall_clock() -> WallClock {
    let t = unsafe { GetLocalTime() };
    WallClock {
        hour: t.wHour,
        minute: t.wMinute,
        second: t.wSecond,
        millis: t.wMilliseconds,
    }
}

/// MONITORINFO.dwFlags: the primary monitor.
const MONITORINFOF_PRIMARY: u32 = 1;

pub struct Display {
    /// Like `\\.\DISPLAY2`: it survives a restart, unlike a handle.
    pub name: String,
    pub primary: bool,
    /// Physical pixels.
    pub size: (i32, i32),
    pub handle: MonitorId,
}

pub fn displays() -> Vec<Display> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        list: LPARAM,
    ) -> BOOL {
        // SAFETY: `list` is the vector passed to EnumDisplayMonitors below, alive for the whole call.
        let list = unsafe { &mut *(list.0 as *mut Vec<Display>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: MONITORINFOEXW starts with a MONITORINFO and cbSize says it is the longer one.
        let known = unsafe { GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()) };
        if known.as_bool() {
            let name = &info.szDevice;
            let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            let screen = info.monitorInfo.rcMonitor;
            list.push(Display {
                name: String::from_utf16_lossy(&name[..end]),
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                size: (screen.right - screen.left, screen.bottom - screen.top),
                handle: monitor.0 as MonitorId,
            });
        }
        BOOL(1)
    }
    let mut list: Vec<Display> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut list as *mut Vec<Display> as isize),
        );
    }
    list
}

/// The monitor with this device name; none if it is not connected.
pub fn monitor_named(name: &str) -> Option<MonitorId> {
    displays()
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| d.handle)
}

/// Windows display languages in the user's order of preference ("ru-RU", "en-US").
pub fn ui_languages() -> Result<Vec<String>, Failed> {
    let mut count = 0u32;
    let mut length = 0u32;
    // SAFETY: the first call only asks for the size; the second writes at most `length`
    // units into a buffer of exactly that size.
    let buffer = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut length)?;
        let mut buffer = vec![0u16; length as usize];
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )?;
        buffer
    };
    // One buffer of names, each ended by a NUL, the whole list by an extra one.
    let tags: Vec<String> = buffer
        .split(|unit| *unit == 0)
        .filter(|name| !name.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    if tags.is_empty() {
        return Err(Failed::because(
            Cause::Missing,
            "no display language listed",
        ));
    }
    Ok(tags)
}

pub fn idle_ms() -> u64 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        if GetLastInputInfo(&mut info).as_bool() {
            GetTickCount().wrapping_sub(info.dwTime) as u64
        } else {
            0
        }
    }
}

pub fn is_busy() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => {
            state == QUNS_BUSY
                || state == QUNS_RUNNING_D3D_FULL_SCREEN
                || state == QUNS_PRESENTATION_MODE
        }
        Err(_) => false,
    }
}

pub fn show_without_focus(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn primary_monitor() -> MonitorId {
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY).0 as MonitorId }
}

pub fn window_monitor(hwnd: HWND) -> MonitorId {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as MonitorId }
}

pub fn cursor_monitor() -> MonitorId {
    let mut point = POINT::default();
    unsafe {
        if GetCursorPos(&mut point).is_err() {
            return primary_monitor();
        }
        MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST).0 as MonitorId
    }
}

pub fn monitor_scale(monitor: MonitorId) -> f32 {
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    unsafe {
        let _ = GetDpiForMonitor(
            HMONITOR(monitor as *mut _),
            MDT_EFFECTIVE_DPI,
            &mut dpi_x,
            &mut dpi_y,
        );
    }
    dpi_x as f32 / 96.0
}

/// Whole monitor in physical pixels, taskbar included.
pub fn monitor_rect(monitor: MonitorId) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetMonitorInfoW(HMONITOR(monitor as *mut _), &mut info);
    }
    info.rcMonitor
}

/// Work area (without the taskbar) in physical pixels.
pub fn work_rect(monitor: MonitorId) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetMonitorInfoW(HMONITOR(monitor as *mut _), &mut info);
    }
    info.rcWork
}

/// Places the window top-center in the monitor's work area. Sizes are logical pixels.
/// Returns the monitor the window ended up on.
pub fn place_top_center(
    hwnd: HWND,
    monitor: MonitorId,
    width: f32,
    height: f32,
    top: f32,
) -> MonitorId {
    let mut handle = HMONITOR(monitor as *mut _);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if !GetMonitorInfoW(handle, &mut info).as_bool() {
            // The monitor was unplugged: fall back to the primary.
            handle = HMONITOR(primary_monitor() as *mut _);
            let _ = GetMonitorInfoW(handle, &mut info);
        }
        let scale = monitor_scale(handle.0 as MonitorId);
        let w = (width * scale).round() as i32;
        let h = (height * scale).round() as i32;
        let work = info.rcWork;
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (top * scale).round() as i32;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
    }
    handle.0 as MonitorId
}

/// Leaves only a top-center rectangle clickable (physical pixels): outside it clicks go to the windows below.
pub fn set_click_area(hwnd: HWND, window: RECT, width: i32, height: i32) {
    let full_w = window.right - window.left;
    let full_h = window.bottom - window.top;
    let w = width.min(full_w);
    let h = height.min(full_h);
    let x = (full_w - w) / 2;
    unsafe {
        // After a successful call the system owns the region; don't delete it.
        let region = CreateRectRgn(x, 0, x + w, h);
        SetWindowRgn(hwnd, Some(region), false);
    }
}

/// The cursor is over this window right now; a window region counts in the hit test.
pub fn cursor_over(hwnd: HWND) -> bool {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).is_ok() && WindowFromPoint(point) == hwnd }
}

/// Takes a named mutex. false = such an instance is already running. The mutex lives until the process ends.
pub fn claim_single_instance(name: &str) -> bool {
    unsafe {
        let Ok(handle) = CreateMutexW(None, true, &HSTRING::from(name)) else {
            return false;
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // A handle left open would keep the name taken after its owner is gone.
            let _ = CloseHandle(handle);
            return false;
        }
        true
    }
}

pub fn wait_single_instance(name: &str, patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    loop {
        if claim_single_instance(name) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A second start hands `path` to the running island's window `to` (`WM_COPYDATA`, tagged
/// `tag`). The island's process may then bring its window to the front: Explorer gave that
/// right to this process, which passes it on.
pub fn hand_over_path(to: HWND, tag: usize, path: &Path) {
    let text: Vec<u16> = path.as_os_str().encode_wide().collect();
    let data = COPYDATASTRUCT {
        dwData: tag,
        cbData: (text.len() * 2) as u32,
        lpData: text.as_ptr() as *mut _,
    };
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(to, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid);
        SendMessageTimeoutW(
            to,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&raw const data as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}

/// The path of a `WM_COPYDATA` tagged `tag`; it lives only while the message is handled.
pub fn handed_path(lparam: LPARAM, tag: usize) -> Option<PathBuf> {
    // SAFETY: WM_COPYDATA's lparam points at the sender's struct for the length of the call.
    let data = unsafe { (lparam.0 as *const COPYDATASTRUCT).as_ref()? };
    if data.dwData != tag || data.lpData.is_null() {
        return None;
    }
    let text =
        unsafe { std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2) };
    Some(PathBuf::from(OsString::from_wide(text)))
}

/// Asks a top-level window to close as if its X was pressed; nothing when there is none.
pub fn close_window_titled(title: &str) {
    unsafe {
        if let Ok(hwnd) = FindWindowW(None, &HSTRING::from(title)) {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn relaunch(args: &[&str]) -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .args(args)
        .spawn()
        .map(drop)
}

/// A plain Windows message: for failures the island can't show itself.
pub fn message_box(text: &str) {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("Great Island"),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// What `ShellExecuteW` returned: above 32 = done, otherwise an error code of its own.
pub fn shell_result(result: HINSTANCE) -> Result<(), Failed> {
    let code = result.0 as usize;
    if code > 32 {
        return Ok(());
    }
    let cause = match code {
        // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, SE_ERR_NOASSOC
        2 | 3 | 31 => Cause::Missing,
        // SE_ERR_ACCESSDENIED
        5 => Cause::NoAccess,
        _ => Cause::Unknown,
    };
    Err(Failed::because(cause, format_args!("ShellExecute: {code}")))
}

pub fn set_autostart(enabled: bool) {
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = w!("Great Island");
    unsafe {
        if enabled {
            let Ok(exe) = std::env::current_exe() else {
                return;
            };
            let value: Vec<u16> = format!("\"{}\"", exe.display())
                .encode_utf16()
                .chain([0])
                .collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key,
                name,
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, name);
        }
    }
}

/// Takes the combination for window `hwnd` (`None` = for the thread). The key is looked up by physical
/// place in the current layout. `false` = another program already holds it.
pub fn register_hotkey(hwnd: Option<HWND>, id: i32, key: &Hotkey) -> bool {
    let vk = match key.key() {
        Some(Key::Place(scan)) => unsafe { MapVirtualKeyW(u32::from(scan), MAPVK_VSC_TO_VK) },
        Some(Key::Virtual(vk)) => u32::from(vk),
        None => 0,
    };
    if vk == 0 {
        return false;
    }
    let mut mods: HOT_KEY_MODIFIERS = MOD_NOREPEAT;
    for (on, flag) in [
        (key.ctrl, MOD_CONTROL),
        (key.alt, MOD_ALT),
        (key.shift, MOD_SHIFT),
        (key.win, MOD_WIN),
    ] {
        if on {
            mods |= flag;
        }
    }
    unsafe { RegisterHotKey(hwnd, id, mods, vk) }.is_ok()
}

/// A key that is not from the settings: by its virtual-key code, with Ctrl or alone.
pub fn register_fixed_hotkey(
    hwnd: HWND,
    id: i32,
    ctrl: bool,
    vk: u16,
) -> windows::core::Result<()> {
    let mods = if ctrl {
        MOD_CONTROL | MOD_NOREPEAT
    } else {
        MOD_NOREPEAT
    };
    unsafe { RegisterHotKey(Some(hwnd), id, mods, u32::from(vk)) }
}

pub fn unregister_hotkey(hwnd: Option<HWND>, id: i32) {
    unsafe {
        let _ = UnregisterHotKey(hwnd, id);
    }
}

pub fn hotkey_free(key: &Hotkey) -> bool {
    const PROBE: i32 = 0xBFFF;
    let free = register_hotkey(None, PROBE, key);
    if free {
        unregister_hotkey(None, PROBE);
    }
    free
}

const KEYBOARD: PCWSTR = w!(r"Control Panel\Keyboard");
const SNIPPING: PCWSTR = w!("PrintScreenKeyForSnippingEnabled");

/// PrtSc opens Windows Snipping Tool (setting "Use the Print screen button to open screen capture").
/// While it is on, both Snipping Tool and the island would react. With no registry entry the default
/// applies: on in Windows 11, off in Windows 10.
pub fn snipping_owns_print_screen() -> bool {
    match read_dword(HKEY_CURRENT_USER, KEYBOARD, SNIPPING) {
        Some(value) => value != 0,
        None => windows_build() >= 22000,
    }
}

/// Gives PrtSc back to programs by switching that Windows setting off. The user can restore it in
/// Windows settings: Accessibility, Keyboard.
pub fn release_print_screen() -> bool {
    let off = 0u32;
    let written = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            KEYBOARD,
            SNIPPING,
            REG_DWORD.0,
            Some((&raw const off).cast()),
            size_of::<u32>() as u32,
        )
    } == ERROR_SUCCESS;
    if written {
        // Windows (and the island) reread the keyboard settings.
        let area: Vec<u16> = r"Control Panel\Keyboard"
            .encode_utf16()
            .chain([0])
            .collect();
        unsafe {
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(area.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                1000,
                None,
            );
        }
    }
    written
}

/// The language code the installer left in the registry ("ru" or "en").
pub fn installer_language() -> Option<String> {
    let mut text = [0u16; 8];
    let mut size = size_of_val(&text) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Great Island"),
            w!("Language"),
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (read == ERROR_SUCCESS).then(|| {
        String::from_utf16_lossy(&text)
            .trim_end_matches('\0')
            .to_string()
    })
}

fn read_dword(root: HKEY, key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            root,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    (read == ERROR_SUCCESS).then_some(value)
}

/// Windows build number: 22000 and above = Windows 11.
pub fn windows_build() -> u32 {
    let mut text = [0u16; 16];
    let mut size = size_of_val(&text) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if read != ERROR_SUCCESS {
        return 0;
    }
    String::from_utf16_lossy(&text)
        .trim_end_matches('\0')
        .parse()
        .unwrap_or(0)
}
