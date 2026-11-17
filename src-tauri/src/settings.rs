//! App settings: the reminder list and general parameters, stored as JSON in the app config folder.

use crate::{
    hotkey::Hotkey,
    lang::{self, Lang, tr, tr_for},
    record::choice::Choice,
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: String,
    pub name: String,
    pub color: String,
    /// "bell", "drop" or "soft" (synthesized in the UI).
    pub tone: String,
    pub interval_min: u32,
    pub duration_sec: u32,
    pub enabled: bool,
    pub reset_on_idle: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub reminders: Vec<Reminder>,
    pub volume: f32,
    pub idle_min: u32,
    /// Show the collapsed island always (otherwise only during a reminder).
    pub show_island: bool,
    /// A monitor's device name, like `\\.\DISPLAY2`; empty = the island follows the cursor.
    pub island_monitor: String,
    pub fullscreen: Fullscreen,
    pub player: bool,
    pub announce_tracks: bool,
    /// 0 = never.
    pub silent_drop_min: u32,
    /// A dimmed island stays live: bars move, track time and timer seconds tick. Otherwise everything
    /// freezes into a static picture and the process doesn't wake up for animation.
    pub live_when_dimmed: bool,
    pub keep_screen_on: bool,
    /// Decorative animations. For now the only one is the wave from the island when an area
    /// selection opens.
    pub animations: bool,
    /// Corner marks around the area of a recording.
    pub area_marks: bool,
    /// Empty = the system Screenshots folder.
    pub screenshot_dir: String,
    pub screenshot_key: Option<Hotkey>,
    /// A screenshot of an area picked first.
    pub area_key: Option<Hotkey>,
    pub viewer_bar: ViewerBar,
    /// Empty = the default folder under Videos.
    pub recordings_dir: String,
    pub record_key: Option<Hotkey>,
    /// Changed only by the island's pre-start panel.
    #[serde(flatten)]
    pub record: Choice,
    /// The video window's sound, 0–1, kept by the island when the window closes.
    pub video_volume: f32,
    pub video_muted: bool,
    /// The video window's length shows the time left instead.
    pub video_remaining: bool,
    pub autostart: bool,
    /// Look for a new version by itself (a day apart, in a quiet moment). The manual check stays.
    pub check_updates: bool,
    pub language: Language,
}

/// The island reads it once at start: a change applies after a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Ru,
    /// Also what an unknown value becomes: one unknown word must not reset all the settings.
    #[default]
    #[serde(other)]
    Auto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewerBar {
    /// Aliases: the first version of the setting had both variants as strips.
    #[default]
    #[serde(alias = "compact", alias = "regular")]
    Strip,
    Floating,
}

/// Behavior in fullscreen. The reminder sound plays regardless.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fullscreen {
    Show,
    Alerts,
    #[default]
    Hide,
}

impl Default for Settings {
    fn default() -> Self {
        Self::named_in(lang::current())
    }
}

impl Settings {
    fn named_in(lang: Lang) -> Self {
        Self {
            reminders: vec![
                reminder(
                    "stretch",
                    &tr_for!(lang, "reminder.stretch"),
                    "#FF9F0A",
                    "bell",
                    50,
                    30,
                    true,
                ),
                reminder(
                    "eyes",
                    &tr_for!(lang, "reminder.eyes"),
                    "#64D2FF",
                    "soft",
                    20,
                    20,
                    true,
                ),
                reminder(
                    "water",
                    &tr_for!(lang, "reminder.water"),
                    "#0A84FF",
                    "drop",
                    60,
                    15,
                    false,
                ),
            ],
            volume: 0.6,
            idle_min: 5,
            show_island: true,
            island_monitor: String::new(),
            fullscreen: Fullscreen::default(),
            player: true,
            announce_tracks: true,
            silent_drop_min: 10,
            live_when_dimmed: false,
            keep_screen_on: true,
            animations: true,
            area_marks: true,
            screenshot_dir: String::new(),
            screenshot_key: Some(Hotkey::print_screen()),
            area_key: None,
            viewer_bar: ViewerBar::default(),
            recordings_dir: String::new(),
            record_key: None,
            record: Choice::default(),
            video_volume: 1.0,
            video_muted: false,
            video_remaining: false,
            autostart: false,
            check_updates: true,
            language: Language::default(),
        }
    }
}

