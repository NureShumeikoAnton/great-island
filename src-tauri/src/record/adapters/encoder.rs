//! H.264 or HEVC into MP4 with Media Foundation, the video card's encoder when there is one.
//!
//! Frames go in as BGRA textures of the video's size. On the GPU path the sink writer converts them to the
//! encoder's format on the video card; if the driver refuses that, frames are read back
//! and the conversion runs on the CPU (heavier, but records).
//!
//! The file always has an AAC sound track, silent when nothing is recorded: Telegram sends
//! a video without one as a GIF.

use super::super::{
    Codec, Plan, SOUND_BITRATE,
    sound::{CHANNELS, RATE},
};
use crate::{gpu, lang::tr};
use std::path::Path;
use windows::{
    Win32::{
        Graphics::Direct3D11::{
            D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, ID3D11Device,
            ID3D11DeviceContext, ID3D11Texture2D,
        },
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonQuality, CODECAPI_AVEncCommonRateControlMode,
            CODECAPI_AVEncMPVGOPSize, IMF2DBuffer, IMFAttributes, IMFByteStream,
            IMFDXGIDeviceManager, IMFMediaBuffer, IMFSinkWriter, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
            MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE,
            MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
            MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
            MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_D3D_MANAGER,
            MF_TRANSCODE_CONTAINERTYPE, MFAudioFormat_AAC, MFAudioFormat_PCM, MFCreateAttributes,
            MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType,
            MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Audio,
            MFMediaType_Video, MFTranscodeContainerType_MPEG4, MFVideoFormat_H264,
            MFVideoFormat_HEVC, MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
            eAVEncCommonRateControlMode_Quality, eAVEncH264VProfile_High,
            eAVEncH265VProfile_Main_420_8,
        },
    },
    core::{Error, GUID, HSTRING, Interface, Result},
};

/// Encoder quality, 0–100. Text on screen stays crisp; still parts cost almost nothing.
const QUALITY: u32 = 70;

/// Sample times and durations are in 100 ns units.
pub const SECOND: u64 = 10_000_000;

const SILENCE_CHUNK: usize = RATE as usize / 2;

/// A frame that waits for the next one: only then its duration is known.
struct Held {
    buffer: IMFMediaBuffer,
    time: u64,
}

pub struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    audio: Option<u32>,
    /// Sound frames written so far: their count gives the time of the next one.
    audio_frames: u64,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    size: (u32, u32),
    fps: u32,
    staging: Option<ID3D11Texture2D>,
    held: Option<Held>,
    // The sink writer uses the device through it; it must outlive the writer.
    _manager: IMFDXGIDeviceManager,
}

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn subtype(codec: Codec) -> GUID {
    match codec {
        Codec::H264 => MFVideoFormat_H264,
        Codec::Hevc => MFVideoFormat_HEVC,
    }
}

fn profile(codec: Codec) -> u32 {
    match codec {
        Codec::H264 => eAVEncH264VProfile_High.0 as u32,
        Codec::Hevc => eAVEncH265VProfile_Main_420_8.0 as u32,
    }
}

fn attributes(count: u32) -> Result<IMFAttributes> {
    let mut attrs = None;
    unsafe { MFCreateAttributes(&mut attrs, count)? };
    attrs.ok_or_else(Error::empty)
}

