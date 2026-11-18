//! Probe for the downloads' folder watch, without the island and without the real Downloads.
//!
//! `cargo run --example download_probe` watches a fresh temp folder and plays in it what a
//! Chromium browser does (a partial file that grows, gets its name and is renamed), what a
//! messenger does (a file written straight) and what a copy does (an old time), then prints
//! what the watch told. To watch a folder of your own and download into it by hand:
//! `cargo run --example download_probe -- "D:\Somewhere" 60`.

use great_island_lib::download::adapters::{Event, Watch};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, scripted) = match args.first() {
        Some(dir) => (PathBuf::from(dir), false),
        None => {
            let dir = std::env::temp_dir().join(format!("download_probe_{}", std::process::id()));
            fs::create_dir_all(&dir).expect("a temp folder");
            (dir, true)
        }
    };
    let seconds = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let watch = Watch::start(dir.clone(), || {}).expect("the folder is watched");
    let started = Instant::now();
    let step = |what: &str, ms: u64| {
        sleep(Duration::from_millis(ms));
        println!("-- {what} (+{} ms)", started.elapsed().as_millis());
        print_events(&watch);
    };
    if !scripted {
        println!("watching {} for {seconds} s", dir.display());
        for _ in 0..seconds * 2 {
            sleep(Duration::from_millis(500));
            print_events(&watch);
        }
        return;
    }

    // The watch hears changes from its first wait on: let its thread get there.
    sleep(Duration::from_millis(300));
    let partial = dir.join("Unconfirmed 1.crdownload");
    let mut file = fs::File::create(&partial).expect("a partial file");
    for _ in 0..5 {
        file.write_all(&[0u8; 200_000]).expect("written");
        file.flush().expect("flushed");
        step("partial grew", 400);
    }
    drop(file);
    fs::rename(&partial, dir.join("report.pdf.crdownload")).expect("named");
    step("partial named", 300);
    fs::rename(dir.join("report.pdf.crdownload"), dir.join("report.pdf")).expect("finished");
    step("renamed to the final name", 500);

    fs::write(dir.join("photo.jpg"), [1u8; 5_000]).expect("written straight");
    step("written straight, before it settles", 300);
    step("after it settled", 2_200);

    let elsewhere =
        std::env::temp_dir().join(format!("download_probe_old_{}.pdf", std::process::id()));
    fs::write(&elsewhere, [2u8; 3_000]).expect("an old file");
    let day_ago = SystemTime::now() - Duration::from_secs(86_400);
    fs::File::options()
        .write(true)
        .open(&elsewhere)
        .and_then(|f| f.set_modified(day_ago))
        .expect("made old");
    fs::rename(&elsewhere, dir.join("old.pdf")).expect("moved in");
    step("an old file moved in (expect nothing)", 2_500);

    drop(watch);
    let _ = fs::remove_dir_all(&dir);
}

fn print_events(watch: &Watch) {
    for event in watch.drain() {
        match event {
            Event::Coming(p) => println!(
                "coming: {} file(s), {} bytes, name {:?}",
                p.files, p.bytes, p.name
            ),
            Event::Arrived(a) => println!(
                "arrived: {} ({} bytes), site {:?}, picture {:?}",
                a.name,
                a.bytes,
                a.source,
                a.thumb.map(|t| (t.width, t.height))
            ),
            Event::Stopped(failed) => println!("stopped: {failed:?}"),
        }
    }
}
