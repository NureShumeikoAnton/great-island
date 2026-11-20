//! Great Island: an island at the top of the screen with break reminders, a player, screenshots and recording.
//!
//! One exe, two modes: without arguments the island (native window, Composition);
//! with `--settings` the Tauri settings window as a separate process.

/// Public for `examples/shot_probe.rs`.
pub mod capture;
mod control;
mod crash;
/// Public for `examples/download_probe.rs`: the folder watch on a real folder.
pub mod devices;
mod dialog;
pub mod download;
mod fault;
mod files;
pub mod gesture;
mod gpu;
mod hotkey;
mod island;
mod journal;
mod lang;
mod loudness;
/// Public for `examples/media_probe.rs`: media-session probe on live players.
pub mod media;
/// Public for `examples/edit_probe.rs`: opening and saving a real recording for the editor.
pub mod montage;
/// Public for `examples/power_probe.rs`: the battery and the load read live.
pub mod notify;
mod persist;
mod platform;
/// Public for `examples/play_probe.rs`: plays a real file through Media Engine.
pub mod play;
mod record;
mod scheduler;
mod select;
mod settings;
mod settings_app;
/// Public for `examples/shot_probe.rs`: probe of capture, PNG and hiding the window from captures.
pub mod shot;
mod sound;
pub mod stage;
/// Public until the island uses it.
pub mod update;

rust_i18n::i18n!("locales", fallback = "en");

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    if has("--settings") {
        crash::install(crash::Role::Settings);
        let island_lang = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--lang="))
            .and_then(lang::Lang::from_code);
        settings_app::run(island_lang);
    } else {
        crash::install(crash::Role::Island);
        let attempt = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--attempt=")?.parse().ok())
            .unwrap_or(1);
        let demo = if has("--demo-failure") {
            Some(island::Demo::Problem(fault::Level::Critical))
        } else if has("--demo-warning") {
            Some(island::Demo::Problem(fault::Level::Warning))
        } else if has("--demo-system") {
            Some(island::Demo::System)
        } else if has("--demo-devices") {
            Some(island::Demo::Devices)
        } else if has("--demo-update") {
            Some(island::Demo::Update)
        } else {
            None
        };
        island::run(has("--restart"), attempt, demo);
    }
}