impl Encoder {
    pub fn new(device: &ID3D11Device, path: &Path, plan: &Plan) -> Result<Self> {
        let manager = unsafe {
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.ok_or_else(Error::empty)?;
            manager.ResetDevice(device, token)?;
            manager
        };
        // Quality rate control is optional for hardware encoders: fall back to plain
        // bitrate, then to the CPU path. A file without the sound track is the last resort.
        let attempts = [(true, true), (true, false), (false, true), (false, false)];
        let mut last = Error::empty();
        for audio in [true, false] {
            for (gpu, quality) in attempts {
                let (writer, stream, audio) =
                    match open(path, plan, gpu.then_some(&manager), quality, audio) {
                        Ok(opened) => opened,
                        Err(e) => {
                            last = e;
                            continue;
                        }
                    };
                crate::journal::info(tr!(
                    "journal.encoder",
                    gpu = gpu,
                    quality = quality,
                    sound = audio.is_some()
                ));
                let size = (plan.width, plan.height);
                let staging = if gpu {
                    None
                } else {
                    Some(gpu::staging(device, size)?)
                };
                return Ok(Self {
                    writer,
                    stream,
                    audio,
                    audio_frames: 0,
                    device: device.clone(),
                    context: unsafe { device.GetImmediateContext()? },
                    size,
                    fps: plan.fps,
                    staging,
                    held: None,
                    _manager: manager,
                });
            }
        }
        Err(last)
    }

    /// Frame `texture` (video size, BGRA) at `time`. It is copied: the caller keeps
    /// drawing into its texture while the encoder works.
    pub fn write(&mut self, texture: &ID3D11Texture2D, time: u64) -> Result<()> {
        let buffer = match &self.staging {
            None => self.gpu_buffer(texture)?,
            Some(staging) => self.cpu_buffer(staging, texture)?,
        };
        if let Some(previous) = self.held.take() {
            self.send(previous, time)?;
        }
        self.held = Some(Held { buffer, time });
        Ok(())
    }

    /// Interleaved samples that follow the ones already written.
    pub fn write_sound(&mut self, samples: &[i16]) -> Result<()> {
        let Some(stream) = self.audio else {
            return Ok(());
        };
        let frames = (samples.len() / CHANNELS) as u64;
        if frames == 0 {
            return Ok(());
        }
        let bytes = (samples.len() * 2) as u32;
        let time = |frames: u64| frames * SECOND / u64::from(RATE);
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes)?;
            let mut target = std::ptr::null_mut();
            buffer.Lock(&mut target, None, None)?;
            std::ptr::copy_nonoverlapping(samples.as_ptr() as *const u8, target, bytes as usize);
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let start = time(self.audio_frames);
            sample.SetSampleTime(start as i64)?;
            sample.SetSampleDuration((time(self.audio_frames + frames) - start) as i64)?;
            self.writer.WriteSample(stream, &sample)?;
        }
        self.audio_frames += frames;
        Ok(())
    }

    /// The last frame lasts until `end`; then the file is closed and playable.
    pub fn finish(mut self, end: u64) -> Result<()> {
        if let Some(last) = self.held.take() {
            let end = end.max(last.time + SECOND / u64::from(self.fps));
            // The file is closed even if its last frame is refused (the recording may be
            // ending because the encoder broke): what is already in it stays playable.
            if let Err(e) = self.send(last, end).and_then(|()| self.silence(end)) {
                crate::journal::warn(tr!("journal.last_frame_failed", error = e));
            }
        }
        unsafe { self.writer.Finalize() }
    }

    fn send(&mut self, frame: Held, until: u64) -> Result<()> {
        unsafe {
            let sample = MFCreateSample()?;
            sample.AddBuffer(&frame.buffer)?;
            sample.SetSampleTime(frame.time as i64)?;
            sample.SetSampleDuration(until.saturating_sub(frame.time).max(1) as i64)?;
            self.writer.WriteSample(self.stream, &sample)?;
        }
        Ok(())
    }

    /// Silence up to `until`, so the sound track is as long as the video.
    fn silence(&mut self, until: u64) -> Result<()> {
        let frames = (until * u64::from(RATE) / SECOND).saturating_sub(self.audio_frames);
        let zeros = vec![0i16; SILENCE_CHUNK * CHANNELS];
        let mut left = frames as usize;
        while left > 0 && self.audio.is_some() {
            let chunk = left.min(SILENCE_CHUNK);
            self.write_sound(&zeros[..chunk * CHANNELS])?;
            left -= chunk;
        }
        Ok(())
    }

    fn gpu_buffer(&self, texture: &ID3D11Texture2D) -> Result<IMFMediaBuffer> {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let mut copy = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut copy))?;
            let copy = copy.ok_or_else(Error::empty)?;
            self.context.CopyResource(&copy, texture);
            let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &copy, 0, false)?;
            // A DXGI buffer starts empty: the writer would drop it.
            let length = buffer.cast::<IMF2DBuffer>()?.GetContiguousLength()?;
            buffer.SetCurrentLength(length)?;
            Ok(buffer)
        }
    }

    fn cpu_buffer(
        &self,
        staging: &ID3D11Texture2D,
        texture: &ID3D11Texture2D,
    ) -> Result<IMFMediaBuffer> {
        let (w, h) = self.size;
        let row = w as usize * 4;
        unsafe {
            self.context.CopyResource(staging, texture);
            let buffer = MFCreateMemoryBuffer((row * h as usize) as u32)?;
            let mut target = std::ptr::null_mut();
            buffer.Lock(&mut target, None, None)?;
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            let result = self
                .context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped));
            if result.is_ok() {
                for y in 0..h as usize {
                    std::ptr::copy_nonoverlapping(
                        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                        target.add(y * row),
                        row,
                    );
                }
                self.context.Unmap(staging, 0);
            }
            buffer.Unlock()?;
            result?;
            buffer.SetCurrentLength((row * h as usize) as u32)?;
            Ok(buffer)
        }
    }
}

