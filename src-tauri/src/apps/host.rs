//! What the island lets an app do: every [`Effect`] is checked against the app's [`Manifest`]
//! before it is carried out, and a timer is held to what keeps the island cheap. A refusal is
//! the app's own bug: the island writes it to the journal and goes on.

use super::{Effect, Kind, Manifest, net};

/// While the page is open: a stopwatch's tenths.
pub const OPEN_TICK_MS: u64 = 100;
/// While it is closed: a live activity changes once a second at most.
pub const CLOSED_TICK_MS: u64 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A fetch from an app that declared no service.
    NotAConnector,
    /// The address is not https or its host is not among the declared ones.
    Undeclared(String),
}

pub fn admit(manifest: &Manifest, effect: &Effect) -> Result<(), Refusal> {
    match effect {
        Effect::Fetch { request, .. } => match &manifest.kind {
            Kind::App => Err(Refusal::NotAConnector),
            Kind::Connector(service) if net::allowed(service, &request.url) => Ok(()),
            Kind::Connector(_) => Err(Refusal::Undeclared(request.url.clone())),
        },
        Effect::Copy(_)
        | Effect::Notify(_)
        | Effect::After { .. }
        | Effect::Every { .. }
        | Effect::Stop { .. }
        | Effect::Save
        | Effect::Close => Ok(()),
    }
}

/// The period an [`Effect::Every`] really gets; asked again when the page opens or closes.
pub fn interval(asked_ms: u64, open: bool) -> u64 {
    asked_ms.max(if open { OPEN_TICK_MS } else { CLOSED_TICK_MS })
}

/// Every app's timers on one clock: the island keeps one Windows timer, for the nearest.
#[derive(Default)]
pub struct Timers {
    list: Vec<Entry>,
}

struct Entry {
    app: &'static str,
    timer: u16,
    due: u64,
    /// `None` — once.
    every: Option<u64>,
}

impl Timers {
    /// [`Effect::After`] and [`Effect::Every`]: the same app and id again moves the timer.
    pub fn set(&mut self, app: &'static str, timer: u16, ms: u64, repeat: bool, now: u64) {
        self.stop(app, timer);
        let ms = ms.max(OPEN_TICK_MS);
        self.list.push(Entry {
            app,
            timer,
            due: now + ms,
            every: repeat.then_some(ms),
        });
    }

    pub fn stop(&mut self, app: &'static str, timer: u16) {
        self.list.retain(|e| (e.app, e.timer) != (app, timer));
    }

    pub fn drop_app(&mut self, app: &'static str) {
        self.list.retain(|e| e.app != app);
    }

    /// What fired by `now`, in the order it was due. A repeating timer comes once however late
    /// it is, and its next time is counted from `now`; `open` is the app whose page is open.
    pub fn due(&mut self, now: u64, open: Option<&str>) -> Vec<(&'static str, u16)> {
        let mut fired: Vec<(u64, &'static str, u16)> = Vec::new();
        self.list.retain_mut(|e| {
            if e.due > now {
                return true;
            }
            fired.push((e.due, e.app, e.timer));
            match e.every {
                Some(ms) => {
                    e.due = now + interval(ms, open == Some(e.app));
                    true
                }
                None => false,
            }
        });
        fired.sort_by_key(|f| f.0);
        fired
            .into_iter()
            .map(|(_, app, timer)| (app, timer))
            .collect()
    }

    pub fn next(&self) -> Option<u64> {
        self.list.iter().map(|e| e.due).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{Icon, Service, net::Request};

    fn manifest(kind: Kind) -> Manifest {
        Manifest {
            id: "test",
            title: String::new(),
            short: String::new(),
            icon: Icon::Calculator,
            kind,
            keyboard: false,
            lifetime: crate::apps::Lifetime::Island,
            fields: Vec::new(),
        }
    }

    fn fetch(url: &str) -> Effect {
        Effect::Fetch {
            tag: 1,
            request: Request::get(url),
        }
    }

    #[test]
    fn an_app_never_reaches_the_network() {
        assert_eq!(
            admit(&manifest(Kind::App), &fetch("https://example.com/")),
            Err(Refusal::NotAConnector)
        );
        assert_eq!(admit(&manifest(Kind::App), &Effect::Save), Ok(()));
    }

    #[test]
    fn a_connector_reaches_only_its_hosts() {
        let m = manifest(Kind::Connector(Service {
            provider: "Example",
            hosts: &["api.example.com"],
            site: "",
            sends: Vec::new(),
            note: String::new(),
        }));
        assert_eq!(admit(&m, &fetch("https://api.example.com/now")), Ok(()));
        assert_eq!(
            admit(&m, &fetch("https://other.example.com/")),
            Err(Refusal::Undeclared("https://other.example.com/".into()))
        );
    }

    #[test]
    fn timers_fire_in_order_and_repeat_from_now() {
        let mut t = Timers::default();
        t.set("a", 1, 20_000, true, 0);
        t.set("b", 7, 5_000, false, 0);
        assert_eq!(t.next(), Some(5_000));
        assert!(t.due(4_999, None).is_empty());
        assert_eq!(t.due(5_000, None), [("b", 7)]);
        assert_eq!(t.next(), Some(20_000));
        // Late (the computer slept): once, and again a period after now.
        assert_eq!(t.due(90_000, None), [("a", 1)]);
        assert_eq!(t.next(), Some(110_000));
        t.set("a", 1, 1_000, true, 90_000);
        assert_eq!(t.next(), Some(91_000), "the same id moves the timer");
        t.drop_app("a");
        assert_eq!(t.next(), None);
    }

    #[test]
    fn a_repeating_timer_of_a_closed_page_ticks_once_a_second_at_most() {
        let mut t = Timers::default();
        t.set("a", 1, 100, true, 0);
        assert_eq!(t.due(100, Some("a")), [("a", 1)]);
        assert_eq!(t.next(), Some(200));
        assert_eq!(t.due(200, None), [("a", 1)]);
        assert_eq!(t.next(), Some(1_200));
    }

    #[test]
    fn a_closed_page_ticks_once_a_second_at_most() {
        assert_eq!(interval(16, true), OPEN_TICK_MS);
        assert_eq!(interval(16, false), CLOSED_TICK_MS);
        assert_eq!(interval(60_000, false), 60_000);
    }
}
