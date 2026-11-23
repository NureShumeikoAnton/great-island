//! Air raid alerts: the first connector. Its source is the public map of Ukraine Alarm
//! (map.ukrainealarm.com): one request for the whole country every 20 s, the chosen place picked
//! out here, so the regular request is the same for everyone. The place's last alerts are asked
//! for only when its page opens.
//!
//! News is the difference against the answer before: an alert began, changed its level, ended.
//! Red and the other threats are urgent; yellow is ordinary news with a softer sound; the end is
//! quiet. While an alert is on, the connector is a page of the ribbon and, if the user wants,
//! a grey siren on the collapsed island.

mod feed;
pub mod regions;

use super::{
    Applet, Chime, Choice, Context, Effect, Event, Icon, Kind, Lifetime, Live, Manifest, Notice,
    Ribbon, Service,
    net::Request,
    prefs::{Field, FieldKind},
    ui::{Align, Node, Style, Symbol, Tint},
};
use crate::lang::tr;
use feed::{Alert, Level, News, Past, Threat};

const FEED: &str = "https://map.ukrainealarm.com/api/v2/data/mapUpdate";
const HISTORY: &str = "https://map.ukrainealarm.com/api/data/lastAlerts/";
const POLL: u16 = 1;
const POLL_MS: u64 = 20_000;
const FEED_TAG: u16 = 1;
const HISTORY_TAG: u16 = 2;
/// The page keeps one height: the island's window is sized to hold it.
const HISTORY_ROWS: usize = 6;
const MINUTE: u64 = 60_000;
const DAY: i64 = 86_400_000;

#[derive(Default)]
pub struct Alarm {
    /// The region id as stored; empty — none chosen.
    place: String,
    /// `None` until the first answer for this place.
    status: Option<Vec<Alert>>,
    history: Vec<Past>,
    /// The last request failed: the page says so, what is shown is the last answer.
    offline: bool,
    open: bool,
}

fn threat_title(threat: Threat) -> String {
    match threat {
        Threat::Air => tr!("app.alarm.air"),
        Threat::Artillery => tr!("app.alarm.artillery"),
        Threat::UrbanFights => tr!("app.alarm.urban_fights"),
        Threat::Chemical => tr!("app.alarm.chemical"),
        Threat::Nuclear => tr!("app.alarm.nuclear"),
    }
    .into()
}

fn region_name(id: u32) -> String {
    regions::get(id).map_or_else(String::new, |r| r.name.to_string())
}

fn duration(ms: u64) -> String {
    let minutes = (ms / MINUTE).max(1);
    if minutes < 60 {
        tr!("app.alarm.minutes", n = minutes).into()
    } else {
        tr!("app.alarm.hours", h = minutes / 60, m = minutes % 60).into()
    }
}

fn local(unix_ms: u64, cx: &Context) -> i64 {
    unix_ms as i64 + cx.utc_offset_ms
}

