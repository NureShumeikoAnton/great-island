//! What the service says, read leniently: the alerts on now across the country (the map's feed),
//! a place's last alerts, and the news a new answer brings against the one before.
//!
//! The feed lists only regions with an alert, and an alert of a district or an oblast is not
//! repeated in every community under it: a place is under an alert when any region of its chain
//! (`regions::chain`) has one of its own.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Threat {
    Air,
    Artillery,
    UrbanFights,
    Chemical,
    Nuclear,
}

/// Only an air alert has a yellow level; the others are red.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Yellow,
    Red,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alert {
    /// Whose own alert it is.
    pub region: u32,
    pub threat: Threat,
    pub level: Level,
    /// Unix ms.
    pub since: u64,
}

fn threat(name: &str) -> Option<Threat> {
    Some(match name {
        "AIR" => Threat::Air,
        "ARTILLERY" => Threat::Artillery,
        "URBAN_FIGHTS" => Threat::UrbanFights,
        "CHEMICAL" => Threat::Chemical,
        "NUCLEAR" => Threat::Nuclear,
        _ => return None,
    })
}

fn level(name: &str) -> Option<Level> {
    match name.to_ascii_uppercase().as_str() {
        "RED" => Some(Level::Red),
        "YELLOW" => Some(Level::Yellow),
        _ => None,
    }
}

