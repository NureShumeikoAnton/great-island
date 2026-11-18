//! Which apps hold the processor: shares of the whole processor over the last 5 minutes, from
//! a snapshot of every process's processor time once a minute. An app is its exe: the many
//! processes of a browser count as one.
//!
//! It gives the list for the fast-drain notice and, without a battery or on the charger,
//! the high-load notice: one app over half of the processor for 5 minutes.

use super::Notice;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub exe: String,
    /// Kernel and user time, 100 ns units.
    pub cpu: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Wall clock, 100 ns units.
    pub at: u64,
    pub cores: u32,
    pub processes: Vec<Process>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Share {
    pub exe: String,
    /// Of the whole processor, all cores.
    pub percent: u8,
}

const WINDOW: usize = 5;
/// Less than this is not worth naming.
const LISTED_FROM: f32 = 10.0;
const LISTED: usize = 3;
const KEPT_FROM: f32 = 1.0;
const LEADER_FROM: u8 = 20;
const HEAVY: f32 = 50.0;
/// The same app is named again only after it fell under this.
const CALM: f32 = 30.0;
const COOLDOWN_MS: u64 = 30 * 60_000;
/// Snapshots further apart than this (the computer slept) don't make a minute.
const GAP_100NS: u64 = 150 * 10_000_000;

#[derive(Debug)]
struct Minute {
    total: f32,
    apps: HashMap<String, f32>,
}

#[derive(Debug, Default)]
pub struct Usage {
    previous: Option<Snapshot>,
    minutes: VecDeque<Minute>,
    /// Apps already named for their current spell of load.
    named: Vec<String>,
    quiet_until: u64,
}

impl Usage {
    pub fn sample(&mut self, snapshot: Snapshot) {
        let Some(previous) = self.previous.replace(snapshot) else {
            return;
        };
        let current = self.previous.as_ref().expect("just set");
        let wall = current.at.saturating_sub(previous.at);
        if wall == 0 || wall > GAP_100NS || current.cores == 0 {
            self.minutes.clear();
            return;
        }
        let before: HashMap<(u32, &str), u64> = previous
            .processes
            .iter()
            .map(|p| ((p.pid, p.exe.as_str()), p.cpu))
            .collect();
        let capacity = wall as f32 * current.cores as f32 / 100.0;
        let mut apps: HashMap<String, f32> = HashMap::new();
        for p in &current.processes {
            // A process that started since the last snapshot spent all its time in this minute.
            let spent = p
                .cpu
                .saturating_sub(*before.get(&(p.pid, p.exe.as_str())).unwrap_or(&0));
            *apps.entry(p.exe.clone()).or_default() += spent as f32 / capacity;
        }
        let total = apps.values().sum::<f32>().min(100.0);
        // The hundreds of idle processes would only take memory: none of them reaches a list.
        apps.retain(|_, percent| *percent >= KEPT_FROM);
        self.minutes.push_back(Minute { total, apps });
        if self.minutes.len() > WINDOW {
            self.minutes.pop_front();
        }
    }

    /// Up to three apps over 10% on average, the busiest first.
    pub fn top(&self) -> Vec<Share> {
        if self.minutes.is_empty() {
            return Vec::new();
        }
        let n = self.minutes.len() as f32;
        let mut sums: HashMap<&str, f32> = HashMap::new();
        for minute in &self.minutes {
            for (exe, percent) in &minute.apps {
                *sums.entry(exe).or_default() += percent / n;
            }
        }
        let mut apps: Vec<(&str, f32)> = sums
            .into_iter()
            .filter(|&(_, p)| p >= LISTED_FROM)
            .collect();
        apps.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        apps.into_iter()
            .take(LISTED)
            .map(|(exe, p)| Share {
                exe: exe.to_string(),
                percent: p.round().min(100.0) as u8,
            })
            .collect()
    }

    /// The processor as a whole, on average over the window.
    pub fn total(&self) -> u8 {
        if self.minutes.is_empty() {
            return 0;
        }
        let sum: f32 = self.minutes.iter().map(|m| m.total).sum();
        (sum / self.minutes.len() as f32).round() as u8
    }

    /// One app over half of the processor in each of the last 5 minutes, said once per spell
    /// and not within half an hour of the last one.
    pub fn heavy(&mut self, now: u64) -> Option<Notice> {
        let latest = self.minutes.back()?;
        self.named
            .retain(|exe| latest.apps.get(exe).copied().unwrap_or(0.0) >= CALM);
        if self.minutes.len() < WINDOW || now < self.quiet_until {
            return None;
        }
        let heavy = latest
            .apps
            .keys()
            .filter(|exe| {
                self.minutes
                    .iter()
                    .all(|m| m.apps.get(*exe).copied().unwrap_or(0.0) >= HEAVY)
            })
            .find(|exe| !self.named.contains(exe))?
            .clone();
        self.named.push(heavy);
        self.quiet_until = now + COOLDOWN_MS;
        Some(Notice::Load {
            cpu: self.total(),
            apps: self.top(),
        })
    }
}

