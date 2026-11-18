//! Window creation: the transparent island window, a hidden window for the tray and service messages,
//! the screenshot flight window with the click "catcher", the viewer window and the area selection.

use windows::{
    Win32::{
        Foundation::{COLORREF, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Dwm::{
            DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_COLOR_NONE,
            DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
            DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::MARGINS,
            WindowsAndMessaging::{
                CS_DBLCLKS, CreateWindowExW, DefWindowProcW, HTTRANSPARENT, IDC_ARROW, IDC_CROSS,
                IDC_HAND, LWA_ALPHA, LoadCursorW, LoadIconW, RegisterClassW, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetLayeredWindowAttributes,
                SetPropW, SetWindowPos, WM_APP, WM_NCHITTEST, WNDCLASS_STYLES, WNDCLASSW,
                WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
                WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
            },
        },
    },
    core::{BOOL, HSTRING, PCWSTR, Result, w},
};

pub const ISLAND_CLASS: PCWSTR = w!("GreatIslandWindow");
/// A second launch finds the running island by this class.
pub const SERVICE_CLASS: PCWSTR = w!("GreatIslandService");
const OVERLAY_CLASS: PCWSTR = w!("GreatIslandOverlay");
const VIEWER_CLASS: PCWSTR = w!("GreatIslandViewer");
const CATCHER_CLASS: PCWSTR = w!("GreatIslandCatcher");
const SELECT_CLASS: PCWSTR = w!("GreatIslandSelect");
/// App icon in the exe resources (embedded by the Tauri build).
const APP_ICON: PCWSTR = PCWSTR(32512 as *const u16);

pub const WM_OPEN_SETTINGS: u32 = WM_APP + 2;
pub const WM_MEDIA: u32 = WM_APP + 3;
pub const WM_CONTROLS: u32 = WM_APP + 4;
pub const WM_RECORD: u32 = WM_APP + 5;
pub const WM_FAULT: u32 = WM_APP + 6;
pub const WM_DEMO: u32 = WM_APP + 7;
/// Media Engine's note for a video window (`play::adapters::Note` packed into the parameters).
pub const WM_VIDEO: u32 = WM_APP + 8;
/// The editor's two engines (`WM_DECK + deck`), the same note as `WM_VIDEO`.
pub const WM_DECK: u32 = WM_APP + 9;
/// The timeline's thumbnails or wave arrived from their threads.
pub const WM_STRIP: u32 = WM_APP + 11;
/// The edited video's save moved on (its thread, to the island).
pub const WM_SAVE: u32 = WM_APP + 12;
/// The update check on its thread has an answer. (WM_APP + 13 is `update::LEAVE_MESSAGE`, sent by the update window.)
pub const WM_UPDATE: u32 = WM_APP + 14;
/// The Downloads folder's watch has news (its thread, to the island).
pub const WM_DOWNLOAD: u32 = WM_APP + 15;

pub type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// `viewer_proc` handles viewer windows, `select_proc` the area selection: their classes are
/// registered here, while the windows are created when they are needed.
pub fn create(
    wndproc: WndProc,
    viewer_proc: WndProc,
    select_proc: WndProc,
) -> Result<(HWND, HWND)> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        let hand = LoadCursorW(None, IDC_HAND)?;
        let cross = LoadCursorW(None, IDC_CROSS)?;
        let icon = LoadIconW(Some(instance.into()), APP_ICON).unwrap_or_default();
        let none = WNDCLASS_STYLES(0);
        let classes = [
            (ISLAND_CLASS, wndproc, none, arrow),
            (SERVICE_CLASS, wndproc, none, arrow),
            (OVERLAY_CLASS, overlay_proc as WndProc, none, arrow),
            // A click on the shot at the center reaches the island as its own.
            (CATCHER_CLASS, wndproc, none, hand),
            (VIEWER_CLASS, viewer_proc, CS_DBLCLKS, arrow),
            (SELECT_CLASS, select_proc, none, cross),
        ];
        for (class, proc, style, cursor) in classes {
            let wc = WNDCLASSW {
                style,
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: class,
                hCursor: cursor,
                hIcon: icon,
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        // The island: no buffer of its own (Composition draws), always on top, not in Alt+Tab,
        // doesn't take focus on click.
        let island = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            ISLAND_CLASS,
            w!("Great Island"),
            WS_POPUP,
            0,
            0,
            100,
            30,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // The service window is never shown: it receives tray and second-launch messages.
        let service = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            SERVICE_CLASS,
            w!("Great Island"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        Ok((island, service))
    }
}

/// Full-monitor flight window over `rect` (physical pixels): topmost and click-through, clicks go to
/// the windows below. The caller shows it.
pub fn create_overlay(rect: RECT) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_LAYERED
                | WS_EX_TRANSPARENT,
            OVERLAY_CLASS,
            w!("Great Island"),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        // A layered window isn't shown without this call.
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        not_fullscreen(hwnd);
        Ok(hwnd)
    }
}

