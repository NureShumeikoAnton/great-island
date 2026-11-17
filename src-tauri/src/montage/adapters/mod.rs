//! The Windows side of the editor: Media Foundation reads a recording (its facts, thumbnails, the
//! sound for the wave) and writes the edited one.

pub mod reader;
pub mod writer;

use crate::fault::Failed;
pub use crate::gpu::video_device;
use windows::{
    Win32::{
        Graphics::Direct3D11::ID3D11Device,
        Media::MediaFoundation::{
            IMFAttributes, IMFDXGIDeviceManager, MF_VERSION, MFCreateAttributes,
            MFCreateDXGIDeviceManager, MFSTARTUP_NOSOCKET, MFShutdown, MFStartup,
        },
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    },
    core::Error,
};

/// COM and Media Foundation for a thread of its own that reads a recording. Media Foundation
/// counts its starts: the window closing meanwhile doesn't stop it under the reader.
pub struct Session;

impl Session {
    pub fn start() -> Result<Self, Failed> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) {
                CoUninitialize();
                return Err(e.into());
            }
        }
        Ok(Self)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

/// Sample times and durations are in 100 ns units.
const SECOND: f64 = 10_000_000.0;

fn attributes(count: u32) -> Result<IMFAttributes, Failed> {
    let mut attrs = None;
    unsafe { MFCreateAttributes(&mut attrs, count)? };
    Ok(attrs.ok_or_else(Error::empty)?)
}

fn manager(device: &ID3D11Device) -> Result<IMFDXGIDeviceManager, Failed> {
    unsafe {
        let mut token = 0;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.ok_or_else(Error::empty)?;
        manager.ResetDevice(device, token)?;
        Ok(manager)
    }
}

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}
