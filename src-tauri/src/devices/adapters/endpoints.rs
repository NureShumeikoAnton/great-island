//! The default sound output and input: which endpoint it is and what Windows knows about it
//! (its description, its device's name, its form factor, its container). Windows tells about a
//! new default by itself; the callback only hands its id to the watch's thread.

use super::watch::Raw;
use crate::devices::{Endpoint, Flow, container_key, form_of};
use std::sync::{Mutex, mpsc::Sender};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
            IMMNotificationClient_Impl, MMDeviceEnumerator, eCapture, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ,
            StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
        },
        UI::Shell::PropertiesSystem::IPropertyStore,
    },
    core::{GUID, HSTRING, PCWSTR, Result},
};
use windows_core::implement;

const fn key(fmtid: u128, pid: u32) -> PROPERTYKEY {
    PROPERTYKEY {
        fmtid: GUID::from_u128(fmtid),
        pid,
    }
}

/// `PKEY_Device_DeviceDesc`: what the endpoint is ("Speakers").
const DESCRIPTION: PROPERTYKEY = key(0xa45c254e_df1c_4efd_8020_67d146a850e0, 2);
/// `PKEY_DeviceInterface_FriendlyName`: its device ("CMF Buds Pro 2").
const ADAPTER: PROPERTYKEY = key(0x026e516e_b814_414b_83cd_856d6fef4822, 2);
/// `PKEY_AudioEndpoint_FormFactor`.
const FORM_FACTOR: PROPERTYKEY = key(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 0);
/// `PKEY_Device_ContainerId`.
const CONTAINER: PROPERTYKEY = key(0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c, 2);

fn data_flow(flow: Flow) -> EDataFlow {
    match flow {
        Flow::Output => eRender,
        Flow::Input => eCapture,
    }
}

pub(super) struct Endpoints {
    devices: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl Endpoints {
    pub(super) fn start(tx: &Sender<Raw>) -> Result<Self> {
        let devices: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let client: IMMNotificationClient = Changes {
            tx: Mutex::new(tx.clone()),
        }
        .into();
        unsafe { devices.RegisterEndpointNotificationCallback(&client)? };
        Ok(Self { devices, client })
    }

    /// The default endpoint's id; `None` without one (no sound device at all).
    pub(super) fn default(&self, flow: Flow) -> Option<String> {
        unsafe {
            let device = self
                .devices
                .GetDefaultAudioEndpoint(data_flow(flow), eConsole)
                .ok()?;
            let raw = device.GetId().ok()?;
            let id = raw.to_string().ok();
            CoTaskMemFree(Some(raw.0 as *const _));
            id
        }
    }

    pub(super) fn read(&self, id: &str) -> Result<Endpoint> {
        let wide = HSTRING::from(id);
        let store = unsafe {
            self.devices
                .GetDevice(PCWSTR(wide.as_ptr()))?
                .OpenPropertyStore(STGM_READ)?
        };
        Ok(Endpoint {
            id: id.to_string(),
            description: text(&store, &DESCRIPTION).unwrap_or_default(),
            adapter: text(&store, &ADAPTER).unwrap_or_default(),
            form: form_of(text(&store, &FORM_FACTOR).and_then(|t| t.parse().ok())),
            container: text(&store, &CONTAINER).map(|c| container_key(&c)),
        })
    }
}

impl Drop for Endpoints {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .devices
                .UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}

/// Any property as text: numbers and GUIDs come out in their usual form.
fn text(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut value = store.GetValue(key).ok()?;
        let shown = PropVariantToStringAlloc(&value).ok();
        let _ = PropVariantClear(&mut value);
        let shown = shown?;
        let out = shown.to_string().ok();
        CoTaskMemFree(Some(shown.0 as *const _));
        out.filter(|s| !s.is_empty())
    }
}

#[implement(IMMNotificationClient)]
struct Changes {
    tx: Mutex<Sender<Raw>>,
}

// Windows asks not to block or call back into the enumerator from these: they only pass the id
// on. The other events are of no use here: a new default always comes as its own event.
impl IMMNotificationClient_Impl for Changes_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, id: &PCWSTR) -> Result<()> {
        // Each role (console, multimedia, communications) reports apart; console is the one
        // Windows' sound menu sets.
        if role != eConsole || id.is_null() {
            return Ok(());
        }
        let flow = if flow == eCapture {
            Flow::Input
        } else {
            Flow::Output
        };
        let id = unsafe { id.to_string().unwrap_or_default() };
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(Raw::Default(flow, id));
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}
