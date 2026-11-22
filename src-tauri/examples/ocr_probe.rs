//! Probe for the text scanner's engine, without the island (TASK_09, stage 1).
//!
//! `cargo run --release --example ocr_probe -- [picture.png ...]` reads each picture (or, with
//! no arguments, the primary monitor right now) in every installed OCR language, at its own
//! size and enlarged twice, and looks for QR codes. It prints the times, the memory and the
//! first lines found. Touches nothing.

use great_island_lib::{
    capture::{Image, Source, adapters::capture},
    files::adapters::read_photo,
    text::{Reader, adapters::WindowsOcr, qr, script},
};
use std::{path::Path, time::Instant};
use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
    System::{
        Com::{COINIT_MULTITHREADED, CoInitializeEx},
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
};

fn memory() -> (usize, usize) {
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
    }
    (c.WorkingSetSize >> 20, c.PeakWorkingSetSize >> 20)
}

fn doubled(image: &Image) -> Image {
    let (w, h) = (image.width * 2, image.height * 2);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let row = ((y / 2) * image.width * 4) as usize;
        for x in 0..w {
            let p = row + (x / 2 * 4) as usize;
            pixels.extend_from_slice(&image.pixels[p..p + 4]);
        }
    }
    Image {
        width: w,
        height: h,
        pixels,
    }
}

fn main() {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut inputs: Vec<(String, Image)> = Vec::new();
    for arg in std::env::args().skip(1) {
        match read_photo(Path::new(&arg)) {
            Ok(image) => inputs.push((arg, image)),
            Err(e) => println!("{arg}: not read ({e:?})"),
        }
    }
    if inputs.is_empty() {
        let monitor = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
        let source = Source::whole(
            monitor.0 as isize,
            unsafe { GetSystemMetrics(SM_CXSCREEN) } as u32,
            unsafe { GetSystemMetrics(SM_CYSCREEN) } as u32,
        );
        let t = Instant::now();
        let (image, _) = capture(source).expect("screen");
        println!("screen grabbed in {:?}", t.elapsed());
        inputs.push(("screen".into(), image));
    }

    let (rest, _) = memory();
    println!("memory at rest: {rest} MB");
    let mut reader = WindowsOcr::new();
    let languages = reader.languages();
    println!(
        "languages: {}",
        languages
            .iter()
            .map(|l| format!("{} ({}, {})", l.short(), l.tag, l.name))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for (name, image) in &inputs {
        println!("\n== {name}: {}×{}", image.width, image.height);
        for language in &languages {
            for (label, picture) in [("×1", None), ("×2", Some(doubled(image)))] {
                let picture = picture.as_ref().unwrap_or(image);
                let t = Instant::now();
                let first = reader.read(picture, &language.tag);
                let cold = t.elapsed();
                let t = Instant::now();
                let page = reader.read(picture, &language.tag);
                let warm = t.elapsed();
                let (now, peak) = memory();
                match (first, page) {
                    (Ok(_), Ok(mut page)) => {
                        script::tidy(&mut page);
                        let words: usize = page.lines.iter().map(|l| l.words.len()).sum();
                        println!(
                            "{} {label}: {} lines, {words} words; first {cold:?}, again {warm:?}; memory {now} MB, peak {peak} MB",
                            language.short(),
                            page.lines.len()
                        );
                        if label == "×1" {
                            for line in page.lines.iter().take(40) {
                                let r = line.rect();
                                println!(
                                    "    [{:.0},{:.0} {:.0}×{:.0}] {}",
                                    r.x,
                                    r.y,
                                    r.width,
                                    r.height,
                                    line.text()
                                );
                            }
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        println!("{} {label}: failed {e:?}", language.short())
                    }
                }
            }
        }
        let t = Instant::now();
        let codes = qr::find(image);
        println!("QR: {} in {:?}", codes.len(), t.elapsed());
        for code in codes {
            println!("    {:?} {}", code.rect, code.content);
        }
    }
    drop(reader);
    let (now, peak) = memory();
    println!("\nafter the reader is gone: {now} MB, peak {peak} MB");
}
