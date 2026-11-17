//! What the pre-start panel needs from Windows about microphones: the list of the plugged-in
//! ones, and a [`Meter`] that listens to one for the panel's bars.

use super::sound::{
    Heard, Opened, WAIT_MS, default_device, device_id, drain, enumerator, plugged, start,
};
use crate::{
    fault::Failed,
    record::{meter::BARS, panel::Microphone, sound::Input},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            DEVICE_STATE_ACTIVE, IAudioClient, IMMDevice, eCapture,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                STGM_READ,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            },
            Threading::WaitForMultipleObjects,
        },
    },
    core::{Error, GUID, Result},
};

/// PKEY_Device_FriendlyName: its own crate feature would bring a whole module for one key.
const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

/// The microphones that are plugged in and on, by the names Windows shows.
pub fn microphones() -> std::result::Result<Vec<Microphone>, Failed> {
    let list = unsafe { enumerator()?.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)? };
    let mut found = Vec::new();
    for i in 0..unsafe { list.GetCount()? } {
        let Ok(device) = (unsafe { list.Item(i) }) else {
            continue;
        };
        let Ok(id) = device_id(&device) else {
            continue;
        };
        let name = friendly_name(&device).unwrap_or_else(|| id.clone());
        found.push(Microphone { id, name });
    }
    Ok(found)
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&FRIENDLY_NAME).ok()?;
        let text = PropVariantToStringAlloc(&value).ok();
        let _ = PropVariantClear(&mut value);
        let text = text?;
        let name = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as _));
        name.filter(|n| !n.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hearing {
    Opening,
    Live,
    /// The microphone can't be opened, or stopped answering.
    Failed,
}

struct Peaks {
    bars: [f32; BARS],
    hearing: Hearing,
}

/// Listens to one microphone for the panel's bars. Each packet goes to the next bar in
/// turn, so the bars move a little apart. Dropping it stops listening.
pub struct Meter {
    stop: Arc<AtomicBool>,
    peaks: Arc<Mutex<Peaks>>,
    thread: Option<JoinHandle<()>>,
}

impl Meter {
    /// `microphone` — the device as it is, `None` — the Windows default.
    pub fn start(microphone: Option<String>) -> std::result::Result<Self, Failed> {
        let stop = Arc::new(AtomicBool::new(false));
        let peaks = Arc::new(Mutex::new(Peaks {
            bars: [0.0; BARS],
            hearing: Hearing::Opening,
        }));
        let (flag, into) = (stop.clone(), peaks.clone());
        let thread = std::thread::Builder::new()
            .name("record-meter".into())
            .spawn(move || meter(microphone, &flag, &into))?;
        Ok(Self {
            stop,
            peaks,
            thread: Some(thread),
        })
    }

    /// The loudest sample of each bar since the last call, 0 to 1.
    pub fn take(&self) -> ([f32; BARS], Hearing) {
        match self.peaks.lock() {
            Ok(mut peaks) => (std::mem::take(&mut peaks.bars), peaks.hearing),
            Err(_) => ([0.0; BARS], Hearing::Failed),
        }
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn meter(microphone: Option<String>, stop: &AtomicBool, peaks: &Mutex<Peaks>) {
    // The audio objects live on this thread only: COM is set up here, not on the island's.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let set = |hearing| {
        if let Ok(mut p) = peaks.lock() {
            p.hearing = hearing;
        }
    };
    let open = || -> Result<Opened> {
        let (device, id) = match &microphone {
            Some(id) => (plugged(id).ok_or_else(Error::empty)?, id.clone()),
            None => default_device(eCapture)?,
        };
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        start(
            client,
            Some(id),
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        )
    };
    match open() {
        Ok(opened) => {
            set(Hearing::Live);
            let next = std::cell::Cell::new(0usize);
            let take = |heard: Heard| {
                let Heard::Packet { samples, .. } = heard else {
                    return;
                };
                let loudest = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
                let bar = next.get() % BARS;
                next.set(bar + 1);
                if let Ok(mut p) = peaks.lock() {
                    p.bars[bar] = p.bars[bar].max(f32::from(loudest) / 32768.0);
                }
            };
            while !stop.load(Ordering::Acquire) {
                unsafe {
                    WaitForMultipleObjects(&[opened.event], false, WAIT_MS);
                }
                if drain(Input::Microphone, &opened.capture, &take).is_err() {
                    set(Hearing::Failed);
                    break;
                }
            }
        }
        Err(_) => set(Hearing::Failed),
    }
    unsafe { CoUninitialize() };
}
