//! Media Engine: decodes and presents the video on the video card into a swap chain of its own,
//! which the window shows as a Composition surface. Nothing in our process runs per frame.
//!
//! The engine calls back on Media Foundation's threads with a bare [`Note`]; the island posts it
//! to its own thread and hands it to [`Video::event`], which reads the engine's state there.

use super::super::{Engine, Event, Info};
use crate::{
    capture::Image,
    fault::{Cause, Failed},
    gpu,
};
use std::{cell::Cell, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::{HANDLE, RECT},
        Graphics::{Direct3D11::ID3D11Device, Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM},
        Media::MediaFoundation::{
            CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine,
            IMFMediaEngineClassFactory, IMFMediaEngineEx, IMFMediaEngineNotify,
            IMFMediaEngineNotify_Impl, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER,
            MF_MEDIA_ENGINE_ERR, MF_MEDIA_ENGINE_ERR_DECODE, MF_MEDIA_ENGINE_ERR_NETWORK,
            MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED, MF_MEDIA_ENGINE_EVENT,
            MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR,
            MF_MEDIA_ENGINE_EVENT_FRAMESTEPCOMPLETED, MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA,
            MF_MEDIA_ENGINE_EVENT_PAUSE, MF_MEDIA_ENGINE_EVENT_PLAYING,
            MF_MEDIA_ENGINE_EVENT_RESOURCELOST, MF_MEDIA_ENGINE_EVENT_SEEKED,
            MF_MEDIA_ENGINE_EVENT_STREAMRENDERINGERROR, MF_MEDIA_ENGINE_EVENT_TIMEUPDATE,
            MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE, MF_MEDIA_ENGINE_SEEK_MODE_NORMAL,
            MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION, MFCreateAttributes,
            MFCreateDXGIDeviceManager, MFSTARTUP_NOSOCKET, MFShutdown, MFStartup,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::{BSTR, Error, HRESULT, Interface, Result, implement},
};

/// What the engine said, as plain numbers: they cross threads and fit a window message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub event: u32,
    pub param1: usize,
    pub param2: u32,
}

impl Note {
    /// (wparam, lparam) of a posted message.
    pub fn pack(self) -> (usize, isize) {
        let wparam = ((self.event as u64) << 32 | self.param2 as u64) as usize;
        (wparam, self.param1 as isize)
    }

    pub fn unpack(wparam: usize, lparam: isize) -> Self {
        let w = wparam as u64;
        Self {
            event: (w >> 32) as u32,
            param1: lparam as usize,
            param2: w as u32,
        }
    }
}

pub struct Video {
    engine: IMFMediaEngine,
    extended: IMFMediaEngineEx,
    device: ID3D11Device,
    _manager: IMFDXGIDeviceManager,
    size: Cell<(u32, u32)>,
    // Last: Media Foundation stops after the engine is gone.
    _started: Started,
}

