//! System notifications on the island (`crate::notify`): the battery and the load.
//!
//! 1. At start: is there a battery, Windows' Energy saver threshold, the power events on the
//!    island's window, the usual drain from `battery.json`, the minute timer.
//! 2. A power event (the charger, every percent, Energy saver) reads the battery at once: low
//!    and critical charge, plugged and unplugged come from it. The first low step waits up to
//!    3 s for Energy saver (`SAVER_WAIT`).
//! 3. Once a minute: every process's processor time and, on a battery, its reading: the usual
//!    drain learns, fast drain, charged and slow charging are checked. On the charger or
//!    without a battery: high load.
//! 4. A notice passes the gate (its setting, Do Not Disturb, a fullscreen app); a recording
//!    holds it until it ends. Shown: its slot's stage kind and its sound.
//! 5. Below 10% on battery the collapsed island keeps a red battery (`battery_badge`).

use super::{App, timers};
use crate::{
    control::ControlId,
    journal,
    lang::tr,
    media::{Icon, icon_of_exe},
    notify::{
        self, Context, Enabled, Notice, Slot, Verdict, Waiting,
        adapters::{power, processes},
        battery::{self, Battery},
        drain::{Baseline, Drain},
        usage::Usage,
    },
    persist,
    settings::Fullscreen,
    sound,
    stage::NoticeKind,
};
use std::{collections::HashMap, sync::Arc};

const SAMPLE_MS: u32 = 60_000;
/// The learnt drain is written every this many minutes on battery, and at exit.
const SAVE_EVERY: u32 = 10;

#[derive(Clone)]
pub(super) struct Face {
    pub(super) name: String,
    pub(super) icon: Option<Arc<Icon>>,
}

#[derive(Default)]
pub(super) struct Notify {
    /// `None` on a computer without a battery.
    battery: Option<Battery>,
    events: Option<power::Events>,
    baseline: Baseline,
    unsaved: u32,
    drain: Drain,
    usage: Usage,
    waiting: Waiting,
    shown: Vec<Notice>,
    faces: HashMap<String, Face>,
    /// The next sample of `--demo-system`.
    demo: usize,
    /// Said in the journal once; a success clears it.
    read_failing: bool,
    sample_failing: bool,
    save_failing: bool,
}

pub(super) fn kind_of(slot: Slot) -> NoticeKind {
    match slot {
        Slot::Low => NoticeKind::LowCharge,
        Slot::Critical => NoticeKind::CriticalCharge,
        Slot::Power => NoticeKind::Power,
        Slot::SlowCharge => NoticeKind::SlowCharge,
        Slot::Usage => NoticeKind::Usage,
    }
}

fn threshold() -> u8 {
    battery::low_threshold(power::saver_threshold().ok().flatten())
}

impl App {
    pub(super) fn start_notify(&mut self) {
        match power::has_battery() {
            Ok(true) => self.notify.battery = Some(Battery::new(threshold())),
            Ok(false) => {}
            Err(failed) => journal::warn(tr!("journal.battery_unreadable", error = failed.detail)),
        }
        if self.notify.battery.is_some() {
            match power::subscribe(self.island) {
                Ok(events) => self.notify.events = Some(events),
                Err(failed) => {
                    journal::warn(tr!("journal.power_events_failed", error = failed.detail))
                }
            }
        }
        self.notify.baseline = persist::load_baseline(&persist::battery_path()).unwrap_or_default();
        self.sample_processes();
        self.set_timer(timers::NOTIFY, SAMPLE_MS);
        self.on_power();
    }

    pub(super) fn stop_notify(&mut self) {
        self.notify.events = None;
        self.save_baseline();
    }

    /// WM_POWERBROADCAST: the charger, the percentage or Energy saver changed, or the computer woke up.
    pub(super) fn on_power(&mut self) {
        if self.notify.battery.is_none() {
            return;
        }
        if let Some(reading) = self.read_battery() {
            self.observe_battery(reading);
        }
    }

    fn read_battery(&mut self) -> Option<battery::Reading> {
        match power::read() {
            Ok(reading) => {
                self.notify.read_failing = false;
                Some(reading)
            }
            Err(failed) => {
                if !std::mem::replace(&mut self.notify.read_failing, true) {
                    journal::warn(tr!("journal.battery_unreadable", error = failed.detail));
                }
                None
            }
        }
    }

