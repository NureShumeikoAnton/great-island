//! Sound and microphone mute on the default device (IAudioEndpointVolume), same as the Windows tray volume button.
//!
//! Windows reports mute changes and default-device changes; then `notify` is called
//! and the island rereads the buttons.

use super::Notify;
use crate::control::Switch;
use crate::fault::{Cause, Failed};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI8, Ordering},
};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole,
            Endpoints::{
                IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                IAudioEndpointVolumeCallback_Impl,
            },
            IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
            MMDeviceEnumerator, eCapture, eCommunications, eConsole, eRender,
        },
        System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree},
    },
    core::{PCWSTR, Result},
};
use windows_core::implement;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Output,
    Input,
}

impl Flow {
    fn data_flow(self) -> EDataFlow {
        match self {
            Self::Output => eRender,
            Self::Input => eCapture,
        }
    }

    /// A call-optimized microphone may be a separate device: mute it too, or it stays live in calls.
    fn roles(self) -> &'static [ERole] {
        match self {
            Self::Output => &[eConsole],
            Self::Input => &[eConsole, eCommunications],
        }
    }
}

pub struct DeviceMute {
    flow: Flow,
    devices: IMMDeviceEnumerator,
    endpoints: Vec<(String, IAudioEndpointVolume)>,
    volume_callback: IAudioEndpointVolumeCallback,
    device_callback: IMMNotificationClient,
    stale: Arc<AtomicBool>,
    /// Why the last `set` failed, and why a default device could not be opened.
    refused: Option<Failed>,
    unopened: Vec<String>,
}

impl DeviceMute {
    pub fn new(flow: Flow, notify: Notify) -> Result<Self> {
        let devices: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let stale = Arc::new(AtomicBool::new(true));
        let device_callback: IMMNotificationClient = DefaultChanged {
            flow: flow.data_flow(),
            stale: stale.clone(),
            notify: notify.clone(),
        }
        .into();
        unsafe { devices.RegisterEndpointNotificationCallback(&device_callback)? };
        let volume_callback: IAudioEndpointVolumeCallback = MuteChanged {
            notify,
            last: AtomicI8::new(-1),
        }
        .into();
        let mut mute = Self {
            flow,
            devices,
            endpoints: Vec::new(),
            volume_callback,
            device_callback,
            stale,
            refused: None,
            unopened: Vec::new(),
        };
        mute.reconnect();
        Ok(mute)
    }

    fn reconnect(&mut self) {
        if !self.stale.swap(false, Ordering::AcqRel) {
            return;
        }
        let mut fresh = Vec::new();
        self.unopened.clear();
        for &role in self.flow.roles() {
            let (id, volume) = match self.endpoint(role) {
                Ok(found) => found,
                Err(e) => {
                    self.unopened.push(format!("{role:?}: {e}"));
                    continue;
                }
            };
            if fresh.iter().any(|(known, _)| *known == id) {
                continue;
            }
            fresh.push((id, volume));
        }
        let same = fresh.len() == self.endpoints.len()
            && fresh
                .iter()
                .zip(&self.endpoints)
                .all(|((a, _), (b, _))| a == b);
        if same {
            return;
        }
        self.disconnect();
        for (_, volume) in &fresh {
            unsafe {
                let _ = volume.RegisterControlChangeNotify(&self.volume_callback);
            }
        }
        self.endpoints = fresh;
    }

    fn endpoint(&self, role: ERole) -> Result<(String, IAudioEndpointVolume)> {
        unsafe {
            let device = self
                .devices
                .GetDefaultAudioEndpoint(self.flow.data_flow(), role)?;
            let raw = device.GetId()?;
            let id = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as *const _));
            let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            Ok((id, volume))
        }
    }

    fn disconnect(&mut self) {
        for (_, volume) in self.endpoints.drain(..) {
            unsafe {
                let _ = volume.UnregisterControlChangeNotify(&self.volume_callback);
            }
        }
    }
}

impl Switch for DeviceMute {
    /// Whether the main device is muted. No device counts as unmuted.
    fn get(&mut self) -> bool {
        self.reconnect();
        self.endpoints
            .first()
            .and_then(|(_, volume)| unsafe { volume.GetMute() }.ok())
            .is_some_and(|muted| muted.as_bool())
    }

    fn set(&mut self, on: bool) -> bool {
        self.reconnect();
        self.refused = None;
        if self.endpoints.is_empty() {
            self.refused = Some(Failed::because(
                Cause::Missing,
                format_args!(
                    "no default {:?} device ({})",
                    self.flow,
                    self.unopened.join("; ")
                ),
            ));
            return false;
        }
        // Toggle all devices even if one fails.
        for (id, volume) in &self.endpoints {
            if let Err(e) = unsafe { volume.SetMute(on, std::ptr::null()) } {
                self.refused.get_or_insert_with(|| {
                    Failed::because(
                        Cause::from_hresult(e.code().0),
                        format_args!("SetMute({on}) on {id}: {e}"),
                    )
                });
            }
        }
        self.refused.is_none()
    }

    fn refusal(&mut self) -> Option<Failed> {
        self.refused.take()
    }
}

impl Drop for DeviceMute {
    fn drop(&mut self) {
        self.disconnect();
        unsafe {
            let _ = self
                .devices
                .UnregisterEndpointNotificationCallback(&self.device_callback);
        }
    }
}

/// Notifies the island only when the muted flag changes, not on volume slider moves.
#[implement(IAudioEndpointVolumeCallback)]
struct MuteChanged {
    notify: Notify,
    /// Last seen muted flag; -1 = not seen yet.
    last: AtomicI8,
}

impl IAudioEndpointVolumeCallback_Impl for MuteChanged_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        let Some(data) = (unsafe { data.as_ref() }) else {
            return Ok(());
        };
        let muted = data.bMuted.as_bool() as i8;
        if self.last.swap(muted, Ordering::AcqRel) != muted {
            (self.notify)();
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DefaultChanged {
    flow: EDataFlow,
    stale: Arc<AtomicBool>,
    notify: Notify,
}

impl DefaultChanged_Impl {
    fn changed(&self) {
        self.stale.store(true, Ordering::Release);
        (self.notify)();
    }
}

impl IMMNotificationClient_Impl for DefaultChanged_Impl {
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
        if flow == self.flow {
            self.changed();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}
