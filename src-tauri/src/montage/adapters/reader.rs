//! Reading a recording: its facts, thumbnails (decoded and scaled on the video card) and its
//! sound as floats for the wave. A source reader per job; each runs on the caller's thread.

use super::{SECOND, attributes, manager, pack, unpack};
use crate::fault::Failed;
use std::path::Path;
use windows::{
    Win32::{
        Graphics::Direct3D11::ID3D11Device,
        Media::MediaFoundation::{
            IMF2DBuffer, IMFDXGIDeviceManager, IMFSample, IMFSourceReader,
            MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_FRAME_RATE,
            MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION,
            MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
            MF_SOURCE_READER_D3D_MANAGER, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
            MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
            MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MFAudioFormat_Float,
            MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFMediaType_Video,
            MFVideoFormat_RGB32,
        },
        System::{Com::StructuredStorage::PROPVARIANT, Variant::VT_I8},
    },
    core::{Error, GUID, HSTRING, Interface},
};

pub const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
pub const AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// `device`: decoding and scaling on the video card.
pub(super) fn open(
    path: &Path,
    device: Option<&IMFDXGIDeviceManager>,
) -> Result<IMFSourceReader, Failed> {
    let attrs = attributes(3)?;
    unsafe {
        if let Some(manager) = device {
            attrs.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        }
        // Conversion and scaling by the reader; with a device the video card does them.
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attrs)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        Ok(reader)
    }
}

pub fn info(path: &Path) -> Result<Info, Failed> {
    let reader = open(path, None)?;
    unsafe {
        let duration = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)?;
        let duration = duration.Anonymous.Anonymous.Anonymous.uhVal as f64 / SECOND;
        let native = reader.GetNativeMediaType(VIDEO, 0)?;
        let (width, height) = unpack(native.GetUINT64(&MF_MT_FRAME_SIZE)?);
        let (num, den) = unpack(native.GetUINT64(&MF_MT_FRAME_RATE)?);
        Ok(Info {
            duration,
            width,
            height,
            fps: (f64::from(num) / f64::from(den.max(1))).round() as u32,
        })
    }
}

pub(super) fn seek(reader: &IMFSourceReader, seconds: f64) -> Result<(), Failed> {
    let mut at = PROPVARIANT::default();
    unsafe {
        let inner = &mut *at.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = (seconds * SECOND) as i64;
        reader.SetCurrentPosition(&GUID::zeroed(), &at)?;
    }
    Ok(())
}

/// The next sample of `stream` and its time; `None` at the end.
pub(super) fn next(
    reader: &IMFSourceReader,
    stream: u32,
) -> Result<Option<(IMFSample, f64)>, Failed> {
    loop {
        let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
        unsafe {
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )?;
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(None);
        }
        if let Some(sample) = sample {
            return Ok(Some((sample, time as f64 / SECOND)));
        }
    }
}

/// BGRA thumbnails `size` big at `times` (keyframes decode fastest). A time past the end gives
/// no thumbnail, so the list may be shorter.
pub fn thumbnails(
    device: &ID3D11Device,
    path: &Path,
    times: &[f64],
    (width, height): (u32, u32),
) -> Result<Vec<Vec<u8>>, Failed> {
    let manager = manager(device)?;
    let reader = open(path, Some(&manager))?;
    unsafe {
        reader.SetStreamSelection(VIDEO, true)?;
        let kind = MFCreateMediaType()?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        kind.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        kind.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        reader.SetCurrentMediaType(VIDEO, None, &kind)?;
    }
    let mut thumbs = Vec::with_capacity(times.len());
    for &time in times {
        seek(&reader, time)?;
        let Some((sample, _)) = next(&reader, VIDEO)? else {
            break;
        };
        thumbs.push(pixels(&sample, (width, height))?);
    }
    Ok(thumbs)
}

fn pixels(sample: &IMFSample, (width, height): (u32, u32)) -> Result<Vec<u8>, Failed> {
    let row = width as usize * 4;
    let mut out = vec![0u8; row * height as usize];
    unsafe {
        let buffer = sample.GetBufferByIndex(0)?.cast::<IMF2DBuffer>()?;
        let (mut first, mut pitch) = (std::ptr::null_mut(), 0i32);
        buffer.Lock2D(&mut first, &mut pitch)?;
        for y in 0..height as usize {
            std::ptr::copy_nonoverlapping(
                first.offset(y as isize * pitch as isize),
                out.as_mut_ptr().add(y * row),
                row,
            );
        }
        buffer.Unlock2D()?;
    }
    for alpha in out.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    Ok(out)
}

/// The sound decoded to floats, in pieces as they come: interleaved samples with the sound's
/// `(sample rate, channels)`. `Ok(false)`: the recording has no sound. `piece` answers whether to go on.
pub fn sound(
    path: &Path,
    mut piece: impl FnMut((u32, usize), &[f32]) -> bool,
) -> Result<bool, Failed> {
    let reader = open(path, None)?;
    unsafe {
        if reader.SetStreamSelection(AUDIO, true).is_err() {
            return Ok(false);
        }
        let kind = MFCreateMediaType()?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        kind.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        reader.SetCurrentMediaType(AUDIO, None, &kind)?;
    }
    let format = unsafe {
        let current = reader.GetCurrentMediaType(AUDIO)?;
        (
            current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?,
            current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? as usize,
        )
    };
    while let Some((sample, _)) = next(&reader, AUDIO)? {
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut data, None, Some(&mut length))?;
            if data.is_null() {
                buffer.Unlock()?;
                return Err(Error::empty().into());
            }
            let more = piece(
                format,
                std::slice::from_raw_parts(data as *const f32, length as usize / 4),
            );
            buffer.Unlock()?;
            if !more {
                break;
            }
        }
    }
    Ok(true)
}
