//! The journal as the settings window shows it: lines parsed back into entries, newest first,
//! a run of the same entry folded into one. Pure: the caller reads the file.
//!
//! A problem's line is written by [`fault_line`] and read back by its code, so its title and reason
//! come out in the language of the window, whatever language wrote them.

use crate::{
    fault::{Cause, Fault, What},
    lang::Lang,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Local time to the second, "2026-11-17 10:28:03".
    pub at: String,
    pub mark: Mark,
    /// "UPD-CHECK/MISSING" for a problem the island reported; none for a plain line.
    pub code: Option<String>,
    /// The problem's title, or the plain line's own text.
    pub title: String,
    pub reason: Option<String>,
    /// What Windows or the code said.
    pub detail: String,
    /// 1 for a single entry.
    pub times: u32,
    /// The newest line as it is in the file.
    pub line: String,
}

impl Entry {
    fn same_as(&self, other: &Entry) -> bool {
        self.mark == other.mark
            && self.code == other.code
            && self.title == other.title
            && self.detail == other.detail
    }
}

/// `CODE Title [subject] · detail`; the title is there for a person reading the file.
pub fn fault_line(fault: &Fault) -> String {
    let mut line = format!("{} {}", fault.code(), fault.title());
    if let Some(subject) = &fault.subject {
        line += &format!(" [{subject}]");
    }
    if !fault.detail.is_empty() {
        line += &format!(" · {}", fault.detail);
    }
    line
}

/// `problems_only` leaves out the plain lines; repeats fold after that, so a problem that came back
/// between two plain lines is still one entry.
pub fn parse(text: &str, problems_only: bool) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for entry in text.lines().rev().filter_map(parse_line) {
        if problems_only && entry.mark == Mark::Info {
            continue;
        }
        match entries.last_mut() {
            Some(newer) if newer.same_as(&entry) => newer.times += 1,
            _ => entries.push(entry),
        }
    }
    entries
}

/// `2026-11-17 10:28:03.318 [error] text`; anything else is not an entry.
fn parse_line(line: &str) -> Option<Entry> {
    let (stamp, rest) = line.split_at_checked(23)?;
    let bytes = stamp.as_bytes();
    if bytes[4] != b'-' || bytes[10] != b' ' || bytes[19] != b'.' {
        return None;
    }
    let (mark, text) = rest.strip_prefix(" [")?.split_once(']')?;
    let mark = match mark {
        "info" => Mark::Info,
        "warn" => Mark::Warn,
        "error" => Mark::Error,
        _ => return None,
    };
    let text = text.trim();
    let mut entry = Entry {
        at: stamp[..19].to_string(),
        mark,
        code: None,
        title: text.to_string(),
        reason: None,
        detail: String::new(),
        times: 1,
        line: line.to_string(),
    };
    if mark != Mark::Info
        && let Some((code, fault)) = problem(text)
    {
        entry.code = Some(code.to_string());
        entry.title = fault.title();
        entry.reason = Some(fault.reason());
        entry.detail = fault.detail;
    }
    Some(entry)
}

fn problem(text: &str) -> Option<(&str, Fault)> {
    let (code, rest) = text.split_once(' ').unwrap_or((text, ""));
    let (what, cause) = code.split_once('/')?;
    let (what, cause) = (What::from_code(what)?, Cause::from_code(cause)?);
    // Lines before 0.7.5 have no separators: "CODE Title subject detail"; their subject stays in the detail.
    let rest = Lang::ALL
        .iter()
        .find_map(|&lang| rest.strip_prefix(what.title_in(lang).as_str()))
        .unwrap_or(rest)
        .trim();
    let (subject, detail) = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((subject, after)) => (Some(subject.to_string()), after.trim_start()),
        None => (None, rest),
    };
    let detail = detail
        .strip_prefix('·')
        .unwrap_or(detail)
        .trim()
        .to_string();
    Some((
        code,
        Fault {
            what,
            cause,
            subject,
            detail,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fault::Failed, lang};

    fn at(text: &str) -> String {
        format!("2026-11-17 10:28:03.318 {text}")
    }

    #[test]
    fn a_problem_comes_back_from_its_line() {
        lang::with(Lang::Ru, || {
            let fault = Fault::new(
                What::UpdateCheck,
                Failed::because(Cause::Missing, "HTTP 404"),
            );
            let entries = parse(&at(&format!("[error] {}", fault_line(&fault))), false);
            assert_eq!(entries.len(), 1);
            let entry = &entries[0];
            assert_eq!(entry.at, "2026-11-17 10:28:03");
            assert_eq!(entry.mark, Mark::Error);
            assert_eq!(entry.code.as_deref(), Some("UPD-CHECK/MISSING"));
            assert_eq!(entry.title, fault.title());
            assert_eq!(entry.reason, Some(fault.reason()));
            assert_eq!(entry.detail, "HTTP 404");
        });
    }

    #[test]
    fn the_subject_reaches_the_reason() {
        lang::with(Lang::En, || {
            let fault = Fault::new(What::ShotKey, Failed::because(Cause::Taken, "0x80070581"))
                .about("Ctrl + Shift + S");
            let entry = &parse(&at(&format!("[warn] {}", fault_line(&fault))), false)[0];
            assert_eq!(entry.reason, Some(fault.reason()));
            assert!(
                entry
                    .reason
                    .as_deref()
                    .unwrap()
                    .contains("Ctrl + Shift + S")
            );
            assert_eq!(entry.detail, "0x80070581");
        });
    }

    #[test]
    fn an_old_line_is_read_in_the_other_language() {
        lang::with(Lang::En, || {
            let line = at("[error] UPD-CHECK/MISSING Проверка обновлений не удалась  HTTP 404");
            let entry = &parse(&line, false)[0];
            assert_eq!(entry.title, What::UpdateCheck.title_in(Lang::En));
            assert_eq!(entry.detail, "HTTP 404");
            assert_eq!(entry.line, line);
        });
    }

    #[test]
    fn repeats_fold_and_the_newest_comes_first() {
        let problem = at("[error] UPD-CHECK/MISSING Update check failed · HTTP 404");
        let text = [
            at("[info] island started"),
            problem.clone(),
            problem.clone(),
            at("[info] recording started"),
            problem.clone(),
            "not a journal line".to_string(),
            at("[info] island stopped"),
        ]
        .join("\n");

        let all = parse(&text, false);
        let titles: Vec<_> = all.iter().map(|e| (e.title.as_str(), e.times)).collect();
        assert_eq!(all.len(), 5);
        assert_eq!(titles[0], ("island stopped", 1));
        assert_eq!(all[2].times, 1);
        assert_eq!(all[3].times, 2);

        let problems = parse(&text, true);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].times, 3);
    }

    #[test]
    fn a_plain_warning_keeps_its_text() {
        let entry = &parse(&at("[warn] the update check failed: HTTP 404"), false)[0];
        assert_eq!(entry.mark, Mark::Warn);
        assert_eq!(entry.code, None);
        assert_eq!(entry.title, "the update check failed: HTTP 404");
    }
}
