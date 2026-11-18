//! The battery's notices from its readings: low charge from Windows' Energy saver threshold
//! every 5%, every percent from 10% (critical), plugged in and unplugged, charged, and
//! charging too slowly for 3 minutes. Each step is said once per discharge; plugging in
//! forgets them.
//!
//! The readings come from `adapters::power`, at Windows' power events and once a minute.

use super::Notice;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub percent: u8,
    pub on_ac: bool,
    pub charging: bool,
    pub saver: bool,
    /// Positive while charging, negative while draining. `None` when Windows doesn't say.
    pub rate_mw: Option<i32>,
    pub remaining_mwh: Option<u32>,
    pub full_mwh: Option<u32>,
    /// Windows' own estimate on battery: smoothed, so it doesn't jump with every minute's drain.
    pub windows_minutes_left: Option<u32>,
}

impl Reading {
    /// What the battery gives away now, in mW; `None` while charging or unknown.
    pub fn drain_mw(&self) -> Option<f32> {
        self.rate_mw.filter(|&r| r < 0).map(|r| -r as f32)
    }

    /// At `drain_mw` from now on.
    pub fn minutes_left(&self, drain_mw: Option<f32>) -> Option<u32> {
        let mw = drain_mw.filter(|&mw| mw > 0.0)?;
        Some((self.remaining_mwh? as f32 / mw * 60.0).round() as u32)
    }

    /// To 100% at the present rate; a maker's charge limit is unknown, so the estimate can run long.
    fn minutes_to_full(&self) -> Option<u32> {
        let rate = self.rate_mw.filter(|&r| r > 0)? as f32;
        let missing = self.full_mwh?.saturating_sub(self.remaining_mwh?) as f32;
        Some((missing / rate * 60.0).round() as u32)
    }

