//! A player app by its AppUserModelID: its processes (to listen to its audio) and how to bring its window
//! to the front (click on the cover).
//!
//! A media session reports only the AppUserModelID, which varies: "Spotify.exe", "Chrome", "MSEdge",
//! a hash for Firefox, "Package!App" for Store apps. So it is matched in several ways.

use std::path::Path;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        Storage::EnhancedStorage::PKEY_AppUserModel_ID,
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, CoCreateInstance, CoTaskMemFree,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
        UI::{
            Shell::{
                AO_NONE, ApplicationActivationManager, IApplicationActivationManager,
                PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
            },
            WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
                IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
            },
        },
    },
    core::{BOOL, HSTRING, PWSTR},
};

#[derive(Debug, Default, PartialEq)]
pub struct AppMatch {
    /// Lowercase exe names: "spotify.exe".
    pub exes: Vec<String>,
    /// Store apps' package family: appears in the audio session identifier.
    pub package: Option<String>,
}

impl AppMatch {
    pub fn matches_exe(&self, exe: &str) -> bool {
        self.exes.iter().any(|e| e.eq_ignore_ascii_case(exe))
    }
}

pub fn resolve(app_id: &str) -> AppMatch {
    let mut found = guess(app_id);
    // Windows carry their own AppUserModelID: this is how e.g. Firefox, with a hash instead of a name, is found.
    for window in top_windows() {
        if window.app_id.as_deref() == Some(app_id)
            && let Some(exe) = &window.exe
            && !found.matches_exe(exe)
        {
            found.exes.push(exe.clone());
        }
    }
    found
}

fn guess(app_id: &str) -> AppMatch {
    if let Some((family, _app)) = app_id.split_once('!') {
        return AppMatch {
            exes: Vec::new(),
            package: Some(family.to_string()),
        };
    }
    let lower = app_id.to_lowercase();
    let exe = if lower.ends_with(".exe") {
        file_name(&lower)
    } else {
        // "Chrome" → chrome.exe, "MSEdge" → msedge.exe.
        format!("{lower}.exe")
    };
    AppMatch {
        exes: vec![exe],
        package: None,
    }
}

/// Brings the player window to the front. A Store app with no window is launched.
pub fn open_app(app_id: &str) -> bool {
    let app = resolve(app_id);
    let windows = top_windows();
    // Windows come top to bottom: the first match is the one used last.
    let target = windows
        .iter()
        .find(|w| w.app_id.as_deref() == Some(app_id))
        .or_else(|| {
            windows
                .iter()
                .find(|w| w.exe.as_deref().is_some_and(|e| app.matches_exe(e)))
        });
    if let Some(window) = target {
        unsafe {
            if IsIconic(window.hwnd).as_bool() {
                let _ = ShowWindow(window.hwnd, SW_RESTORE);
            }
            return SetForegroundWindow(window.hwnd).as_bool();
        }
    }
    if app.package.is_some() {
        return activate_package(app_id);
    }
    false
}

fn activate_package(app_id: &str) -> bool {
    unsafe {
        let manager: windows::core::Result<IApplicationActivationManager> =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER);
        manager
            .and_then(|m| m.ActivateApplication(&HSTRING::from(app_id), &HSTRING::new(), AO_NONE))
            .is_ok()
    }
}

struct TopWindow {
    hwnd: HWND,
    pid: u32,
    exe: Option<String>,
    app_id: Option<String>,
}

fn top_windows() -> Vec<TopWindow> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        let visible = unsafe {
            IsWindowVisible(hwnd).as_bool()
                && GetWindowTextLengthW(hwnd) > 0
                && GetWindow(hwnd, GW_OWNER).is_err()
        };
        if visible {
            list.push(hwnd);
        }
        true.into()
    }

    let mut handles: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    handles
        .into_iter()
        .map(|hwnd| {
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            TopWindow {
                hwnd,
                pid,
                exe: process_exe(pid),
                app_id: window_app_id(hwnd),
            }
        })
        .collect()
}

fn window_app_id(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        let mut value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let text = PropVariantToStringAlloc(&value).ok();
        let _ = PropVariantClear(&mut value);
        let text = text?;
        let result = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as _));
        result.filter(|s| !s.is_empty())
    }
}

/// Full path to the player window's exe: the Windows shell returns the app icon from it.
pub fn exe_path(app_id: &str) -> Option<String> {
    let app = resolve(app_id);
    top_windows()
        .into_iter()
        .find(|w| {
            w.app_id.as_deref() == Some(app_id)
                || w.exe.as_deref().is_some_and(|e| app.matches_exe(e))
        })
        .and_then(|w| process_path(w.pid))
}

pub fn process_exe(pid: u32) -> Option<String> {
    process_path(pid).map(|path| file_name(&path.to_lowercase()))
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_exe_from_app_id() {
        assert!(guess("Spotify.exe").matches_exe("spotify.exe"));
        assert!(guess("Chrome").matches_exe("chrome.exe"));
        assert!(guess("MSEdge").matches_exe("msedge.exe"));
        assert!(guess(r"C:\Program Files\VideoLAN\VLC\vlc.exe").matches_exe("vlc.exe"));
    }

    #[test]
    fn store_apps_are_matched_by_package() {
        let app = guess("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify");
        assert_eq!(
            app.package.as_deref(),
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0")
        );
    }
}
