//! Settings window: the same exe with `--settings`. Runs on Tauri and WebView2, but only while the
//! window is open: when it closes the whole process exits. The island notices a save by the
//! modification time of settings.json.

use crate::{
    fault::{Fault, What},
    hotkey::{Hotkey, Problem},
    journal,
    lang::{self, Lang, tr, tr_for},
    persist, platform, record,
    settings::{Language, Settings},
    shot, sound,
};
use std::path::PathBuf;
use tauri::{WebviewUrl, WebviewWindowBuilder};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree,
        },
        UI::{
            Shell::{
                FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
                SHCreateItemFromParsingName, SIGDN_FILESYSPATH, ShellExecuteW,
            },
            WindowsAndMessaging::{
                FindWindowW, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
            },
        },
    },
    core::{HSTRING, PCWSTR, w},
};

fn title(lang: Lang) -> String {
    tr_for!(lang, "settings.window_title").into_owned()
}

#[tauri::command]
fn get_settings() -> Settings {
    Settings::load(&persist::settings_path())
}

#[tauri::command]
fn save_settings(settings: Settings) -> Result<Settings, String> {
    let path = persist::settings_path();
    let settings = settings.sanitized().keeping_panel(&path);
    settings.save(&path).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn current_language() -> &'static str {
    lang::current().code()
}

#[tauri::command]
fn language_pending(choice: Language) -> bool {
    lang::pending(choice)
}

fn select_language(choice: Language) {
    if let Some(failed) = lang::select(choice) {
        // The island tells the user; this process only leaves a line.
        journal::fault(&Fault::new(What::Language, failed));
    }
}

#[tauri::command]
fn preview_sound(tone: String, volume: f32) {
    sound::play(&tone, sound::Kind::Start, volume);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyCheck {
    label: String,
    /// "unknown", "tooPlain", "busy" (taken by another program) or nothing.
    problem: Option<String>,
    /// True when this is plain PrtSc and Windows Snipping Tool currently owns it.
    snipping: bool,
}

#[tauri::command]
fn check_hotkey(key: Hotkey) -> KeyCheck {
    let problem = match key.check() {
        Some(Problem::Unknown) => Some("unknown"),
        Some(Problem::TooPlain) => Some("tooPlain"),
        // The island holds the saved combination itself: checking it is pointless.
        None if island_holds(&key) => None,
        None if !platform::hotkey_free(&key) => Some("busy"),
        None => None,
    };
    KeyCheck {
        label: key.label(),
        problem: problem.map(String::from),
        snipping: key.is_print_screen() && platform::snipping_owns_print_screen(),
    }
}

fn island_holds(key: &Hotkey) -> bool {
    let saved = Settings::load(&persist::settings_path());
    [&saved.screenshot_key, &saved.area_key, &saved.record_key]
        .into_iter()
        .any(|held| held.as_ref() == Some(key))
}

#[derive(serde::Serialize)]
struct DisplayInfo {
    name: String,
    primary: bool,
    width: i32,
    height: i32,
}

#[tauri::command]
fn list_displays() -> Vec<DisplayInfo> {
    platform::displays()
        .into_iter()
        .map(|d| DisplayInfo {
            name: d.name,
            primary: d.primary,
            width: d.size.0,
            height: d.size.1,
        })
        .collect()
}

#[tauri::command]
fn open_journal() -> bool {
    let path = journal::path();
    if !path.exists() {
        journal::info(tr!("journal.journal_opened"));
    }
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(path.as_path()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    platform::shell_result(result).is_ok()
}

/// `false` = the setting could not be written.
#[tauri::command]
fn release_print_screen() -> bool {
    platform::release_print_screen()
}

#[tauri::command]
fn default_screenshot_dir() -> String {
    shot::adapters::default_dir().display().to_string()
}

#[tauri::command]
fn default_recordings_dir() -> String {
    record::adapters::default_dir().display().to_string()
}

/// `recordings` picks the recordings folder, otherwise screenshots. `None` = cancelled.
#[tauri::command]
async fn pick_folder(
    window: tauri::WebviewWindow,
    current: String,
    recordings: bool,
) -> Option<String> {
    // The window is passed as a number so it can be handed to another thread.
    let owner = window.hwnd().ok().map(|hwnd| hwnd.0 as isize);
    // The dialog has its own thread with its own COM; wait for the answer there.
    std::thread::spawn(move || pick(owner, &current, recordings))
        .join()
        .ok()
        .flatten()
}

fn pick(owner: Option<isize>, current: &str, recordings: bool) -> Option<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        let (title, default) = if recordings {
            (
                tr!("settings.pick_recordings"),
                record::adapters::default_dir(),
            )
        } else {
            (
                tr!("settings.pick_screenshots"),
                shot::adapters::default_dir(),
            )
        };
        let _ = dialog.SetTitle(&HSTRING::from(&*title));
        let start = match current.trim() {
            "" => default,
            path => PathBuf::from(path),
        };
        if let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_path()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        // Cancel arrives as an error: that is the "nothing picked" case.
        dialog.Show(owner.map(|h| HWND(h as *mut _))).ok()?;
        let name = dialog
            .GetResult()
            .ok()?
            .GetDisplayName(SIGDN_FILESYSPATH)
            .ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path
    }
}

/// `island_lang` = the language of the island that opened this window.
pub fn run(island_lang: Option<Lang>) {
    match island_lang {
        Some(lang) => lang::fix(lang),
        None => select_language(Settings::load(&persist::settings_path()).language),
    }

    if !platform::claim_single_instance("Local\\GreatIsland.Settings") {
        unsafe {
            // The open window may have been started under another language.
            let open = Lang::ALL
                .into_iter()
                .find_map(|lang| FindWindowW(None, &HSTRING::from(title(lang))).ok());
            if let Some(hwnd) = open {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        return;
    }

    tauri::Builder::default()
        .setup(|app| {
            WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
                .title(title(lang::current()))
                .inner_size(520.0, 660.0)
                .min_inner_size(440.0, 420.0)
                .center()
                .build()?
                .set_focus()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            current_language,
            language_pending,
            preview_sound,
            default_screenshot_dir,
            default_recordings_dir,
            pick_folder,
            check_hotkey,
            release_print_screen,
            list_displays,
            open_journal
        ])
        .run(tauri::generate_context!())
        // The panic hook tells the user: without WebView2 there is no window to say it in.
        .expect("the settings window did not open");
}
