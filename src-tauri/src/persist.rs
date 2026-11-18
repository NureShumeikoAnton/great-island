//! Where the app's files live, and saving timers between runs.

use crate::{notify::drain::Baseline, scheduler::SavedTimer};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Same folder the Tauri version used, so settings aren't lost.
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("com.greatisland.app")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn state_path() -> PathBuf {
    config_dir().join("timers.json")
}

pub fn update_memory_path() -> PathBuf {
    config_dir().join("update.json")
}

/// Where an update stands (`crate::update::Status`): the island writes it, the other windows read it.
pub fn update_status_path() -> PathBuf {
    config_dir().join("update-status.json")
}

pub fn update_offer_path() -> PathBuf {
    config_dir().join("update-offer.json")
}

pub fn battery_path() -> PathBuf {
    config_dir().join("battery.json")
}

pub fn load_baseline(path: &Path) -> Option<Baseline> {
    let baseline: Baseline = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    Some(baseline.sanitized())
}

pub fn save_baseline(baseline: &Baseline, path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string(baseline)?)?;
    fs::rename(tmp, path)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedState {
    pub saved_at_ms: u64,
    pub paused: bool,
    pub timers: Vec<SavedTimer>,
    #[serde(default)]
    pub hidden: Option<Hidden>,
    #[serde(default)]
    pub quiet: bool,
}

/// Hidden from the tray for a while or until the user brings it back;
/// only priority notifications show meanwhile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Hidden {
    Until(u64),
    UntilShown,
}

impl Hidden {
    /// Hide for this many minutes; `None` = until the user brings it back.
    pub fn for_minutes(minutes: Option<u32>) -> Self {
        match minutes {
            Some(m) => Self::Until(now_ms() + m as u64 * 60_000),
            None => Self::UntilShown,
        }
    }

    pub fn expired(self) -> bool {
        matches!(self, Self::Until(at) if now_ms() >= at)
    }

    /// Minutes left, rounded up; `None` = no limit.
    pub fn minutes_left(self) -> Option<u64> {
        match self {
            Self::Until(at) => Some(at.saturating_sub(now_ms()).div_ceil(60_000)),
            Self::UntilShown => None,
        }
    }
}

impl SavedState {
    pub fn new(paused: bool, timers: Vec<SavedTimer>, hidden: Option<Hidden>) -> Self {
        Self {
            saved_at_ms: now_ms(),
            paused,
            timers,
            hidden,
            quiet: false,
        }
    }

    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
    }

    /// Writes via a temp file so a shutdown mid-write can't corrupt the state.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn offline_ms(&self) -> u64 {
        now_ms().saturating_sub(self.saved_at_ms)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_state_without_hidden_still_loads() {
        let state: SavedState =
            serde_json::from_str(r#"{"savedAtMs":1,"paused":false,"timers":[]}"#).unwrap();
        assert_eq!(state.hidden, None);
    }

    #[test]
    fn hidden_round_trips() {
        for hidden in [Hidden::Until(5), Hidden::UntilShown] {
            let text =
                serde_json::to_string(&SavedState::new(false, vec![], Some(hidden))).unwrap();
            let back: SavedState = serde_json::from_str(&text).unwrap();
            assert_eq!(back.hidden, Some(hidden));
        }
        assert!(Hidden::Until(5).expired());
        assert!(!Hidden::UntilShown.expired());
    }
}