/// "2026-11-23T06:50:37.626626Z" (UTC, the fraction and the Z optional) to Unix ms; the
/// service's "no date" (year 1) and anything unreadable are `None`.
pub fn unix_ms(text: &str) -> Option<u64> {
    let num = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if y < 1970 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from 1970-01-01 to the date (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3_600 + mm * 60 + ss;
    u64::try_from(secs * 1000).ok()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawRegion {
    active_alerts: Vec<RawAlert>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawAlert {
    region_id: String,
    #[serde(rename = "type")]
    kind: String,
    last_update: String,
    active_alert_levels: Vec<RawLevel>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawLevel {
    alert_level: String,
    created_at: String,
}

#[derive(Deserialize)]
struct RawFeed {
    alerts: Vec<RawRegion>,
}

/// Every region's own alerts now. An entry or an alert that doesn't read is skipped alone; an
/// answer without the list is an error (the service changed or sent something else).
pub fn parse_feed(body: &[u8]) -> Result<Vec<Alert>, String> {
    let feed: RawFeed = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let mut out: Vec<Alert> = Vec::new();
    for raw in feed.alerts.iter().flat_map(|r| &r.active_alerts) {
        let (Some(threat), Ok(region)) = (threat(&raw.kind), raw.region_id.parse::<u32>()) else {
            continue;
        };
        let levels: Vec<(Level, Option<u64>)> = raw
            .active_alert_levels
            .iter()
            .filter_map(|l| Some((level(&l.alert_level)?, unix_ms(&l.created_at))))
            .collect();
        let strongest = levels.iter().max_by_key(|l| l.0);
        let level = match (threat, strongest) {
            (Threat::Air, Some(l)) => l.0,
            // An air alert with levels none of which reads: not one we can tell.
            (Threat::Air, None) if !raw.active_alert_levels.is_empty() => continue,
            _ => Level::Red,
        };
        let since = strongest
            .and_then(|l| l.1)
            .or_else(|| unix_ms(&raw.last_update))
            .unwrap_or(0);
        let alert = Alert {
            region,
            threat,
            level,
            since,
        };
        // A parent's alert comes again in every listed child: once is enough.
        if !out.iter().any(|a| a.region == region && a.threat == threat) {
            out.push(alert);
        }
    }
    Ok(out)
}

/// What is on for a place, one alert per threat: the strongest level, then the earliest.
pub fn status(all: &[Alert], chain: &[u32]) -> Vec<Alert> {
    let mut out: Vec<Alert> = Vec::new();
    for alert in all.iter().filter(|a| chain.contains(&a.region)) {
        match out.iter_mut().find(|a| a.threat == alert.threat) {
            Some(kept)
                if (alert.level, std::cmp::Reverse(alert.since))
                    > (kept.level, std::cmp::Reverse(kept.since)) =>
            {
                *kept = *alert;
            }
            Some(_) => {}
            None => out.push(*alert),
        }
    }
    out.sort_by_key(|a| a.threat);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum News {
    Began(Alert),
    /// The same threat at another level.
    Changed(Alert),
    Ended {
        alert: Alert,
        lasted_ms: u64,
    },
}

pub fn news(before: &[Alert], after: &[Alert], now_ms: u64) -> Vec<News> {
    let mut out = Vec::new();
    for a in after {
        match before.iter().find(|b| b.threat == a.threat) {
            None => out.push(News::Began(*a)),
            Some(b) if b.level != a.level => out.push(News::Changed(*a)),
            Some(_) => {}
        }
    }
    for b in before {
        if !after.iter().any(|a| a.threat == b.threat) {
            out.push(News::Ended {
                alert: *b,
                lasted_ms: now_ms.saturating_sub(b.since),
            });
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Past {
    pub threat: Threat,
    pub level: Level,
    pub start: u64,
    /// `None` — still on.
    pub end: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawHistory {
    alarms: Vec<RawPast>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawPast {
    start_date: String,
    end_date: String,
    alert_type: String,
    alert_level: String,
}

/// A place's last alerts (its own and its parents'), newest first.
pub fn parse_history(body: &[u8]) -> Result<Vec<Past>, String> {
    let raw: Vec<RawHistory> = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let mut out: Vec<Past> = raw
        .iter()
        .flat_map(|h| &h.alarms)
        .filter_map(|p| {
            let threat = threat(&p.alert_type)?;
            Some(Past {
                threat,
                level: match threat {
                    Threat::Air => level(&p.alert_level).unwrap_or(Level::Red),
                    _ => Level::Red,
                },
                start: unix_ms(&p.start_date)?,
                end: unix_ms(&p.end_date),
            })
        })
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.start));
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"{"alerts":[
        {"regionId":"1313","regionType":"Community","activeAlerts":[
            {"regionId":"1313","type":"ARTILLERY","lastUpdate":"2024-05-20T09:31:37Z","activeAlertLevels":[{"alertLevel":"Red","createdAt":"2024-05-20T09:31:37Z"}]},
            {"regionId":"122","type":"AIR","lastUpdate":"2026-11-23T06:50:37.626626Z","activeAlertLevels":[{"alertLevel":"Red","reason":"","createdAt":"2026-11-23T06:50:37.753664Z"}]}]},
        {"regionId":"122","regionType":"District","activeAlerts":[
            {"regionId":"122","type":"AIR","lastUpdate":"2026-11-23T06:50:37Z","activeAlertLevels":[{"alertLevel":"Red","createdAt":"2026-11-23T06:50:37.753664Z"}]}]},
        {"regionId":"36","regionType":"District","activeAlerts":[
            {"regionId":"36","type":"AIR","lastUpdate":"x","activeAlertLevels":[{"alertLevel":"Yellow","createdAt":"2026-11-23T01:29:24Z"}]},
            {"regionId":"36","type":"SOMETHING_NEW","lastUpdate":"x","activeAlertLevels":[]}]},
        {"broken":true}
    ],"actionIndex":{"lastActionIndex":1}}"#;

    #[test]
    fn dates_read_as_utc() {
        assert_eq!(unix_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            unix_ms("2026-11-23T06:50:37.626626Z"),
            Some(1_795_416_637_000)
        );
        assert_eq!(unix_ms("0001-01-01T00:00:00"), None);
        assert_eq!(unix_ms("soon"), None);
    }

    #[test]
    fn the_feed_gives_each_region_s_own_alerts_once() {
        let all = parse_feed(FEED.as_bytes()).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            all[1],
            Alert {
                region: 122,
                threat: Threat::Air,
                level: Level::Red,
                since: 1_795_416_637_000
            }
        );
        assert_eq!(all[2].level, Level::Yellow);
        assert!(parse_feed(b"<html>").is_err());
        assert!(parse_feed(b"{}").is_err());
    }

    #[test]
    fn a_place_is_under_its_district_s_alert() {
        let all = parse_feed(FEED.as_bytes()).unwrap();
        // A community of Chuhuivskyi district (122) that has no entry of its own.
        let on = status(&all, &[1312, 122, 22]);
        assert_eq!(on.len(), 1);
        assert_eq!((on[0].region, on[0].threat), (122, Threat::Air));
        let both = status(&all, &[1313, 122, 22]);
        assert_eq!(
            both.iter().map(|a| a.threat).collect::<Vec<_>>(),
            [Threat::Air, Threat::Artillery]
        );
        assert!(status(&all, &[31]).is_empty());
    }

    #[test]
    fn the_stronger_level_wins_within_a_chain() {
        let air = |region, level, since| Alert {
            region,
            threat: Threat::Air,
            level,
            since,
        };
        let all = [air(36, Level::Yellow, 5), air(4, Level::Red, 9)];
        assert_eq!(status(&all, &[155, 36, 4]), [air(4, Level::Red, 9)]);
    }

    #[test]
    fn news_is_what_began_changed_and_ended() {
        let air = |level, since| Alert {
            region: 36,
            threat: Threat::Air,
            level,
            since,
        };
        let yellow = [air(Level::Yellow, 1_000)];
        let red = [air(Level::Red, 2_000)];
        assert_eq!(news(&[], &yellow, 1_500), [News::Began(yellow[0])]);
        assert!(news(&yellow, &yellow, 1_500).is_empty());
        assert_eq!(news(&yellow, &red, 2_000), [News::Changed(red[0])]);
        assert_eq!(
            news(&red, &[], 62_000),
            [News::Ended {
                alert: red[0],
                lasted_ms: 60_000
            }]
        );
    }

    #[test]
    fn history_is_newest_first_with_the_open_one_unended() {
        let body = r#"[{"regionId":"155","alarms":[
            {"startDate":"2026-11-22T20:54:15Z","endDate":"2026-11-22T20:58:46Z","alertType":"AIR","alertLevel":"Yellow"},
            {"startDate":"2026-11-23T08:36:18Z","endDate":"0001-01-01T00:00:00","alertType":"AIR","alertLevel":"Red"},
            {"startDate":"2026-11-23T01:00:00Z","endDate":"2026-11-23T02:00:00Z","alertType":"INFO","alertLevel":""}]}]"#;
        let past = parse_history(body.as_bytes()).unwrap();
        assert_eq!(past.len(), 2);
        assert_eq!((past[0].level, past[0].end), (Level::Red, None));
        assert_eq!(past[1].level, Level::Yellow);
        assert!(past[1].end.is_some());
    }
}
