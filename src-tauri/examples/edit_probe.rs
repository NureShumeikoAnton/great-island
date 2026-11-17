//! Probe of the editor's Windows side on a real recording: how long opening takes (the facts,
//! the timeline's thumbnails, the sound's wave) and a save of an edit with every effect (a cut,
//! a moved fragment, 2×, a muted fragment, a smooth zoom, a dissolve) in each compression mode.
//!
//! `cargo run --release --example edit_probe -- <recording.mp4> <out-folder>`

use great_island_lib::montage::{
    Montage,
    adapters::{reader, video_device, writer},
    curve::{Frame, Transition, Zoom},
    squeeze::{Source, Squeeze, plan},
    strip::{Peaks, thumb_times},
};
use std::{path::PathBuf, time::Instant};
use windows::Win32::{
    Media::MediaFoundation::{MF_VERSION, MFSTARTUP_NOSOCKET, MFStartup},
    System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
};

const THUMB: (u32, u32) = (160, 90);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(path), Some(dir)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: edit_probe <recording.mp4> <out-folder>");
        return;
    };
    let (path, dir) = (PathBuf::from(path), PathBuf::from(dir));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).expect("MFStartup");
    }
    let device = video_device().expect("video device");

    let started = Instant::now();
    let info = reader::info(&path).expect("info");
    println!(
        "facts: {:.1} s, {}x{} at {} fps, in {} ms",
        info.duration,
        info.width,
        info.height,
        info.fps,
        started.elapsed().as_millis()
    );

    let started = Instant::now();
    let times = thumb_times(info.duration);
    let thumbs = reader::thumbnails(&device, &path, &times, THUMB).expect("thumbnails");
    println!(
        "thumbnails: {} of {} in {} ms",
        thumbs.len(),
        times.len(),
        started.elapsed().as_millis()
    );

    let started = Instant::now();
    let mut peaks = None;
    let had = reader::sound(&path, |(rate, channels), samples| {
        peaks
            .get_or_insert_with(|| Peaks::new(rate, channels))
            .push(samples);
        true
    })
    .expect("sound");
    let peaks = peaks.map(Peaks::finish).unwrap_or_default();
    println!(
        "wave: sound {had}, {} peaks, in {} ms",
        peaks.len(),
        started.elapsed().as_millis()
    );

    let d = info.duration;
    let mut montage = Montage::new(d);
    for share in [0.25, 0.5, 0.75] {
        assert!(montage.split(d * share));
    }
    montage.remove(1);
    montage.move_to(2, 0);
    montage.set_speed(1, 2.0);
    montage.set_muted(2, true);
    montage.set_zoom(
        0,
        Some(Zoom {
            frame: Frame::around((0.3, 0.3), 0.5),
            smooth: true,
        }),
    );
    montage.set_transition(0, Some(Transition::Dissolve));
    montage.set_transition(1, Some(Transition::Slide));
    println!("edit: {:.2} s of result", montage.duration());

    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let source = Source {
        width: info.width,
        height: info.height,
        fps: info.fps,
        bytes,
        duration: d,
    };
    for squeeze in Squeeze::ALL {
        let output = plan(squeeze, &source, montage.duration());
        let out = dir.join(format!("edit_probe_{squeeze:?}.mp4"));
        let started = Instant::now();
        let mut last = 0.0;
        let saved = writer::save(&device, &path, &out, &montage, &output, |done| last = done);
        let took = started.elapsed().as_secs_f64();
        if let Err(e) = saved {
            println!("{squeeze:?}: save failed: {e:?}");
            continue;
        }
        let written = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let check = reader::info(&out)
            .map(|i| {
                format!(
                    "{:.2} s, {}x{} at {} fps",
                    i.duration, i.width, i.height, i.fps
                )
            })
            .unwrap_or_else(|e| format!("doesn't open: {e:?}"));
        let starts = montage.starts();
        let moments = [
            ("zoom", starts[0] + 0.3),
            ("dissolve", starts[1] + montage.overlap(0) / 2.0),
            ("slide", starts[2] + montage.overlap(1) / 2.0),
        ];
        let size = ((output.width / 4) & !1, (output.height / 4) & !1);
        let times: Vec<f64> = moments.iter().map(|m| m.1).collect();
        if let Ok(frames) = reader::thumbnails(&device, &out, &times, size) {
            for ((name, _), pixels) in moments.iter().zip(frames) {
                let file = dir.join(format!("edit_probe_{squeeze:?}_{name}.png"));
                if let Err(e) = png(&pixels, size, &file) {
                    println!("frame {name}: {e:?}");
                }
            }
        }
        println!(
            "{squeeze:?}: planned {}x{} at {} fps ~{:.1} MB; saved in {took:.2} s ({:.1}x real time), \
             {:.2} MB, progress {:.2}; the file: {check}",
            output.width,
            output.height,
            output.fps,
            output.bytes as f64 / 1e6,
            montage.duration() / took,
            written as f64 / 1e6,
            last
        );
    }
}

fn png(
    pixels: &[u8],
    (width, height): (u32, u32),
    path: &std::path::Path,
) -> windows::core::Result<()> {
    use windows::Win32::{
        Foundation::GENERIC_WRITE,
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR,
            GUID_WICPixelFormat32bppBGR, IWICImagingFactory, WICBitmapEncoderNoCache,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    };
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = factory.CreateBitmapFromMemory(
            width,
            height,
            &GUID_WICPixelFormat32bppBGR,
            width * 4,
            pixels,
        )?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&windows::core::HSTRING::from(path), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let (mut frame, mut options) = (None, None);
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()
    }
}
