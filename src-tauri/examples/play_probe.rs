//! Probe for the playback system layer, without a window.
//!
//! `cargo run --release --example play_probe -- <video file>` opens the file through Media Engine,
//! muted, and drives it through the player's rules: plays two seconds, pauses, skips, steps a
//! frame both ways, drags the bar, plays at 4×, plays to the end. Prints every engine event with
//! the time it came and what the rules made of it. Doesn't change any file.

use great_island_lib::play::{
    Event, Phase, Player, SPEEDS,
    adapters::{Card, Note, Video},
    clock,
};
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

struct Probe {
    player: Player<Video>,
    notes: Receiver<Note>,
    began: Instant,
}

impl Probe {
    fn now(&self) -> u64 {
        self.began.elapsed().as_millis() as u64
    }

    /// Feeds the engine's events to the player for `wait`, or until `done` says so.
    fn pump(&mut self, wait: Duration, done: impl Fn(&Event) -> bool) -> bool {
        let until = Instant::now() + wait;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            let Ok(note) = self.notes.recv_timeout(left) else {
                break;
            };
            let Some(event) = self.player.engine().event(note) else {
                continue;
            };
            let now = self.now();
            let update = self.player.on_event(event.clone(), now);
            println!(
                "{now:>6} ms  {event:?}  -> {:?} at {:.2}s{}{}",
                self.player.phase(),
                self.player.position(now),
                if update.bar { ", bar again" } else { "" },
                update
                    .problem
                    .map(|p| format!(", problem {p:?}"))
                    .unwrap_or_default(),
            );
            if done(&event) {
                return true;
            }
        }
        false
    }

    fn step(&mut self, what: &str) {
        let now = self.now();
        println!(
            "---- {what} (at {}, {:?})",
            clock(self.player.position(now), self.player.duration()),
            self.player.phase()
        );
    }
}

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("give a video file"));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let (tx, notes) = channel();
    let began = Instant::now();
    let card = Card::new().expect("video device");
    let video = Video::open(&path, &card, move |note| {
        let _ = tx.send(note);
    })
    .expect("Media Engine did not open the file");
    let player = Player::new(video, 0.0, true);
    let mut probe = Probe {
        player,
        notes,
        began,
    };

    probe.step("load, then start");
    probe.player.start(probe.now()).unwrap();
    if !probe.pump(Duration::from_secs(10), |e| {
        matches!(e, Event::Loaded(_) | Event::Failed(_))
    }) {
        println!("nothing loaded in 10 s");
        return;
    }
    let Some(info) = probe.player.info() else {
        return;
    };
    println!(
        "info: {}×{}, {:.3} s, picture {}, sound {}",
        info.size.0, info.size.1, info.duration, info.picture, info.sound
    );
    if probe.player.phase() == Phase::Broken {
        probe.pump(Duration::from_secs(1), |_| false);
        return;
    }
    match probe.player.engine().swap_chain() {
        Ok(handle) => println!("swap chain handle: {:?}", handle.0),
        Err(e) => println!("swap chain: {e}"),
    }

    probe.step("play 2 s");
    probe.pump(Duration::from_secs(2), |_| false);
    match probe.player.engine().frame() {
        Ok(frame) => {
            let lit = frame
                .pixels
                .chunks(4)
                .filter(|p| p[0] | p[1] | p[2] > 16)
                .count();
            println!(
                "frame {}×{}, {:.0}% not black",
                frame.width,
                frame.height,
                lit as f64 * 100.0 / (frame.width * frame.height) as f64
            );
        }
        Err(e) => println!("frame: {e}"),
    }

    probe.step("pause");
    let now = probe.now();
    probe.player.toggle(now).unwrap();
    probe.pump(Duration::from_millis(500), |_| false);

    probe.step("skip forward 5 s");
    let now = probe.now();
    probe.player.skip(true, now).unwrap();
    probe.pump(Duration::from_secs(2), |e| matches!(e, Event::Seeked(_)));

    probe.step("frame forward, frame back");
    probe.player.step(true).unwrap();
    probe.pump(Duration::from_secs(2), |e| matches!(e, Event::Seeked(_)));
    if let Err(e) = probe.player.step(false) {
        println!("frame back refused: {e:?}");
    }
    probe.pump(Duration::from_secs(2), |e| matches!(e, Event::Seeked(_)));

    probe.step("drag the bar 10% → 40% → 30%, release");
    let now = probe.now();
    probe.player.grab(0.1, now).unwrap();
    probe.player.drag(0.4).unwrap();
    probe.player.drag(0.3).unwrap();
    probe.pump(Duration::from_millis(800), |_| false);
    let now = probe.now();
    probe.player.release(0.3, now).unwrap();
    probe.pump(Duration::from_secs(2), |e| matches!(e, Event::Seeked(_)));

    probe.step("play at 4×");
    let now = probe.now();
    if let Err(e) = probe.player.set_speed(SPEEDS.len() - 1, now) {
        println!("4× refused: {e:?}");
    }
    probe.player.toggle(now).unwrap();
    let ended = probe.pump(Duration::from_secs(info.duration as u64 / 4 + 5), |e| {
        matches!(e, Event::Ended)
    });
    probe.step(if ended {
        "ended"
    } else {
        "did not end in time"
    });
    println!("total {:?}", probe.began.elapsed());
}
