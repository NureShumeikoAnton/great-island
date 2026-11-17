//! Problems the island tells the user about: what failed, why, and how loudly.
//!
//! Adapters return [`Failed`] (a cause plus the raw system error); the island turns it
//! into a [`Fault`] and reports it in one place. The level, the code and every text are
//! properties of [`What`] and [`Cause`], never of the call site.

use crate::lang::tr;
use std::{fmt, io};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// It worked with a compromise, or a click did nothing.
    Warning,
    /// The user asked for something and did not get it, or lost data.
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    RecordStart,
    RecordBroken,
    RecordLost,
    RecordStopped,
    RecordFolder,
    RecordKey,
    RecordMic,
    RecordMicLost,
    RecordSound,
    RecordHevc,
    VideoOpen,
    VideoStopped,
    VideoSound,
    VideoControl,
    VideoSave,
    RecordDelete,
    ShotTake,
    ShotSave,
    ShotFolder,
    ShotKey,
    ShotDelete,
    ShotEdit,
    ShotOpen,
    AreaOpen,
    AreaKey,
    FolderOpen,
    Copy,
    SettingsOpen,
    SettingsRestored,
    SettingsReset,
    SettingsSave,
    TimersSave,
    Sound,
    Microphone,
    Crashed,
    Restart,
    Language,
    PlayerLevel,
    UpdateCheck,
    UpdateInstall,
}

impl What {
    #[cfg(test)]
    pub const ALL: [What; 40] = [
        What::RecordStart,
        What::RecordBroken,
        What::RecordLost,
        What::RecordStopped,
        What::RecordFolder,
        What::RecordKey,
        What::RecordMic,
        What::RecordMicLost,
        What::RecordSound,
        What::RecordHevc,
        What::VideoOpen,
        What::VideoStopped,
        What::VideoSound,
        What::VideoControl,
        What::VideoSave,
        What::RecordDelete,
        What::ShotTake,
        What::ShotSave,
        What::ShotFolder,
        What::ShotKey,
        What::ShotDelete,
        What::ShotEdit,
        What::ShotOpen,
        What::AreaOpen,
        What::AreaKey,
        What::FolderOpen,
        What::Copy,
        What::SettingsOpen,
        What::SettingsRestored,
        What::SettingsReset,
        What::SettingsSave,
        What::TimersSave,
        What::Sound,
        What::Microphone,
        What::Crashed,
        What::Restart,
        What::Language,
        What::PlayerLevel,
        What::UpdateCheck,
        What::UpdateInstall,
    ];

