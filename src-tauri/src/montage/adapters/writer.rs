//! Writing the edited recording. Every frame of the result is put together on the video card:
//! the D3D11 video processor takes the decoded frame of its fragment (the zoom's crop of it),
//! scales it to the saved size and lays the incoming fragment of a transition over it (opacity,
//! shift, scale, over black); the encoder takes the frame from there, it never leaves the card.
//! The sound is decoded, sped up with its pitch kept (`stretch`), silent where muted, crossed at
//! transitions (`splice`) and encoded to AAC.
//!
//! Which frame shows what is [`Montage::shot_at`], the same answer the editor watches. A
//! fragment's frames come from a reader of its own, sought to its start: two readers in turns,
//! as the editor's two engines.

use super::{
    SECOND, attributes, manager, pack,
    reader::{AUDIO, VIDEO, next, open, seek},
};
use crate::fault::Failed;
use crate::montage::{
    Montage,
    curve::{self, Frame, Layer},
    splice::Splice,
    squeeze::{Output, SOUND_BITRATE},
    stretch::Stretch,
};
use std::mem::ManuallyDrop;
use std::path::Path;
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_VIDEO_ENCODER, D3D11_TEX2D_ARRAY_VPOV,
                D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0,
                D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
                D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
                D3D11_VPOV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2DARRAY, ID3D11Device,
                ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
                ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
                ID3D11VideoProcessorOutputView,
            },
            Dxgi::Common::DXGI_RATIONAL,
        },
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode,
            CODECAPI_AVEncMPVGOPSize, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaType, IMFSample,
            IMFSinkWriter, IMFSourceReader, IMFVideoSampleAllocatorEx, MF_E_SAMPLEALLOCATOR_EMPTY,
            MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
            MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
            MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO,
            MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SA_D3D11_BINDFLAGS,
            MF_SINK_WRITER_D3D_MANAGER, MF_TRANSCODE_CONTAINERTYPE, MFAudioFormat_AAC,
            MFAudioFormat_Float, MFAudioFormat_PCM, MFCreateMediaType, MFCreateMemoryBuffer,
            MFCreateSample, MFCreateSinkWriterFromURL, MFCreateVideoSampleAllocatorEx,
            MFMediaType_Audio, MFMediaType_Video, MFTranscodeContainerType_MPEG4,
            MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
            eAVEncCommonRateControlMode_UnconstrainedVBR, eAVEncH264VProfile_High,
        },
    },
    core::{Error, HSTRING, Interface},
};

/// Output frames the encoder may hold at once.
const POOL: u32 = 16;
/// Sound is written in pieces of about this length.
const SOUND_PIECE: f64 = 0.25;

/// Writes `montage` of the recording `path` into `out` as `output` says. `progress` gets the
/// share done, 0..1.
pub fn save(
    device: &ID3D11Device,
    path: &Path,
    out: &Path,
    montage: &Montage,
    output: &Output,
    mut progress: impl FnMut(f64),
) -> Result<(), Failed> {
    let manager = manager(device)?;
    let mut feeds = [Feed::open(path, &manager)?, Feed::open(path, &manager)?];
    let source = feeds[0].size;
    let size = (output.width, output.height);
    let mixer = Mixer::new(device, source, size, output.fps)?;
    let mut sound = Sound::open(path, montage)?;
    let (writer, video, audio) =
        writer(out, &manager, (output, montage.duration()), sound.as_ref())?;
    let allocator = allocator(&manager, size, output.fps)?;
    unsafe { writer.BeginWriting()? };

    let fps = f64::from(output.fps.max(1));
    let count = (montage.duration() * fps).round().max(1.0) as u64;
    let frame = (SECOND / fps) as i64;
    let fragments = montage.fragments();
    for n in 0..count {
        let t = n as f64 / fps;
        let shot = montage.shot_at(t);
        let mut pieces = vec![(shot.main, curve::PLAIN)];
        if let Some(incoming) = shot.incoming {
            let (out, inc) = curve::layers(incoming.transition, incoming.progress);
            pieces[0].1 = out;
            pieces.push((incoming.piece, inc));
        }
        let mut layers = Vec::with_capacity(2);
        for (piece, layer) in pieces {
            let feed = &mut feeds[piece.index % 2];
            if feed.fragment != Some(piece.index) {
                feed.start(piece.index, fragments[piece.index].from)?;
            }
            if let Some(sample) = feed.at(piece.source)? {
                layers.push((sample.clone(), piece.frame, layer));
            }
        }
        if !layers.is_empty() {
            let target = take(&allocator)?;
            mixer.draw(&target, &layers)?;
            unsafe {
                target.SetSampleTime(n as i64 * frame)?;
                target.SetSampleDuration(frame)?;
                writer.WriteSample(video, &target)?;
            }
        }
        if let (Some(sound), Some(audio)) = (sound.as_mut(), audio) {
            sound.fill(&writer, audio, t + 1.0 / fps)?;
        }
        progress((n + 1) as f64 / count as f64);
    }
    if let (Some(sound), Some(audio)) = (sound.as_mut(), audio) {
        sound.fill(&writer, audio, f64::INFINITY)?;
    }
    unsafe { writer.Finalize()? };
    Ok(())
}

