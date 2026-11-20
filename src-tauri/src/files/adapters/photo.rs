//! Photos through Windows' own codecs (WIC): any format Windows reads, turned upright by its
//! EXIF orientation; written back in a format Windows writes, keeping the camera's metadata.
//! Reading a large photo takes a fraction of a second: not on the island's thread.

use crate::{
    capture::Image,
    fault::{Cause, Failed},
    files::{
        open::{Format, Turn},
        picture::Album,
    },
};
use std::{
    ffi::OsString,
    fs::{self, File, FileTimes},
    os::windows::fs::FileTimesExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatBmp, GUID_ContainerFormatJpeg,
            GUID_ContainerFormatPng, GUID_ContainerFormatTiff, GUID_WICPixelFormat24bppBGR,
            GUID_WICPixelFormat32bppBGRA, GUID_WICPixelFormat32bppPBGRA, IWICBitmapFrameDecode,
            IWICBitmapFrameEncode, IWICImagingFactory, IWICMetadataBlockReader,
            IWICMetadataBlockWriter, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
            WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                StructuredStorage::PROPVARIANT,
            },
            Variant::{VT_UI2, VT_UI4},
        },
    },
    core::{GUID, HSTRING, Interface, PCWSTR, Result, w},
};

/// Larger than this a picture can't be one surface on the video card (Direct3D 11's limit).
const MAX_SIDE: u32 = 16384;
const ORIENTATION: PCWSTR = w!("System.Photo.Orientation");
/// Kept when the metadata can't be copied as a whole (another container): the date and the
/// camera, by their names that every format shares.
const KEPT: [PCWSTR; 3] = [
    w!("System.Photo.DateTaken"),
    w!("System.Photo.CameraManufacturer"),
    w!("System.Photo.CameraModel"),
];

fn factory() -> Result<IWICImagingFactory> {
    unsafe {
        // Read and written on threads of their own, where COM isn't started yet.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
    }
}

fn first_frame(factory: &IWICImagingFactory, path: &Path) -> Result<IWICBitmapFrameDecode> {
    unsafe {
        let decoder = factory.CreateDecoderFromFilename(
            &HSTRING::from(path),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        decoder.GetFrame(0)
    }
}

/// Premultiplied BGRA, upright.
pub fn read(path: &Path) -> std::result::Result<Image, Failed> {
    let factory = factory()?;
    let frame = first_frame(&factory, path)?;
    let (mut width, mut height) = (0, 0);
    unsafe { frame.GetSize(&mut width, &mut height)? };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(Failed::because(
            Cause::Unsupported,
            format_args!("{width}×{height}"),
        ));
    }
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    unsafe {
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )?;
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
    }
    let image = Image {
        width,
        height,
        pixels,
    };
    Ok(Turn::of_exif(orientation(&frame)).apply(image))
}

fn orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    let read = || -> Result<u16> {
        let mut value = PROPVARIANT::default();
        unsafe {
            frame
                .GetMetadataQueryReader()?
                .GetMetadataByName(ORIENTATION, &mut value)?;
            let inner = &value.Anonymous.Anonymous;
            Ok(match inner.vt {
                VT_UI2 => inner.Anonymous.uiVal,
                VT_UI4 => inner.Anonymous.ulVal as u16,
                _ => 1,
            })
        }
    };
    read().unwrap_or(1)
}

fn container(format: Format) -> Option<GUID> {
    match format {
        Format::Png => Some(GUID_ContainerFormatPng),
        Format::Jpeg => Some(GUID_ContainerFormatJpeg),
        Format::Bmp => Some(GUID_ContainerFormatBmp),
        Format::Tiff => Some(GUID_ContainerFormatTiff),
        Format::Webp | Format::Heif | Format::Avif => None,
    }
}

fn encode(path: &Path, image: &Image, format: Format, like: Option<&Path>) -> Result<()> {
    let container = container(format).ok_or_else(windows::core::Error::empty)?;
    let factory = factory()?;
    unsafe {
        let bitmap = factory.CreateBitmapFromMemory(
            image.width,
            image.height,
            &GUID_WICPixelFormat32bppPBGRA,
            image.width * 4,
            &image.pixels,
        )?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&container, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(image.width, image.height)?;
        let opaque = image.pixels.iter().skip(3).step_by(4).all(|&a| a == 255);
        let mut pixel_format = if opaque || format == Format::Jpeg {
            GUID_WICPixelFormat24bppBGR
        } else {
            GUID_WICPixelFormat32bppBGRA
        };
        frame.SetPixelFormat(&mut pixel_format)?;
        if let Some(like) = like {
            keep_metadata(&factory, &frame, like, container)?;
        }
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()
    }
}

/// The original's metadata goes into the new frame; the pixels are already upright, so the
/// orientation becomes 1, else other programs would turn the photo once more.
fn keep_metadata(
    factory: &IWICImagingFactory,
    frame: &IWICBitmapFrameEncode,
    like: &Path,
    container: GUID,
) -> Result<()> {
    unsafe {
        let decoder = factory.CreateDecoderFromFilename(
            &HSTRING::from(like),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let same = decoder.GetContainerFormat()? == container;
        let source = decoder.GetFrame(0)?;
        let writer = frame.GetMetadataQueryWriter()?;
        if !same {
            // Another container: whole blocks don't fit it, the date and the camera do. The
            // orientation isn't copied, so there is nothing to undo.
            let reader = source.GetMetadataQueryReader()?;
            for name in KEPT {
                let mut value = PROPVARIANT::default();
                if reader.GetMetadataByName(name, &mut value).is_ok() {
                    let _ = writer.SetMetadataByName(name, &value);
                }
            }
            return Ok(());
        }
        let blocks = source.cast::<IWICMetadataBlockReader>()?;
        frame
            .cast::<IWICMetadataBlockWriter>()?
            .InitializeFromBlockReader(&blocks)?;
        let mut upright = PROPVARIANT::default();
        let inner = &mut *upright.Anonymous.Anonymous;
        inner.vt = VT_UI2;
        inner.Anonymous.uiVal = 1;
        // A file without the tag may refuse it; then there was nothing to turn either.
        let set = writer.SetMetadataByName(ORIENTATION, &upright);
        if set.is_err() && orientation(&source) != 1 {
            return set;
        }
    }
    Ok(())
}

pub struct Disk;

impl Album for Disk {
    fn write(
        &mut self,
        path: &Path,
        image: &Image,
        format: Format,
        like: &Path,
    ) -> std::result::Result<(), Failed> {
        let mut name = OsString::from(path.as_os_str());
        name.push(".tmp");
        let tmp = PathBuf::from(name);
        // Without the metadata rather than not at all: a camera's tags another codec refuses.
        let written =
            encode(&tmp, image, format, Some(like)).or_else(|_| encode(&tmp, image, format, None));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        let created = fs::metadata(path).and_then(|m| m.created()).ok();
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        // The swapped-in file is new to the disk: it keeps the original's creation date.
        if let Some(created) = created
            && let Ok(file) = File::options().write(true).open(path)
        {
            let _ = file.set_times(FileTimes::new().set_created(created));
        }
        Ok(())
    }

    fn free_name(&self, dir: &Path, stem: &str, ext: &str) -> PathBuf {
        crate::capture::adapters::free_name(dir, stem, ext)
    }

    fn recycle(&mut self, path: &Path) -> std::result::Result<bool, Failed> {
        super::bin::recycle(path)
    }
}