    pub fn level(self) -> Level {
        match self {
            Self::RecordStart
            | Self::RecordBroken
            | Self::RecordLost
            | Self::ShotTake
            | Self::ShotSave
            | Self::AreaOpen
            | Self::VideoOpen
            | Self::VideoStopped
            | Self::VideoSave
            | Self::UpdateCheck
            | Self::UpdateInstall => Level::Critical,
            _ => Level::Warning,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::RecordStart => "REC-START",
            Self::RecordBroken => "REC-BROKEN",
            Self::RecordLost => "REC-LOST",
            Self::RecordStopped => "REC-STOPPED",
            Self::RecordFolder => "REC-FOLDER",
            Self::RecordKey => "REC-KEY",
            Self::RecordMic => "REC-MIC",
            Self::RecordMicLost => "REC-MIC-LOST",
            Self::RecordSound => "REC-SOUND",
            Self::RecordHevc => "REC-HEVC",
            Self::VideoOpen => "REC-OPEN",
            Self::VideoStopped => "VIDEO-STOPPED",
            Self::VideoSound => "VIDEO-SOUND",
            Self::VideoControl => "VIDEO-CONTROL",
            Self::VideoSave => "VIDEO-SAVE",
            Self::RecordDelete => "REC-DELETE",
            Self::ShotTake => "SHOT-TAKE",
            Self::ShotSave => "SHOT-SAVE",
            Self::ShotFolder => "SHOT-FOLDER",
            Self::ShotKey => "SHOT-KEY",
            Self::ShotDelete => "SHOT-DELETE",
            Self::ShotEdit => "SHOT-EDIT",
            Self::ShotOpen => "SHOT-OPEN",
            Self::AreaOpen => "AREA-OPEN",
            Self::AreaKey => "AREA-KEY",
            Self::FolderOpen => "FOLDER-OPEN",
            Self::Copy => "COPY",
            Self::SettingsOpen => "SET-OPEN",
            Self::SettingsRestored => "SET-RESTORED",
            Self::SettingsReset => "SET-RESET",
            Self::SettingsSave => "SET-SAVE",
            Self::TimersSave => "TIMERS-SAVE",
            Self::Sound => "SOUND",
            Self::Microphone => "MIC",
            Self::Crashed => "CRASH",
            Self::Restart => "RESTART",
            Self::Language => "LANG",
            Self::PlayerLevel => "PLAYER-LEVEL",
            Self::UpdateCheck => "UPD-CHECK",
            Self::UpdateInstall => "UPD-INSTALL",
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::RecordStart => tr!("fault.title.record_start"),
            Self::RecordBroken => tr!("fault.title.record_broken"),
            Self::RecordLost => tr!("fault.title.record_lost"),
            Self::RecordStopped => tr!("fault.title.record_stopped"),
            Self::RecordFolder => tr!("fault.title.record_folder"),
            Self::RecordKey => tr!("fault.title.record_key"),
            Self::RecordMic => tr!("fault.title.record_mic"),
            Self::RecordMicLost => tr!("fault.title.record_mic_lost"),
            Self::RecordSound => tr!("fault.title.record_sound"),
            Self::RecordHevc => tr!("fault.title.record_hevc"),
            Self::VideoOpen => tr!("fault.title.video_open"),
            Self::VideoStopped => tr!("fault.title.video_stopped"),
            Self::VideoSound => tr!("fault.title.video_sound"),
            Self::VideoControl => tr!("fault.title.video_control"),
            Self::VideoSave => tr!("fault.title.video_save"),
            Self::RecordDelete => tr!("fault.title.record_delete"),
            Self::ShotTake => tr!("fault.title.shot_take"),
            Self::ShotSave => tr!("fault.title.shot_save"),
            Self::ShotFolder => tr!("fault.title.shot_folder"),
            Self::ShotKey => tr!("fault.title.shot_key"),
            Self::ShotDelete => tr!("fault.title.shot_delete"),
            Self::ShotEdit => tr!("fault.title.shot_edit"),
            Self::ShotOpen => tr!("fault.title.shot_open"),
            Self::AreaOpen => tr!("fault.title.area_open"),
            Self::AreaKey => tr!("fault.title.area_key"),
            Self::FolderOpen => tr!("fault.title.folder_open"),
            Self::Copy => tr!("fault.title.copy"),
            Self::SettingsOpen => tr!("fault.title.settings_open"),
            Self::SettingsRestored => tr!("fault.title.settings_restored"),
            Self::SettingsReset => tr!("fault.title.settings_reset"),
            Self::SettingsSave => tr!("fault.title.settings_save"),
            Self::TimersSave => tr!("fault.title.timers_save"),
            Self::Sound => tr!("fault.title.sound"),
            Self::Microphone => tr!("fault.title.microphone"),
            Self::Crashed => tr!("fault.title.crashed"),
            Self::Restart => tr!("fault.title.restart"),
            Self::Language => tr!("fault.title.language"),
            Self::PlayerLevel => tr!("fault.title.player_level"),
            Self::UpdateCheck => tr!("fault.title.update_check"),
            Self::UpdateInstall => tr!("fault.title.update_install"),
        }
        .into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    DiskFull,
    NoAccess,
    Busy,
    Missing,
    Graphics,
    Unsupported,
    Taken,
    MonitorGone,
    DisplayChanged,
    Unknown,
}

impl Cause {
    #[cfg(test)]
    pub const ALL: [Cause; 10] = [
        Cause::DiskFull,
        Cause::NoAccess,
        Cause::Busy,
        Cause::Missing,
        Cause::Graphics,
        Cause::Unsupported,
        Cause::Taken,
        Cause::MonitorGone,
        Cause::DisplayChanged,
        Cause::Unknown,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::DiskFull => "DISK-FULL",
            Self::NoAccess => "NO-ACCESS",
            Self::Busy => "BUSY",
            Self::Missing => "MISSING",
            Self::Graphics => "GRAPHICS",
            Self::Unsupported => "UNSUPPORTED",
            Self::Taken => "TAKEN",
            Self::MonitorGone => "MONITOR-GONE",
            Self::DisplayChanged => "DISPLAY-CHANGED",
            Self::Unknown => "UNKNOWN",
        }
    }

