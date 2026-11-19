//! The devices watched on a thread of their own. The Bluetooth watchers and the sound endpoints
//! hand what Windows tells to this thread; here a device's link, kind and battery are put
//! together by its container into a [`Device`], and only what changed goes to the island.
//! While nothing changes the thread sleeps in its wait.

use super::{
    bluetooth::{self, Props, Source, Value},
    endpoints::Endpoints,
};
use crate::{
    devices::{Device, Event, Flow, container_key, kind_of},
    fault::{Cause, Failed},
};
use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

pub(super) enum Raw {
    Changed {
        source: Source,
        id: String,
        props: Props,
    },
    Gone {
        source: Source,
        id: String,
    },
    Listed(Source),
    Default(Flow, String),
    Stop,
}

pub struct Watch {
    tx: Sender<Raw>,
    thread: Option<JoinHandle<()>>,
    events: Receiver<Event>,
}

impl Watch {
    /// `notify` is called on the watch's thread after it sends events.
    pub fn start(notify: impl Fn() + Send + 'static) -> Result<Self, Failed> {
        let (tx, raw) = mpsc::channel();
        let (out, events) = mpsc::channel();
        let (started, setup) = mpsc::sync_channel(1);
        let own = tx.clone();
        let thread = std::thread::Builder::new()
            .name("devices".into())
            .spawn(move || run(&own, &raw, &out, &notify, &started))?;
        let watch = Self {
            tx,
            thread: Some(thread),
            events,
        };
        match setup.recv() {
            Ok(Ok(())) => Ok(watch),
            Ok(Err(failed)) => Err(failed),
            Err(_) => Err(Failed::because(
                Cause::Unsupported,
                "the devices thread ended",
            )),
        }
    }

    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.tx.send(Raw::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    tx: &Sender<Raw>,
    raw: &Receiver<Raw>,
    out: &Sender<Event>,
    notify: &dyn Fn(),
    started: &mpsc::SyncSender<Result<(), Failed>>,
) {
    // The watchers' callbacks come on Windows' thread pool; the endpoints live here.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let setup = || -> Result<_, Failed> {
        let endpoints = Endpoints::start(tx)?;
        let paired = bluetooth::watch(Source::Paired, tx)?;
        let nodes = bluetooth::watch(Source::Nodes, tx)?;
        Ok((endpoints, [paired, nodes]))
    };
    let (endpoints, watchers) = match setup() {
        Ok(parts) => parts,
        Err(failed) => {
            let _ = started.send(Err(failed));
            return;
        }
    };
    let _ = started.send(Ok(()));

    let mut state = State::default();
    let mut sent = false;
    for flow in [Flow::Output, Flow::Input] {
        if let Some(id) = endpoints.default(flow)
            && let Ok(endpoint) = endpoints.read(&id)
        {
            sent |= out.send(Event::Default(flow, endpoint)).is_ok();
        }
    }
    if sent {
        notify();
    }
    for message in raw {
        let events = match message {
            Raw::Stop => break,
            Raw::Default(flow, id) => match endpoints.read(&id) {
                Ok(endpoint) => vec![Event::Default(flow, endpoint)],
                // Gone again by the time it is read: the next default comes as its own event.
                Err(_) => Vec::new(),
            },
            Raw::Changed { source, id, props } => state.changed(source, id, &props),
            Raw::Gone { source, id } => state.gone(source, &id),
            Raw::Listed(source) => state.listed(source),
        };
        let any = !events.is_empty();
        for event in events {
            let _ = out.send(event);
        }
        if any {
            notify();
        }
    }
    for watcher in watchers {
        let _ = watcher.Stop();
    }
}

#[derive(Default)]
struct Paired {
    name: String,
    categories: Vec<String>,
    major: Option<u32>,
    minor: Option<u32>,
    appearance: Option<u32>,
    container: Option<String>,
    connected: bool,
}

