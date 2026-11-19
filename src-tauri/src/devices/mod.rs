//! Devices: what the island tells when a Bluetooth device connects or leaves, runs low, or
//! Windows moves the sound or the microphone to another device. Rules only, under tests:
//! [`kind_of`] and [`look_of`] read what a device is and which picture it gets, [`Tracker`]
//! turns what the adapter reads into notices.
//!
//! The Windows side is in `adapters/`.

pub mod adapters;

use crate::notify;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Headphones,
    Headset,
    Speaker,
    Mouse,
    Keyboard,
    Gamepad,
    Phone,
    Other,
}

impl Kind {
    /// Connecting and leaving are told only for what plays sound: a mouse or a phone wakes up
    /// and sleeps all day.
    pub fn audio(self) -> bool {
        matches!(self, Self::Headphones | Self::Headset | Self::Speaker)
    }
}

/// What Windows knows about a paired device: its category list (`Audio.Headphone`,
/// `Input.Mouse`), the Bluetooth class of device and the LE appearance. The category is the
/// most precise; the classes are for devices Windows has no category for.
pub fn kind_of(
    categories: &[String],
    major: Option<u32>,
    minor: Option<u32>,
    appearance: Option<u32>,
) -> Kind {
    for c in categories {
        let c = c.to_ascii_lowercase();
        let kind = match () {
            _ if c.starts_with("audio.headphone") => Kind::Headphones,
            _ if c.starts_with("communication.headset") => Kind::Headset,
            _ if c.starts_with("audio.speaker") || c.starts_with("multimedia.speaker") => {
                Kind::Speaker
            }
            _ if c.starts_with("input.mouse") || c.starts_with("input.touchpad") => Kind::Mouse,
            _ if c.starts_with("input.keyboard") => Kind::Keyboard,
            _ if c.starts_with("input.gaming") || c.starts_with("input.gamepad") => Kind::Gamepad,
            _ if c.starts_with("communication.phone") => Kind::Phone,
            _ => continue,
        };
        return kind;
    }
    // Class of device: major 4 is audio and video, 5 a peripheral, 2 a phone.
    match (major, minor) {
        (Some(4), Some(1 | 2)) => return Kind::Headset,
        (Some(4), Some(6)) => return Kind::Headphones,
        (Some(4), Some(5 | 7)) => return Kind::Speaker,
        (Some(5), Some(m)) if m & 0x30 == 0x20 => return Kind::Mouse,
        (Some(5), Some(m)) if m & 0x30 == 0x10 => return Kind::Keyboard,
        (Some(5), Some(m)) if m & 0x0f == 2 => return Kind::Gamepad,
        (Some(2), _) => return Kind::Phone,
        _ => {}
    }
    // LE appearance: the category is the upper ten bits.
    match appearance.map(|a| (a >> 6, a & 0x3f)) {
        Some((0x21, 2)) => Kind::Headset,
        Some((0x21, _)) => Kind::Headphones,
        Some((0x0f, 1)) => Kind::Keyboard,
        Some((0x0f, 2)) => Kind::Mouse,
        Some((0x0f, 3 | 4)) => Kind::Gamepad,
        Some((0x01, _)) => Kind::Phone,
        _ => Kind::Other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// AirPods Pro 3 (the Pro 2 looks the same).
    AirPodsPro,
    /// Galaxy Buds4 and Buds4 Pro.
    GalaxyBuds,
    /// Galaxy Buds3 and Buds3 Pro.
    GalaxyBuds3,
    /// Sony WH-1000XM6 (the XM4 and XM5 are close enough).
    OverEar,
    /// Any other earbuds or headset.
    Earbuds,
    Speaker,
    Mouse,
    Keyboard,
    Gamepad,
    Device,
}

impl Look {
    pub fn has_case(self) -> bool {
        matches!(
            self,
            Self::AirPodsPro | Self::GalaxyBuds | Self::GalaxyBuds3 | Self::Earbuds
        )
    }
}

/// By the name Windows shows: the maker's own, unless the user renamed the device.
pub fn look_of(name: &str, kind: Kind) -> Look {
    let n = name.to_lowercase();
    let has = |p: &str| n.contains(p);
    if has("airpods pro") {
        Look::AirPodsPro
    } else if has("buds4") || has("buds 4") {
        Look::GalaxyBuds
    } else if has("buds3") || has("buds 3") {
        Look::GalaxyBuds3
    } else if has("wh-1000xm") || has("wh1000xm") {
        Look::OverEar
    } else {
        match kind {
            Kind::Headphones | Kind::Headset => Look::Earbuds,
            Kind::Speaker => Look::Speaker,
            Kind::Mouse => Look::Mouse,
            Kind::Keyboard => Look::Keyboard,
            Kind::Gamepad => Look::Gamepad,
            Kind::Phone | Kind::Other => Look::Device,
        }
    }
}

/// A device's container, the key that ties its Bluetooth entry, its battery and its sound
/// endpoints together. Windows writes it with or without braces, in either case.
pub fn container_key(raw: &str) -> String {
    raw.trim_matches(|c| c == '{' || c == '}')
        .to_ascii_uppercase()
}

/// What a sound endpoint is, by Windows' form factor (`EndpointFormFactor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Speakers,
    Headphones,
    Headset,
    /// HDMI or DisplayPort: a monitor or a TV.
    Display,
    LineOut,
    Microphone,
    Other,
}