/// The sink writer: H.264 at the output's size and rate (by quality when it has no bitrate),
/// AAC when the recording has sound.
fn writer(
    out: &Path,
    manager: &IMFDXGIDeviceManager,
    (output, duration): (&Output, f64),
    sound: Option<&Sound>,
) -> Result<(IMFSinkWriter, u32, Option<u32>), Failed> {
    let attrs = attributes(3)?;
    unsafe {
        attrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, manager)?;
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
    }
    // "As is": the recording's own mean bitrate. Encoding by quality (as the recordings are made)
    // came out three times the recording's size: decoded frames carry the first encoding's noise.
    let mean = (output.bytes as f64 * 8.0 / duration.max(0.1)) as u32;
    let bitrate = output
        .bitrate
        .unwrap_or_else(|| mean.saturating_sub(SOUND_BITRATE).max(MIN_MEAN));
    let writer =
        unsafe { MFCreateSinkWriterFromURL(&HSTRING::from(out.as_os_str()), None, &attrs)? };
    let (video, audio) = streams(&writer, output, bitrate, sound)?;
    Ok((writer, video, audio))
}

const MIN_MEAN: u32 = 500_000;

fn streams(
    writer: &IMFSinkWriter,
    output: &Output,
    bitrate: u32,
    sound: Option<&Sound>,
) -> windows::core::Result<(u32, Option<u32>)> {
    unsafe {
        let size = pack(output.width, output.height);
        let rate = pack(output.fps, 1);
        let kind = MFCreateMediaType()?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        kind.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        kind.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        kind.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
        kind.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
        kind.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        kind.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        kind.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
        let video = writer.AddStream(&kind)?;
        let input = nv12(output.width, output.height, output.fps)?;
        let params = attributes(3).map_err(|_| Error::empty())?;
        params.SetUINT32(&CODECAPI_AVEncMPVGOPSize, output.fps * 2)?;
        // Without the mode said aloud hardware encoders keep their own (often far above it).
        params.SetUINT32(
            &CODECAPI_AVEncCommonRateControlMode,
            eAVEncCommonRateControlMode_UnconstrainedVBR.0 as u32,
        )?;
        params.SetUINT32(&CODECAPI_AVEncCommonMeanBitRate, bitrate)?;
        writer.SetInputMediaType(video, &input, &params)?;
        let audio = match sound {
            Some(sound) => {
                let kind = MFCreateMediaType()?;
                kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                kind.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                kind.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                kind.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sound.rate)?;
                kind.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, sound.channels as u32)?;
                kind.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, SOUND_BITRATE / 8)?;
                let stream = writer.AddStream(&kind)?;
                writer.SetInputMediaType(stream, &pcm(sound.rate, sound.channels)?, None)?;
                Some(stream)
            }
            None => None,
        };
        Ok((video, audio))
    }
}