fn reminder(
    id: &str,
    name: &str,
    color: &str,
    tone: &str,
    interval_min: u32,
    duration_sec: u32,
    reset_on_idle: bool,
) -> Reminder {
    Reminder {
        id: id.into(),
        name: name.into(),
        color: color.into(),
        tone: tone.into(),
        interval_min,
        duration_sec,
        enabled: true,
        reset_on_idle,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    Restored,
    Reset,
}

fn parse(text: &str) -> Option<Settings> {
    serde_json::from_str::<Settings>(text)
        .ok()
        .map(Settings::sanitized)
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("backup.json")
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        Self::recover(path).0
    }

    fn first_run(installed: Option<Language>) -> Self {
        let choice = installed.unwrap_or_default();
        Self {
            language: choice,
            ..Self::named_in(lang::resolve(choice))
        }
    }

    /// Reads the settings. A good file is copied aside each time it is read. A damaged
    /// one is moved to `settings.broken.json` — never overwritten by the next save — and
    /// that copy takes its place; with no copy, defaults. No file at all is the first run.
    pub fn recover(path: &Path) -> (Self, Option<Recovery>) {
        Self::recover_with(path, lang::installer_choice)
    }

    fn recover_with(
        path: &Path,
        installed: impl FnOnce() -> Option<Language>,
    ) -> (Self, Option<Recovery>) {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return (Self::first_run(installed()), None);
            }
            Err(_) => String::new(),
        };
        let backup = backup_path(path);
        if let Some(settings) = parse(&text) {
            if fs::read_to_string(&backup).ok().as_deref() != Some(text.as_str()) {
                let _ = fs::write(&backup, &text);
            }
            return (settings, None);
        }
        let _ = fs::rename(path, path.with_extension("broken.json"));
        let copy = fs::read_to_string(&backup)
            .ok()
            .and_then(|text| parse(&text).map(|settings| (settings, text)));
        match copy {
            Some((settings, text)) => {
                let _ = fs::write(path, text);
                (settings, Some(Recovery::Restored))
            }
            None => (Self::default(), Some(Recovery::Reset)),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        // Via a temp file: the island reads settings from another process and must never see a half-written file.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)
    }

    /// The settings window saves its whole copy, which may be older than the island's last
    /// change: what the island owns (the panel's choice, the video window's sound and length) is taken
    /// from the file.
    pub fn keeping_panel(mut self, path: &Path) -> Self {
        let file = Self::load(path);
        self.record = file.record;
        self.video_volume = file.video_volume;
        self.video_muted = file.video_muted;
        self.video_remaining = file.video_remaining;
        self
    }

    pub fn sanitized(mut self) -> Self {
        self.volume = self.volume.clamp(0.0, 1.0);
        self.idle_min = self.idle_min.clamp(1, 120);
        self.silent_drop_min = self.silent_drop_min.min(240);
        for r in &mut self.reminders {
            r.interval_min = r.interval_min.clamp(1, 600);
            r.duration_sec = r.duration_sec.clamp(5, 300);
            r.name = r.name.trim().to_string();
            if r.name.is_empty() {
                r.name = tr!("reminder.unnamed").into();
            }
            if !matches!(r.tone.as_str(), "bell" | "drop" | "soft") {
                r.tone = "bell".into();
            }
        }
        if self
            .screenshot_key
            .as_ref()
            .is_some_and(|k| k.check().is_some())
        {
            self.screenshot_key = None;
        }
        // One key can't do two things: the screenshot keeps it, then the area screenshot.
        if self
            .area_key
            .as_ref()
            .is_some_and(|k| k.check().is_some() || Some(k) == self.screenshot_key.as_ref())
        {
            self.area_key = None;
        }
        if self.record_key.as_ref().is_some_and(|k| {
            k.check().is_some()
                || Some(k) == self.screenshot_key.as_ref()
                || Some(k) == self.area_key.as_ref()
        }) {
            self.record_key = None;
        }
        let mut seen = std::collections::HashSet::new();
        self.reminders
            .retain(|r| !r.id.is_empty() && seen.insert(r.id.clone()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unknown value would reset all settings, reminders included.
    #[test]
    fn old_viewer_bar_names_still_load() {
        for old in ["compact", "regular", "strip"] {
            let text = format!(r#"{{"viewerBar":"{old}","volume":0.3}}"#);
            let settings: Settings = serde_json::from_str(&text).expect("читается");
            assert_eq!(settings.viewer_bar, ViewerBar::Strip);
            assert_eq!(settings.volume, 0.3);
        }
    }

    /// Old settings with no key get PrtSc; an explicit "no key" stays.
    #[test]
    fn screenshot_key_defaults_to_print_screen() {
        let old: Settings = serde_json::from_str(r#"{"volume":0.3}"#).expect("читается");
        assert_eq!(old.screenshot_key, Some(Hotkey::print_screen()));
        let none: Settings = serde_json::from_str(r#"{"screenshotKey":null}"#).expect("читается");
        assert_eq!(none.screenshot_key, None);
    }

    /// Settings saved before screen recording keep everything and get no record key.
    #[test]
    fn old_settings_load_without_recording() {
        let old: Settings =
            serde_json::from_str(r#"{"volume":0.3,"viewerBar":"floating"}"#).expect("читается");
        assert_eq!(old.volume, 0.3);
        assert_eq!(old.viewer_bar, ViewerBar::Floating);
        assert_eq!(old.record_key, None);
        assert_eq!(old.area_key, None);
        assert!(old.animations);
        assert!(old.area_marks);
        assert_eq!(old.record, Choice::default());
        assert!(old.record.fade);
        assert_eq!(old.recordings_dir, "");
        assert_eq!(old.island_monitor, "");
        assert!(old.check_updates);
        assert_eq!(old.video_volume, 1.0);
        assert!(!old.video_muted);
        assert!(!old.video_remaining);
    }

    /// The recording choices were flat switches before the panel; they keep their keys.
    #[test]
    fn recording_choices_keep_their_flat_keys() {
        let old: Settings = serde_json::from_str(
            r#"{"volume":0.3,"recordCountdown":true,"recordFade":false,"recordSound":"both"}"#,
        )
        .expect("loads");
        assert_eq!(old.volume, 0.3);
        assert_eq!(old.record.countdown.seconds(), 3);
        assert!(!old.record.fade);
        assert_eq!(old.record.sound, crate::record::sound::Sound::Both);
        let text = serde_json::to_string(&old).expect("writes");
        assert!(text.contains(r#""recordSound":"both""#), "{text}");
        assert!(!text.contains(r#""record":"#), "{text}");
    }

    #[test]
    fn the_panel_choice_survives_a_stale_settings_window() {
        let dir = std::env::temp_dir().join(format!("bi-panel-{}", std::process::id()));
        let path = dir.join("settings.json");
        let mut island = Settings::default();
        island.record.countdown = island.record.countdown.next();
        island.save(&path).expect("saves");
        let window = Settings {
            volume: 0.2,
            ..Settings::default()
        };
        let saved = window.keeping_panel(&path);
        assert_eq!(saved.volume, 0.2);
        assert_eq!(saved.record.countdown.seconds(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    /// An unknown language must not take the reminders down with it.
    #[test]
    fn language_loads_from_old_and_unknown_values() {
        let old: Settings = serde_json::from_str(r#"{"volume":0.3}"#).expect("loads");
        assert_eq!(old.language, Language::Auto);
        for (text, expected) in [
            ("ru", Language::Ru),
            ("en", Language::En),
            ("auto", Language::Auto),
            ("de", Language::Auto),
            ("", Language::Auto),
        ] {
            let json = format!(r#"{{"language":"{text}","volume":0.3}}"#);
            let settings: Settings = serde_json::from_str(&json).expect("loads");
            assert_eq!(settings.language, expected, "{text:?}");
            assert_eq!(settings.volume, 0.3);
        }
        let saved = serde_json::to_string(&Settings::default()).expect("saves");
        assert!(saved.contains(r#""language":"auto""#));
    }

    #[test]
    fn default_reminders_are_named_in_the_current_language() {
        let names = |lang| {
            crate::lang::with(lang, || {
                Settings::default()
                    .reminders
                    .into_iter()
                    .map(|r| r.name)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(names(crate::lang::Lang::Ru), ["Разминка", "Глаза", "Вода"]);
        assert_eq!(names(crate::lang::Lang::En), ["Stretch", "Eyes", "Water"]);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("great-island-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("папка");
        dir.join("settings.json")
    }

    #[test]
    fn damaged_settings_come_back_from_the_last_good_copy() {
        let path = scratch("restore");
        let good = Settings {
            volume: 0.3,
            ..Settings::default()
        };
        good.save(&path).expect("сохраняется");
        assert_eq!(Settings::recover(&path), (good.clone(), None));

        fs::write(&path, "{ это не настройки").expect("пишется");
        let (settings, recovery) = Settings::recover(&path);
        assert_eq!(recovery, Some(Recovery::Restored));
        assert_eq!(settings, good);
        assert!(path.with_extension("broken.json").exists());
        assert_eq!(Settings::recover(&path), (good, None));
    }

    #[test]
    fn damaged_settings_without_a_copy_reset_once() {
        let path = scratch("reset");
        fs::write(&path, "половина фай").expect("пишется");
        let (settings, recovery) = Settings::recover(&path);
        assert_eq!(recovery, Some(Recovery::Reset));
        assert_eq!(settings, Settings::default());
        assert!(path.with_extension("broken.json").exists());
        assert_eq!(Settings::recover(&path).1, None);
    }

    #[test]
    fn first_run_is_not_a_problem() {
        let path = scratch("first");
        assert_eq!(
            Settings::recover_with(&path, || None),
            (Settings::default(), None)
        );
        assert!(!path.with_extension("broken.json").exists());
    }

    /// Russian picked in the installer on an English Windows.
    #[test]
    fn first_run_follows_the_installers_language() {
        let path = scratch("installed");
        let (settings, recovery) = Settings::recover_with(&path, || Some(Language::Ru));
        assert_eq!(recovery, None);
        assert_eq!(settings.language, Language::Ru);
        assert_eq!(settings.reminders[0].name, "Разминка");
        let (english, _) = Settings::recover_with(&path, || Some(Language::En));
        assert_eq!(english.reminders[0].name, "Stretch");
    }

    #[test]
    fn record_key_gives_way_to_the_screenshot() {
        let mut settings = Settings {
            record_key: Some(Hotkey::print_screen()),
            ..Settings::default()
        };
        settings = settings.sanitized();
        assert_eq!(settings.record_key, None);
        assert_eq!(settings.screenshot_key, Some(Hotkey::print_screen()));
    }

    #[test]
    fn keys_give_way_in_order_shot_area_record() {
        let key = Hotkey {
            code: "KeyA".into(),
            ctrl: true,
            alt: false,
            shift: true,
            win: false,
        };
        let shared = Settings {
            area_key: Some(Hotkey::print_screen()),
            ..Settings::default()
        };
        assert_eq!(shared.sanitized().area_key, None);
        let settings = Settings {
            area_key: Some(key.clone()),
            record_key: Some(key.clone()),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(settings.area_key, Some(key));
        assert_eq!(settings.record_key, None);
    }
}