pub fn form_of(factor: Option<u32>) -> Form {
    match factor {
        Some(1) => Form::Speakers,
        Some(3) => Form::Headphones,
        Some(5 | 6) => Form::Headset,
        Some(9) => Form::Display,
        Some(2 | 8) => Form::LineOut,
        Some(4) => Form::Microphone,
        _ => Form::Other,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// [`container_key`] form.
    pub container: Option<String>,
    pub connected: bool,
    /// What Windows' own Bluetooth page shows; most headsets, mice and keyboards report it.
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub id: String,
    /// What the endpoint is ("Speakers", "Headphones (3.5mm connector)").
    pub description: String,
    /// The device it belongs to ("CMF Buds Pro 2 Hands-Free").
    pub adapter: String,
    pub form: Form,
    pub container: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Output,
    Input,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Device(Device),
    Gone(String),
    /// The first listing is over: what changes from here is news.
    Ready,
    Default(Flow, Endpoint),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Connected {
        id: String,
        name: String,
        look: Look,
        battery: Option<u8>,
    },
    Disconnected {
        id: String,
        name: String,
        look: Look,
    },
    Low {
        name: String,
        look: Look,
        percent: u8,
    },
    Switched {
        flow: Flow,
        name: String,
        form: Form,
    },
}

/// Where a notice sits on the island: a newer one in a slot replaces the older.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Connected,
    Disconnected,
    Low,
    Switched,
}

impl Notice {
    pub fn kind(&self) -> notify::Kind {
        match self {
            Self::Connected { .. } | Self::Disconnected { .. } => notify::Kind::Devices,
            Self::Low { .. } => notify::Kind::DeviceBattery,
            Self::Switched { .. } => notify::Kind::AudioSwitch,
        }
    }

    pub fn slot(&self) -> Slot {
        match self {
            Self::Connected { .. } => Slot::Connected,
            Self::Disconnected { .. } => Slot::Disconnected,
            Self::Low { .. } => Slot::Low,
            Self::Switched { .. } => Slot::Switched,
        }
    }
}

/// "Connected" waits this long for the battery: Windows reads it a moment after the link.
pub const BATTERY_WAIT_MS: u64 = 3_000;
/// A switch of the sound to a device that is connecting is part of the connection: it waits
/// this long for the link, and within this after the link it is not said apart.
pub const SAME_MOMENT_MS: u64 = 5_000;
const LEVELS: [u8; 2] = [20, 10];
/// A level is said again once the battery has climbed this far above it.
const RECHARGED: u8 = 5;

struct Known {
    device: Device,
    /// The lowest level said in this discharge.
    said: Option<u8>,
    /// Since when it is connected, for joining a sound switch to the connection.
    linked_at: Option<u64>,
}

/// The lowest level the charge is at or under.
fn level_of(percent: u8) -> Option<u8> {
    LEVELS.iter().copied().filter(|&l| percent <= l).min()
}