fn nv12(width: u32, height: u32, fps: u32) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let kind = MFCreateMediaType()?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        kind.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        kind.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        kind.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
        kind.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        kind.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(kind)
    }
}

fn pcm(rate: u32, channels: usize) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let block = channels as u32 * 2;
        let kind = MFCreateMediaType()?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        kind.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        kind.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        kind.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        kind.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
        kind.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block)?;
        kind.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * block)?;
        kind.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        Ok(kind)
    }
}

/// Output frames on the video card, reused once the encoder lets them go.
fn allocator(
    manager: &IMFDXGIDeviceManager,
    (width, height): (u32, u32),
    fps: u32,
) -> Result<IMFVideoSampleAllocatorEx, Failed> {
    let kind = nv12(width, height, fps)?;
    // Some drivers don't take the encoder's bind flag: then render target alone.
    let binds = [
        (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32,
        D3D11_BIND_RENDER_TARGET.0 as u32,
    ];
    let mut last = Error::empty();
    for bind in binds {
        let made = unsafe {
            let mut raw = std::ptr::null_mut();
            MFCreateVideoSampleAllocatorEx(&IMFVideoSampleAllocatorEx::IID, &mut raw)?;
            let allocator = IMFVideoSampleAllocatorEx::from_raw(raw);
            allocator.SetDirectXManager(manager)?;
            let attrs = attributes(1)?;
            attrs.SetUINT32(&MF_SA_D3D11_BINDFLAGS, bind)?;
            allocator
                .InitializeSampleAllocatorEx(4, POOL, &attrs, &kind)
                .map(|_| allocator)
        };
        match made {
            Ok(allocator) => return Ok(allocator),
            Err(e) => last = e,
        }
    }
    Err(last.into())
}

/// The next free output frame; while the encoder holds them all, a moment's wait.
fn take(allocator: &IMFVideoSampleAllocatorEx) -> Result<IMFSample, Failed> {
    loop {
        match unsafe { allocator.AllocateSample() } {
            Ok(sample) => {
                // The encoder takes a buffer as empty until its length says otherwise.
                unsafe {
                    let buffer = sample.GetBufferByIndex(0)?;
                    buffer.SetCurrentLength(buffer.GetMaxLength()?)?;
                }
                return Ok(sample);
            }
            Err(e) if e.code() == MF_E_SAMPLEALLOCATOR_EMPTY => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// The texture of a sample on the video card and its slice in it (decoders hand out slices of
/// one array texture).
fn surface(sample: &IMFSample) -> windows::core::Result<(ID3D11Texture2D, u32)> {
    unsafe {
        let buffer = sample.GetBufferByIndex(0)?.cast::<IMFDXGIBuffer>()?;
        let mut raw = std::ptr::null_mut();
        buffer.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
        let texture = ID3D11Texture2D::from_raw(raw);
        Ok((texture, buffer.GetSubresourceIndex()?))
    }
}

/// A fragment's frames: a reader decoding on the video card from the fragment's start.
struct Feed {
    reader: IMFSourceReader,
    size: (u32, u32),
    fragment: Option<usize>,
    held: Option<(IMFSample, f64)>,
    next: Option<(IMFSample, f64)>,
    ended: bool,
}

impl Feed {
    fn open(path: &Path, manager: &IMFDXGIDeviceManager) -> Result<Self, Failed> {
        let reader = open(path, Some(manager))?;
        let size = unsafe {
            reader.SetStreamSelection(VIDEO, true)?;
            let native = reader.GetNativeMediaType(VIDEO, 0)?;
            let kind = MFCreateMediaType()?;
            kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            kind.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            kind.SetUINT64(&MF_MT_FRAME_SIZE, native.GetUINT64(&MF_MT_FRAME_SIZE)?)?;
            kind.SetUINT64(&MF_MT_FRAME_RATE, native.GetUINT64(&MF_MT_FRAME_RATE)?)?;
            reader.SetCurrentMediaType(VIDEO, None, &kind)?;
            super::unpack(native.GetUINT64(&MF_MT_FRAME_SIZE)?)
        };
        Ok(Self {
            reader,
            size,
            fragment: None,
            held: None,
            next: None,
            ended: false,
        })
    }

    fn start(&mut self, fragment: usize, from: f64) -> Result<(), Failed> {
        self.held = None;
        self.next = None;
        self.ended = false;
        self.fragment = Some(fragment);
        seek(&self.reader, from)
    }

    /// The last frame at or before `source` (the first one, when the seek landed after it).
    fn at(&mut self, source: f64) -> Result<Option<&IMFSample>, Failed> {
        loop {
            if self.next.is_none() && !self.ended {
                self.next = next(&self.reader, VIDEO)?;
                self.ended = self.next.is_none();
            }
            match &self.next {
                Some((_, time)) if *time <= source + 1e-4 || self.held.is_none() => {
                    self.held = self.next.take();
                }
                _ => break,
            }
            if self.ended {
                break;
            }
        }
        Ok(self.held.as_ref().map(|(sample, _)| sample))
    }
}

/// The D3D11 video processor: up to two decoded frames, cropped, scaled, shifted and faded, over
/// black, into an output frame.
struct Mixer {
    device: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    source: (u32, u32),
    size: (u32, u32),
}

impl Mixer {
    fn new(
        device: &ID3D11Device,
        source: (u32, u32),
        size: (u32, u32),
        fps: u32,
    ) -> Result<Self, Failed> {
        let video: ID3D11VideoDevice = device.cast()?;
        let context: ID3D11VideoContext = unsafe { device.GetImmediateContext()? }.cast()?;
        let rate = DXGI_RATIONAL {
            Numerator: fps,
            Denominator: 1,
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: source.0,
            InputHeight: source.1,
            OutputFrameRate: rate,
            OutputWidth: size.0,
            OutputHeight: size.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let enumerator = unsafe { video.CreateVideoProcessorEnumerator(&desc)? };
        let processor = unsafe { video.CreateVideoProcessor(&enumerator, 0)? };
        unsafe {
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA {
                        R: 0.0,
                        G: 0.0,
                        B: 0.0,
                        A: 1.0,
                    },
                },
            };
            context.VideoProcessorSetOutputBackgroundColor(&processor, false, &black);
            for stream in 0..2 {
                context.VideoProcessorSetStreamFrameFormat(
                    &processor,
                    stream,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                context.VideoProcessorSetStreamAutoProcessingMode(&processor, stream, false);
            }
        }
        Ok(Self {
            device: video,
            context,
            enumerator,
            processor,
            source,
            size,
        })
    }

    fn draw(&self, target: &IMFSample, layers: &[(IMFSample, Frame, Layer)]) -> Result<(), Failed> {
        let (texture, index) = surface(target)?;
        let view = self.output_view(&texture, index)?;
        let mut streams = Vec::with_capacity(layers.len());
        for (sample, frame, layer) in layers {
            // A layer wholly off the output (a slide's first moment) is not drawn.
            let Some((source, dest)) = self.rects(*frame, *layer) else {
                continue;
            };
            let (texture, slice) = surface(sample)?;
            let input = self.input_view(&texture, slice)?;
            let i = streams.len() as u32;
            unsafe {
                self.context.VideoProcessorSetStreamSourceRect(
                    &self.processor,
                    i,
                    true,
                    Some(&source),
                );
                self.context
                    .VideoProcessorSetStreamDestRect(&self.processor, i, true, Some(&dest));
                self.context.VideoProcessorSetStreamAlpha(
                    &self.processor,
                    i,
                    true,
                    layer.opacity.clamp(0.0, 1.0) as f32,
                );
            }
            streams.push(D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(input)),
                ..Default::default()
            });
        }
        // Streams left over from a frame with more layers are switched off.
        for i in streams.len() as u32..2 {
            unsafe {
                self.context
                    .VideoProcessorSetStreamAlpha(&self.processor, i, false, 1.0);
            }
        }
        let drawn = unsafe {
            self.context
                .VideoProcessorBlt(&self.processor, &view, 0, &streams)
        };
        for stream in &mut streams {
            unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        }
        Ok(drawn?)
    }

    /// The zoom's crop of the source, and where the layer puts it on the output; the part that
    /// would fall outside the output is cut off the source too.
    fn rects(&self, frame: Frame, layer: Layer) -> Option<(RECT, RECT)> {
        let (sw, sh) = (f64::from(self.source.0), f64::from(self.source.1));
        let (w, h) = (f64::from(self.size.0), f64::from(self.size.1));
        let crop = (frame.x * sw, frame.y * sh, frame.side * sw, frame.side * sh);
        let (dw, dh) = (w * layer.scale, h * layer.scale);
        let dest = ((w - dw) / 2.0 + layer.shift * w, (h - dh) / 2.0, dw, dh);
        // NV12 halves the colour planes: every edge on an even pixel.
        let even_down = |v: f64| (v / 2.0).floor() * 2.0;
        let even_up = |v: f64| (v / 2.0).ceil() * 2.0;
        let left = even_down(dest.0.max(0.0));
        let top = even_down(dest.1.max(0.0));
        let right = even_up((dest.0 + dest.2).min(w)).min(w);
        let bottom = even_up((dest.1 + dest.3).min(h)).min(h);
        if right - left < 4.0 || bottom - top < 4.0 {
            return None;
        }
        let (kx, ky) = (crop.2 / dest.2, crop.3 / dest.3);
        let source = RECT {
            left: even_down((crop.0 + (left - dest.0) * kx).max(0.0)) as i32,
            top: even_down((crop.1 + (top - dest.1) * ky).max(0.0)) as i32,
            right: even_up(crop.0 + (right - dest.0) * kx).min(sw) as i32,
            bottom: even_up(crop.1 + (bottom - dest.1) * ky).min(sh) as i32,
        };
        if source.right - source.left < 4 || source.bottom - source.top < 4 {
            return None;
        }
        let dest = RECT {
            left: left as i32,
            top: top as i32,
            right: right as i32,
            bottom: bottom as i32,
        };
        Some((source, dest))
    }

    fn input_view(
        &self,
        texture: &ID3D11Texture2D,
        slice: u32,
    ) -> windows::core::Result<ID3D11VideoProcessorInputView> {
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: slice,
                },
            },
        };
        let mut view = None;
        unsafe {
            self.device.CreateVideoProcessorInputView(
                texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )?;
        }
        view.ok_or_else(Error::empty)
    }

    fn output_view(
        &self,
        texture: &ID3D11Texture2D,
        slice: u32,
    ) -> windows::core::Result<ID3D11VideoProcessorOutputView> {
        let desc = if slice == 0 {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            }
        } else {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2DArray: D3D11_TEX2D_ARRAY_VPOV {
                        MipSlice: 0,
                        FirstArraySlice: slice,
                        ArraySize: 1,
                    },
                },
            }
        };
        let mut view = None;
        unsafe {
            self.device.CreateVideoProcessorOutputView(
                texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )?;
        }
        view.ok_or_else(Error::empty)
    }
}

