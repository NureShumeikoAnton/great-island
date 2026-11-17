//! The update rules: which version counts as newer, what the user may do with an offer, when a check
//! may run. No network and no Windows: a manifest text and the clock come in, a decision goes out.
//! Fetching, checking the signature and installing are the adapters' job (`adapters/`, later).

pub mod signature;

use crate::fault::{Cause, Failed};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

pub const POSTPONE_MS: u64 = 60 * 60 * 1000;
/// A day of one-hour postponements; after that the offer can only be taken.
pub const MAX_POSTPONES: u32 = 24;
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

#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Installer {
    pub url: String,
    /// The text of the `.sig` file made when the release was signed.
    pub signature: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
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
        if !manifest.installer.url.starts_with("https://") || manifest.installer.signature.is_empty()
        {
            return Err(Failed::because(
                Cause::Unknown,
                "update manifest: the installer needs an https address and a signature",
            ));
        }
        Ok(manifest)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Memory {
    /// Unix ms of the last check that reached the server.
    pub checked_at: Option<u64>,
    /// The version the postponements below were made for: a newer one starts over.
    pub offered: Option<Version>,
    pub postponed: u32,
    /// Unix ms.
    pub postponed_until: Option<u64>,
    pub skipped: Option<Version>,
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

    /// Returns when the offer comes back (unix ms).
    pub fn postpone(&mut self, version: Version, now: u64) -> u64 {
        if self.offered != Some(version) {
            self.offered = Some(version);
            self.postponed = 0;
        }
        self.postponed += 1;
        let until = now + POSTPONE_MS;
        self.postponed_until = Some(until);
        until
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

pub fn judge(current: Version, latest: Version, memory: &Memory, now: u64) -> Verdict {
    let Some(bump) = Bump::between(current, latest) else {
        return Verdict::UpToDate;
    };
    if bump.can_skip() && memory.skipped == Some(latest) {
        return Verdict::Skipped;
    }
    let ours = memory.offered == Some(latest);
    if ours
        && let Some(until) = memory.postponed_until
        // A clock set back must not hide the offer for longer than one postponement.
        && until > now
        && until - now <= POSTPONE_MS
    {
        return Verdict::Waiting { until };
    }
    let used = if ours { memory.postponed } else { 0 };
    Verdict::Offer(Offer {
        bump,
        can_skip: bump.can_skip(),
        can_postpone: used < MAX_POSTPONES,
    })
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

    #[test]
    fn a_postponed_offer_waits_an_hour() {
        let mut memory = Memory::default();
        let until = memory.postpone(v("0.7.0"), 1_000);
        assert_eq!(until, 1_000 + POSTPONE_MS);
        assert_eq!(
            judge(v("0.6.1"), v("0.7.0"), &memory, 2_000),
            Verdict::Waiting { until }
        );
        assert!(matches!(
            judge(v("0.6.1"), v("0.7.0"), &memory, until),
            Verdict::Offer(_)
        ));
    }

    #[test]
    fn a_clock_set_back_does_not_hide_the_offer() {
        let mut memory = Memory::default();
        memory.postpone(v("0.7.0"), 10 * POSTPONE_MS);
        assert!(matches!(
            judge(v("0.6.1"), v("0.7.0"), &memory, 0),
            Verdict::Offer(_)
        ));
    }

    #[test]
    fn the_twenty_fourth_postponement_is_the_last() {
        let mut memory = Memory::default();
        for round in 1..=MAX_POSTPONES {
            let now = u64::from(round) * POSTPONE_MS;
            let Verdict::Offer(offer) = judge(v("0.6.1"), v("0.7.0"), &memory, now) else {
                panic!("round {round}: an offer");
            };
            assert!(offer.can_postpone, "round {round}");
            memory.postpone(v("0.7.0"), now);
        }
        let now = u64::from(MAX_POSTPONES + 1) * POSTPONE_MS;
        let Verdict::Offer(offer) = judge(v("0.6.1"), v("0.7.0"), &memory, now) else {
            panic!("an offer");
        };
        assert!(!offer.can_postpone);
    }

    #[test]
    fn a_newer_version_starts_the_postponements_over() {
        let mut memory = Memory::default();
        for _ in 0..MAX_POSTPONES {
            memory.postpone(v("0.7.0"), 0);
        }
        let Verdict::Offer(offer) = judge(v("0.6.1"), v("0.7.1"), &memory, 0) else {
            panic!("an offer");
        };
        assert!(offer.can_postpone);
        memory.postpone(v("0.7.1"), 0);
        assert_eq!(memory.postponed, 1);
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
        let plain =
            r#"{"version":"0.7.0","installer":{"url":"http://example.org/a.exe","signature":"sig"}}"#;
        let unsigned =
            r#"{"version":"0.7.0","installer":{"url":"https://example.org/a.exe","signature":""}}"#;
        let broken =
            r#"{"version":"seven","installer":{"url":"https://example.org/a.exe","signature":"sig"}}"#;
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