    fn reason(self) -> String {
        match self {
            Self::DiskFull => tr!("fault.cause.disk_full"),
            Self::NoAccess => tr!("fault.cause.no_access"),
            Self::Busy => tr!("fault.cause.busy"),
            Self::Missing => tr!("fault.cause.missing"),
            Self::Graphics => tr!("fault.cause.graphics"),
            Self::Unsupported => tr!("fault.cause.unsupported"),
            Self::Taken => tr!("fault.cause.taken"),
            Self::MonitorGone => tr!("fault.cause.monitor_gone"),
            Self::DisplayChanged => tr!("fault.cause.display_changed"),
            Self::Unknown => tr!("fault.cause.unknown"),
        }
        .into_owned()
    }

    pub fn from_os(code: u32) -> Self {
        match code {
            // ERROR_ACCESS_DENIED, ERROR_WRITE_PROTECT, ERROR_PRIVILEGE_NOT_HELD
            5 | 19 | 1314 => Self::NoAccess,
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            39 | 112 => Self::DiskFull,
            // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
            32 | 33 => Self::Busy,
            // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_NOT_READY, ERROR_BAD_NETPATH,
            // ERROR_BAD_NET_NAME, ERROR_INVALID_NAME, ERROR_NOT_FOUND
            2 | 3 | 21 | 53 | 67 | 123 | 1168 => Self::Missing,
            // ERROR_HOTKEY_ALREADY_REGISTERED
            1409 => Self::Taken,
            // ERROR_NOT_SUPPORTED, ERROR_CALL_NOT_IMPLEMENTED
            50 | 120 => Self::Unsupported,
            _ => Self::Unknown,
        }
    }

