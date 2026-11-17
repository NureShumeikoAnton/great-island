//! Break reminders on the island: what the scheduler's events do (the sound, the reminder
//! notice), pause and "Done", and rewinding a timer by holding it.
//!
//! When a reminder is due is decided by `crate::scheduler`.

use super::{App, log, timers};
use crate::{
    platform,
    scheduler::{Env, Event},
    sound,
    stage::NoticeKind,
};
use std::time::Instant;

const SKIP_STEP_MS: u32 = 33;
/// Hold-to-rewind speed in timer seconds per second held. Starts slowly so a few seconds can be rewound,
/// and doubles every SKIP_DOUBLING_S seconds until it hits SKIP_MAX (12 minutes per second).
/// Roughly: 1 s held = 13 s of the timer, 2 s = a minute, 3.5 s = 9 minutes, 7 s = 50 minutes.
const SKIP_START: f32 = 6.0;
const SKIP_DOUBLING_S: f32 = 0.5;
const SKIP_MAX: f32 = 720.0;

pub(super) struct Reminders {
    /// The tone the running reminder started with: it ends with the same one.
    last_tone: String,
    pub(super) skip: Option<Skip>,
}

impl Reminders {
    pub(super) fn new() -> Self {
        Self {
            last_tone: "bell".into(),
            skip: None,
        }
    }
}

/// A timer is being rewound by holding it.
pub(super) struct Skip {
    pub(super) id: String,
    started: Instant,
    last: Instant,
}

impl App {
    pub(super) fn tick_reminders(&mut self, dt: u64) {
        let env = Env {
            idle_ms: platform::idle_ms(),
        };
        let events = self.scheduler.tick(dt, env);
        self.on_reminder_events(&events);
    }

    pub(super) fn on_reminder_events(&mut self, events: &[Event]) {
        if events.is_empty() {
            return;
        }
        let volume = self.settings.volume;
        if events.contains(&Event::AlertStarted) {
            if let Some(tone) = self.scheduler.alert_tone() {
                self.reminders.last_tone = tone;
            }
            // Not follow_monitor: the alert is on screen at once and must not wait for the island to be at rest.
            self.monitor = self.wanted_monitor();
            self.relayout();
            sound::play(&self.reminders.last_tone, sound::Kind::Start, volume);
        }
        if events.contains(&Event::AlertFinished { natural: true }) {
            sound::play(&self.reminders.last_tone, sound::Kind::End, volume);
        }
        let now = self.now_ms();
        if events.contains(&Event::AlertStarted) {
            self.stage.post(NoticeKind::Alert, now);
        }
        if events
            .iter()
            .any(|e| matches!(e, Event::AlertFinished { .. }))
        {
            self.stage.end(NoticeKind::Alert, now);
            self.stage.post(NoticeKind::AlertDone, now);
            self.save();
        }
        self.schedule_stage();
        self.sync_visibility();
        self.render();
        self.sync_rings();
    }

    pub(super) fn toggle_reminders(&mut self) {
        let events = self.scheduler.set_paused(!self.scheduler.is_paused());
        self.save();
        self.on_reminder_events(&events);
    }

    /// "Done" on a reminder.
    pub(super) fn finish_alert(&mut self) {
        let events = self.scheduler.done();
        self.on_reminder_events(&events);
    }

    pub(super) fn start_skip(&mut self, id: String) {
        let now = Instant::now();
        self.reminders.skip = Some(Skip {
            id,
            started: now,
            last: now,
        });
        self.set_timer(timers::SKIP, SKIP_STEP_MS);
        self.render();
        self.sync_rings();
        if let Some(skip) = &self.reminders.skip {
            log(self.scene.latch_ring(&skip.id));
        }
    }

    pub(super) fn on_skip_step(&mut self) {
        let Some(skip) = &mut self.reminders.skip else {
            return;
        };
        let now = Instant::now();
        let dt = now.duration_since(skip.last).as_secs_f32();
        skip.last = now;
        let held = now.duration_since(skip.started).as_secs_f32();
        let speed = (SKIP_START * (held / SKIP_DOUBLING_S).exp2()).min(SKIP_MAX);
        let id = skip.id.clone();

        let moved = self.scheduler.skip(&id, (speed * 1000.0 * dt) as i64);
        let left = self
            .scheduler
            .snapshot()
            .timers
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.left_ms);
        if !moved || left.is_none_or(|l| l <= 0) {
            self.stop_skip();
            // Rewound to zero: the reminder comes at once, without waiting for a tick.
            let events = self.scheduler.tick(0, Env { idle_ms: 0 });
            self.on_reminder_events(&events);
            return;
        }
        self.render();
        self.sync_rings();
    }

    pub(super) fn stop_skip(&mut self) {
        if self.reminders.skip.take().is_none() {
            return;
        }
        self.kill_timer(timers::SKIP);
        if !self.pointer.inside {
            self.expanded = false;
        }
        self.save();
        self.render();
        self.sync_rings();
    }
}