    pub fn gauge(&self, usual_mw: Option<f32>) -> Gauge {
        let (supply, minutes) = if !self.on_ac {
            let left = self
                .windows_minutes_left
                .or_else(|| self.minutes_left(self.drain_mw().or(usual_mw)));
            (Supply::Battery, left.map(Minutes::Left))
        } else if self.charging {
            (
                Supply::Charging,
                self.minutes_to_full().map(Minutes::ToFull),
            )
        } else {
            (Supply::Plugged, None)
        };
        Gauge {
            percent: self.percent,
            supply,
            saver: self.saver,
            minutes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Supply {
    Battery,
    Charging,
    /// On the charger without charging: full, or held at a charge limit.
    Plugged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Minutes {
    Left(u32),
    ToFull(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gauge {
    pub percent: u8,
    pub supply: Supply,
    pub saver: bool,
    pub minutes: Option<Minutes>,
}

pub const CRITICAL: u8 = 10;
/// When Windows sets no Energy saver threshold.
pub const DEFAULT_LOW: u8 = 20;
const STEP: u8 = 5;
/// The low threshold keeps one yellow step above the critical level.
const LOWEST_LOW: u8 = CRITICAL + STEP;
const HIGHEST_LOW: u8 = 50;
/// At the threshold Windows turns Energy saver on by itself, a moment after the charge
/// crossed it: the notice waits for that to say both at once.
pub const SAVER_WAIT_MS: u64 = 3_000;
const SLOW_FOR_MS: u64 = 180_000;
/// A charge rate that would fill an empty battery in more hours than this is slow.
const SLOW_HOURS: f32 = 8.0;
/// Above this every battery charges slowly by design.
const SLOW_BELOW: u8 = 80;
/// A pause in charging below this is not "charged".
const CHARGED_FROM: u8 = 50;
/// At a charge limit Windows tops the battery up by a percent or two and stops again, over
/// and over: a stop after a smaller gain is such a top-up, not "charged".
const CHARGED_GAIN: u8 = 5;

/// Windows' Energy saver threshold, when it has one, kept in the range that leaves a yellow step.
pub fn low_threshold(windows: Option<u8>) -> u8 {
    windows
        .unwrap_or(DEFAULT_LOW)
        .clamp(LOWEST_LOW, HIGHEST_LOW)
}

#[derive(Debug)]
pub struct Battery {
    low: u8,
    last: Option<Reading>,
    /// The lowest step said in this discharge.
    said: Option<u8>,
    /// The first low step waits for Energy saver until the deadline.
    waiting: Option<u64>,
    /// Since the plug: the charge when charging was first seen.
    charging_from: Option<u8>,
    /// Once per plug.
    charged_said: bool,
    slow_since: Option<u64>,
    slow_said: bool,
}

impl Battery {
    pub fn new(low: u8) -> Self {
        Self {
            low,
            last: None,
            said: None,
            waiting: None,
            charging_from: None,
            charged_said: false,
            slow_since: None,
            slow_said: false,
        }
    }

    pub fn set_low(&mut self, low: u8) {
        self.low = low;
    }

    pub fn low(&self) -> u8 {
        self.low
    }

    /// The red battery in the collapsed island.
    pub fn badge(&self) -> Option<u8> {
        self.last
            .filter(|r| !r.on_ac && r.percent <= CRITICAL)
            .map(|r| r.percent)
    }

    pub fn last(&self) -> Option<Reading> {
        self.last
    }

    pub fn next_wake(&self) -> Option<u64> {
        self.waiting
    }

    /// The first reading only sets the scene: it can say a low charge, never a plug.
    /// `usual_mw` is the usual drain, for the time left when the real one isn't known yet.
    pub fn observe(&mut self, r: Reading, now: u64, usual_mw: Option<f32>) -> Vec<Notice> {
        let mut out = Vec::new();
        let previous = self.last.replace(r);
        if let Some(p) = previous
            && p.on_ac != r.on_ac
        {
            if r.on_ac {
                self.said = None;
                self.waiting = None;
                self.charging_from = None;
                self.charged_said = false;
                self.slow_since = None;
                self.slow_said = false;
                out.push(Notice::Plugged { percent: r.percent });
            } else {
                let minutes_left = r.minutes_left(r.drain_mw().or(usual_mw));
                out.push(Notice::Unplugged {
                    percent: r.percent,
                    minutes_left,
                });
            }
        }
        if r.on_ac {
            self.charge(previous, r, now, &mut out);
        } else {
            self.discharge(r, now, usual_mw, &mut out);
        }
        out
    }

    /// The saver wait ran out.
    pub fn tick(&mut self, now: u64) -> Vec<Notice> {
        match (self.waiting, self.last) {
            (Some(deadline), Some(r)) if now >= deadline => {
                self.waiting = None;
                vec![Notice::Low {
                    percent: r.percent,
                    saver: r.saver,
                }]
            }
            _ => Vec::new(),
        }
    }

    fn step(&self, percent: u8) -> Option<u8> {
        if percent <= CRITICAL {
            Some(percent)
        } else if percent <= self.low {
            Some(self.low - STEP * ((self.low - percent) / STEP))
        } else {
            None
        }
    }

    fn discharge(&mut self, r: Reading, now: u64, usual_mw: Option<f32>, out: &mut Vec<Notice>) {
        if self.waiting.is_some() && r.saver {
            self.waiting = None;
            out.push(Notice::Low {
                percent: r.percent,
                saver: true,
            });
        }
        let Some(step) = self.step(r.percent) else {
            return;
        };
        if self.said.is_some_and(|said| step >= said) {
            return;
        }
        self.said = Some(step);
        if step <= CRITICAL {
            self.waiting = None;
            out.push(Notice::Critical {
                percent: r.percent,
                minutes_left: r.minutes_left(r.drain_mw().or(usual_mw)),
            });
        } else if step == self.low && !r.saver {
            self.waiting = Some(now + SAVER_WAIT_MS);
        } else {
            out.push(Notice::Low {
                percent: r.percent,
                saver: r.saver,
            });
        }
    }

    fn charge(&mut self, previous: Option<Reading>, r: Reading, now: u64, out: &mut Vec<Notice>) {
        if r.charging {
            self.charging_from.get_or_insert(r.percent);
        } else if !self.charged_said
            && previous.is_some_and(|p| p.on_ac && p.charging)
            && r.percent >= CHARGED_FROM
            && self
                .charging_from
                .is_some_and(|from| r.percent >= from.saturating_add(CHARGED_GAIN))
        {
            self.charged_said = true;
            out.push(Notice::Charged { percent: r.percent });
        }

        let slow = r.charging && r.percent < SLOW_BELOW && is_slow(&r);
        if !slow {
            self.slow_since = None;
            return;
        }
        let since = *self.slow_since.get_or_insert(now);
        if !self.slow_said && now - since >= SLOW_FOR_MS {
            self.slow_said = true;
            out.push(Notice::SlowCharge { percent: r.percent });
        }
    }
}

fn is_slow(r: &Reading) -> bool {
    match (r.rate_mw, r.full_mwh) {
        (Some(rate), Some(full)) if rate > 0 => (rate as f32) < full as f32 / SLOW_HOURS,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_battery(percent: u8) -> Reading {
        Reading {
            percent,
            on_ac: false,
            charging: false,
            saver: false,
            rate_mw: Some(-8_000),
            remaining_mwh: Some(percent as u32 * 500),
            full_mwh: Some(50_000),
            windows_minutes_left: None,
        }
    }

    #[test]
    fn the_gauge_prefers_windows_estimate_and_tells_the_time_to_full() {
        let r = on_battery(40);
        // 20 000 mWh at 8 W.
        assert_eq!(r.gauge(None).minutes, Some(Minutes::Left(150)));
        let windows = Reading {
            windows_minutes_left: Some(170),
            ..r
        };
        assert_eq!(windows.gauge(None).minutes, Some(Minutes::Left(170)));
        // 30 000 mWh missing at 30 W.
        let g = charging(40, 30_000).gauge(None);
        assert_eq!(
            (g.supply, g.minutes),
            (Supply::Charging, Some(Minutes::ToFull(60)))
        );
        let held = Reading {
            charging: false,
            ..charging(80, 0)
        };
        assert_eq!(
            (held.gauge(None).supply, held.gauge(None).minutes),
            (Supply::Plugged, None)
        );
    }

    fn charging(percent: u8, rate_mw: i32) -> Reading {
        Reading {
            on_ac: true,
            charging: true,
            rate_mw: Some(rate_mw),
            ..on_battery(percent)
        }
    }

    fn lows(notices: &[Notice]) -> Vec<(u8, bool)> {
        notices
            .iter()
            .filter_map(|n| match n {
                Notice::Low { percent, saver } => Some((*percent, *saver)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_threshold_follows_windows_but_leaves_a_yellow_step() {
        assert_eq!(low_threshold(None), 20);
        assert_eq!(low_threshold(Some(30)), 30);
        assert_eq!(low_threshold(Some(10)), 15);
        assert_eq!(low_threshold(Some(100)), 50);
    }

    #[test]
    fn low_steps_are_said_once_every_five_percent() {
        let mut b = Battery::new(20);
        let mut said = Vec::new();
        for (i, p) in (16..=30).rev().enumerate() {
            let mut r = on_battery(p);
            r.saver = p <= 20;
            said.extend(lows(&b.observe(r, i as u64 * 60_000, None)));
        }
        assert_eq!(said, [(20, true)]);
        let more = b.observe(
            Reading {
                saver: true,
                ..on_battery(15)
            },
            0,
            None,
        );
        assert_eq!(lows(&more), [(15, true)]);
        let again = b.observe(
            Reading {
                saver: true,
                ..on_battery(14)
            },
            0,
            None,
        );
        assert!(again.is_empty());
    }

    #[test]
    fn the_first_step_waits_for_energy_saver() {
        let mut b = Battery::new(20);
        b.observe(on_battery(21), 0, None);
        assert!(b.observe(on_battery(20), 1_000, None).is_empty());
        assert_eq!(b.next_wake(), Some(1_000 + SAVER_WAIT_MS));
        let r = Reading {
            saver: true,
            ..on_battery(20)
        };
        assert_eq!(lows(&b.observe(r, 2_500, None)), [(20, true)]);
        assert_eq!(b.next_wake(), None);
    }

    #[test]
    fn without_energy_saver_the_wait_ends_in_a_plain_low() {
        let mut b = Battery::new(20);
        b.observe(on_battery(20), 0, None);
        assert!(b.tick(SAVER_WAIT_MS - 1).is_empty());
        assert_eq!(lows(&b.tick(SAVER_WAIT_MS)), [(20, false)]);
        assert!(b.tick(SAVER_WAIT_MS + 1).is_empty());
    }

    #[test]
    fn every_percent_from_ten_is_critical_with_the_time_left() {
        let mut b = Battery::new(20);
        b.observe(on_battery(11), 0, None);
        let mut said = Vec::new();
        for p in (8..=10).rev() {
            for n in b.observe(on_battery(p), 0, None) {
                if let Notice::Critical {
                    percent,
                    minutes_left,
                } = n
                {
                    said.push((percent, minutes_left));
                }
            }
        }
        // 500 mWh a percent at 8 W: 10% lasts 37.5 minutes.
        assert_eq!(said, [(10, Some(38)), (9, Some(34)), (8, Some(30))]);
        assert_eq!(b.badge(), Some(8));
    }

    #[test]
    fn plugging_in_forgets_the_steps_and_hides_the_badge() {
        let mut b = Battery::new(20);
        b.observe(on_battery(9), 0, None);
        let plugged = b.observe(charging(9, 30_000), 0, None);
        assert_eq!(plugged, [Notice::Plugged { percent: 9 }]);
        assert_eq!(b.badge(), None);
        let unplugged = b.observe(on_battery(9), 0, Some(10_000.0));
        assert!(matches!(
            unplugged[..],
            [
                Notice::Unplugged { percent: 9, .. },
                Notice::Critical { percent: 9, .. }
            ]
        ));
    }

    #[test]
    fn unplugging_tells_the_time_left_at_the_usual_drain_until_the_real_one_is_known() {
        let mut b = Battery::new(20);
        b.observe(charging(60, 30_000), 0, None);
        let r = Reading {
            rate_mw: None,
            ..on_battery(60)
        };
        // 30 000 mWh at the usual 10 W.
        assert_eq!(
            b.observe(r, 0, Some(10_000.0)),
            [Notice::Unplugged {
                percent: 60,
                minutes_left: Some(180)
            }]
        );
    }

    #[test]
    fn a_start_on_the_charger_says_nothing() {
        let mut b = Battery::new(20);
        assert!(b.observe(charging(50, 30_000), 0, None).is_empty());
    }

    #[test]
    fn charged_once_when_charging_stops_on_the_charger() {
        let mut b = Battery::new(20);
        b.observe(charging(70, 30_000), 0, None);
        let stopped = |percent| Reading {
            charging: false,
            rate_mw: Some(0),
            ..charging(percent, 0)
        };
        assert_eq!(
            b.observe(stopped(80), 60_000, None),
            [Notice::Charged { percent: 80 }]
        );
        assert!(b.observe(stopped(80), 120_000, None).is_empty());
        for minute in 0..6 {
            let at = 180_000 + minute * 120_000;
            assert!(b.observe(charging(79, 10_000), at, None).is_empty());
            assert!(b.observe(stopped(80), at + 60_000, None).is_empty());
        }
    }

    #[test]
    fn top_ups_at_the_charge_limit_are_not_charged() {
        let mut b = Battery::new(20);
        let stopped = Reading {
            charging: false,
            rate_mw: Some(0),
            ..charging(80, 0)
        };
        b.observe(stopped, 0, None);
        for minute in 0..6 {
            let at = 60_000 + minute * 120_000;
            assert!(b.observe(charging(79, 10_000), at, None).is_empty());
            assert!(b.observe(stopped, at + 60_000, None).is_empty());
        }
    }

    #[test]
    fn plugged_in_full_is_not_charged() {
        let mut b = Battery::new(20);
        let full = Reading {
            charging: false,
            ..charging(100, 0)
        };
        assert!(b.observe(full, 0, None).is_empty());
        assert!(b.observe(full, 60_000, None).is_empty());
    }

    #[test]
    fn slow_charging_is_said_after_three_minutes_once() {
        let mut b = Battery::new(20);
        // 50 Wh in more than 8 h: under 6.25 W.
        let slow = charging(40, 5_000);
        let mut said = Vec::new();
        for minute in 0..=5 {
            said.extend(b.observe(slow, minute * 60_000, None));
        }
        assert_eq!(said, [Notice::SlowCharge { percent: 40 }]);
    }

    #[test]
    fn a_short_slow_spell_or_a_nearly_full_battery_is_not_slow() {
        let mut b = Battery::new(20);
        let mut said = Vec::new();
        said.extend(b.observe(charging(40, 5_000), 0, None));
        said.extend(b.observe(charging(40, 5_000), 120_000, None));
        said.extend(b.observe(charging(41, 40_000), 180_000, None));
        said.extend(b.observe(charging(41, 5_000), 240_000, None));
        for minute in 0..5 {
            said.extend(b.observe(charging(85, 2_000), 300_000 + minute * 60_000, None));
        }
        assert!(said.is_empty(), "{said:?}");
    }
}