/// The app named in the short notice: clearly ahead of the others.
pub fn leader(apps: &[Share]) -> Option<&Share> {
    let first = apps.first()?;
    let second = apps.get(1).map_or(0, |s| s.percent);
    (first.percent >= LEADER_FROM && first.percent >= second.saturating_mul(2)).then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 10_000_000;
    const MIN: u64 = 60 * SECOND;

    /// Two cores: a minute of the whole processor is 120 s of time.
    fn snapshot(minute: u64, used: &[(u32, &str, u64)]) -> Snapshot {
        Snapshot {
            at: minute * MIN,
            cores: 2,
            processes: used
                .iter()
                .map(|&(pid, exe, seconds)| Process {
                    pid,
                    exe: exe.into(),
                    cpu: seconds * SECOND,
                })
                .collect(),
        }
    }

    #[test]
    fn processes_of_one_exe_count_as_one_app() {
        let mut u = Usage::default();
        u.sample(snapshot(
            0,
            &[(1, "chrome", 0), (2, "chrome", 0), (3, "code", 0)],
        ));
        u.sample(snapshot(
            1,
            &[(1, "chrome", 24), (2, "chrome", 12), (3, "code", 6)],
        ));
        assert_eq!(
            u.top(),
            [Share {
                exe: "chrome".into(),
                percent: 30
            }]
        );
        assert_eq!(u.total(), 35);
    }

    #[test]
    fn the_list_is_up_to_three_apps_over_ten_percent() {
        let mut u = Usage::default();
        u.sample(snapshot(0, &[]));
        u.sample(snapshot(
            1,
            &[
                (1, "a", 48),
                (2, "b", 36),
                (3, "c", 24),
                (4, "d", 14),
                (5, "e", 6),
            ],
        ));
        let exes: Vec<_> = u.top().into_iter().map(|s| s.exe).collect();
        assert_eq!(exes, ["a", "b", "c"]);
    }

    #[test]
    fn a_new_process_counts_its_whole_time() {
        let mut u = Usage::default();
        u.sample(snapshot(0, &[(1, "a", 100)]));
        u.sample(snapshot(1, &[(1, "a", 100), (7, "b", 30)]));
        assert_eq!(u.top()[0].exe, "b");
        assert_eq!(u.top()[0].percent, 25);
    }

    #[test]
    fn the_leader_is_clearly_ahead() {
        let share = |p| Share {
            exe: "x".into(),
            percent: p,
        };
        assert!(leader(&[share(46), share(14)]).is_some());
        assert!(leader(&[share(24), share(18)]).is_none());
        assert!(leader(&[share(15)]).is_none());
        assert!(leader(&[share(22)]).is_some());
        assert!(leader(&[]).is_none());
    }

    /// One app, "obs", on a two-core processor: 72 s a minute is 60% of it.
    #[derive(Default)]
    struct Load {
        usage: Usage,
        cpu: u64,
        minute: u64,
    }

    impl Load {
        fn run(&mut self, minutes: u64, per_minute: u64) -> Vec<u64> {
            let mut fired = Vec::new();
            for _ in 0..minutes {
                self.cpu += per_minute;
                self.usage
                    .sample(snapshot(self.minute, &[(9, "obs", self.cpu)]));
                if self.usage.heavy(self.minute * 60_000).is_some() {
                    fired.push(self.minute);
                }
                self.minute += 1;
            }
            fired
        }
    }

    #[test]
    fn heavy_after_five_minutes_over_half_once_per_spell() {
        assert_eq!(Load::default().run(12, 72), [5]);
    }

    #[test]
    fn a_new_spell_of_the_same_app_waits_for_calm_and_half_an_hour() {
        let mut load = Load::default();
        let mut fired = load.run(12, 72);
        fired.extend(load.run(6, 6));
        // Heavy again from minute 22, but the half hour since minute 5 ends at 35.
        fired.extend(load.run(20, 72));
        assert_eq!(fired, [5, 35]);
    }

    #[test]
    fn sleep_breaks_the_window() {
        let mut u = Usage::default();
        u.sample(snapshot(0, &[(1, "a", 0)]));
        u.sample(snapshot(1, &[(1, "a", 60)]));
        u.sample(snapshot(30, &[(1, "a", 120)]));
        assert!(u.top().is_empty());
    }
}
