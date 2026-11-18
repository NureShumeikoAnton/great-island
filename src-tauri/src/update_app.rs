//! The update window: the same exe with `--update`, on Tauri and WebView2 like the settings window.
//! It owns nothing. The island opens it when a release waits for an answer, and it shows what the island
//! writes to `update-status.json`: the notes and the choices, then the download. What the user presses goes
//! to the island as a command (`update::Command`); the island does the work (and closes this window when it
//! installs). The settings' "About" reads the same status through the same two commands.

use crate::{
    journal,
    lang::{self, Lang},
    persist, platform,
    settings::Settings,
    update::{
        COMMAND_MESSAGE, Command, Status,
        adapters::{self, process},
    },
};
use serde::Serialize;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{
    LogicalSize, PhysicalPosition, Position, Size, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const WIDTH: f64 = 460.0;
const START_HEIGHT: f64 = 420.0;
/// The window is shown once the page has told its height; if it never does, after this long.
const SHOW_ANYWAY: Duration = Duration::from_secs(4);

static SHOWN: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    /// An island is there to take commands (and to say more than the file does).
    running: bool,
    status: Status,
    /// Unix ms of the last check that got an answer; none before the first.
    checked_at: Option<u64>,
}

#[tauri::command]
pub fn update_status() -> StatusView {
    StatusView {
        running: process::island_runs(),
        status: adapters::load_status(),
        checked_at: adapters::load_memory().checked_at,
    }
}

/// "check", "openWindow", "install", "postpone" or "skip". False: no island to hear it, or no such command.
#[tauri::command]
pub fn update_command(name: String) -> bool {
    let command = match name.as_str() {
        "check" => Command::Check,
        "openWindow" => Command::OpenWindow,
        "install" => Command::Install,
        "postpone" => Command::Postpone,
        "skip" => Command::Skip,
        _ => return false,
    };
    process::send(command, COMMAND_MESSAGE)
}

#[tauri::command]
fn update_exit(app: tauri::AppHandle) {
    app.exit(0);
}

/// In the middle of the island's monitor (or of the cursor's, when nothing is pinned), at this height.
fn place(window: &WebviewWindow, height: f64) {
    let monitor = match Settings::load(&persist::settings_path())
        .island_monitor
        .as_str()
    {
        "" => platform::cursor_monitor(),
        name => platform::monitor_named(name).unwrap_or_else(platform::primary_monitor),
    };
    let scale = f64::from(platform::monitor_scale(monitor));
    let work = platform::work_rect(monitor);
    let (w, h) = (
        (WIDTH * scale).round() as i32,
        (height * scale).round() as i32,
    );
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.top + (work.bottom - work.top - h) / 2;
    let _ = window.set_size(Size::Logical(LogicalSize::new(WIDTH, height)));
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
}

/// The page found its height. The first time the window takes it in the middle of the screen and
/// appears, so that nobody sees it jump from the first guess to the real size; after that it may have
/// been dragged, so it keeps its place and only changes height.
#[tauri::command]
fn update_fit(window: WebviewWindow, height: f64) {
    let height = height.clamp(160.0, 640.0);
    if SHOWN.load(Ordering::Acquire) {
        let _ = window.set_size(Size::Logical(LogicalSize::new(WIDTH, height)));
        return;
    }
    place(&window, height);
    if !SHOWN.swap(true, Ordering::AcqRel) {
        let _ = window.show();
    }
}

pub fn run(island_lang: Option<Lang>) {
    match island_lang {
        Some(lang) => lang::fix(lang),
        None => {
            crate::settings_app::select_language(Settings::load(&persist::settings_path()).language)
        }
    }
    if !platform::claim_single_instance("Local\\GreatIsland.Update") {
        return;
    }
    tauri::Builder::default()
        .setup(|app| {
            let window =
                WebviewWindowBuilder::new(app, "update", WebviewUrl::App("index.html".into()))
                    .title(lang::tr!("update.window_title"))
                    .inner_size(WIDTH, START_HEIGHT)
                    .decorations(false)
                    .transparent(true)
                    .shadow(false)
                    .resizable(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    // The user is away when this opens, and may be typing when it does not: no stealing of the keyboard.
                    .focused(false)
                    .visible(false)
                    .build()?;
            place(&window, START_HEIGHT);
            std::thread::spawn(move || {
                std::thread::sleep(SHOW_ANYWAY);
                if !SHOWN.swap(true, Ordering::AcqRel) {
                    journal::warn("the update window did not report its size".to_string());
                    let _ = window.show();
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            update_status,
            update_command,
            update_exit,
            update_fit,
            crate::settings_app::current_language
        ])
        .run(crate::settings_app::context())
        .expect("the update window did not open");
}
