//! Probe for Space in Explorer and for photos opened from the disk, without the island.
//!
//! `cargo run --example explorer_probe` prints, for every open Explorer window and the
//! desktop, what is selected in its active tab (what Space would open there), and whether the
//! focus is in its list of files. Nothing is clicked or changed.
//!
//! `cargo run --example explorer_probe -- --photo "C:\Pictures\IMG_1.jpg"` reads the photo the
//! way the viewer does (its size once upright), then saves it into a temp copy the way the
//! editor's Save does and reads the copy back. The original isn't touched.

use great_island_lib::files::{
    adapters::{
        Disk,
        explorer::{Ask, Front, Reader, in_file_list},
        read_photo,
    },
    open::Format,
    picture::Album,
};
use std::{path::PathBuf, sync::mpsc, time::Duration};
use windows::{Win32::UI::WindowsAndMessaging::FindWindowExW, core::w};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, path] = args.as_slice()
        && flag == "--photo"
    {
        photo(PathBuf::from(path));
        return;
    }
    selections();
}

fn selections() {
    let mut fronts = Vec::new();
    let mut after = None;
    while let Ok(hwnd) = unsafe { FindWindowExW(None, after, w!("CabinetWClass"), None) } {
        fronts.extend(Front::of(hwnd));
        after = Some(hwnd);
    }
    if let Ok(desktop) = unsafe { FindWindowExW(None, None, w!("Progman"), None) } {
        fronts.extend(Front::of(desktop));
    }
    if fronts.is_empty() {
        println!("no Explorer window is open");
    }
    let (tx, rx) = mpsc::channel();
    let reader = Reader::start(move || {
        let _ = tx.send(());
    })
    .expect("the reader starts");
    for (token, front) in fronts.into_iter().enumerate() {
        reader.ask(Ask {
            token: token as u64,
            front,
        });
        let answered = rx.recv_timeout(Duration::from_secs(5)).is_ok();
        let item = reader.latest().and_then(|a| a.item);
        println!(
            "{front:?} focus in the file list: {}; selected: {}",
            in_file_list(front),
            match (answered, item) {
                (false, _) => "no answer in 5 s".to_string(),
                (true, None) => "nothing (or not a file on disk)".to_string(),
                (true, Some(item)) => format!(
                    "{}{}",
                    item.path.display(),
                    if item.folder { " (folder)" } else { "" }
                ),
            }
        );
    }
}

fn photo(path: PathBuf) {
    let started = std::time::Instant::now();
    let image = match read_photo(&path) {
        Ok(image) => image,
        Err(failed) => {
            println!("not read: {:?} {}", failed.cause, failed.detail);
            return;
        }
    };
    println!(
        "read {}×{} upright in {} ms",
        image.width,
        image.height,
        started.elapsed().as_millis()
    );
    let Some(format) = Format::of(&path.to_string_lossy()) else {
        println!("not a photo format the viewer reads");
        return;
    };
    let format = if format.writable() {
        format
    } else {
        Format::Jpeg
    };
    let copy = std::env::temp_dir().join(format!(
        "explorer_probe_{}.{}",
        std::process::id(),
        format.extension()
    ));
    let started = std::time::Instant::now();
    match Disk.write(&copy, &image, format, &path) {
        Ok(()) => println!(
            "written as {format:?} to {} in {} ms",
            copy.display(),
            started.elapsed().as_millis()
        ),
        Err(failed) => {
            println!("not written: {:?} {}", failed.cause, failed.detail);
            return;
        }
    }
    match read_photo(&copy) {
        Ok(back) => println!(
            "read back {}×{}: {}",
            back.width,
            back.height,
            if (back.width, back.height) == (image.width, image.height) {
                "the same size, upright"
            } else {
                "A DIFFERENT SIZE"
            }
        ),
        Err(failed) => println!("not read back: {:?} {}", failed.cause, failed.detail),
    }
    let _ = std::fs::remove_file(&copy);
}