#[derive(Default)]
pub struct Tracker {
    ready: bool,
    devices: HashMap<String, Known>,
    /// "Connected" notices waiting for the battery: the device and until when.
    waiting: Vec<(String, u64)>,
    /// Sound switches waiting for their device's link: the notice, its container and until when.
    switches: Vec<(Notice, String, u64)>,
    output: Option<String>,
    input: Option<String>,
}

impl Tracker {
    pub fn observe(&mut self, event: Event, now: u64) -> Vec<Notice> {
        match event {
            Event::Ready => {
                self.ready = true;
                Vec::new()
            }
            Event::Device(device) => self.device(device, now),
            Event::Gone(id) => match self.devices.get(&id) {
                Some(known) => {
                    let device = Device {
                        connected: false,
                        ..known.device.clone()
                    };
                    let notices = self.device(device, now);
                    self.devices.remove(&id);
                    notices
                }
                None => Vec::new(),
            },
            Event::Default(flow, endpoint) => self.switched(flow, endpoint, now),
        }
    }

    /// The notices whose wait is over.
    pub fn tick(&mut self, now: u64) -> Vec<Notice> {
        let mut out = Vec::new();
        let (due, rest): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|(_, at)| *at <= now);
        self.waiting = rest;
        for (id, _) in due {
            if let Some(known) = self.devices.get(&id) {
                out.push(connected(&known.device));
            }
        }
        let (due, rest): (Vec<_>, Vec<_>) =
            self.switches.drain(..).partition(|(_, _, at)| *at <= now);
        self.switches = rest;
        out.extend(due.into_iter().map(|(notice, _, _)| notice));
        out
    }

    pub fn next_wake(&self) -> Option<u64> {
        let waiting = self.waiting.iter().map(|(_, at)| *at);
        let switches = self.switches.iter().map(|(_, _, at)| *at);
        waiting.chain(switches).min()
    }

    fn device(&mut self, device: Device, now: u64) -> Vec<Notice> {
        let id = device.id.clone();
        let ready = self.ready;
        let (was_connected, said, linked_at) = match self.devices.get(&id) {
            Some(k) => (k.device.connected, k.said, k.linked_at),
            None => (false, None, None),
        };
        let mut known = Known {
            device,
            said,
            linked_at,
        };
        let mut out = Vec::new();
        let d = &known.device;
        let linking = d.connected && !was_connected;
        let leaving = !d.connected && was_connected;

        if linking {
            known.linked_at = Some(now);
            // "Connected" shows the charge: no separate low notice for it.
            known.said = d.battery.and_then(level_of);
            if ready && d.kind.audio() {
                if d.battery.is_some() {
                    out.push(connected(d));
                } else {
                    self.waiting.push((id.clone(), now + BATTERY_WAIT_MS));
                }
            }
            if let Some(container) = &d.container {
                // The sound came to this device as it connected: it is part of the link.
                self.switches.retain(|(_, c, _)| c != container);
            }
        } else if leaving {
            known.linked_at = None;
            let was_waiting = self.waiting.iter().any(|(w, _)| *w == id);
            self.waiting.retain(|(w, _)| *w != id);
            // Gone before it was even shown: a blink of the link, not news.
            if ready && d.kind.audio() && !was_waiting {
                out.push(Notice::Disconnected {
                    id: id.clone(),
                    name: d.name.clone(),
                    look: look_of(&d.name, d.kind),
                });
            }
        } else if d.connected
            && let Some(percent) = d.battery
        {
            if self.waiting.iter().any(|(w, _)| *w == id) {
                self.waiting.retain(|(w, _)| *w != id);
                known.said = level_of(percent);
                out.push(connected(d));
            } else {
                if let Some(s) = known.said
                    && percent >= s.saturating_add(RECHARGED)
                {
                    known.said = level_of(percent);
                }
                if let Some(level) = level_of(percent)
                    && known.said.is_none_or(|s| level < s)
                {
                    known.said = Some(level);
                    if ready {
                        out.push(Notice::Low {
                            name: d.name.clone(),
                            look: look_of(&d.name, d.kind),
                            percent,
                        });
                    }
                }
            }
        } else if !ready {
            known.said = d.battery.and_then(level_of);
        }
        self.devices.insert(id, known);
        out
    }

    fn switched(&mut self, flow: Flow, endpoint: Endpoint, now: u64) -> Vec<Notice> {
        let last = match flow {
            Flow::Output => &mut self.output,
            Flow::Input => &mut self.input,
        };
        let first = last.is_none();
        if last.as_deref() == Some(endpoint.id.as_str()) {
            return Vec::new();
        }
        *last = Some(endpoint.id.clone());
        if first {
            return Vec::new();
        }
        let owner = endpoint.container.as_ref().and_then(|c| {
            self.devices
                .values()
                .find(|k| k.device.container.as_ref() == Some(c))
        });
        let name = match owner {
            Some(k) => k.device.name.clone(),
            None => endpoint_name(&endpoint),
        };
        let form = match (flow, endpoint.form) {
            (Flow::Input, Form::Headset) => Form::Headset,
            (Flow::Input, _) => Form::Microphone,
            (Flow::Output, form) => form,
        };
        let notice = Notice::Switched { flow, name, form };
        match (owner, &endpoint.container) {
            (Some(k), _) if k.linked_at.is_some_and(|at| now < at + SAME_MOMENT_MS) => Vec::new(),
            (Some(k), Some(container)) if !k.device.connected => {
                let container = container.clone();
                self.switches.retain(
                    |(n, _, _)| !matches!(n, Notice::Switched { flow: f, .. } if *f == flow),
                );
                self.switches
                    .push((notice, container, now + SAME_MOMENT_MS));
                Vec::new()
            }
            _ => vec![notice],
        }
    }
}