    fn observe_battery(&mut self, reading: battery::Reading) {
        let now = self.now_ms();
        let usual = self.notify.baseline.usual_mw(reading.full_mwh);
        let Some(battery) = &mut self.notify.battery else {
            return;
        };
        let badge = battery.badge();
        let notices = battery.observe(reading, now, usual);
        let badge_changed = battery.badge() != badge;
        match battery.next_wake() {
            Some(at) => self.set_timer(timers::SAVER_WAIT, at.saturating_sub(now).max(1) as u32),
            None => self.kill_timer(timers::SAVER_WAIT),
        }
        if reading.on_ac {
            self.notify.drain.reset();
        }
        for notice in notices {
            self.deliver(notice);
        }
        if badge_changed {
            self.render();
        }
    }

    pub(super) fn on_saver_wait(&mut self) {
        self.kill_timer(timers::SAVER_WAIT);
        let now = self.now_ms();
        let notices = match &mut self.notify.battery {
            Some(battery) => battery.tick(now),
            None => Vec::new(),
        };
        for notice in notices {
            self.deliver(notice);
        }
    }

    pub(super) fn on_notify_sample(&mut self) {
        self.sample_processes();
        let now = self.now_ms();
        let mut on_ac = true;
        if self.notify.battery.is_some()
            && let Some(reading) = self.read_battery()
        {
            if let Some(battery) = &mut self.notify.battery {
                battery.set_low(threshold());
            }
            self.observe_battery(reading);
            on_ac = reading.on_ac;
            if !on_ac && let Some(mw) = reading.drain_mw() {
                self.learn_drain(mw);
                let usual = self.notify.baseline.usual_mw(reading.full_mwh);
                if let Some(usual) = usual
                    && let Some(average) = self.notify.drain.sample(now, mw, usual)
                {
                    let notice = Notice::Drain {
                        minutes_left: reading.minutes_left(Some(average)),
                        watts: average / 1000.0,
                        apps: self.notify.usage.top(),
                    };
                    self.deliver(notice);
                }
            }
        }
        if on_ac && let Some(notice) = self.notify.usage.heavy(now) {
            self.deliver(notice);
        }
    }

    /// Only the fast-drain and the load notices need the processes. Turned on again, the old
    /// snapshot is too far back to count: the window starts anew.
    fn sample_processes(&mut self) {
        if !self.settings.notify_usage {
            return;
        }
        match processes::snapshot() {
            Ok(snapshot) => {
                self.notify.sample_failing = false;
                self.notify.usage.sample(snapshot);
            }
            Err(failed) => {
                if !std::mem::replace(&mut self.notify.sample_failing, true) {
                    journal::warn(tr!("journal.processes_unreadable", error = failed.detail));
                }
            }
        }
    }

    fn learn_drain(&mut self, mw: f32) {
        self.notify.baseline.add(mw);
        self.notify.unsaved += 1;
        if self.notify.unsaved >= SAVE_EVERY {
            self.save_baseline();
        }
    }

    fn save_baseline(&mut self) {
        if self.notify.unsaved == 0 {
            return;
        }
        match persist::save_baseline(&self.notify.baseline, &persist::battery_path()) {
            Ok(()) => {
                self.notify.unsaved = 0;
                self.notify.save_failing = false;
            }
            Err(e) => {
                if !std::mem::replace(&mut self.notify.save_failing, true) {
                    journal::warn(tr!("journal.battery_memory_failed", error = e));
                }
            }
        }
    }

    fn deliver(&mut self, notice: Notice) {
        match notify::gate(&notice, self.notify_enabled(), self.notice_context()) {
            Verdict::Show => self.show_notice(notice),
            Verdict::Wait => {
                let now = self.now_ms();
                self.notify.waiting.hold(notice, now);
            }
            Verdict::Drop => {}
        }
    }

    pub(super) fn notify_enabled(&self) -> Enabled {
        Enabled {
            low_charge: self.settings.notify_low_charge,
            charging: self.settings.notify_charging,
            usage: self.settings.notify_usage,
            downloads: self.settings.notify_downloads,
            devices: self.settings.notify_devices,
            device_battery: self.settings.notify_device_battery,
            audio_switch: self.settings.notify_audio_switch,
        }
    }

    pub(super) fn notice_context(&self) -> Context {
        Context {
            quiet: self.controls.is_on(ControlId::DoNotDisturb),
            fullscreen: self.fullscreen && self.settings.fullscreen != Fullscreen::Show,
            recording: self.recording.live.is_some() || self.recording.countdown.is_some(),
        }
    }