/// The sound of the result, written as the frames go: each fragment read from its start,
/// trimmed to it, sped up, silenced when muted, then joined by `splice`.
struct Sound {
    reader: IMFSourceReader,
    rate: u32,
    channels: usize,
    fragments: Vec<crate::montage::Fragment>,
    parts: Vec<(usize, usize, usize)>,
    index: usize,
    stretch: Option<Stretch>,
    splice: Splice,
    out: Vec<f32>,
    written: u64,
}

impl Sound {
    /// `None`: the recording has no sound.
    fn open(path: &Path, montage: &Montage) -> Result<Option<Self>, Failed> {
        let reader = open(path, None)?;
        let (rate, channels) = unsafe {
            if reader.SetStreamSelection(AUDIO, true).is_err() {
                return Ok(None);
            }
            let kind = MFCreateMediaType()?;
            kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            kind.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
            reader.SetCurrentMediaType(AUDIO, None, &kind)?;
            let current = reader.GetCurrentMediaType(AUDIO)?;
            (
                current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?,
                current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? as usize,
            )
        };
        Ok(Some(Self {
            reader,
            rate,
            channels: channels.max(1),
            fragments: montage.fragments().to_vec(),
            parts: montage.sound_parts(rate),
            index: 0,
            stretch: None,
            splice: Splice::new(channels.max(1)),
            out: Vec::new(),
            written: 0,
        }))
    }

