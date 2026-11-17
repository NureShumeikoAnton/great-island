//! Tray icon and its menu on plain Win32.

use crate::lang::tr;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, HICON,
                IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LoadIconW, LoadImageW, MF_CHECKED,
                MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, SM_CXSMICON, SM_CYSMICON,
                SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                TrackPopupMenu, WM_NULL,
            },
        },
    },
    core::{HSTRING, PCWSTR},
};

pub const WM_TRAY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Command {
    TogglePause,
    ToggleIsland,
    Settings,
    Restart,
    Quit,
    /// Hide the island for this many minutes; `None` = until the user brings it back.
    Hide(Option<u32>),
    Unhide,
}

/// Minutes to hide the island for, in menu order.
const HIDE_CHOICES: [Option<u32>; 4] = [Some(15), Some(60), Some(180), None];

fn hide_label(minutes: Option<u32>) -> String {
    match minutes {
        Some(15) => tr!("tray.hide_15"),
        Some(60) => tr!("tray.hide_60"),
        Some(180) => tr!("tray.hide_180"),
        _ => tr!("tray.hide_until"),
    }
    .into_owned()
}

const COMMANDS: [(u32, Command); 10] = [
    (1, Command::TogglePause),
    (2, Command::ToggleIsland),
    (3, Command::Settings),
    (4, Command::Quit),
    (5, Command::Hide(HIDE_CHOICES[0])),
    (6, Command::Hide(HIDE_CHOICES[1])),
    (7, Command::Hide(HIDE_CHOICES[2])),
    (8, Command::Hide(HIDE_CHOICES[3])),
    (9, Command::Unhide),
    (10, Command::Restart),
];

#[derive(Clone, Copy, Debug)]
pub enum HiddenState {
    Shown,
    Hidden(Option<u64>),
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            icon: load_icon(),
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        for (slot, ch) in data.szTip.iter_mut().zip("Great Island".encode_utf16()) {
            *slot = ch;
        }
        data
    }

    /// Adds the icon. A repeated call is needed when Explorer restarts.
    pub fn add(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &self.data());
        }
    }

    pub fn remove(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
        }
    }
}

/// Shows the menu at the cursor and returns the chosen command.
/// The menu is modal, so call it outside app-state handling.
pub fn show_menu(
    hwnd: HWND,
    paused: bool,
    show_island: bool,
    hidden: HiddenState,
) -> Option<Command> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let pause_label = if paused {
            tr!("tray.resume")
        } else {
            tr!("tray.pause")
        };
        let _ = AppendMenuW(menu, MF_STRING, 1, &HSTRING::from(&*pause_label));
        match hidden {
            HiddenState::Shown => {
                // The submenu is destroyed together with the menu.
                if let Ok(submenu) = CreatePopupMenu() {
                    for (id, minutes) in (5..).zip(HIDE_CHOICES) {
                        let label = HSTRING::from(hide_label(minutes));
                        let _ = AppendMenuW(submenu, MF_STRING, id, &label);
                    }
                    let title = HSTRING::from(&*tr!("tray.hide"));
                    let _ = AppendMenuW(menu, MF_POPUP, submenu.0 as usize, &title);
                }
            }
            HiddenState::Hidden(left) => {
                let label = HSTRING::from(unhide_label(left));
                let _ = AppendMenuW(menu, MF_STRING, 9, &label);
            }
        }
        let island_flags = if show_island {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        let always = HSTRING::from(&*tr!("tray.show_always"));
        let _ = AppendMenuW(menu, island_flags, 2, &always);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 3, &HSTRING::from(&*tr!("tray.settings")));
        let _ = AppendMenuW(menu, MF_STRING, 10, &HSTRING::from(&*tr!("tray.restart")));
        let _ = AppendMenuW(menu, MF_STRING, 4, &HSTRING::from(&*tr!("tray.quit")));

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // Without this the menu doesn't close on a click outside it.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        COMMANDS
            .iter()
            .find(|(id, _)| *id as i32 == chosen.0)
            .map(|(_, c)| *c)
    }
}

fn unhide_label(minutes_left: Option<u64>) -> String {
    let left = match minutes_left {
        None => return tr!("tray.unhide").into_owned(),
        Some(m) if m < 60 => tr!("time.minutes", m = m),
        Some(m) if m % 60 == 0 => tr!("time.hours", h = m / 60),
        Some(m) => tr!("time.hours_minutes", h = m / 60, m = m % 60),
    };
    tr!("tray.unhide_left", left = left).into_owned()
}

/// App icon from the exe resources (the Tauri build embeds it under number 32512).
fn load_icon() -> HICON {
    unsafe {
        let module = GetModuleHandleW(None).ok();
        let size = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let embedded = LoadImageW(
            module.map(Into::into),
            PCWSTR(32512 as *const u16),
            IMAGE_ICON,
            size.0,
            size.1,
            LR_DEFAULTCOLOR,
        );
        match embedded {
            Ok(handle) => HICON(handle.0),
            Err(_) => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::unhide_label;
    use crate::lang::{self, Lang};

    #[test]
    fn unhide_label_shows_time_left() {
        lang::with(Lang::Ru, || {
            assert_eq!(unhide_label(None), "Показать остров");
            assert_eq!(unhide_label(Some(42)), "Показать остров (ещё 42 мин)");
            assert_eq!(unhide_label(Some(120)), "Показать остров (ещё 2 ч)");
            assert_eq!(unhide_label(Some(125)), "Показать остров (ещё 2 ч 5 мин)");
        });
        lang::with(Lang::En, || {
            assert_eq!(unhide_label(None), "Show island");
            assert_eq!(unhide_label(Some(42)), "Show island (42 min left)");
            assert_eq!(unhide_label(Some(120)), "Show island (2 h left)");
            assert_eq!(unhide_label(Some(125)), "Show island (2 h 5 min left)");
        });
    }
}
