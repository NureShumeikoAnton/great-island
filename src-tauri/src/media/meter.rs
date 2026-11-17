//! Real-time player loudness, for the equalizer bars.
//!
//! Windows computes each audio session's peak level itself, so reading costs almost nothing: no audio
//! capture, no FFT. First look for the player's own audio, else the overall system output; if neither
//! exists the bars use a decorative animation (the view decides). Windows reports output device changes,
//! and the island reconnects on them.

use super::app::{self, AppMatch};
use crate::fault::Failed;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::{PROPERTYKEY, S_OK},
        Media::Audio::{
            DEVICE_STATE, EDataFlow, ERole, Endpoints::IAudioMeterInformation, Handset, Headphones,
            Headset, IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator,
            IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
            PKEY_AudioEndpoint_FormFactor, eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ,
            StructuredStorage::PropVariantToUInt32,
        },
    },
    core::{Interface, PCWSTR, Result},
};
use windows_core::implement;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelSource {
    Player,
    System,
    None,
}

/// Peak channel levels, 0 to 1 (linear amplitude).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    pub left: f32,
    pub right: f32,
}

pub struct Meter {
    source: LevelSource,
    meters: Vec<IAudioMeterInformation>,
    stale: Arc<AtomicBool>,
    watch: Option<(IMMDeviceEnumerator, IMMNotificationClient)>,
    /// What the last `attach` looked for and found, for the journal.
    matched: String,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    pub fn new() -> Self {
        Self {
            source: LevelSource::None,
            meters: Vec::new(),
            stale: Arc::new(AtomicBool::new(false)),
            watch: None,
            matched: String::new(),
        }
    }

    pub fn describe(&self) -> &str {
        &self.matched
    }

    pub fn source(&self) -> LevelSource {
        self.source
    }

    /// The default output changed or a device went away since the last `attach`: the meters may read
    /// a device that is no longer heard, or fail.
    pub fn output_changed(&self) -> bool {
        self.stale.load(Ordering::Acquire)
    }

    /// Connects to the app's audio; if that fails, to the overall system output. An error leaves the
    /// meter without a source.
    pub fn attach(&mut self, app_id: &str) -> std::result::Result<(), Failed> {
        self.detach();
        self.stale.store(false, Ordering::Release);
        let devices: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        self.watch(&devices);
        let device = unsafe { devices.GetDefaultAudioEndpoint(eRender, eMultimedia)? };
        let app = app::resolve(app_id);
        self.meters = session_meters(&device, &app).unwrap_or_default();
        self.matched = format!("{app:?}, audio sessions: {}", self.meters.len());
        if !self.meters.is_empty() {
            self.source = LevelSource::Player;
            return Ok(());
        }
        let meter = unsafe { device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None)? };
        self.meters = vec![meter];
        self.source = LevelSource::System;
        Ok(())
    }

    pub fn detach(&mut self) {
        self.meters.clear();
        self.source = LevelSource::None;
    }

    /// Current level; `None` without a source. An error (device unplugged) resets the source until
    /// the next `attach`.
    pub fn sample(&mut self) -> std::result::Result<Option<Level>, Failed> {
        if self.meters.is_empty() {
            return Ok(None);
        }
        let mut level = Level::default();
        for meter in &self.meters {
            match read(meter) {
                Ok(l) => {
                    level.left = level.left.max(l.left);
                    level.right = level.right.max(l.right);
                }
                Err(e) => {
                    self.detach();
                    return Err(e.into());
                }
            }
        }
        Ok(Some(level))
    }

    /// Registered once. Without it the meter still reconnects, only later: after a failed read.
    fn watch(&mut self, devices: &IMMDeviceEnumerator) {
        if self.watch.is_some() {
            return;
        }
        let client: IMMNotificationClient = OutputChanged {
            stale: self.stale.clone(),
        }
        .into();
        if unsafe { devices.RegisterEndpointNotificationCallback(&client) }.is_ok() {
            self.watch = Some((devices.clone(), client));
        }
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        if let Some((devices, client)) = self.watch.take() {
            unsafe {
                let _ = devices.UnregisterEndpointNotificationCallback(&client);
            }
        }
    }
}

/// Called on a system thread: only raises the flag, the island reconnects on its own thread.
#[implement(IMMNotificationClient)]
struct OutputChanged {
    stale: Arc<AtomicBool>,
}

impl OutputChanged_Impl {
    fn changed(&self) {
        self.stale.store(true, Ordering::Release);
    }
}

impl IMMNotificationClient_Impl for OutputChanged_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, _role: ERole, _id: &PCWSTR) -> Result<()> {
        if flow == eRender {
            self.changed();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

fn read(meter: &IAudioMeterInformation) -> Result<Level> {
    unsafe {
        // An idle session (a browser keeps them for closed tabs) has no channels, and asking it for
        // any fails with E_INVALIDARG.
        let channels = meter.GetMeteringChannelCount()?.min(8) as usize;
        if channels == 0 {
            return Ok(Level::default());
        }
        let mut peaks = [0f32; 8];
        meter.GetChannelsPeakValues(&mut peaks[..channels])?;
        let left = peaks[0];
        let right = if channels > 1 { peaks[1] } else { left };
        Ok(Level { left, right })
    }
}

pub fn output_is_headphones() -> bool {
    let form = unsafe {
        default_output()
            .and_then(|device| device.OpenPropertyStore(STGM_READ))
            .and_then(|store| store.GetValue(&PKEY_AudioEndpoint_FormFactor))
            .and_then(|value| PropVariantToUInt32(&value))
    };
    form.is_ok_and(|f| {
        let f = f as i32;
        f == Headphones.0 || f == Headset.0 || f == Handset.0
    })
}

fn default_output() -> Result<IMMDevice> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        devices.GetDefaultAudioEndpoint(eRender, eMultimedia)
    }
}

/// Meters of all the app's audio sessions: browsers play from a separate process,
/// so match by exe name, not by the window's process.
fn session_meters(device: &IMMDevice, app: &AppMatch) -> Result<Vec<IAudioMeterInformation>> {
    unsafe {
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        let mut meters = Vec::new();
        for i in 0..sessions.GetCount()? {
            let Ok(control) = sessions.GetSession(i) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            // S_OK means system sounds, S_FALSE an ordinary app.
            if control2.IsSystemSoundsSession() == S_OK {
                continue;
            }
            let exe = control2.GetProcessId().ok().and_then(app::process_exe);
            let by_exe = exe.as_deref().is_some_and(|e| app.matches_exe(e));
            let by_package = app.package.as_deref().is_some_and(|family| {
                session_identifier(&control2).is_some_and(|id| id.contains(family))
            });
            if (by_exe || by_package)
                && let Ok(meter) = control.cast::<IAudioMeterInformation>()
            {
                meters.push(meter);
            }
        }
        Ok(meters)
    }
}

fn session_identifier(control: &IAudioSessionControl2) -> Option<String> {
    unsafe {
        let id = control.GetSessionIdentifier().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as _));
        text
    }
}