fn connected(d: &Device) -> Notice {
    Notice::Connected {
        id: d.id.clone(),
        name: d.name.clone(),
        look: look_of(&d.name, d.kind),
        battery: d.battery,
    }
}

/// An endpoint of no known device: the computer's own or a wired one. Its description says
/// what it is; the adapter's name is the fallback.
pub fn endpoint_name(endpoint: &Endpoint) -> String {
    let description = endpoint.description.trim();
    if !description.is_empty() {
        return description.to_string();
    }
    endpoint
        .adapter
        .trim()
        .trim_end_matches(" Hands-Free")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDS: &str = "31188775-5EBE-5921-8FA9-3B38DE1FE885";
    /// The container Windows gives the computer itself: built-in speakers and jacks.
    const OWN_CONTAINER: &str = "00000000-0000-0000-FFFF-FFFFFFFFFFFF";

    fn buds(connected: bool, battery: Option<u8>) -> Event {
        Event::Device(Device {
            id: "bt-buds".into(),
            name: "Buds4 Pro".into(),
            kind: Kind::Headphones,
            container: Some(BUDS.into()),
            connected,
            battery,
        })
    }

    fn mouse(battery: u8) -> Event {
        Event::Device(Device {
            id: "bt-mouse".into(),
            name: "MX Master 3S".into(),
            kind: Kind::Mouse,
            container: Some("69E42E99-F1CB-5EE3-AE93-2B9F33044D81".into()),
            connected: true,
            battery: Some(battery),
        })
    }

    fn output(id: &str, container: Option<&str>, form: Form) -> Event {
        Event::Default(
            Flow::Output,
            Endpoint {
                id: id.into(),
                description: "Speakers".into(),
                adapter: "Surface High Definition Audio".into(),
                form,
                container: container.map(str::to_string),
            },
        )
    }

    fn ready() -> Tracker {
        let mut t = Tracker::default();
        t.observe(buds(false, None), 0);
        t.observe(output("speakers", Some(OWN_CONTAINER), Form::Speakers), 0);
        t.observe(Event::Ready, 0);
        t
    }

    #[test]
    fn the_kind_comes_from_the_category_then_the_classes() {
        let cat = |c: &str| vec![c.to_string()];
        assert_eq!(
            kind_of(&cat("Audio.Headphone"), None, None, None),
            Kind::Headphones
        );
        assert_eq!(
            kind_of(
                &cat("Communication.Headset.Bluetooth"),
                Some(4),
                Some(6),
                None
            ),
            Kind::Headset
        );
        assert_eq!(
            kind_of(&cat("Input.Mouse"), None, None, Some(0x03c2)),
            Kind::Mouse
        );
        assert_eq!(kind_of(&[], Some(4), Some(6), None), Kind::Headphones);
        assert_eq!(kind_of(&[], Some(5), Some(0x10), None), Kind::Keyboard);
        assert_eq!(kind_of(&[], None, None, Some(0x0841)), Kind::Headphones);
        assert_eq!(kind_of(&[], None, None, Some(0x03c4)), Kind::Gamepad);
        assert_eq!(kind_of(&[], None, None, None), Kind::Other);
    }

    #[test]
    fn known_headphones_get_their_own_picture() {
        assert_eq!(look_of("AirPods Pro", Kind::Headphones), Look::AirPodsPro);
        assert_eq!(
            look_of("Galaxy Buds4 Pro (A1B2)", Kind::Headset),
            Look::GalaxyBuds
        );
        assert_eq!(look_of("Buds4 Pro", Kind::Headphones), Look::GalaxyBuds);
        assert_eq!(
            look_of("Galaxy Buds3 Pro", Kind::Headset),
            Look::GalaxyBuds3
        );
        assert_eq!(
            look_of("Redmi Buds 8 Lite", Kind::Headphones),
            Look::Earbuds
        );
        assert_eq!(look_of("WH-1000XM6", Kind::Headphones), Look::OverEar);
        assert_eq!(look_of("CMF Buds Pro 2", Kind::Headset), Look::Earbuds);
        assert_eq!(look_of("MX Master 3S", Kind::Mouse), Look::Mouse);
        assert!(Look::GalaxyBuds.has_case() && !Look::OverEar.has_case());
    }

    #[test]
    fn containers_match_whatever_windows_wrote() {
        assert_eq!(
            container_key("{31188775-5ebe-5921-8fa9-3b38de1fe885}"),
            BUDS
        );
        assert_eq!(container_key(BUDS), BUDS);
    }

    #[test]
    fn the_first_listing_is_quiet() {
        let mut t = Tracker::default();
        assert!(t.observe(buds(true, Some(15)), 0).is_empty());
        assert!(t.observe(mouse(8), 0).is_empty());
        t.observe(Event::Ready, 0);
        assert!(
            t.observe(mouse(7), 1_000).is_empty(),
            "10% was passed before start"
        );
    }

    #[test]
    fn connected_shows_the_battery_or_waits_for_it() {
        let mut t = ready();
        let n = t.observe(buds(true, Some(70)), 100);
        assert!(matches!(
            &n[..],
            [Notice::Connected {
                battery: Some(70),
                look: Look::GalaxyBuds,
                ..
            }]
        ));

        let mut t = ready();
        assert!(t.observe(buds(true, None), 100).is_empty());
        assert_eq!(t.next_wake(), Some(100 + BATTERY_WAIT_MS));
        let n = t.observe(buds(true, Some(55)), 900);
        assert!(matches!(
            &n[..],
            [Notice::Connected {
                battery: Some(55),
                ..
            }]
        ));
        assert_eq!(t.next_wake(), None);

        let mut t = ready();
        t.observe(buds(true, None), 100);
        let n = t.tick(100 + BATTERY_WAIT_MS);
        assert!(matches!(&n[..], [Notice::Connected { battery: None, .. }]));
    }

    #[test]
    fn leaving_is_said_unless_the_link_only_blinked() {
        let mut t = ready();
        t.observe(buds(true, Some(70)), 0);
        let n = t.observe(buds(false, Some(70)), 60_000);
        assert!(matches!(&n[..], [Notice::Disconnected { .. }]));

        let mut t = ready();
        t.observe(buds(true, None), 0);
        assert!(t.observe(buds(false, None), 500).is_empty());
        assert!(t.tick(BATTERY_WAIT_MS).is_empty());
    }

    #[test]
    fn a_mouse_is_quiet_about_its_link_but_not_its_battery() {
        let mut t = ready();
        assert!(t.observe(mouse(40), 0).is_empty());
        let n = t.observe(mouse(20), 1_000);
        assert!(matches!(
            &n[..],
            [Notice::Low {
                percent: 20,
                look: Look::Mouse,
                ..
            }]
        ));
        assert!(
            t.observe(mouse(19), 2_000).is_empty(),
            "said once per level"
        );
        let n = t.observe(mouse(9), 3_000);
        assert!(matches!(&n[..], [Notice::Low { percent: 9, .. }]));
        assert!(t.observe(mouse(14), 4_000).is_empty(), "charging back up");
        assert!(t.observe(mouse(10), 5_000).is_empty(), "10% is still said");
        t.observe(mouse(30), 6_000);
        let n = t.observe(mouse(20), 7_000);
        assert_eq!(n.len(), 1, "a new discharge says it again");
    }

    #[test]
    fn a_jump_below_both_levels_is_said_once() {
        let mut t = ready();
        t.observe(mouse(40), 0);
        let n = t.observe(mouse(8), 1_000);
        assert!(matches!(&n[..], [Notice::Low { percent: 8, .. }]));
    }

    #[test]
    fn the_sound_switch_is_said_unless_it_comes_with_the_link() {
        let mut t = ready();
        let n = t.observe(output("hdmi", Some("AAAA"), Form::Display), 0);
        assert!(matches!(
            &n[..],
            [Notice::Switched {
                flow: Flow::Output,
                form: Form::Display,
                ..
            }]
        ));
        assert!(
            t.observe(output("hdmi", Some("AAAA"), Form::Display), 10)
                .is_empty(),
            "same device"
        );

        // The link first, then the sound.
        t.observe(buds(true, Some(70)), 1_000);
        assert!(
            t.observe(output("buds", Some(BUDS), Form::Headphones), 1_500)
                .is_empty()
        );

        // The sound first, then the link.
        let mut t = ready();
        assert!(
            t.observe(output("buds", Some(BUDS), Form::Headphones), 0)
                .is_empty()
        );
        t.observe(buds(true, Some(70)), 300);
        assert!(t.tick(SAME_MOMENT_MS).is_empty());

        // Back to the speakers after the buds left: said, by the endpoint's own name.
        let n = t.observe(
            output("speakers", Some(OWN_CONTAINER), Form::Speakers),
            60_000,
        );
        assert!(matches!(&n[..], [Notice::Switched { name, .. }] if name == "Speakers"));
    }

    #[test]
    fn a_switch_to_a_known_device_is_named_after_it() {
        let mut t = ready();
        t.observe(buds(true, Some(70)), 0);
        let n = t.observe(
            output("buds", Some(BUDS), Form::Headphones),
            SAME_MOMENT_MS + 1,
        );
        assert!(matches!(&n[..], [Notice::Switched { name, .. }] if name == "Buds4 Pro"));
    }

    #[test]
    fn the_microphone_switch_shows_a_microphone() {
        let mut t = ready();
        let mic = |id: &str| {
            Event::Default(
                Flow::Input,
                Endpoint {
                    id: id.into(),
                    description: "Microphone Array".into(),
                    adapter: String::new(),
                    form: Form::Microphone,
                    container: Some(OWN_CONTAINER.into()),
                },
            )
        };
        assert!(
            t.observe(mic("a"), 0).is_empty(),
            "the first one is where it was"
        );
        let n = t.observe(mic("b"), 10);
        assert!(matches!(
            &n[..],
            [Notice::Switched {
                flow: Flow::Input,
                form: Form::Microphone,
                ..
            }]
        ));
    }

    #[test]
    fn an_endpoint_is_named_by_its_description() {
        let e = |description: &str, adapter: &str| Endpoint {
            id: String::new(),
            description: description.into(),
            adapter: adapter.into(),
            form: Form::Other,
            container: None,
        };
        assert_eq!(
            endpoint_name(&e("Surface Omnisonic Speakers", "Surface HD Audio")),
            "Surface Omnisonic Speakers"
        );
        assert_eq!(
            endpoint_name(&e("", "CMF Buds Pro 2 Hands-Free")),
            "CMF Buds Pro 2"
        );
    }
}
