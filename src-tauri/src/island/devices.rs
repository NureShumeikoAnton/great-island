//! Devices on the island (`crate::devices`).
//!
//! 1. At start, and when one of the three settings is turned on: the devices are watched on a
//!    thread of their own (`devices::adapters::Watch`); with all three off the watch ends and
//!    is forgotten.
//! 2. The first listing is quiet: what is connected at start is not news.
//! 3. Headphones connect: "Connected" in the large size with their picture and battery; it
//!    waits up to 3 s for the battery (`DEVICE_WAIT`). They leave: "Disconnected", small.
//! 4. A device's battery falls to 20% or 10%: "Low battery", small, once per level.
//! 5. Windows moves the sound or the microphone: "Sound switched" with the device's name, but
//!    not when it is the headphones that have just connected.
//! 6. Each passes the system notifications' gate: quiet in Do Not Disturb and over a fullscreen
//!    app; a recording holds them until it ends, while they are fresh.

use super::{App, timers, window::WM_DEVICES};
use crate::{
    devices::{Flow, Form, Look, Notice, Slot, Tracker, adapters::Watch},
    journal,
    lang::tr,
    notify::{self, STALE_MS, Verdict},
    stage::NoticeKind,
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

#[derive(Default)]
pub(super) struct Devices {
    watch: Option<Watch>,
    tracker: Tracker,
    shown: Vec<Notice>,
    /// Held by a recording, with the time each came.
    held: Vec<(Notice, u64)>,
    /// The next sample of `--demo-devices`.
    demo: usize,
}

fn kind_of(slot: Slot) -> NoticeKind {
    match slot {
        Slot::Connected => NoticeKind::DeviceConnected,
        Slot::Disconnected => NoticeKind::DeviceDisconnected,
        Slot::Low => NoticeKind::DeviceLow,
        Slot::Switched => NoticeKind::AudioSwitch,
    }
}

impl App {
    /// At start and after the settings changed: the watch runs exactly while one of the
    /// devices' settings is on.
    pub(super) fn sync_devices(&mut self) {
        let s = &self.settings;
        let want = s.notify_devices || s.notify_device_battery || s.notify_audio_switch;
        if want == self.devices.watch.is_some() {
            return;
        }
        if !want {
            self.stop_devices();
            return;
        }
        // HWND can't cross threads, a number can.
        let service = self.service.0 as isize;
        let notify = move || unsafe {
            let _ = PostMessageW(
                Some(HWND(service as *mut _)),
                WM_DEVICES,
                WPARAM(0),
                LPARAM(0),
            );
        };
        match Watch::start(notify) {
            Ok(watch) => self.devices.watch = Some(watch),
            Err(failed) => journal::warn(tr!("journal.devices_unwatched", error = failed.detail)),
        }
    }

    /// Turned on again, the first listing is quiet again: what was connected meanwhile is not news.
    pub(super) fn stop_devices(&mut self) {
        self.devices.watch = None;
        self.devices.tracker = Tracker::default();
        self.devices.held.clear();
        self.kill_timer(timers::DEVICE_WAIT);
    }

    /// WM_DEVICES: the watch has news.
    pub(super) fn on_devices(&mut self) {
        let events = match &self.devices.watch {
            Some(watch) => watch.drain(),
            None => return,
        };
        let now = self.now_ms();
        let notices: Vec<Notice> = events
            .into_iter()
            .flat_map(|event| self.devices.tracker.observe(event, now))
            .collect();
        self.deliver_devices(notices);
    }

    pub(super) fn on_device_wait(&mut self) {
        self.kill_timer(timers::DEVICE_WAIT);
        let notices = self.devices.tracker.tick(self.now_ms());
        self.deliver_devices(notices);
    }

    fn deliver_devices(&mut self, notices: Vec<Notice>) {
        for notice in notices {
            let verdict = notify::allow(
                notice.kind(),
                false,
                self.notify_enabled(),
                self.notice_context(),
            );
            match verdict {
                Verdict::Show => self.show_device(notice),
                Verdict::Wait => {
                    let now = self.now_ms();
                    self.devices.held.retain(|(n, _)| n.slot() != notice.slot());
                    self.devices.held.push((notice, now));
                }
                Verdict::Drop => {}
            }
        }
        let now = self.now_ms();
        match self.devices.tracker.next_wake() {
            Some(at) => self.set_timer(timers::DEVICE_WAIT, at.saturating_sub(now).max(1) as u32),
            None => self.kill_timer(timers::DEVICE_WAIT),
        }
    }

    fn show_device(&mut self, notice: Notice) {
        let now = self.now_ms();
        // Gone while "Connected" is still up: that notice is no longer true.
        if let Notice::Disconnected { id, .. } = &notice {
            let shows_it = self
                .devices
                .shown
                .iter()
                .any(|n| matches!(n, Notice::Connected { id: c, .. } if c == id));
            if shows_it {
                self.stage.end(NoticeKind::DeviceConnected, now);
            }
        }
        let slot = notice.slot();
        self.devices.shown.retain(|n| n.slot() != slot);
        self.devices.shown.push(notice);
        self.stage.post(kind_of(slot), now);
        // A newer notice in a shown slot keeps the stage as it is: the content still changed.
        self.after_stage(true);
    }

    /// After the stage changed: a slot that left the stage forgets its notice; a recording that
    /// ended lets the held ones out while they are fresh.
    pub(super) fn sync_devices_stage(&mut self) {
        let stage = &self.stage;
        self.devices.shown.retain(|n| stage.has(kind_of(n.slot())));
        if !self.devices.held.is_empty() && !self.notice_context().recording {
            let now = self.now_ms();
            let fresh: Vec<Notice> = std::mem::take(&mut self.devices.held)
                .into_iter()
                .filter(|(_, at)| now.saturating_sub(*at) < STALE_MS)
                .map(|(notice, _)| notice)
                .collect();
            self.deliver_devices(fresh);
        }
    }

    pub(super) fn device_notice(&self) -> Option<&Notice> {
        let kind = self.stage.shown().notice?.kind;
        self.devices
            .shown
            .iter()
            .find(|n| kind_of(n.slot()) == kind)
    }

    /// `--demo-devices`: each start with the flag shows the next notice, past every setting and
    /// Do Not Disturb.
    pub(super) fn demo_devices(&mut self) {
        let connected = |name: &str, look, battery| Notice::Connected {
            id: format!("demo-{name}"),
            name: name.into(),
            look,
            battery,
        };
        let switched = |flow, name: &str, form| Notice::Switched {
            flow,
            name: name.into(),
            form,
        };
        let samples = [
            connected("Buds4 Pro", Look::GalaxyBuds, Some(70)),
            connected("AirPods Pro", Look::AirPodsPro, Some(85)),
            connected("Galaxy Buds3 Pro", Look::GalaxyBuds3, Some(45)),
            connected("Redmi Buds 8 Lite", Look::Earbuds, Some(15)),
            connected("WH-1000XM6", Look::OverEar, Some(60)),
            connected("CMF Buds Pro 2", Look::Earbuds, None),
            Notice::Disconnected {
                id: "demo-Buds4 Pro".into(),
                name: "Buds4 Pro".into(),
                look: Look::GalaxyBuds,
            },
            Notice::Low {
                name: "MX Master 3S".into(),
                look: Look::Mouse,
                percent: 10,
            },
            switched(Flow::Output, "Buds4 Pro", Form::Headphones),
            switched(Flow::Output, "DELL U2723QE", Form::Display),
            switched(Flow::Input, "CMF Buds Pro 2", Form::Microphone),
            switched(
                Flow::Input,
                "Microphone Array (Intel Smart Sound Technology for Digital Microphones)",
                Form::Microphone,
            ),
        ];
        let notice = samples[self.devices.demo % samples.len()].clone();
        self.devices.demo += 1;
        self.show_device(notice);
    }
}