    pub fn from_hresult(code: i32) -> Self {
        let code = code as u32;
        // FACILITY_WIN32: a Win32 code wrapped into an HRESULT.
        if code & 0xFFFF_0000 == 0x8007_0000 {
            return Self::from_os(code & 0xFFFF);
        }
        match code {
            // STG_E_ACCESSDENIED
            0x8003_0005 => Self::NoAccess,
            // STG_E_MEDIUMFULL
            0x8003_0070 => Self::DiskFull,
            // DXGI_ERROR_*: unsupported, device removed, hung, reset, driver internal error;
            // D3DDDIERR_DEVICEREMOVED.
            0x887A_0004 | 0x887A_0005 | 0x887A_0006 | 0x887A_0007 | 0x887A_0020 | 0x8876_0870 => {
                Self::Graphics
            }
            // E_NOTIMPL, E_NOINTERFACE, REGDB_E_CLASSNOTREG, MF_E_PLATFORM_NOT_INITIALIZED,
            // MF_E_UNSUPPORTED_BYTESTREAM_TYPE (no reader for the file), MF_E_TOPO_CODEC_NOT_FOUND
            // (no decoder, e.g. HEVC without its Store extension).
            0x8000_4001 | 0x8000_4002 | 0x8004_0154 | 0xC00D_36B0 | 0xC00D_36C4 | 0xC00D_5212 => {
                Self::Unsupported
            }
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed {
    pub cause: Cause,
    pub detail: String,
}

impl Failed {
    pub fn new(cause: Cause) -> Self {
        Self {
            cause,
            detail: String::new(),
        }
    }

    pub fn because(cause: Cause, detail: impl fmt::Display) -> Self {
        Self {
            cause,
            detail: detail.to_string(),
        }
    }
}

impl From<io::Error> for Failed {
    fn from(e: io::Error) -> Self {
        let cause = match e.raw_os_error() {
            Some(code) => Cause::from_os(code as u32),
            None => match e.kind() {
                io::ErrorKind::NotFound => Cause::Missing,
                io::ErrorKind::PermissionDenied => Cause::NoAccess,
                io::ErrorKind::StorageFull => Cause::DiskFull,
                _ => Cause::Unknown,
            },
        };
        Self::because(cause, e)
    }
}

impl From<windows::core::Error> for Failed {
    fn from(e: windows::core::Error) -> Self {
        Self::because(Cause::from_hresult(e.code().0), e)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub what: What,
    pub cause: Cause,
    pub subject: Option<String>,
    pub detail: String,
}

impl Fault {
    pub fn new(what: What, failed: Failed) -> Self {
        Self {
            what,
            cause: failed.cause,
            subject: None,
            detail: failed.detail,
        }
    }

    pub fn of(what: What, cause: Cause) -> Self {
        Self::new(what, Failed::new(cause))
    }

    pub fn about(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    pub fn level(&self) -> Level {
        self.what.level()
    }

    pub fn title(&self) -> String {
        self.what.title()
    }

    pub fn reason(&self) -> String {
        use Cause::*;
        use What::*;
        let fixed = match (self.what, self.cause) {
            (RecordBroken, _) => tr!("fault.reason.record_broken"),
            (RecordStopped, DiskFull) => tr!("fault.reason.record_stopped_disk_full"),
            (RecordStopped, MonitorGone) => tr!("fault.reason.record_stopped_monitor_gone"),
            (RecordStopped, DisplayChanged) => tr!("fault.reason.record_stopped_display_changed"),
            (RecordFolder, _) => tr!("fault.reason.record_folder"),
            (RecordMic, NoAccess) => tr!("fault.reason.record_mic_no_access"),
            (RecordMic, _) => tr!("fault.reason.record_mic"),
            (RecordMicLost, _) => tr!("fault.reason.record_mic_lost"),
            (RecordSound, _) => tr!("fault.reason.record_sound"),
            (RecordHevc, _) => tr!("fault.reason.record_hevc"),
            (ShotFolder, _) => tr!("fault.reason.shot_folder"),
            (RecordStart, Unsupported) => tr!("fault.reason.record_start_unsupported"),
            (RecordStart, Missing) => tr!("fault.reason.record_start_missing"),
            (ShotTake, _) => tr!("fault.reason.shot_take"),
            (Copy, Busy) => tr!("fault.reason.copy_busy"),
            (RecordKey | ShotKey | AreaKey, Taken) => {
                return match &self.subject {
                    Some(key) => tr!("fault.reason.key_taken", key = key).into_owned(),
                    None => Taken.reason(),
                };
            }
            (Sound | Microphone, _) => tr!("fault.reason.device"),
            (VideoOpen, Unsupported) => tr!("fault.reason.video_open"),
            (VideoSound, _) => tr!("fault.reason.video_sound"),
            (SettingsRestored, _) => tr!("fault.reason.settings_restored"),
            (SettingsReset, _) => tr!("fault.reason.settings_reset"),
            (Crashed, _) => tr!("fault.reason.crashed"),
            (Language, _) => tr!("fault.reason.language"),
            (PlayerLevel, _) => tr!("fault.reason.player_level"),
            (UpdateCheck, Missing) => tr!("fault.reason.update_check_missing"),
            (UpdateCheck, Unknown) => tr!("fault.reason.update_check"),
            (UpdateInstall, Unknown) => tr!("fault.reason.update_install"),
            (_, cause) => return cause.reason(),
        };
        fixed.into_owned()
    }

    pub fn code(&self) -> String {
        format!("{}/{}", self.what.code(), self.cause.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{self, Lang};
    use std::collections::HashSet;

    #[test]
    fn every_problem_has_its_own_code_and_a_title() {
        let codes: HashSet<_> = What::ALL.iter().map(|w| w.code()).collect();
        assert_eq!(codes.len(), What::ALL.len());
        let causes: HashSet<_> = Cause::ALL.iter().map(|c| c.code()).collect();
        assert_eq!(causes.len(), Cause::ALL.len());
        for lang in Lang::ALL {
            lang::with(lang, || {
                for what in What::ALL {
                    assert!(!what.title().is_empty(), "{lang:?} {what:?}");
                    // The capsule is one line on a 400 px island.
                    assert!(what.title().chars().count() <= 30, "{lang:?} {what:?}");
                }
            });
        }
    }

    #[test]
    fn every_pair_has_a_reason_that_fits_the_card() {
        for lang in Lang::ALL {
            lang::with(lang, || {
                for what in What::ALL {
                    for cause in Cause::ALL {
                        let reason = Fault::of(what, cause).about("Ctrl + Shift + R").reason();
                        let length = reason.chars().count();
                        assert!(
                            length > 0 && length <= 44,
                            "{lang:?} {what:?}/{cause:?}: {reason}"
                        );
                    }
                }
            });
        }
    }

    #[test]
    fn lost_work_is_critical_and_the_rest_is_a_warning() {
        assert_eq!(What::ShotSave.level(), Level::Critical);
        assert_eq!(What::RecordStart.level(), Level::Critical);
        assert_eq!(What::Copy.level(), Level::Warning);
        assert_eq!(What::RecordStopped.level(), Level::Warning);
    }

    #[test]
    fn a_missing_microphone_says_how_to_go_on() {
        let missing = Fault::of(What::RecordMic, Cause::Missing);
        assert_eq!(missing.level(), Level::Warning);
        assert_eq!(missing.code(), "REC-MIC/MISSING");
        lang::with(Lang::Ru, || {
            assert_eq!(missing.title(), "Микрофон недоступен");
            assert_eq!(missing.reason(), "Нажмите ещё раз, чтобы записать без него");
        });
        let blocked = Fault::of(What::RecordMic, Cause::NoAccess);
        lang::with(Lang::En, || assert!(blocked.reason().contains("Windows")));
    }

    #[test]
    fn system_codes_become_causes() {
        assert_eq!(Cause::from_os(112), Cause::DiskFull);
        assert_eq!(Cause::from_os(32), Cause::Busy);
        assert_eq!(Cause::from_os(1409), Cause::Taken);
        assert_eq!(Cause::from_hresult(0x8007_0005_u32 as i32), Cause::NoAccess);
        assert_eq!(Cause::from_hresult(0x887A_0005_u32 as i32), Cause::Graphics);
        assert_eq!(
            Cause::from_hresult(0x8004_0154_u32 as i32),
            Cause::Unsupported
        );
        assert_eq!(
            Cause::from_hresult(0xC00D_5212_u32 as i32),
            Cause::Unsupported
        );
        assert_eq!(Cause::from_hresult(0x8000_FFFF_u32 as i32), Cause::Unknown);
    }

    #[test]
    fn io_errors_keep_the_system_words_for_the_journal() {
        let failed = Failed::from(io::Error::from_raw_os_error(5));
        assert_eq!(failed.cause, Cause::NoAccess);
        assert!(!failed.detail.is_empty());
        let plain = Failed::from(io::Error::new(io::ErrorKind::NotFound, "нет"));
        assert_eq!(plain.cause, Cause::Missing);
    }

    #[test]
    fn the_key_is_named_in_the_reason() {
        let fault = Fault::of(What::RecordKey, Cause::Taken).about("Ctrl + Shift + R");
        assert_eq!(fault.code(), "REC-KEY/TAKEN");
        lang::with(Lang::Ru, || {
            assert_eq!(fault.reason(), "Ctrl + Shift + R заняла другая программа");
        });
        lang::with(Lang::En, || {
            assert_eq!(
                fault.reason(),
                "Ctrl + Shift + R is held by another program"
            );
        });
    }

    #[test]
    fn a_language_that_was_not_detected_is_a_warning_with_its_own_code() {
        let fault = Fault::of(What::Language, Cause::Unknown);
        assert_eq!(fault.code(), "LANG/UNKNOWN");
        assert_eq!(fault.level(), Level::Warning);
        lang::with(Lang::En, || {
            assert!(fault.reason().starts_with("English is used"))
        });
    }
}