    /// Reads and writes on until the sound reaches `until` (result seconds), or its end.
    fn fill(&mut self, writer: &IMFSinkWriter, stream: u32, until: f64) -> Result<(), Failed> {
        while self.index < self.fragments.len()
            && (self.written as f64 + (self.out.len() / self.channels) as f64)
                / f64::from(self.rate)
                < until
        {
            self.step()?;
            if self.out.len() / self.channels >= (f64::from(self.rate) * SOUND_PIECE) as usize {
                self.write(writer, stream)?;
            }
        }
        if self.index >= self.fragments.len() {
            self.write(writer, stream)?;
        }
        Ok(())
    }

    /// One more piece of the current fragment, or its end.
    fn step(&mut self) -> Result<(), Failed> {
        let f = self.fragments[self.index];
        if self.stretch.is_none() {
            let (length, before, after) = self.parts[self.index];
            self.splice.begin(length, before, after);
            self.stretch = Some(Stretch::new(self.rate, self.channels, f.speed));
            seek(&self.reader, f.from)?;
        }
        let mut sped = Vec::new();
        let piece = next(&self.reader, AUDIO)?;
        let over = match &piece {
            None => true,
            Some((_, time)) => *time >= f.to,
        };
        if let Some((sample, time)) = piece.filter(|_| !over) {
            let samples = floats(&sample)?;
            let c = self.channels;
            let frames = samples.len() / c;
            let rate = f64::from(self.rate);
            // Only the part inside the fragment: the seek lands a little before its start.
            let first = (((f.from - time) * rate).ceil().max(0.0) as usize).min(frames);
            let last = (((f.to - time) * rate).floor().max(0.0) as usize).min(frames);
            if last > first
                && !f.muted
                && let Some(stretch) = &mut self.stretch
            {
                stretch.push(&samples[first * c..last * c], &mut sped);
            }
        }
        if over
            && let Some(mut stretch) = self.stretch.take()
            && !f.muted
        {
            stretch.finish(&mut sped);
        }
        self.splice.push(&sped, &mut self.out);
        if over {
            self.index += 1;
            let last = self.index == self.fragments.len();
            self.splice.end(&mut self.out, last);
        }
        Ok(())
    }

    fn write(&mut self, writer: &IMFSinkWriter, stream: u32) -> Result<(), Failed> {
        let frames = self.out.len() / self.channels;
        if frames == 0 {
            return Ok(());
        }
        let bytes = (frames * self.channels * 2) as u32;
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            let target = std::slice::from_raw_parts_mut(data as *mut i16, frames * self.channels);
            for (t, s) in target.iter_mut().zip(&self.out) {
                *t = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let rate = f64::from(self.rate);
            sample.SetSampleTime((self.written as f64 / rate * SECOND) as i64)?;
            sample.SetSampleDuration((frames as f64 / rate * SECOND) as i64)?;
            writer.WriteSample(stream, &sample)?;
        }
        self.written += frames as u64;
        self.out.clear();
        Ok(())
    }
}

fn floats(sample: &IMFSample) -> Result<Vec<f32>, Failed> {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer()?;
        let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
        buffer.Lock(&mut data, None, Some(&mut length))?;
        let samples = if data.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data as *const f32, length as usize / 4).to_vec()
        };
        buffer.Unlock()?;
        Ok(samples)
    }
}