impl Video {
    /// Starts loading `path`; [`Event::Loaded`] or [`Event::Failed`] follows through `notify`.
    pub fn open(path: &Path, notify: impl Fn(Note) + Send + Sync + 'static) -> Result<Self> {
        let started = Started::new()?;
        let device = gpu::video_device()?;
        let mut token = 0;
        let mut manager = None;
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager)? };
        let manager = manager.ok_or_else(Error::empty)?;
        unsafe { manager.ResetDevice(&device, token)? };

        let callback: IMFMediaEngineNotify = Notify {
            send: Box::new(notify),
        }
        .into();
        let mut attributes: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attributes, 3)? };
        let attributes = attributes.ok_or_else(Error::empty)?;
        let engine = unsafe {
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &callback)?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attributes.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            factory.CreateInstance(0, &attributes)?
        };
        let extended: IMFMediaEngineEx = engine.cast()?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        unsafe {
            // Before the source: the engine picks its presenting path when it loads.
            extended.EnableWindowlessSwapchainMode(true)?;
            engine.SetSource(&BSTR::from_wide(&wide))?;
        }
        Ok(Self {
            engine,
            extended,
            device,
            _manager: manager,
            size: Cell::new((0, 0)),
            _started: started,
        })
    }

    /// What a note means, read from the engine on the island's thread. `None` = nothing for the player.
    pub fn event(&self, note: Note) -> Option<Event> {
        let event = MF_MEDIA_ENGINE_EVENT(note.event as i32);
        let time = || unsafe { self.engine.GetCurrentTime() };
        Some(match event {
            MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA => match self.loaded() {
                Ok(info) => Event::Loaded(info),
                Err(e) => Event::Failed(e.into()),
            },
            MF_MEDIA_ENGINE_EVENT_PLAYING => Event::Playing(time()),
            MF_MEDIA_ENGINE_EVENT_PAUSE => Event::Paused(time()),
            MF_MEDIA_ENGINE_EVENT_TIMEUPDATE => Event::Time(time()),
            MF_MEDIA_ENGINE_EVENT_SEEKED | MF_MEDIA_ENGINE_EVENT_FRAMESTEPCOMPLETED => {
                Event::Seeked(time())
            }
            MF_MEDIA_ENGINE_EVENT_ENDED => Event::Ended,
            MF_MEDIA_ENGINE_EVENT_ERROR | MF_MEDIA_ENGINE_EVENT_STREAMRENDERINGERROR => {
                Event::Failed(failure(note))
            }
            MF_MEDIA_ENGINE_EVENT_RESOURCELOST => {
                Event::Failed(Failed::because(Cause::Graphics, "video device lost"))
            }
            _ => return None,
        })
    }

    fn loaded(&self) -> Result<Info> {
        let (mut width, mut height) = (0, 0);
        let picture = unsafe { self.engine.HasVideo().as_bool() };
        if picture {
            unsafe {
                self.engine
                    .GetNativeVideoSize(Some(&mut width), Some(&mut height))?;
                // The swap chain at the video's own size: the window scales it as a whole.
                let dst = RECT {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                };
                self.extended.UpdateVideoStream(None, Some(&dst), None)?;
            }
        }
        self.size.set((width, height));
        Ok(Info {
            duration: unsafe { self.engine.GetDuration() },
            size: (width, height),
            picture,
            sound: unsafe { self.engine.HasAudio().as_bool() },
        })
    }

    /// Lets the file go before the engine is dropped (the edited video replaces it).
    pub fn close(&self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }

    /// The swap chain the engine presents into, for a Composition surface. Owned by the engine:
    /// valid while it lives and not to be closed.
    pub fn swap_chain(&self) -> Result<HANDLE> {
        unsafe { self.extended.GetVideoSwapchainHandle() }
    }

    /// The frame on screen now, at the video's size.
    pub fn frame(&self) -> Result<Image> {
        let (width, height) = self.size.get();
        if width == 0 || height == 0 {
            return Err(Error::empty());
        }
        let texture = gpu::texture(&self.device, (width, height))?;
        let dst = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        unsafe { self.engine.TransferVideoFrame(&texture, None, &dst, None)? };
        Ok(Image {
            width,
            height,
            pixels: gpu::read_back(&self.device, &texture, (width, height))?,
        })
    }
}

impl Engine for Video {
    fn play(&mut self) -> std::result::Result<(), Failed> {
        unsafe { self.engine.Play() }.map_err(Failed::from)
    }

    fn pause(&mut self) -> std::result::Result<(), Failed> {
        unsafe { self.engine.Pause() }.map_err(Failed::from)
    }

    fn seek(&mut self, to: f64, exact: bool) -> std::result::Result<(), Failed> {
        let mode = if exact {
            MF_MEDIA_ENGINE_SEEK_MODE_NORMAL
        } else {
            MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE
        };
        unsafe { self.extended.SetCurrentTimeEx(to, mode) }.map_err(Failed::from)
    }

    fn set_rate(&mut self, rate: f64) -> std::result::Result<(), Failed> {
        // Play() resets the rate to the default one (seen live): both are set.
        unsafe {
            self.engine.SetDefaultPlaybackRate(rate)?;
            self.engine.SetPlaybackRate(rate)?;
        }
        Ok(())
    }

    fn set_volume(&mut self, volume: f64, muted: bool) -> std::result::Result<(), Failed> {
        unsafe {
            self.engine.SetVolume(volume)?;
            self.engine.SetMuted(muted)?;
        }
        Ok(())
    }

    fn step(&mut self, forward: bool) -> std::result::Result<(), Failed> {
        unsafe { self.extended.FrameStep(forward) }.map_err(Failed::from)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        // Without it the engine keeps its threads and the file open after the last reference.
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

/// ERROR carries the engine's own code in `param1` and the system's in `param2`.
fn failure(note: Note) -> Failed {
    let hresult = HRESULT(note.param2 as i32);
    if hresult.is_err() {
        return Error::from_hresult(hresult).into();
    }
    let cause = match MF_MEDIA_ENGINE_ERR(note.param1 as i32) {
        MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED | MF_MEDIA_ENGINE_ERR_DECODE => Cause::Unsupported,
        MF_MEDIA_ENGINE_ERR_NETWORK => Cause::Missing,
        _ => Cause::Unknown,
    };
    Failed::because(cause, format!("media engine error {}", note.param1))
}

struct Started;

impl Started {
    fn new() -> Result<Self> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)? };
        Ok(Self)
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

#[implement(IMFMediaEngineNotify)]
struct Notify {
    send: Box<dyn Fn(Note) + Send + Sync>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> Result<()> {
        (self.send)(Note {
            event,
            param1,
            param2,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_survives_a_window_message() {
        let note = Note {
            event: 1014,
            param1: 4,
            param2: 0xC00D_5212,
        };
        let (wparam, lparam) = note.pack();
        assert_eq!(Note::unpack(wparam, lparam), note);
    }
}