    fn show_notice(&mut self, notice: Notice) {
        if let Notice::Drain { apps, .. } | Notice::Load { apps, .. } = &notice {
            for share in apps {
                self.notify
                    .faces
                    .entry(share.exe.clone())
                    .or_insert_with(|| Face {
                        name: processes::app_name(&share.exe)
                            .unwrap_or_else(|| processes::file_stem(&share.exe)),
                        icon: icon_of_exe(&share.exe).map(Arc::new),
                    });
            }
        }
        let volume = self.settings.volume;
        match &notice {
            Notice::Low { .. } => sound::cue(sound::Cue::Pulse, volume),
            Notice::Critical { .. } => sound::cue(sound::Cue::Fall, volume),
            Notice::Plugged { .. } => sound::play("breath", sound::Kind::Start, volume),
            Notice::Unplugged { .. } => sound::play("breath", sound::Kind::End, volume),
            _ => {}
        }
        let slot = notice.slot();
        self.notify.shown.retain(|n| n.slot() != slot);
        self.notify.shown.push(notice);
        self.stage.post(kind_of(slot), self.now_ms());
        // A newer notice in a shown slot keeps the stage as it is: the content still changed.
        self.after_stage(true);
    }

    /// `--demo-system`: each start with the flag shows the next notice, past every setting and
    /// Do Not Disturb, with its sound.
    pub(super) fn demo_system(&mut self) {
        let app = |exe: &str, percent| notify::Share {
            exe: exe.into(),
            percent,
        };
        let windows = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let explorer = windows + r"\explorer.exe";
        let me = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let samples = [
            Notice::Low {
                percent: 20,
                saver: true,
            },
            Notice::Low {
                percent: 15,
                saver: false,
            },
            Notice::Critical {
                percent: 10,
                minutes_left: Some(36),
            },
            Notice::Critical {
                percent: 9,
                minutes_left: Some(31),
            },
            Notice::Plugged { percent: 47 },
            Notice::Unplugged {
                percent: 47,
                minutes_left: Some(200),
            },
            Notice::Charged { percent: 100 },
            Notice::SlowCharge { percent: 34 },
            Notice::Drain {
                minutes_left: Some(65),
                watts: 21.0,
                apps: vec![app(&explorer, 46), app(&me, 14)],
            },
            Notice::Drain {
                minutes_left: Some(65),
                watts: 21.0,
                apps: Vec::new(),
            },
            Notice::Load {
                cpu: 81,
                apps: vec![app(&explorer, 24), app(&me, 18), app(r"C:\missing.exe", 12)],
            },
        ];
        let notice = samples[self.notify.demo % samples.len()].clone();
        self.notify.demo += 1;
        self.show_notice(notice);
    }

    /// After the stage changed: a slot that left the stage forgets its notice; a recording that
    /// ended lets the held ones out.
    pub(super) fn sync_notify(&mut self) {
        let stage = &self.stage;
        self.notify.shown.retain(|n| stage.has(kind_of(n.slot())));
        if !self.notify.shown.iter().any(|n| n.slot() == Slot::Usage) {
            self.notify.faces.clear();
        }
        if !self.notice_context().recording {
            let now = self.now_ms();
            for notice in self.notify.waiting.release(now) {
                self.deliver(notice);
            }
        }
    }

    pub(super) fn system_notice(&self) -> Option<&Notice> {
        let kind = self.stage.shown().notice?.kind;
        self.notify.shown.iter().find(|n| kind_of(n.slot()) == kind)
    }

    pub(super) fn face(&self, exe: &str) -> Option<&Face> {
        self.notify.faces.get(exe)
    }

    /// The last reading for the control center's bottom line; `None` without a battery or
    /// before one could be read.
    pub(super) fn battery_gauge(&self) -> Option<(battery::Gauge, u8)> {
        let battery = self.notify.battery.as_ref()?;
        let r = battery.last()?;
        let usual = self.notify.baseline.usual_mw(r.full_mwh);
        Some((r.gauge(usual), battery.low()))
    }

    pub(super) fn battery_badge(&self) -> Option<u8> {
        self.notify
            .battery
            .as_ref()
            .and_then(Battery::badge)
            .filter(|_| self.settings.notify_low_charge)
    }
}