/// An invisible window over the shot at the center of the screen (`rect` in physical pixels): it catches
/// the click on the shot and doesn't take focus. Almost fully transparent, but not to zero:
/// Windows lets clicks pass through a window with zero opacity.
pub fn create_catcher(rect: RECT) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            CATCHER_CLASS,
            w!("Great Island"),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA)?;
        Ok(hwnd)
    }
}

/// The area selection over `rect` (physical pixels), initially hidden. Unlike the island it takes
/// the focus: Esc and Alt must reach it, not the window under it (Alt would open that one's menu).
pub fn create_selector(rect: RECT) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            SELECT_CLASS,
            w!("Great Island"),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        not_fullscreen(hwnd);
        Ok(hwnd)
    }
}

/// Windows takes a topmost window over the whole monitor for a fullscreen app, and the island
/// steps aside for those (`platform::is_busy`). This shell property says the window is not one.
fn not_fullscreen(hwnd: HWND) {
    unsafe {
        let _ = SetPropW(
            hwnd,
            w!("NonRudeHWND"),
            Some(HANDLE(std::ptr::without_provenance_mut(1))),
        );
    }
}

/// Windows 11 rounds a viewer's corners; a full-screen video has square ones.
/// Rounded corners and the 1-pixel border Windows 11 draws around a framed window; a viewer in
/// full screen has neither, or the border shows as lines along the screen's edges.
pub fn set_framed(hwnd: HWND, framed: bool) {
    let (corners, border) = if framed {
        (DWMWCP_ROUND, DWMWA_COLOR_DEFAULT)
    } else {
        (DWMWCP_DONOTROUND, DWMWA_COLOR_NONE)
    };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corners).cast(),
            size_of_val(&corners) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const border).cast(),
            size_of_val(&border) as u32,
        );
    }
}

/// Viewer window (a screenshot or a video) at `rect` (physical pixels), initially hidden.
///
/// An ordinary app window (taskbar button, Alt+Tab) but with no title or frame: the shot fills the whole
/// window. Windows itself draws the shadow and rounded corners, so the frame styles stay and the frame's
/// place is given to the shot (`WM_NCCALCSIZE` in the window handler). The appear animation is off:
/// the window opens by the shot's flight.
pub fn create_viewer(rect: RECT, title: &str) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
            VIEWER_CLASS,
            &HSTRING::from(title),
            WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let on = BOOL::from(true);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const on).cast(),
            size_of::<BOOL>() as u32,
        );
        set_framed(hwnd, true);
        set_transitions(hwnd, false);
        // A frameless window gets a shadow if the frame reaches into the window by at least a pixel;
        // the shot is opaque and covers it.
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        Ok(hwnd)
    }
}

pub fn set_transitions(hwnd: HWND, enabled: bool) {
    let disabled = BOOL::from(!enabled);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            (&raw const disabled).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