fn open(
    path: &Path,
    plan: &Plan,
    manager: Option<&IMFDXGIDeviceManager>,
    quality: bool,
    audio: bool,
) -> Result<(IMFSinkWriter, u32, Option<u32>)> {
    unsafe {
        let attrs = attributes(3)?;
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        if let Some(manager) = manager {
            attrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, manager)?;
        }
        let writer =
            MFCreateSinkWriterFromURL(&HSTRING::from(path), None::<&IMFByteStream>, &attrs)?;

        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        output.SetGUID(&MF_MT_SUBTYPE, &subtype(plan.codec))?;
        output.SetUINT32(&MF_MT_AVG_BITRATE, plan.bitrate())?;
        output.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        output.SetUINT32(&MF_MT_MPEG2_PROFILE, profile(plan.codec))?;
        output.SetUINT64(&MF_MT_FRAME_SIZE, pack(plan.width, plan.height))?;
        output.SetUINT64(&MF_MT_FRAME_RATE, pack(plan.fps, 1))?;
        output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        let stream = writer.AddStream(&output)?;

        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, pack(plan.width, plan.height))?;
        input.SetUINT64(&MF_MT_FRAME_RATE, pack(plan.fps, 1))?;
        input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        if manager.is_none() {
            // Positive stride: rows top to bottom, as they come from the texture.
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, plan.width * 4)?;
        }

        let params = attributes(3)?;
        params.SetUINT32(&CODECAPI_AVEncMPVGOPSize, plan.fps * 2)?;
        if quality {
            params.SetUINT32(
                &CODECAPI_AVEncCommonRateControlMode,
                eAVEncCommonRateControlMode_Quality.0 as u32,
            )?;
            params.SetUINT32(&CODECAPI_AVEncCommonQuality, QUALITY)?;
        }
        writer.SetInputMediaType(stream, &input, &params)?;

        let audio = if audio {
            Some(sound_stream(&writer)?)
        } else {
            None
        };
        writer.BeginWriting()?;
        Ok((writer, stream, audio))
    }
}

fn sound_stream(writer: &IMFSinkWriter) -> Result<u32> {
    unsafe {
        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        output.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)?;
        output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, SOUND_BITRATE / 8)?;
        let stream = writer.AddStream(&output)?;

        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)?;
        input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, CHANNELS as u32 * 2)?;
        input.SetUINT32(
            &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            RATE * CHANNELS as u32 * 2,
        )?;
        writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;
        Ok(stream)
    }
}