fn clock(unix_ms: u64, cx: &Context) -> String {
    let minutes = local(unix_ms, cx).rem_euclid(DAY) / MINUTE as i64;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn day(unix_ms: u64, cx: &Context) -> String {
    let days = |ms: i64| ms.div_euclid(DAY);
    let (then, today) = (days(local(unix_ms, cx)), days(local(cx.unix_ms, cx)));
    match today - then {
        0 => tr!("app.alarm.today").into(),
        1 => tr!("app.alarm.yesterday").into(),
        _ => {
            // Civil date of a day count (Howard Hinnant's civil_from_days).
            let z = then + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            format!("{d:02}.{m:02}")
        }
    }
}

fn level_word(past: &Past) -> String {
    match (past.threat, past.level) {
        (Threat::Air, Level::Yellow) => tr!("app.alarm.level_yellow").into(),
        (Threat::Air, Level::Red) => tr!("app.alarm.level_red").into(),
        (threat, _) => threat_title(threat),
    }
}

impl Alarm {
    fn place_id(&self) -> Option<u32> {
        self.place
            .parse()
            .ok()
            .filter(|&id| regions::get(id).is_some())
    }

    fn feed(&self) -> Vec<Effect> {
        if self.place_id().is_none() {
            return Vec::new();
        }
        vec![Effect::Fetch {
            tag: FEED_TAG,
            request: Request::get(FEED),
        }]
    }

    fn history(&self) -> Vec<Effect> {
        match self.place_id() {
            Some(id) => vec![Effect::Fetch {
                tag: HISTORY_TAG,
                request: Request::get(format!("{HISTORY}{id}")),
            }],
            None => Vec::new(),
        }
    }

    fn notice(&self, news: News, cx: &Context) -> Notice {
        let red_over = cx.values.toggle("fullscreen_red");
        let region = |alert: &Alert| region_name(alert.region);
        match news {
            News::Began(alert) | News::Changed(alert) => match (alert.threat, alert.level) {
                (Threat::Air, Level::Yellow) => Notice {
                    symbol: Symbol::Siren,
                    tint: Tint::Caution,
                    title: threat_title(Threat::Air),
                    text: tr!("app.alarm.yellow", region = region(&alert)).into(),
                    sound: Some(Chime::Caution),
                    urgent: false,
                    over_fullscreen: cx.values.toggle("fullscreen_yellow"),
                },
                (threat, _) => Notice {
                    symbol: if threat == Threat::Air {
                        Symbol::Siren
                    } else {
                        Symbol::Warning
                    },
                    tint: Tint::Alarm,
                    title: threat_title(threat),
                    text: region(&alert),
                    sound: Some(Chime::Alarm),
                    urgent: true,
                    over_fullscreen: red_over,
                },
            },
            News::Ended { alert, lasted_ms } => Notice {
                symbol: Symbol::Check,
                tint: Tint::Clear,
                title: if alert.threat == Threat::Air {
                    tr!("app.alarm.all_clear_air").into()
                } else {
                    tr!("app.alarm.all_clear").into()
                },
                text: tr!(
                    "app.alarm.lasted",
                    region = region(&alert),
                    duration = duration(lasted_ms)
                )
                .into(),
                sound: Some(Chime::Clear),
                urgent: false,
                over_fullscreen: match alert.level {
                    Level::Red => red_over,
                    Level::Yellow => cx.values.toggle("fullscreen_yellow"),
                },
            },
        }
    }

    fn answer(&mut self, body: &[u8], cx: &Context) -> Vec<Effect> {
        let (Ok(all), Some(id)) = (feed::parse_feed(body), self.place_id()) else {
            self.offline = true;
            return Vec::new();
        };
        self.offline = false;
        let now = feed::status(&all, &regions::chain(id));
        let before = self.status.replace(now.clone()).unwrap_or_default();
        let news = feed::news(&before, &now, cx.unix_ms);
        // The page shows the place's history: an alert that began or ended belongs in it.
        let mut effects: Vec<Effect> = news
            .into_iter()
            .map(|n| Effect::Notify(self.notice(n, cx)))
            .collect();
        if self.open && !effects.is_empty() {
            effects.extend(self.history());
        }
        effects
    }

    fn values_changed(&mut self, cx: &Context) -> Vec<Effect> {
        let place = cx.values.text("place");
        if place == self.place {
            return Vec::new();
        }
        self.place = place.to_string();
        self.status = None;
        self.history.clear();
        self.offline = false;
        let mut effects = self.feed();
        if self.open {
            effects.extend(self.history());
        }
        effects
    }

    fn status_lines(&self, cx: &Context) -> Vec<(String, String)> {
        let Some(id) = self.place_id() else {
            return vec![(
                tr!("app.alarm.no_place").into(),
                tr!("app.alarm.no_place_hint").into(),
            )];
        };
        let offline = || -> String { tr!("app.alarm.offline").into() };
        match &self.status {
            None if self.offline => vec![(offline(), region_name(id))],
            None => vec![(tr!("app.alarm.waiting").into(), region_name(id))],
            Some(on) if on.is_empty() => {
                let text = if self.offline {
                    offline()
                } else {
                    region_name(id)
                };
                vec![(tr!("app.alarm.calm").into(), text)]
            }
            Some(on) => on
                .iter()
                .map(|a| {
                    let mut text = format!(
                        "{} · {}",
                        region_name(a.region),
                        duration(cx.unix_ms.saturating_sub(a.since))
                    );
                    if a.threat == Threat::Air && a.level == Level::Yellow {
                        text = tr!("app.alarm.yellow", region = text).into();
                    }
                    (threat_title(a.threat), text)
                })
                .collect(),
        }
    }

    fn history_row(past: &Past, cx: &Context) -> Node {
        let left = match past.end {
            None => tr!(
                "app.alarm.since",
                day = day(past.start, cx),
                time = clock(past.start, cx)
            )
            .into(),
            Some(end) => format!(
                "{} {}–{}",
                day(past.start, cx),
                clock(past.start, cx),
                clock(end, cx)
            ),
        };
        let lasted = past.end.unwrap_or(cx.unix_ms).saturating_sub(past.start);
        let right = format!("{} · {}", duration(lasted), level_word(past));
        Node::Row(vec![
            Node::text(left, Style::Body, Align::Start),
            Node::text(right, Style::Caption, Align::End),
        ])
    }
}

impl Applet for Alarm {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "air_alarm",
            title: tr!("app.alarm.title").into(),
            short: tr!("app.alarm.short").into(),
            icon: Icon::Siren,
            kind: Kind::Connector(Service {
                provider: "Ukraine Alarm",
                site: "https://map.ukrainealarm.com/",
                hosts: &["map.ukrainealarm.com"],
                sends: vec![
                    tr!("app.alarm.sends_address").into(),
                    tr!("app.alarm.sends_feed").into(),
                    tr!("app.alarm.sends_place").into(),
                ],
                note: tr!("app.alarm.note").into(),
            }),
            keyboard: false,
            lifetime: Lifetime::Island,
            fields: vec![
                Field {
                    key: "place",
                    label: tr!("app.alarm.place").into(),
                    sub: tr!("app.alarm.place_sub").into(),
                    group: Some(String::new()),
                    kind: FieldKind::Lookup,
                },
                Field {
                    key: "fullscreen_red",
                    label: tr!("app.alarm.fullscreen_red").into(),
                    sub: tr!("app.alarm.fullscreen_red_sub").into(),
                    group: Some(tr!("app.alarm.fullscreen").into()),
                    kind: FieldKind::Toggle { default: true },
                },
                Field {
                    key: "fullscreen_yellow",
                    label: tr!("app.alarm.fullscreen_yellow").into(),
                    sub: String::new(),
                    group: None,
                    kind: FieldKind::Toggle { default: false },
                },
                Field {
                    key: "badge",
                    label: tr!("app.alarm.badge").into(),
                    sub: tr!("app.alarm.badge_sub").into(),
                    group: Some(String::new()),
                    kind: FieldKind::Toggle { default: true },
                },
            ],
        }
    }

    fn page(&self, cx: &Context) -> Node {
        let mut parts = Vec::new();
        let mut lines = self.status_lines(cx);
        // At most two threats at once are shown; the rest wait for the first to end.
        lines.resize(2, (String::new(), String::new()));
        for (title, text) in lines {
            parts.push(Node::text(title, Style::Title, Align::Start));
            parts.push(Node::text(text, Style::Caption, Align::Start));
        }
        parts.push(Node::Spacer);
        let recent = if self.history.is_empty() {
            String::new()
        } else {
            tr!("app.alarm.recent").into()
        };
        parts.push(Node::text(recent, Style::Caption, Align::Start));
        for i in 0..HISTORY_ROWS {
            parts.push(match self.history.get(i) {
                Some(past) => Self::history_row(past, cx),
                None => Node::Row(vec![
                    Node::text("", Style::Body, Align::Start),
                    Node::text("", Style::Caption, Align::End),
                ]),
            });
        }
        Node::Column(parts)
    }

    fn update(&mut self, event: Event, cx: &Context) -> Vec<Effect> {
        match event {
            Event::Started => {
                self.place = cx.values.text("place").to_string();
                let mut effects = vec![Effect::Every {
                    timer: POLL,
                    ms: POLL_MS,
                }];
                effects.extend(self.feed());
                effects
            }
            Event::Values => self.values_changed(cx),
            Event::Timer(POLL) => self.feed(),
            Event::Opened => {
                self.open = true;
                self.history()
            }
            Event::Closed => {
                self.open = false;
                Vec::new()
            }
            Event::Fetched { tag, result } => match (tag, result) {
                (FEED_TAG, Ok(response)) => self.answer(&response.body, cx),
                (FEED_TAG, Err(_)) => {
                    self.offline = true;
                    Vec::new()
                }
                (HISTORY_TAG, Ok(response)) => {
                    if let Ok(past) = feed::parse_history(&response.body) {
                        self.history = past;
                    }
                    Vec::new()
                }
                _ => Vec::new(),
            },
            // No keys, no presses: the page is to be read.
            Event::Timer(_) | Event::Press(_) | Event::Key(_) => Vec::new(),
        }
    }

    fn live(&self, cx: &Context) -> Option<Live> {
        let alert = self.status.as_ref()?.first()?;
        let (tint, symbol) = match (alert.threat, alert.level) {
            (Threat::Air, Level::Yellow) => (Tint::Caution, Symbol::Siren),
            (Threat::Air, Level::Red) => (Tint::Alarm, Symbol::Siren),
            _ => (Tint::Alarm, Symbol::Warning),
        };
        Some(Live {
            badge: cx.values.toggle("badge").then_some(Icon::Siren),
            ribbon: Some(Ribbon {
                symbol,
                tint,
                title: threat_title(alert.threat),
                text: tr!(
                    "app.alarm.region_since",
                    region = region_name(alert.region),
                    time = clock(alert.since, cx)
                )
                .into(),
                value: duration(cx.unix_ms.saturating_sub(alert.since)),
            }),
        })
    }

    fn lookup(&self, key: &str, query: &str) -> Vec<Choice> {
        if key != "place" {
            return Vec::new();
        }
        regions::search(query, 60)
            .into_iter()
            .map(|r| Choice {
                value: r.id.to_string(),
                label: r.name.to_string(),
                detail: regions::place_of(r.id),
            })
            .collect()
    }

    fn describe(&self, key: &str, value: &str) -> Option<Choice> {
        let region = regions::get(value.parse().ok()?).filter(|_| key == "place")?;
        Some(Choice {
            value: value.to_string(),
            label: region.name.to_string(),
            detail: regions::place_of(region.id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{net::Response, prefs::Values};
    use crate::lang::{self, Lang};

    fn values(place: &str) -> Values {
        let m = Alarm::default().manifest();
        let prefs: crate::apps::prefs::Prefs =
            serde_json::from_value(serde_json::json!({ "values": { "place": place } })).unwrap();
        Values::resolve(&m.fields, Some(&prefs))
    }

    fn cx(values: &Values, unix_ms: u64) -> Context<'_> {
        Context {
            now: 0,
            unix_ms,
            utc_offset_ms: 2 * 3_600_000,
            values,
        }
    }

    fn feed(json: &str) -> Event {
        Event::Fetched {
            tag: FEED_TAG,
            result: Ok(Response {
                status: 200,
                body: json.as_bytes().to_vec(),
            }),
        }
    }

    fn outline(node: &Node) -> String {
        match node {
            Node::Column(c) | Node::Row(c) => {
                format!("[{}]", c.iter().map(outline).collect::<Vec<_>>().join(","))
            }
            Node::Text { style, .. } => format!("{style:?}"),
            other => format!("{other:?}"),
        }
    }

    /// 2026-11-23 08:00 UTC: 10:00 in Kyiv.
    const EIGHT: u64 = 1_795_420_800_000;

    const KYIV_RED: &str = r#"{"alerts":[{"regionId":"31","activeAlerts":[{"regionId":"31","type":"AIR","lastUpdate":"2026-11-23T08:00:00Z","activeAlertLevels":[{"alertLevel":"Red","createdAt":"2026-11-23T08:00:00Z"}]}]}]}"#;
    const QUIET: &str = r#"{"alerts":[]}"#;

    #[test]
    fn nothing_is_asked_without_a_place() {
        let v = values("");
        let mut alarm = Alarm::default();
        let started = alarm.update(Event::Started, &cx(&v, 0));
        assert_eq!(started.len(), 1, "only the timer: {started:?}");
        assert!(alarm.update(Event::Timer(POLL), &cx(&v, 0)).is_empty());
        assert!(alarm.update(Event::Opened, &cx(&v, 0)).is_empty());
    }

    #[test]
    fn an_alert_begins_and_ends_with_its_notices() {
        lang::with(Lang::Ru, || {
            let v = values("31");
            let mut alarm = Alarm::default();
            let started = alarm.update(Event::Started, &cx(&v, 0));
            assert!(matches!(started[1], Effect::Fetch { tag: FEED_TAG, .. }));

            let began = alarm.update(feed(KYIV_RED), &cx(&v, EIGHT + 10_000));
            let [Effect::Notify(notice)] = &began[..] else {
                panic!("{began:?}")
            };
            assert_eq!(notice.title, "Воздушная тревога");
            assert_eq!(notice.text, "м. Київ");
            assert!(notice.urgent && notice.over_fullscreen);
            assert!(
                alarm
                    .update(feed(KYIV_RED), &cx(&v, EIGHT + 30_000))
                    .is_empty()
            );
            let live = alarm.live(&cx(&v, EIGHT + 30_000)).unwrap();
            assert_eq!(live.badge, Some(Icon::Siren));
            assert_eq!(live.ribbon.unwrap().text, "м. Київ · с 10:00");

            let ended = alarm.update(feed(QUIET), &cx(&v, EIGHT + 47 * MINUTE));
            let [Effect::Notify(notice)] = &ended[..] else {
                panic!("{ended:?}")
            };
            assert_eq!(notice.title, "Отбой воздушной тревоги");
            assert_eq!(notice.text, "м. Київ · длилась 47 мин");
            assert!(!notice.urgent);
            assert!(alarm.live(&cx(&v, 0)).is_none());
        });
    }

    #[test]
    fn a_failed_answer_keeps_the_last_one() {
        let v = values("31");
        let mut alarm = Alarm::default();
        alarm.update(Event::Started, &cx(&v, 0));
        alarm.update(feed(KYIV_RED), &cx(&v, EIGHT));
        let failed = Event::Fetched {
            tag: FEED_TAG,
            result: Err(crate::fault::Failed::because(
                crate::fault::Cause::Unknown,
                "timeout",
            )),
        };
        assert!(alarm.update(failed, &cx(&v, 0)).is_empty());
        assert!(alarm.offline);
        assert!(alarm.live(&cx(&v, 0)).is_some(), "the alert stays shown");
    }

    #[test]
    fn the_page_keeps_its_height() {
        lang::with(Lang::Ru, || {
            let v = values("31");
            let mut alarm = Alarm::default();
            let shape = |a: &Alarm| outline(&a.page(&cx(&v, 1_795_421_000_000)));
            let empty = shape(&alarm);
            alarm.update(Event::Started, &cx(&v, 0));
            alarm.update(feed(KYIV_RED), &cx(&v, 1_795_421_000_000));
            assert_eq!(shape(&alarm), empty);
        });
    }

    #[test]
    fn the_place_is_found_and_named() {
        let alarm = Alarm::default();
        let found = alarm.lookup("place", "вінниц");
        assert_eq!(found[0].label, "Вінницька область");
        let chosen = alarm.describe("place", "155").unwrap();
        assert_eq!(chosen.detail, "Вінницький район, Вінницька область");
        assert!(alarm.describe("place", "nope").is_none());
    }

    #[test]
    fn days_and_times_are_local() {
        lang::with(Lang::Ru, || {
            let v = values("");
            // 2026-11-23 10:30 in Kyiv (UTC+2).
            let c = cx(&v, 1_795_422_600_000);
            assert_eq!(clock(1_795_422_600_000, &c), "10:30");
            assert_eq!(day(1_795_422_600_000, &c), "сегодня");
            assert_eq!(day(1_795_422_600_000 - 86_400_000, &c), "вчера");
            assert_eq!(day(1_795_422_600_000 - 2 * 86_400_000, &c), "21.11");
        });
    }
}