#[derive(Default)]
struct Node {
    container: Option<String>,
    battery: Option<u8>,
}

#[derive(Default)]
struct State {
    paired: HashMap<String, Paired>,
    nodes: HashMap<String, Node>,
    sent: HashMap<String, Device>,
    ready: bool,
}

fn text(props: &Props, key: &str) -> Option<String> {
    match props.get(key)? {
        Value::Text(t) => Some(t.clone()),
        _ => None,
    }
}

fn number(props: &Props, key: &str) -> Option<u32> {
    match props.get(key)? {
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

impl State {
    /// An update carries only what changed: the rest stays as it was.
    fn changed(&mut self, source: Source, id: String, props: &Props) -> Vec<Event> {
        match source {
            Source::Paired => {
                let p = self.paired.entry(id.clone()).or_default();
                if let Some(name) = text(props, bluetooth::NAME) {
                    p.name = name;
                }
                if let Some(Value::Flag(on)) = props.get(bluetooth::CONNECTED) {
                    p.connected = *on;
                }
                if let Some(Value::Texts(list)) = props.get(bluetooth::CATEGORY) {
                    p.categories = list.clone();
                }
                p.major = number(props, bluetooth::MAJOR).or(p.major);
                p.minor = number(props, bluetooth::MINOR).or(p.minor);
                p.appearance = number(props, bluetooth::APPEARANCE).or(p.appearance);
                if let Some(c) = text(props, bluetooth::PAIRED_CONTAINER) {
                    p.container = Some(container_key(&c));
                }
                self.send(|_, other| other == id.as_str())
            }
            Source::Nodes => {
                let n = self.nodes.entry(id).or_default();
                if let Some(c) = text(props, bluetooth::NODE_CONTAINER) {
                    n.container = Some(container_key(&c));
                }
                if let Some(b) = number(props, bluetooth::BATTERY) {
                    n.battery = u8::try_from(b.min(100)).ok();
                }
                let container = n.container.clone();
                self.send(|p, _| container.is_some() && p.container == container)
            }
        }
    }

    fn gone(&mut self, source: Source, id: &str) -> Vec<Event> {
        match source {
            Source::Paired => {
                self.paired.remove(id);
                match self.sent.remove(id) {
                    Some(_) => vec![Event::Gone(id.to_string())],
                    None => Vec::new(),
                }
            }
            Source::Nodes => match self.nodes.remove(id).and_then(|n| n.container) {
                Some(container) => self.send(|p, _| p.container.as_deref() == Some(&container)),
                None => Vec::new(),
            },
        }
    }

    /// The paired devices come within a moment, but their listing ends only after Windows has
    /// searched the air for new ones, tens of seconds. The nodes' listing (every device of the
    /// computer, a few seconds) ends long after the paired ones have come: it marks the start.
    fn listed(&mut self, source: Source) -> Vec<Event> {
        if source != Source::Nodes || self.ready {
            return Vec::new();
        }
        self.ready = true;
        vec![Event::Ready]
    }

    /// The devices `pick` chooses, those that differ from what the island last got.
    fn send(&mut self, pick: impl Fn(&Paired, &str) -> bool) -> Vec<Event> {
        let mut events = Vec::new();
        for (id, p) in &self.paired {
            if p.name.is_empty() || !pick(p, id) {
                continue;
            }
            let battery = p.container.as_ref().and_then(|c| {
                self.nodes
                    .values()
                    .filter(|n| n.container.as_ref() == Some(c))
                    .filter_map(|n| n.battery)
                    .min()
            });
            let device = Device {
                id: id.clone(),
                name: p.name.clone(),
                kind: kind_of(&p.categories, p.major, p.minor, p.appearance),
                container: p.container.clone(),
                connected: p.connected,
                battery,
            };
            if self.sent.get(id) != Some(&device) {
                self.sent.insert(id.clone(), device.clone());
                events.push(Event::Device(device));
            }
        }
        events
    }
}
