//! The battery from Windows: the charge, the charger and Energy saver (GetSystemPowerStatus),
//! watts and watt-hours (the battery report of Windows.Devices.Power), Windows' own Energy
//! saver threshold (the power plan), and the power events the island's window receives.

use crate::{
    fault::{Cause, Failed},
    notify::battery::Reading,
};
use windows::{
    Devices::Power::Battery,
    Win32::{
        Foundation::{HANDLE, HLOCAL, HWND, LocalFree, WIN32_ERROR},
        System::Power::{
            GetSystemPowerStatus, HPOWERNOTIFY, PowerGetActiveScheme, PowerReadDCValueIndex,
            RegisterPowerSettingNotification, SYSTEM_POWER_STATUS,
            UnregisterPowerSettingNotification,
        },
        UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
    },
    core::GUID,
};

/// `BatteryFlag`: no system battery (a desktop), unknown.
const NO_BATTERY: u8 = 128;
const UNKNOWN: u8 = 255;
const CHARGING: u8 = 8;
/// `SystemStatusFlag`: Energy saver (battery saver before Windows 11) is on.
const SAVER_ON: u8 = 1;

fn status() -> Result<SYSTEM_POWER_STATUS, Failed> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status)? };
    Ok(status)
}

pub fn has_battery() -> Result<bool, Failed> {
    let flag = status()?.BatteryFlag;
    Ok(flag != NO_BATTERY && flag != UNKNOWN)
}

pub fn read() -> Result<Reading, Failed> {
    let s = status()?;
    if s.BatteryLifePercent == UNKNOWN {
        return Err(Failed::because(
            Cause::Missing,
            "battery percentage unknown",
        ));
    }
    let report = Battery::AggregateBattery().and_then(|b| b.GetReport()).ok();
    let field =
        |get: &dyn Fn(&windows::Devices::Power::BatteryReport) -> windows::core::Result<i32>| {
            report.as_ref().and_then(|r| get(r).ok())
        };
    Ok(Reading {
        percent: s.BatteryLifePercent.min(100),
        on_ac: s.ACLineStatus == 1,
        charging: s.BatteryFlag & CHARGING != 0,
        saver: s.SystemStatusFlag & SAVER_ON != 0,
        rate_mw: field(&|r| r.ChargeRateInMilliwatts()?.Value()),
        remaining_mwh: field(&|r| r.RemainingCapacityInMilliwattHours()?.Value())
            .and_then(|v| u32::try_from(v).ok()),
        full_mwh: field(&|r| r.FullChargeCapacityInMilliwattHours()?.Value())
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v > 0),
        // Seconds; u32::MAX while Windows has no estimate (on the charger, just unplugged).
        windows_minutes_left: (s.BatteryLifeTime != u32::MAX).then_some(s.BatteryLifeTime / 60),
    })
}

/// The power plan's Energy saver group and its "turn on at" percentage on battery.
const ENERGY_SAVER: GUID = GUID::from_u128(0xde830923_a562_41af_a086_e3a2c6bad2da);
const SAVER_THRESHOLD: GUID = GUID::from_u128(0xe69653ca_cf7f_4f05_aa73_cb833fa90ad4);

/// `None` when the plan doesn't turn Energy saver on by itself (0 = never, 100 = always).
pub fn saver_threshold() -> Result<Option<u8>, Failed> {
    let mut scheme: *mut GUID = std::ptr::null_mut();
    let mut value = 0u32;
    unsafe {
        PowerGetActiveScheme(None, &mut scheme).ok()?;
        let read = PowerReadDCValueIndex(
            None,
            Some(scheme),
            Some(&ENERGY_SAVER),
            Some(&SAVER_THRESHOLD),
            &mut value,
        );
        // The scheme GUID is allocated by Windows for us.
        let _ = LocalFree(Some(HLOCAL(scheme.cast())));
        WIN32_ERROR(read).ok()?;
    }
    Ok(u8::try_from(value).ok().filter(|v| (1..100).contains(v)))
}

/// The power source, the percentage (every percent) and Energy saver: GUID_ACDC_POWER_SOURCE,
/// GUID_BATTERY_PERCENTAGE_REMAINING, GUID_POWER_SAVING_STATUS (Windows 10) and
/// GUID_ENERGY_SAVER_STATUS (Windows 11).
const EVENTS: [GUID; 4] = [
    GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548),
    GUID::from_u128(0xa7ad8041_b45a_4cae_87a3_eecbb468a9e1),
    GUID::from_u128(0xe00958c0_c213_4ace_ac77_fecced2eeea5),
    GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000),
];

/// While it lives, the window gets WM_POWERBROADCAST / PBT_POWERSETTINGCHANGE for each of them,
/// and once right away with the current value.
pub struct Events(Vec<HPOWERNOTIFY>);

/// An event Windows doesn't know (GUID_ENERGY_SAVER_STATUS on Windows 10) is skipped; it fails
/// only when none could be subscribed.
pub fn subscribe(window: HWND) -> Result<Events, Failed> {
    let mut handles = Vec::new();
    let mut last = None;
    for guid in &EVENTS {
        match unsafe {
            RegisterPowerSettingNotification(HANDLE(window.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE)
        } {
            Ok(handle) => handles.push(handle),
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) if handles.is_empty() => Err(e.into()),
        _ => Ok(Events(handles)),
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        for handle in self.0.drain(..) {
            unsafe {
                let _ = UnregisterPowerSettingNotification(handle);
            }
        }
    }
}
