//! System notifications: what the island tells about the computer by itself, the battery and
//! the load. Rules only, under tests: `battery` turns power readings into notices, `drain`
//! knows the usual drain and when it is too fast, `usage` which apps hold the processor.
//! [`allow`] decides whether a kind may show now, for these, for downloads
//! (`crate::download`) and for devices (`crate::devices`); [`Waiting`] keeps the notices a
//! recording holds.
//!
//! The Windows side is in `adapters/`.

pub mod adapters;
pub mod battery;
pub mod drain;
pub mod usage;

pub use usage::Share;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    LowCharge,
    /// Plugged in, unplugged, charged, slow charging.
    Charging,
    Usage,
    Downloads,
    /// A device connected or left.
    Devices,
    DeviceBattery,
    /// Windows moved the sound or the microphone to another device.
    AudioSwitch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enabled {
    pub low_charge: bool,
    pub charging: bool,
    pub usage: bool,
    pub downloads: bool,
    pub devices: bool,
    pub device_battery: bool,
    pub audio_switch: bool,
}

impl Enabled {
    pub fn allows(self, kind: Kind) -> bool {
        match kind {
            Kind::LowCharge => self.low_charge,
            Kind::Charging => self.charging,
            Kind::Usage => self.usage,
            Kind::Downloads => self.downloads,
            Kind::Devices => self.devices,
            Kind::DeviceBattery => self.device_battery,
            Kind::AudioSwitch => self.audio_switch,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Low {
        percent: u8,
        saver: bool,
    },
    Critical {
        percent: u8,
        minutes_left: Option<u32>,
    },
    Plugged {
        percent: u8,
    },
    Unplugged {
        percent: u8,
        minutes_left: Option<u32>,
    },
    /// Full, or stopped at the charge limit Windows or the maker set.
    Charged {
        percent: u8,
    },
    SlowCharge {
        percent: u8,
    },
    Drain {
        minutes_left: Option<u32>,
        watts: f32,
        apps: Vec<Share>,
    },
    Load {
        cpu: u8,
        apps: Vec<Share>,
    },
}

/// Where a notice sits on the island. One slot holds one notice: a newer one replaces the
/// older (9% replaces 10%), different slots queue on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Low,
    Critical,
    Power,
    SlowCharge,
    Usage,
}

impl Notice {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Low { .. } | Self::Critical { .. } => Kind::LowCharge,
            Self::Plugged { .. }
            | Self::Unplugged { .. }
            | Self::Charged { .. }
            | Self::SlowCharge { .. } => Kind::Charging,
            Self::Drain { .. } | Self::Load { .. } => Kind::Usage,
        }
    }

    pub fn slot(&self) -> Slot {
        match self {
            Self::Low { .. } => Slot::Low,
            Self::Critical { .. } => Slot::Critical,
            Self::Plugged { .. } | Self::Unplugged { .. } | Self::Charged { .. } => Slot::Power,
            Self::SlowCharge { .. } => Slot::SlowCharge,
            Self::Drain { .. } | Self::Load { .. } => Slot::Usage,
        }
    }

    /// Shows through Do Not Disturb, a fullscreen app and a recording.
    pub fn urgent(&self) -> bool {
        matches!(self, Self::Critical { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub quiet: bool,
    /// A fullscreen app, unless the island is set to show as usual over it.
    pub fullscreen: bool,
    pub recording: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Show,
    /// After the recording, if still fresh.
    Wait,
    Drop,
}

pub fn gate(notice: &Notice, enabled: Enabled, context: Context) -> Verdict {
    allow(notice.kind(), notice.urgent(), enabled, context)
}

/// `urgent`: shows through Do Not Disturb, a fullscreen app and a recording.
pub fn allow(kind: Kind, urgent: bool, enabled: Enabled, context: Context) -> Verdict {
    if !enabled.allows(kind) {
        Verdict::Drop
    } else if urgent {
        Verdict::Show
    } else if context.quiet || context.fullscreen {
        Verdict::Drop
    } else if context.recording {
        Verdict::Wait
    } else {
        Verdict::Show
    }
}

/// Older than this, a held notice is no longer news when the recording ends.
pub const STALE_MS: u64 = 120_000;

/// Notices held by a recording: the newest per slot.
#[derive(Debug, Default)]
pub struct Waiting {
    held: Vec<(Notice, u64)>,
}

impl Waiting {
    pub fn hold(&mut self, notice: Notice, now: u64) {
        self.held.retain(|(n, _)| n.slot() != notice.slot());
        self.held.push((notice, now));
    }

    /// The fresh ones, oldest first; the rest are dropped.
    pub fn release(&mut self, now: u64) -> Vec<Notice> {
        std::mem::take(&mut self.held)
            .into_iter()
            .filter(|(_, at)| now.saturating_sub(*at) < STALE_MS)
            .map(|(n, _)| n)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Enabled = Enabled {
        low_charge: true,
        charging: true,
        usage: true,
        downloads: true,
        devices: true,
        device_battery: true,
        audio_switch: true,
    };

    fn low() -> Notice {
        Notice::Low {
            percent: 20,
            saver: false,
        }
    }

    fn critical() -> Notice {
        Notice::Critical {
            percent: 9,
            minutes_left: None,
        }
    }

    #[test]
    fn only_the_critical_charge_breaks_through() {
        for context in [
            Context {
                quiet: true,
                ..Context::default()
            },
            Context {
                fullscreen: true,
                ..Context::default()
            },
        ] {
            assert_eq!(gate(&low(), ALL, context), Verdict::Drop);
            assert_eq!(gate(&critical(), ALL, context), Verdict::Show);
        }
        let recording = Context {
            recording: true,
            ..Context::default()
        };
        assert_eq!(gate(&low(), ALL, recording), Verdict::Wait);
        assert_eq!(gate(&critical(), ALL, recording), Verdict::Show);
        assert_eq!(gate(&low(), ALL, Context::default()), Verdict::Show);
    }

    #[test]
    fn a_switched_off_kind_never_shows() {
        let off = Enabled {
            low_charge: false,
            ..ALL
        };
        assert_eq!(gate(&critical(), off, Context::default()), Verdict::Drop);
        let plugged = Notice::Plugged { percent: 40 };
        assert_eq!(gate(&plugged, off, Context::default()), Verdict::Show);
        let no_downloads = Enabled {
            downloads: false,
            ..ALL
        };
        let calm = Context::default();
        assert_eq!(
            allow(Kind::Downloads, false, no_downloads, calm),
            Verdict::Drop
        );
        assert_eq!(allow(Kind::Downloads, false, ALL, calm), Verdict::Show);
    }

    #[test]
    fn a_recording_keeps_the_newest_per_slot_while_fresh() {
        let mut waiting = Waiting::default();
        waiting.hold(Notice::Plugged { percent: 40 }, 0);
        waiting.hold(low(), 1_000);
        waiting.hold(
            Notice::Low {
                percent: 15,
                saver: true,
            },
            90_000,
        );
        let released = waiting.release(STALE_MS + 500);
        assert_eq!(
            released,
            [Notice::Low {
                percent: 15,
                saver: true
            }]
        );
        assert!(waiting.release(STALE_MS + 600).is_empty());
    }
}
