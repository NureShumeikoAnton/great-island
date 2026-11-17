//! Probe for the "Now playing" core against live players, without the island.
//!
//! `cargo run --example media_probe` prints the sessions, the one the island would pick,
//! announce and pause events, and the player's audio level. Ctrl+C exits.
//! `cargo run --example media_probe -- open` also brings the player window to the front.

use great_island_lib::media::{Media, Meter, PlayerEvent, Tracker};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let open = std::env::args().any(|a| a == "open");
    let (ping, wake) = mpsc::channel();
    let media = Media::start(move || {
        let _ = ping.send(());
    });
    let started = Instant::now();
    let now = || started.elapsed().as_millis() as u64;

    let mut tracker = Tracker::new();
    let mut meter = Meter::new();
    let mut attached: Option<String> = None;
    let mut last_level = Instant::now();

    loop {
        let _ = wake.recv_timeout(Duration::from_millis(100));
        let mut events = Vec::new();
        for update in media.drain() {
            println!("{:>7} мс  {update:?}", now());
            events.extend(tracker.apply(update, now()));
        }
        events.extend(tracker.tick(now()));
        for event in events {
            let title = tracker
                .current()
                .map(|s| s.title.clone())
                .unwrap_or_default();
            println!("{:>7} мс  >>> {event:?}: {title}", now());
            if event == PlayerEvent::Announce && open {
                let app = tracker
                    .current()
                    .map(|s| s.app_id.clone())
                    .unwrap_or_default();
                println!(
                    "            окно плеера вперёд: {}",
                    great_island_lib::media::open_app(&app)
                );
            }
        }

        let current = tracker.current().map(|s| (s.key.clone(), s.app_id.clone()));
        if current.as_ref().map(|c| &c.0) != attached.as_ref() {
            attached = current.as_ref().map(|c| c.0.clone());
            match &current {
                Some((key, app_id)) => {
                    let _ = meter.attach(app_id);
                    println!(
                        "{:>7} мс  текущая сессия: {key}, присутствие {:?}, звук: {:?}",
                        now(),
                        tracker.presence(now()),
                        meter.source()
                    );
                }
                None => {
                    meter.detach();
                    println!("{:>7} мс  плеера нет", now());
                }
            }
        }

        if last_level.elapsed() >= Duration::from_millis(500)
            && let Ok(Some(level)) = meter.sample()
        {
            last_level = Instant::now();
            let bar = |v: f32| "█".repeat((v.sqrt() * 20.0) as usize);
            println!(
                "            L {:<20} R {}",
                bar(level.left),
                bar(level.right)
            );
        }
    }
}
