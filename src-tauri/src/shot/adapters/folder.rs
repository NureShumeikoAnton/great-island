//! Screenshots folder: PNG through the built-in Windows encoder (WIC), no libraries of our own.

use super::super::{Gallery, file_stem};
use crate::{
    capture::{
        Image, Stamp,
        adapters::{free_name, known_folder},
    },
    fault::Failed,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::GENERIC_WRITE,
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR,
            GUID_WICPixelFormat32bppBGR, IWICImagingFactory, WICBitmapEncoderNoCache,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        },
        UI::Shell::{FOLDERID_Pictures, FOLDERID_Screenshots},
    },
    core::{HSTRING, Result},
};

pub struct Folder {
    dir: PathBuf,
    fallback: Option<Failed>,
}

impl Folder {
    /// `custom` is the folder from the settings; an empty string = the system Screenshots folder.
    pub fn new(custom: &str) -> Self {
        let custom = custom.trim();
        let dir = if custom.is_empty() {
            default_dir()
        } else {
            PathBuf::from(custom)
        };
        Self {
            dir,
            fallback: None,
        }
    }

    pub fn take_fallback(&mut self) -> Option<Failed> {
        self.fallback.take()
    }
}

fn save_into(dir: &Path, image: &Image, stamp: Stamp) -> std::result::Result<PathBuf, Failed> {
    fs::create_dir_all(dir)?;
    let path = free_name(dir, &file_stem(&stamp), "png");
    if let Err(e) = write_png(image, &path) {
        let _ = fs::remove_file(&path);
        return Err(e.into());
    }
    Ok(path)
}

impl Gallery for Folder {
    fn save(&mut self, image: &Image, stamp: Stamp) -> std::result::Result<PathBuf, Failed> {
        let failed = match save_into(&self.dir, image, stamp) {
            Ok(path) => return Ok(path),
            Err(failed) => failed,
        };
        // A shot already taken is worth more than the folder it was meant for.
        let default = default_dir();
        if default == self.dir {
            return Err(failed);
        }
        let path = save_into(&default, image, stamp)?;
        self.fallback = Some(failed);
        Ok(path)
    }

    fn remove(&mut self, path: &Path) -> std::result::Result<(), Failed> {
        match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    fn write(&mut self, path: &Path, image: &Image) -> std::result::Result<(), Failed> {
        // Written next to it first, then swapped in: if the write fails, the previous file stays intact.
        let tmp = path.with_extension("png.tmp");
        let written = write_png(image, &tmp)
            .map_err(Failed::from)
            .and_then(|()| fs::rename(&tmp, path).map_err(Failed::from));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written
    }
}

/// The system Screenshots folder (Pictures\\Screenshots): Win+PrtScn saves there too.
pub fn default_dir() -> PathBuf {
    known_folder(&FOLDERID_Screenshots)
        .or_else(|| known_folder(&FOLDERID_Pictures).map(|p| p.join("Screenshots")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn write_png(image: &Image, path: &Path) -> Result<()> {
    unsafe {
        // Saving also runs on a separate thread, where COM isn't started yet.
        // On the island's thread this call changes nothing.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = factory.CreateBitmapFromMemory(
            image.width,
            image.height,
            &GUID_WICPixelFormat32bppBGR,
            image.width * 4,
            &image.pixels,
        )?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(image.width, image.height)?;
        // Without transparency the file is smaller: 24 bits per pixel. The encoder does the conversion.
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()
    }
}
