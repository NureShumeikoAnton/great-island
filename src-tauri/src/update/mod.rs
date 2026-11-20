//! The update rules: which version counts as newer, what the user may do with an offer, when a check
//! may run. No network and no Windows: a manifest text and the clock come in, a decision goes out.
//! Fetching, checking the signature and installing are the adapters' job (`adapters/`, later).

pub mod adapters;
pub mod sheet;
pub mod signature;

use crate::fault::{Cause, Failed};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

/// Where the releases are announced: the manifest of the newest one.
pub const MANIFEST_URL: &str =
    "https://github.com/NureShumeikoAnton/great-island/releases/latest/download/latest.json";
/// The window message (WM_APP + 13) that carries a [`Command`] to the island, in `wparam`.
pub const COMMAND_MESSAGE: u32 = 0x8000 + 13;

pub const POSTPONE_MS: u64 = 60 * 60 * 1000;
/// An offer can be postponed (an hour at a time) for this long after it was first shown; then it can only be taken.
pub const POSTPONE_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
pub const CHECK_EVERY_MS: u64 = 24 * 60 * 60 * 1000;
/// A check or an offer waits for this long without input.
pub const IDLE_BEFORE_MS: u64 = 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl FromStr for Version {
    type Err = String;

    /// "0.7.0" or "v0.7.0". Anything else (a pre-release suffix too) is refused: an unknown version
    /// must not look newer than ours.
    fn from_str(text: &str) -> Result<Self, String> {
        let mut parts = text.trim().trim_start_matches('v').split('.');
        let mut next = || {
            parts
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .ok_or_else(|| format!("not a version: {text:?}"))
        };
        let version = Self {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        match parts.next() {
            None => Ok(version),
            Some(_) => Err(format!("not a version: {text:?}")),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    /// `None`: `latest` is not newer.
    pub fn between(current: Version, latest: Version) -> Option<Self> {
        if latest <= current {
            None
        } else if latest.major != current.major {
            Some(Self::Major)
        } else if latest.minor != current.minor {
            Some(Self::Minor)
        } else {
            Some(Self::Patch)
        }
    }

    /// Only a patch may be passed over; before 1.0 a minor is a real change too.
    pub fn can_skip(self) -> bool {
        self == Self::Patch
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Notes {
    #[serde(default)]
    pub en: String,
    #[serde(default)]
    pub ru: String,
}

impl Notes {
    /// The text in the language, else in the other one: a release without one language still tells what changed.
    pub fn pick(&self, code: &str) -> &str {
        let (first, second) = if code == "ru" {
            (&self.ru, &self.en)
        } else {
            (&self.en, &self.ru)
        };
        if first.is_empty() { second } else { first }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installer {
    pub url: String,
    /// The text of the `.sig` file made when the release was signed.
    pub signature: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: Version,
    #[serde(default)]
    pub notes: Notes,
    pub installer: Installer,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, Failed> {
        let manifest: Self = serde_json::from_str(text)
            .map_err(|e| Failed::because(Cause::Unknown, format_args!("update manifest: {e}")))?;
        // The installer is run as it comes: never over a link that can be read or changed on the way.
        let url = &manifest.installer.url;
        let local = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost");
        if !(url.starts_with("https://") || local) || manifest.installer.signature.is_empty() {
            return Err(Failed::because(
                Cause::Unknown,
                "update manifest: the installer needs an https address and a signature",
            ));
        }
        Ok(manifest)
    }
}

/// What the island remembers between runs, in a file of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Memory {
    /// Unix ms of the last check that reached the server.
    pub checked_at: Option<u64>,
    /// The version the fields below belong to: a newer one starts over.
    pub offered: Option<Version>,
    /// Unix ms when `offered` was first shown: from then on it may be postponed for a day.
    pub offered_at: Option<u64>,
    /// Unix ms.
    pub postponed_until: Option<u64>,
    pub skipped: Option<Version>,
}

/// A postponement that was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Postponed {
    /// When the offer comes back (unix ms).
    pub until: u64,
    /// Until when (unix ms) it may be postponed again.
    pub deadline: u64,
}

impl Memory {
    /// A clock set back counts as due: waiting for it to catch up could take years.
    pub fn check_due(&self, now: u64) -> bool {
        self.checked_at
            .is_none_or(|at| at > now || now - at >= CHECK_EVERY_MS)
    }

    pub fn checked(&mut self, now: u64) {
        self.checked_at = Some(now);
    }

    /// The offer is on screen. The first time for a version starts its day of postponements.
    pub fn shown(&mut self, version: Version, now: u64) {
        let again = self.offered == Some(version) && self.offered_at.is_some_and(|at| at <= now);
        if !again {
            self.offered = Some(version);
            self.offered_at = Some(now);
            self.postponed_until = None;
        }
    }

    /// Until when (unix ms) `version` may be postponed; none before it was shown.
    pub fn deadline(&self, version: Version) -> Option<u64> {
        if self.offered != Some(version) {
            return None;
        }
        self.offered_at.map(|at| at + POSTPONE_WINDOW_MS)
    }

    /// `None`: the day is over, the offer can only be taken.
    pub fn postpone(&mut self, version: Version, now: u64) -> Option<Postponed> {
        self.shown(version, now);
        let deadline = self.deadline(version)?;
        if now >= deadline {
            return None;
        }
        // The last postponement ends with the day, not an hour after it.
        let until = (now + POSTPONE_MS).min(deadline);
        self.postponed_until = Some(until);
        Some(Postponed { until, deadline })
    }

    pub fn skip(&mut self, version: Version) {
        self.skipped = Some(version);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub bump: Bump,
    pub can_skip: bool,
    pub can_postpone: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    UpToDate,
    Skipped,
    /// Postponed: the offer comes back at this unix ms.
    Waiting {
        until: u64,
    },
    Offer(Offer),
}

fn offer(bump: Bump, memory: &Memory, latest: Version, now: u64) -> Verdict {
    Verdict::Offer(Offer {
        bump,
        can_skip: bump.can_skip(),
        can_postpone: memory
            .deadline(latest)
            .is_none_or(|deadline| now < deadline),
    })
}

pub fn judge(current: Version, latest: Version, memory: &Memory, now: u64) -> Verdict {
    let Some(bump) = Bump::between(current, latest) else {
        return Verdict::UpToDate;
    };
    if bump.can_skip() && memory.skipped == Some(latest) {
        return Verdict::Skipped;
    }
    if memory.offered == Some(latest)
        && let Some(until) = memory.postponed_until
        // A clock set back must not hide the offer for longer than one postponement.
        && until > now
        && until - now <= POSTPONE_MS
    {
        return Verdict::Waiting { until };
    }
    offer(bump, memory, latest, now)
}

/// A check the user asked for: a skipped or postponed version is offered again.
pub fn judge_manual(current: Version, latest: Version, memory: &Memory, now: u64) -> Verdict {
    match Bump::between(current, latest) {
        Some(bump) => offer(bump, memory, latest, now),
        None => Verdict::UpToDate,
    }
}

/// Where an update stands. The island owns it, writes it to `update-status.json` at every change, and shows it
/// (its notices and the update window); the settings only read it and send [`Command`]s: there is one source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Nothing is going on and nothing is known.
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// A check could not be made.
    Failed,
    /// A newer release waits for the user's answer.
    Available,
    Downloading,
    Verifying,
    Installing,
    /// The installer was not run; `error` says why.
    InstallFailed,
}

/// The release on offer, as the views need it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferView {
    pub current: Version,
    pub latest: Version,
    pub notes: Notes,
    pub can_skip: bool,
    pub can_postpone: bool,
    /// The user passed this version over once: it is offered because they asked.
    pub skipped: bool,
    pub size: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Status {
    pub phase: Phase,
    /// The user asked for this check (the settings' button).
    pub manual: bool,
    pub error: Option<String>,
    pub offer: Option<OfferView>,
    /// Bytes of the installer that came, and how many are expected (0 = not known).
    pub done: u64,
    pub total: u64,
    /// Unix ms of the last change.
    pub at: u64,
}

/// What another process may ask the island to do, as the number that travels in a window message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Check now, even if a check is not due and the user is not away.
    Check = 1,
    /// Show the update window for the release found.
    OpenWindow = 2,
    Install = 3,
    Postpone = 4,
    Skip = 5,
}

impl Command {
    pub fn from_code(code: usize) -> Option<Self> {
        [
            Self::Check,
            Self::OpenWindow,
            Self::Install,
            Self::Postpone,
            Self::Skip,
        ]
        .into_iter()
        .find(|command| *command as usize == code)
    }
}

/// A moment on the user's own clock, for the notices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}

impl LocalTime {
    /// "15:40".
    pub fn clock(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// "18.11, 14:20".
    pub fn date_time(self) -> String {
        format!("{:02}.{:02}, {}", self.day, self.month, self.clock())
    }
}

/// A minute without input and nothing important going on; `busy` is the island's call.
pub fn calm(idle_ms: u64, busy: bool) -> bool {
    !busy && idle_ms >= IDLE_BEFORE_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        text.parse().expect("a version")
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(v("0.10.0") > v("0.9.0"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert_eq!(v("v0.7.0"), v("0.7.0"));
        assert_eq!(v("0.7.0").to_string(), "0.7.0");
    }

    #[test]
    fn odd_versions_are_refused() {
        for text in ["", "1", "1.2", "1.2.3.4", "1.2.x", "1.2.3-beta", "-1.0.0"] {
            assert!(text.parse::<Version>().is_err(), "{text}");
        }
    }

    #[test]
    fn only_a_patch_can_be_skipped() {
        let bump = |a, b| Bump::between(v(a), v(b));
        assert_eq!(bump("0.6.1", "0.6.2"), Some(Bump::Patch));
        assert_eq!(bump("0.6.1", "0.7.0"), Some(Bump::Minor));
        assert_eq!(bump("0.9.0", "1.0.0"), Some(Bump::Major));
        assert_eq!(bump("0.6.1", "0.6.1"), None);
        assert_eq!(bump("0.6.1", "0.6.0"), None);
        assert!(Bump::Patch.can_skip());
        assert!(!Bump::Minor.can_skip());
        assert!(!Bump::Major.can_skip());
    }

    #[test]
    fn a_newer_version_is_offered() {
        let verdict = judge(v("0.6.1"), v("0.7.0"), &Memory::default(), 0);
        assert_eq!(
            verdict,
            Verdict::Offer(Offer {
                bump: Bump::Minor,
                can_skip: false,
                can_postpone: true,
            })
        );
        assert_eq!(
            judge(v("0.7.0"), v("0.7.0"), &Memory::default(), 0),
            Verdict::UpToDate
        );
    }

    #[test]
    fn a_skipped_patch_stays_quiet_until_a_newer_one() {
        let mut memory = Memory::default();
        memory.skip(v("0.6.2"));
        assert_eq!(judge(v("0.6.1"), v("0.6.2"), &memory, 0), Verdict::Skipped);
        assert!(matches!(
            judge(v("0.6.1"), v("0.6.3"), &memory, 0),
            Verdict::Offer(_)
        ));
    }

    #[test]
    fn a_skipped_minor_is_still_offered() {
        let mut memory = Memory::default();
        memory.skip(v("0.7.0"));
        assert!(matches!(
            judge(v("0.6.1"), v("0.7.0"), &memory, 0),
            Verdict::Offer(_)
        ));
    }

    const HOUR: u64 = POSTPONE_MS;

    #[test]
    fn a_postponed_offer_waits_an_hour() {
        let mut memory = Memory::default();
        memory.shown(v("0.7.0"), 1_000);
        let postponed = memory.postpone(v("0.7.0"), 1_000).expect("allowed");
        assert_eq!(postponed.until, 1_000 + HOUR);
        assert_eq!(postponed.deadline, 1_000 + POSTPONE_WINDOW_MS);
        assert_eq!(
            judge(v("0.6.1"), v("0.7.0"), &memory, 2_000),
            Verdict::Waiting {
                until: postponed.until
            }
        );
        assert!(matches!(
            judge(v("0.6.1"), v("0.7.0"), &memory, postponed.until),
            Verdict::Offer(_)
        ));
    }

    #[test]
    fn a_clock_set_back_does_not_hide_the_offer() {
        let mut memory = Memory::default();
        memory.shown(v("0.7.0"), 10 * HOUR);
        memory.postpone(v("0.7.0"), 10 * HOUR);
        assert!(matches!(
            judge(v("0.6.1"), v("0.7.0"), &memory, 0),
            Verdict::Offer(_)
        ));
    }

    #[test]
    fn postponing_ends_a_day_after_the_first_showing() {
        let mut memory = Memory::default();
        memory.shown(v("0.7.0"), 0);
        let deadline = POSTPONE_WINDOW_MS;
        for hour in 1..24 {
            let now = hour * HOUR;
            let Verdict::Offer(offer) = judge(v("0.6.1"), v("0.7.0"), &memory, now) else {
                panic!("hour {hour}: an offer");
            };
            assert!(offer.can_postpone, "hour {hour}");
            assert!(memory.postpone(v("0.7.0"), now).is_some(), "hour {hour}");
        }
        let Verdict::Offer(offer) = judge(v("0.6.1"), v("0.7.0"), &memory, deadline) else {
            panic!("an offer");
        };
        assert!(!offer.can_postpone);
        assert_eq!(memory.postpone(v("0.7.0"), deadline), None);
    }

    #[test]
    fn the_last_postponement_is_cut_to_the_deadline() {
        let mut memory = Memory::default();
        memory.shown(v("0.7.0"), 0);
        let now = POSTPONE_WINDOW_MS - HOUR / 2;
        let postponed = memory.postpone(v("0.7.0"), now).expect("allowed");
        assert_eq!(postponed.until, POSTPONE_WINDOW_MS);
    }

    #[test]
    fn a_newer_version_starts_its_own_day() {
        let mut memory = Memory::default();
        memory.shown(v("0.7.0"), 0);
        let late = POSTPONE_WINDOW_MS + HOUR;
        let Verdict::Offer(old) = judge(v("0.6.1"), v("0.7.0"), &memory, late) else {
            panic!("an offer");
        };
        assert!(!old.can_postpone);
        let Verdict::Offer(new) = judge(v("0.6.1"), v("0.7.1"), &memory, late) else {
            panic!("an offer");
        };
        assert!(new.can_postpone);
        let postponed = memory.postpone(v("0.7.1"), late).expect("allowed");
        assert_eq!(postponed.deadline, late + POSTPONE_WINDOW_MS);
    }

    #[test]
    fn showing_again_does_not_extend_the_day() {
        let mut memory = Memory::default();
        memory.shown(v("0.7.0"), 0);
        memory.shown(v("0.7.0"), 5 * HOUR);
        assert_eq!(memory.deadline(v("0.7.0")), Some(POSTPONE_WINDOW_MS));
        assert_eq!(memory.deadline(v("0.7.1")), None);
    }

    #[test]
    fn a_manual_check_offers_what_was_skipped_or_postponed() {
        let mut memory = Memory::default();
        memory.skip(v("0.6.2"));
        assert_eq!(judge(v("0.6.1"), v("0.6.2"), &memory, 0), Verdict::Skipped);
        assert!(matches!(
            judge_manual(v("0.6.1"), v("0.6.2"), &memory, 0),
            Verdict::Offer(_)
        ));
        memory.shown(v("0.7.0"), 0);
        memory.postpone(v("0.7.0"), 0);
        assert!(matches!(
            judge(v("0.6.1"), v("0.7.0"), &memory, 1),
            Verdict::Waiting { .. }
        ));
        assert!(matches!(
            judge_manual(v("0.6.1"), v("0.7.0"), &memory, 1),
            Verdict::Offer(_)
        ));
        assert_eq!(
            judge_manual(v("0.7.0"), v("0.7.0"), &memory, 1),
            Verdict::UpToDate
        );
    }

    #[test]
    fn commands_travel_as_numbers() {
        for command in [
            Command::Check,
            Command::OpenWindow,
            Command::Install,
            Command::Postpone,
            Command::Skip,
        ] {
            assert_eq!(Command::from_code(command as usize), Some(command));
        }
        assert_eq!(Command::from_code(0), None);
        assert_eq!(Command::from_code(99), None);
    }

    #[test]
    fn a_status_survives_its_file() {
        let status = Status {
            phase: Phase::Available,
            manual: true,
            error: None,
            offer: Some(OfferView {
                current: v("0.7.0"),
                latest: v("0.7.1"),
                notes: Notes {
                    en: "Fixes".into(),
                    ru: "Исправления".into(),
                },
                can_skip: true,
                can_postpone: true,
                skipped: false,
                size: 42,
            }),
            done: 0,
            total: 0,
            at: 7,
        };
        let text = serde_json::to_string(&status).expect("saves");
        assert!(text.contains(r#""phase":"available""#));
        assert_eq!(
            serde_json::from_str::<Status>(&text).expect("loads"),
            status
        );
        assert_eq!(
            serde_json::from_str::<Status>("{}").expect("loads"),
            Status::default()
        );
    }

    #[test]
    fn a_moment_is_written_the_short_way() {
        let time = LocalTime {
            month: 11,
            day: 8,
            hour: 9,
            minute: 5,
        };
        assert_eq!(time.clock(), "09:05");
        assert_eq!(time.date_time(), "08.11, 09:05");
    }

    #[test]
    fn a_manifest_may_point_at_a_server_on_this_computer() {
        let local = r#"{"version":"0.7.0","installer":{"url":"http://127.0.0.1:8123/a.exe","signature":"sig"}}"#;
        assert!(Manifest::parse(local).is_ok());
        let remote = r#"{"version":"0.7.0","installer":{"url":"http://example.org/a.exe","signature":"sig"}}"#;
        assert!(Manifest::parse(remote).is_err());
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let mut memory = Memory::default();
        assert!(memory.check_due(0));
        memory.checked(1_000);
        assert!(!memory.check_due(1_000 + CHECK_EVERY_MS - 1));
        assert!(memory.check_due(1_000 + CHECK_EVERY_MS));
        assert!(memory.check_due(500), "a clock set back");
    }

    #[test]
    fn it_is_calm_after_a_minute_of_nothing_important() {
        assert!(!calm(IDLE_BEFORE_MS - 1, false));
        assert!(calm(IDLE_BEFORE_MS, false));
        assert!(!calm(IDLE_BEFORE_MS, true));
    }

    #[test]
    fn a_manifest_carries_the_notes_in_both_languages() {
        let manifest = Manifest::parse(
            r#"{"version":"0.7.0",
                "notes":{"en":"Updates","ru":"Обновления"},
                "installer":{"url":"https://example.org/setup.exe","signature":"sig","size":42}}"#,
        )
        .expect("loads");
        assert_eq!(manifest.version, v("0.7.0"));
        assert_eq!(manifest.notes.pick("ru"), "Обновления");
        assert_eq!(manifest.notes.pick("en"), "Updates");
        assert_eq!(manifest.installer.size, 42);
    }

    #[test]
    fn notes_fall_back_to_the_other_language() {
        let notes = Notes {
            en: String::new(),
            ru: "Только русский".into(),
        };
        assert_eq!(notes.pick("en"), "Только русский");
    }

    #[test]
    fn a_manifest_without_https_or_a_signature_is_refused() {
        let plain = r#"{"version":"0.7.0","installer":{"url":"http://example.org/a.exe","signature":"sig"}}"#;
        let unsigned =
            r#"{"version":"0.7.0","installer":{"url":"https://example.org/a.exe","signature":""}}"#;
        let broken = r#"{"version":"seven","installer":{"url":"https://example.org/a.exe","signature":"sig"}}"#;
        for text in [plain, unsigned, broken, "not json"] {
            assert!(Manifest::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn an_empty_memory_file_loads() {
        let memory: Memory = serde_json::from_str("{}").expect("loads");
        assert_eq!(memory, Memory::default());
        let saved = serde_json::to_string(&Memory::default()).expect("saves");
        assert_eq!(
            serde_json::from_str::<Memory>(&saved).expect("loads"),
            Memory::default()
        );
    }
}
