//! Timer logic with no ties to windows or the system, so tests can drive it.
//!
//! `tick` is called once a second with the elapsed time and the system state (how long the user hasn't
//! touched the mouse or keyboard). Fullscreen mode isn't part of it: a reminder always arrives,
//! the window decides whether to show it.

use crate::settings::Reminder;
use serde::{Deserialize, Serialize};

/// Reminders due within this window are shown together with the current one.
const MERGE_WINDOW_MS: i64 = 60_000;
/// A gap between ticks bigger than this means the computer slept.
const SLEEP_GAP_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    pub idle_ms: u64,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    AlertStarted,
    /// `natural`: the display time ran out by itself, rather than Done being pressed.
    AlertFinished {
        natural: bool,
    },
    Changed,
}

struct Timer {
    cfg: Reminder,
    left_ms: i64,
    held: bool,
}

impl Timer {
    fn new(cfg: Reminder) -> Self {
        let left_ms = interval_ms(&cfg);
        Self {
            cfg,
            left_ms,
            held: false,
        }
    }

    fn restart(&mut self) {
        self.left_ms = interval_ms(&self.cfg);
    }

    fn counts(&self) -> bool {
        self.cfg.enabled && !self.held
    }
}

fn interval_ms(cfg: &Reminder) -> i64 {
    cfg.interval_min as i64 * 60_000
}

