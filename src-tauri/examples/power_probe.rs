//! Probe for the system notifications' readings, without the island.
//!
//! `cargo run --example power_probe` prints the battery as Windows reports it, Windows' Energy
//! saver threshold, and after `SECONDS` the apps that held the processor meanwhile.
//! `cargo run --example power_probe -- 30` waits 30 s instead of 10.

use great_island_lib::notify::{
    adapters::{power, processes},
    battery, usage,
};
use std::time::{Duration, Instant};

const SECONDS: u64 = 10;

fn main() {
    let wait = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(SECONDS);
    match power::has_battery() {
        Ok(true) => match power::read() {
            Ok(r) => println!("battery: {r:?}"),
            Err(e) => println!("battery: unreadable {e:?}"),
        },
        Ok(false) => println!("battery: none"),
        Err(e) => println!("battery: unknown {e:?}"),
    }
    match power::saver_threshold() {
        Ok(t) => println!(
            "energy saver threshold: {t:?} -> low charge from {}%",
            battery::low_threshold(t)
        ),
        Err(e) => println!("energy saver threshold: unreadable {e:?}"),
    }

    let started = Instant::now();
    let mut usage = usage::Usage::default();
    let first = processes::snapshot().expect("processes");
    let spent = started.elapsed();
    println!(
        "processes: {} readable, {} cores, snapshot took {spent:?}",
        first.processes.len(),
        first.cores
    );
    usage.sample(first);
    std::thread::sleep(Duration::from_secs(wait));
    usage.sample(processes::snapshot().expect("processes"));
    println!("processor over {wait} s: {}%", usage.total());
    let top = usage.top();
    let leader = usage::leader(&top).map(|s| s.exe.clone());
    for share in &top {
        let name =
            processes::app_name(&share.exe).unwrap_or_else(|| processes::file_stem(&share.exe));
        let mark = if Some(&share.exe) == leader.as_ref() {
            " (leader)"
        } else {
            ""
        };
        println!("  {:>3}%  {name}  {}{mark}", share.percent, share.exe);
    }
    if top.is_empty() {
        println!("  no app over 10%");
    }
}
