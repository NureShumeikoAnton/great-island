//! Apps and connectors (rules): what an app is to the island, with no Windows in it.
//!
//! An app is a type implementing [`Applet`]. It keeps its own state, answers an [`Event`] with
//! [`Effect`]s and describes its page as a [`ui::Node`] tree; the island draws the tree in its own
//! look, so an app never draws and never touches the system. A connector is an app whose
//! [`Kind`] names a [`Service`]: it may reach the network, only the hosts it declares, and only
//! once the user has turned it on in the settings. [`host`] checks every effect against the app's
//! [`Manifest`] before the island carries it out; [`catalog`] is the list of apps and which are on.
//!
//! A new app: a module under `apps/`, an `Applet` implementation with its tests, a line in
//! [`catalog::all`], its texts in every language catalog.

pub mod alarm;
pub mod calculator;
pub mod catalog;
pub mod host;
pub mod net;
pub mod prefs;
pub mod ui;

use crate::fault::Failed;
use net::{Request, Response};
use prefs::{Field, Values};
use ui::{Action, Node, Symbol, Tint};

pub struct Manifest {
    /// The key of the app in `settings.json` and in its saved state: never renamed.
    pub id: &'static str,
    pub title: String,
    /// The island's tile and page bar, where the title doesn't fit; empty — the title.
    pub short: String,
    pub icon: Icon,
    pub kind: Kind,
    /// While its page is open the island takes the keyboard and hands the keys to the app; it
    /// gives the keyboard back when the page closes.
    pub keyboard: bool,
    pub lifetime: Lifetime,
    pub fields: Vec<Field>,
}

/// How long what the user did in an app lasts. The island carries it out, the app doesn't
/// clear itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// One opening: when the page closes, the island makes the app anew.
    Page,
    /// While the island runs, with the page closed too (a running stopwatch). Across a restart
    /// only what [`Applet::state`] keeps.
    Island,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    App,
    Connector(Service),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    /// The provider's own name, as it writes it: not translated.
    pub provider: &'static str,
    /// The service's page for people, linked in the settings.
    pub site: &'static str,
    /// Exact host names, https only. Anything else the connector asks for is refused.
    pub hosts: &'static [&'static str],
    /// What leaves the computer and which requests go, one point each: the terms the user
    /// confirms before the connector is turned on.
    pub sends: Vec<String>,
    /// The closing line of the terms; empty — none.
    pub note: String,
}

/// The settings window draws the same icon by this name (`src/settings/pages/Apps.tsx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Icon {
    Calculator,
    Siren,
}

pub struct Context<'a> {
    /// Island milliseconds: for timers.
    pub now: u64,
    /// The wall clock, for what the user reads as a time or a date.
    pub unix_ms: u64,
    /// Local time minus UTC, now.
    pub utc_offset_ms: i64,
    pub values: &'a Values,
}

impl<'a> Context<'a> {
    /// The settings alone: time zero, UTC.
    pub fn bare(values: &'a Values) -> Self {
        Self {
            now: 0,
            unix_ms: 0,
            utc_offset_ms: 0,
            values,
        }
    }
}

/// One item of a [`prefs::FieldKind::Lookup`] list.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Choice {
    /// What is stored.
    pub value: String,
    pub label: String,
    /// Where it is, under the label; empty — nothing.
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// A typed character, after the keyboard layout: `+` is a character, wherever it lies.
    Char(char),
    Enter,
    Backspace,
    Delete,
    /// Ctrl+C.
    Copy,
}

#[derive(Debug)]
pub enum Event {
    /// The island made the app: at its start, or the user turned the app on. Before any other.
    Started,
    Opened,
    /// The app keeps running: its timers and live activity stay.
    Closed,
    Press(Action),
    /// Only while the page is open. Esc never comes here: it closes the page.
    Key(Key),
    Timer(u16),
    /// The answer to an [`Effect::Fetch`] with the same tag.
    Fetched {
        tag: u16,
        result: Result<Response, Failed>,
    },
    Values,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Text to the clipboard. Nothing is said: the pressed part has already shrunk under the click.
    Copy(String),
    Notify(Notice),
    /// One [`Event::Timer`] after `ms`. The same id again moves the timer.
    After {
        timer: u16,
        ms: u64,
    },
    /// An [`Event::Timer`] every `ms`, as [`host::interval`] allows.
    Every {
        timer: u16,
        ms: u64,
    },
    Stop {
        timer: u16,
    },
    /// Connectors only: a GET to a declared host; the answer comes as [`Event::Fetched`].
    Fetch {
        tag: u16,
        request: Request,
    },
    /// [`Applet::state`] changed and is written to the disk.
    Save,
    Close,
}

/// How long it stays is the island's call; a newer notice of the app takes its place.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub symbol: Symbol,
    pub tint: Tint,
    pub title: String,
    /// Empty — the title alone.
    pub text: String,
    pub sound: Option<Chime>,
    /// News the user must not miss: it passes Do Not Disturb and goes before other news.
    pub urgent: bool,
    /// Shown over a fullscreen app whatever the island's fullscreen setting says.
    pub over_fullscreen: bool,
}

/// The island's own sounds for an app's news.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chime {
    Alarm,
    /// Softer than the alarm.
    Caution,
    Clear,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    /// A grey glyph on the collapsed island, next to Do Not Disturb's moon.
    pub badge: Option<Icon>,
    /// A page of the activity ribbon while something goes on.
    pub ribbon: Option<Ribbon>,
}

/// Collapsed: the symbol and the value; under the cursor: all of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ribbon {
    pub symbol: Symbol,
    pub tint: Tint,
    pub title: String,
    pub text: String,
    /// Short: a duration, a count.
    pub value: String,
}

pub trait Applet {
    fn manifest(&self) -> Manifest;
    fn page(&self, cx: &Context) -> Node;
    fn update(&mut self, event: Event, cx: &Context) -> Vec<Effect>;

    /// Asked again after every update: the island compares and redraws what changed.
    fn live(&self, _cx: &Context) -> Option<Live> {
        None
    }

    /// What survives a restart. `None` — nothing to keep.
    fn state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Called once, before the first event, with what [`Applet::state`] gave last time. A value
    /// the app no longer understands is ignored, never a reason to fail.
    fn restore(&mut self, _state: &serde_json::Value) {}

    /// The items of a lookup field that match what the user typed (empty: the first ones), for
    /// the settings window.
    fn lookup(&self, _key: &str, _query: &str) -> Vec<Choice> {
        Vec::new()
    }

    /// How a stored lookup value reads; `None` — not an item any more.
    fn describe(&self, _key: &str, _value: &str) -> Option<Choice> {
        None
    }
}