fn whole_seconds(ms: i64) -> i64 {
    (ms.max(0) + 500) / 1000 * 1000
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTimer {
    pub id: String,
    /// If the interval was changed in settings, the saved remainder is no longer valid.
    pub interval_min: u32,
    pub left_ms: i64,
}

struct Alert {
    ids: Vec<String>,
    left_ms: i64,
    duration_ms: i64,
}

pub struct Scheduler {
    timers: Vec<Timer>,
    alert: Option<Alert>,
    idle_threshold_ms: u64,
    paused: bool,
    away: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    pub id: String,
    pub name: String,
    pub color: String,
    pub tone: String,
    pub left_ms: i64,
    pub interval_ms: i64,
    pub held: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertView {
    pub ids: Vec<String>,
    pub left_ms: i64,
    pub duration_ms: i64,
}

/// State for the UI. The UI counts the time itself from the moment it was received.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub paused: bool,
    pub away: bool,
    pub timers: Vec<TimerView>,
    pub alert: Option<AlertView>,
}

impl Scheduler {
    pub fn new(reminders: &[Reminder], idle_min: u32) -> Self {
        Self {
            timers: reminders.iter().cloned().map(Timer::new).collect(),
            alert: None,
            idle_threshold_ms: idle_min as u64 * 60_000,
            paused: false,
            away: false,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn has_alert(&self) -> bool {
        self.alert.is_some()
    }

    pub fn alert_tone(&self) -> Option<String> {
        let id = self.alert.as_ref()?.ids.first()?;
        self.timers
            .iter()
            .find(|t| &t.cfg.id == id)
            .map(|t| t.cfg.tone.clone())
    }

    pub fn tick(&mut self, dt_ms: u64, env: Env) -> Vec<Event> {
        let mut events = Vec::new();
        if self.paused {
            return events;
        }

        // Away: resettable timers jump to a full interval and wait for the return.
        let away = env.idle_ms >= self.idle_threshold_ms;
        if away != self.away {
            self.away = away;
            for t in self.timers.iter_mut().filter(|t| t.cfg.reset_on_idle) {
                t.held = away;
                if away {
                    t.restart();
                }
            }
            events.push(Event::Changed);
        }

        let dt = dt_ms as i64;
        let in_alert = |id: &str, alert: &Option<Alert>| {
            alert
                .as_ref()
                .is_some_and(|a| a.ids.iter().any(|x| x == id))
        };
        for t in &mut self.timers {
            if t.counts() && !in_alert(&t.cfg.id, &self.alert) {
                t.left_ms -= dt;
            }
        }

        // The computer slept longer than the idle threshold: that is a break too.
        if dt_ms > SLEEP_GAP_MS && dt_ms >= self.idle_threshold_ms {
            for t in self.timers.iter_mut().filter(|t| t.cfg.reset_on_idle) {
                t.restart();
            }
            events.push(Event::Changed);
        }

        if let Some(alert) = &mut self.alert {
            alert.left_ms -= dt;
            if alert.left_ms <= 0 {
                self.finish_alert();
                events.push(Event::AlertFinished { natural: true });
            }
        }

        let due = self.timers.iter().any(|t| t.counts() && t.left_ms <= 0);
        if due && self.alert.is_none() && !self.away {
            self.start_alert();
            events.push(Event::AlertStarted);
        }

        events
    }

    pub fn done(&mut self) -> Vec<Event> {
        if self.alert.is_none() {
            return Vec::new();
        }
        self.finish_alert();
        vec![Event::AlertFinished { natural: false }]
    }

    pub fn set_paused(&mut self, paused: bool) -> Vec<Event> {
        if paused == self.paused {
            return Vec::new();
        }
        self.paused = paused;
        let mut events = vec![Event::Changed];
        if paused && self.alert.is_some() {
            self.finish_alert();
            events.push(Event::AlertFinished { natural: false });
        }
        events
    }

    pub fn apply(&mut self, reminders: &[Reminder], idle_min: u32) -> Vec<Event> {
        self.idle_threshold_ms = idle_min as u64 * 60_000;
        let mut old: Vec<Timer> = std::mem::take(&mut self.timers);
        self.timers = reminders
            .iter()
            .cloned()
            .map(|cfg| {
                let kept = old
                    .iter()
                    .position(|t| t.cfg.id == cfg.id && t.cfg.interval_min == cfg.interval_min)
                    .map(|i| old.swap_remove(i));
                match kept {
                    Some(t) => Timer { cfg, ..t },
                    None => Timer::new(cfg),
                }
            })
            .collect();

        let mut events = vec![Event::Changed];
        let alert_valid = self.alert.as_ref().is_none_or(|a| {
            a.ids
                .iter()
                .all(|id| self.timers.iter().any(|t| &t.cfg.id == id && t.cfg.enabled))
        });
        if !alert_valid {
            self.alert = None;
            events.push(Event::AlertFinished { natural: false });
        }
        events
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            paused: self.paused,
            away: self.away,
            timers: self
                .timers
                .iter()
                .filter(|t| t.cfg.enabled)
                .map(|t| TimerView {
                    id: t.cfg.id.clone(),
                    name: t.cfg.name.clone(),
                    color: t.cfg.color.clone(),
                    tone: t.cfg.tone.clone(),
                    left_ms: whole_seconds(t.left_ms),
                    interval_ms: interval_ms(&t.cfg),
                    held: t.held,
                })
                .collect(),
            alert: self.alert.as_ref().map(|a| AlertView {
                ids: a.ids.clone(),
                left_ms: whole_seconds(a.left_ms),
                duration_ms: a.duration_ms,
            }),
        }
    }

    /// Rewind: timer `id` moves closer to its reminder by `ms`. Returns false if the timer can't be rewound right now.
    pub fn skip(&mut self, id: &str, ms: i64) -> bool {
        let in_alert = self
            .alert
            .as_ref()
            .is_some_and(|a| a.ids.iter().any(|x| x == id));
        match self.timers.iter_mut().find(|t| t.cfg.id == id) {
            Some(t) if t.counts() && !in_alert => {
                t.left_ms = (t.left_ms - ms).max(0);
                true
            }
            _ => false,
        }
    }

    /// Timer state for saving to disk. The shown reminder counts as done, so it doesn't pop up again after a restart.
    pub fn export(&self) -> Vec<SavedTimer> {
        let alerted = |id: &str| {
            self.alert
                .as_ref()
                .is_some_and(|a| a.ids.iter().any(|x| x == id))
        };
        self.timers
            .iter()
            .map(|t| SavedTimer {
                id: t.cfg.id.clone(),
                interval_min: t.cfg.interval_min,
                left_ms: if alerted(&t.cfg.id) {
                    interval_ms(&t.cfg)
                } else {
                    t.left_ms
                },
            })
            .collect()
    }

    /// Restores saved timers. `offline_ms` is how long the app was closed: that time counts,
    /// and a long absence counts as a break.
    pub fn restore(&mut self, saved: &[SavedTimer], paused: bool, offline_ms: u64) {
        for t in &mut self.timers {
            if let Some(s) = saved
                .iter()
                .find(|s| s.id == t.cfg.id && s.interval_min == t.cfg.interval_min)
            {
                t.left_ms = s.left_ms.min(interval_ms(&t.cfg));
            }
        }
        self.paused = paused;
        if paused {
            return;
        }
        let was_break = offline_ms >= self.idle_threshold_ms;
        for t in self.timers.iter_mut().filter(|t| t.cfg.enabled) {
            if was_break && t.cfg.reset_on_idle {
                t.restart();
            } else {
                t.left_ms -= offline_ms as i64;
            }
        }
    }

    fn start_alert(&mut self) {
        // Most overdue first: it picks the sound and color.
        let mut due: Vec<&Timer> = self
            .timers
            .iter()
            .filter(|t| t.counts() && t.left_ms <= MERGE_WINDOW_MS)
            .collect();
        due.sort_by_key(|t| t.left_ms);
        let duration_ms = due
            .iter()
            .map(|t| t.cfg.duration_sec as i64 * 1000)
            .max()
            .unwrap_or(30_000);
        self.alert = Some(Alert {
            ids: due.iter().map(|t| t.cfg.id.clone()).collect(),
            left_ms: duration_ms,
            duration_ms,
        });
    }

    fn finish_alert(&mut self) {
        if let Some(alert) = self.alert.take() {
            for t in self
                .timers
                .iter_mut()
                .filter(|t| alert.ids.contains(&t.cfg.id))
            {
                t.restart();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn rem(id: &str, interval_min: u32, duration_sec: u32, reset_on_idle: bool) -> Reminder {
        Reminder {
            id: id.into(),
            name: id.into(),
            color: "#fff".into(),
            tone: "bell".into(),
            interval_min,
            duration_sec,
            enabled: true,
            reset_on_idle,
        }
    }

    fn active() -> Env {
        Env { idle_ms: 0 }
    }

    fn advance(s: &mut Scheduler, ms: u64, env: Env) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..ms / 1000 {
            events.extend(s.tick(1000, env));
        }
        events
    }

    fn left(s: &Scheduler, id: &str) -> i64 {
        s.snapshot()
            .timers
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .left_ms
    }

    fn alert_ids(s: &Scheduler) -> Vec<String> {
        s.snapshot().alert.map(|a| a.ids).unwrap_or_default()
    }

    #[test]
    fn alert_fires_when_due_and_restarts_after_duration() {
        let mut s = Scheduler::new(&[rem("a", 10, 30, true)], 5);
        assert!(advance(&mut s, 10 * MIN - 1000, active()).is_empty());
        assert_eq!(s.tick(1000, active()), vec![Event::AlertStarted]);
        assert_eq!(left(&s, "a"), 0);

        assert_eq!(
            advance(&mut s, 30_000, active()),
            vec![Event::AlertFinished { natural: true }]
        );
        assert_eq!(left(&s, "a"), 10 * MIN as i64);
        assert!(!s.has_alert());
    }

    #[test]
    fn done_restarts_immediately() {
        let mut s = Scheduler::new(&[rem("a", 1, 30, true)], 5);
        advance(&mut s, MIN, active());
        assert_eq!(s.done(), vec![Event::AlertFinished { natural: false }]);
        assert_eq!(left(&s, "a"), MIN as i64);
    }

    #[test]
    fn close_reminders_are_merged_into_one_alert() {
        let mut s = Scheduler::new(
            &[
                rem("a", 10, 20, true),
                rem("b", 11, 40, true),
                rem("c", 30, 20, true),
            ],
            5,
        );
        advance(&mut s, 10 * MIN + 20_000, active());
        assert_eq!(alert_ids(&s), vec!["a", "b"]);
        assert_eq!(s.snapshot().alert.unwrap().duration_ms, 40_000);
    }

    #[test]
    fn going_away_resets_and_holds_idle_timers() {
        let mut s = Scheduler::new(&[rem("eyes", 20, 20, true), rem("water", 60, 15, false)], 5);
        advance(&mut s, 10 * MIN, active());
        advance(&mut s, 5 * MIN, Env { idle_ms: 5 * MIN });
        assert_eq!(left(&s, "eyes"), 20 * MIN as i64);
        assert_eq!(left(&s, "water"), 45 * MIN as i64);

        advance(&mut s, 10 * MIN, Env { idle_ms: 15 * MIN });
        assert_eq!(left(&s, "eyes"), 20 * MIN as i64);
        assert_eq!(left(&s, "water"), 35 * MIN as i64);

        s.tick(1000, active());
        assert_eq!(left(&s, "eyes"), 20 * MIN as i64 - 1000);
    }

    #[test]
    fn due_reminder_waits_while_user_is_away() {
        let mut s = Scheduler::new(&[rem("water", 1, 15, false)], 5);
        assert!(!advance(&mut s, 2 * MIN, Env { idle_ms: 6 * MIN }).contains(&Event::AlertStarted));
        assert!(s.tick(1000, active()).contains(&Event::AlertStarted));
    }

    #[test]
    fn long_sleep_counts_as_break() {
        let mut s = Scheduler::new(&[rem("eyes", 20, 20, true), rem("water", 60, 15, false)], 5);
        advance(&mut s, 10 * MIN, active());
        s.tick(30 * MIN, active());
        assert_eq!(left(&s, "eyes"), 20 * MIN as i64);
        assert_eq!(left(&s, "water"), 20 * MIN as i64);
    }

    #[test]
    fn pause_freezes_timers_and_cancels_alert() {
        let mut s = Scheduler::new(&[rem("a", 1, 30, true), rem("b", 10, 30, true)], 5);
        advance(&mut s, MIN, active());
        assert!(s.has_alert());
        s.set_paused(true);
        assert!(!s.has_alert());
        let before = left(&s, "b");
        advance(&mut s, 5 * MIN, active());
        assert_eq!(left(&s, "b"), before);
    }

    #[test]
    fn apply_keeps_progress_of_unchanged_timers() {
        let mut s = Scheduler::new(&[rem("a", 10, 30, true), rem("b", 10, 30, true)], 5);
        advance(&mut s, 3 * MIN, active());
        s.apply(
            &[
                rem("a", 10, 45, true),
                rem("b", 20, 30, true),
                rem("c", 5, 30, true),
            ],
            5,
        );
        assert_eq!(left(&s, "a"), 7 * MIN as i64);
        assert_eq!(left(&s, "b"), 20 * MIN as i64);
        assert_eq!(left(&s, "c"), 5 * MIN as i64);
    }

    #[test]
    fn disabled_timers_are_hidden_and_never_fire() {
        let mut off = rem("a", 1, 30, true);
        off.enabled = false;
        let mut s = Scheduler::new(&[off], 5);
        assert!(advance(&mut s, 5 * MIN, active()).is_empty());
        assert!(s.snapshot().timers.is_empty());
    }

    #[test]
    fn skip_brings_reminder_closer_and_can_trigger_it() {
        let mut s = Scheduler::new(&[rem("a", 10, 30, true)], 5);
        assert!(s.skip("a", 4 * MIN as i64));
        assert_eq!(left(&s, "a"), 6 * MIN as i64);
        assert!(s.skip("a", 60 * MIN as i64));
        assert_eq!(left(&s, "a"), 0);
        assert_eq!(s.tick(1000, active()), vec![Event::AlertStarted]);
        assert!(!s.skip("a", 1000));
    }

    #[test]
    fn restore_counts_short_offline_time() {
        let mut before = Scheduler::new(&[rem("a", 50, 30, true), rem("w", 60, 15, false)], 5);
        advance(&mut before, 10 * MIN, active());
        let saved = before.export();

        let mut after = Scheduler::new(&[rem("a", 50, 30, true), rem("w", 60, 15, false)], 5);
        after.restore(&saved, false, 2 * MIN);
        assert_eq!(left(&after, "a"), 38 * MIN as i64);
        assert_eq!(left(&after, "w"), 48 * MIN as i64);
    }

    #[test]
    fn restore_after_long_absence_is_a_break() {
        let mut before = Scheduler::new(&[rem("a", 50, 30, true), rem("w", 60, 15, false)], 5);
        advance(&mut before, 10 * MIN, active());
        let saved = before.export();

        let mut after = Scheduler::new(&[rem("a", 50, 30, true), rem("w", 60, 15, false)], 5);
        after.restore(&saved, false, 30 * MIN);
        assert_eq!(left(&after, "a"), 50 * MIN as i64);
        assert_eq!(left(&after, "w"), 20 * MIN as i64);
    }

    #[test]
    fn restore_keeps_paused_timers_and_ignores_changed_intervals() {
        let mut before = Scheduler::new(&[rem("a", 50, 30, true), rem("b", 20, 30, true)], 5);
        advance(&mut before, 10 * MIN, active());
        let saved = before.export();

        let mut after = Scheduler::new(&[rem("a", 50, 30, true), rem("b", 25, 30, true)], 5);
        after.restore(&saved, true, 3 * 60 * MIN);
        assert!(after.is_paused());
        assert_eq!(left(&after, "a"), 40 * MIN as i64);
        assert_eq!(left(&after, "b"), 25 * MIN as i64);
    }

    #[test]
    fn export_treats_shown_alert_as_done() {
        let mut s = Scheduler::new(&[rem("a", 1, 30, true)], 5);
        advance(&mut s, MIN, active());
        assert!(s.has_alert());
        assert_eq!(s.export()[0].left_ms, MIN as i64);
    }
}
