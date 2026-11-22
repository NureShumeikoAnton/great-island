//! Links and e-mail addresses in the found text, and what opening one means.
//!
//! Only what can't be mistaken for anything else: a scheme (`https://`), `www.`, or
//! `name@domain.tld`. A bare `example.com` stays text: version numbers and file names look
//! the same. A link is found within one line; a reader rarely keeps one whole across a break.

use crate::text::layout::{Layout, Pos};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub from: Pos,
    pub to: Pos,
    /// What opening it starts: the address with a scheme, `mailto:` for an e-mail.
    pub target: String,
}

pub fn find(layout: &Layout) -> Vec<Link> {
    let mut links = Vec::new();
    for (line, laid) in layout.lines.iter().enumerate() {
        let chars = &laid.chars;
        let mut start = 0;
        while start < chars.len() {
            if chars[start].is_whitespace() {
                start += 1;
                continue;
            }
            let mut end = start;
            while end < chars.len() && !chars[end].is_whitespace() {
                end += 1;
            }
            let (s, e) = trimmed(chars, start, end);
            if s < e
                && let Some(target) = target(&chars[s..e].iter().collect::<String>())
            {
                links.push(Link {
                    from: Pos { line, at: s },
                    to: Pos { line, at: e },
                    target,
                });
            }
            start = end;
        }
    }
    links
}

/// Without the punctuation around a token: brackets, quotes, a full stop after it.
fn trimmed(chars: &[char], mut s: usize, mut e: usize) -> (usize, usize) {
    while s < e && matches!(chars[s], '(' | '[' | '<' | '«' | '"' | '\'' | '“') {
        s += 1;
    }
    while e > s
        && matches!(
            chars[e - 1],
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '>' | '»' | '"' | '\'' | '”'
        )
    {
        e -= 1;
    }
    (s, e)
}

fn target(token: &str) -> Option<String> {
    let lower = token.to_lowercase();
    if ["https://", "http://"].iter().any(|s| lower.starts_with(s)) {
        return (token.len() > lower.find("//")? + 3).then(|| token.to_owned());
    }
    if lower.starts_with("www.") && has_domain(&token[4..]) {
        return Some(format!("https://{token}"));
    }
    let (local, domain) = token.split_once('@')?;
    (!local.is_empty() && !domain.contains('@') && has_domain(domain))
        .then(|| format!("mailto:{token}"))
}

/// `name.tld` with letters in the last part.
fn has_domain(text: &str) -> bool {
    let host = text.split(['/', '?', '#']).next().unwrap_or("");
    let mut parts = host.rsplit('.');
    let tld = parts.next().unwrap_or("");
    parts.next().is_some_and(|name| !name.is_empty())
        && tld.chars().count() >= 2
        && tld.chars().all(char::is_alphabetic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Line, Page, Rect, Word};

    fn layout(text: &str) -> Layout {
        let words = text
            .split(' ')
            .enumerate()
            .map(|(i, w)| Word {
                text: w.into(),
                rect: Rect {
                    x: i as f32 * 100.0,
                    y: 0.0,
                    width: 90.0,
                    height: 14.0,
                },
                spaced: i > 0,
            })
            .collect();
        Layout::new(&Page {
            lines: vec![Line { words }],
        })
    }

    fn targets(text: &str) -> Vec<String> {
        find(&layout(text)).into_iter().map(|l| l.target).collect()
    }

    #[test]
    fn a_link_with_a_scheme_is_found_without_the_full_stop_after_it() {
        let links = find(&layout("Visit https://support.microsoft.com/help."));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "https://support.microsoft.com/help");
        assert_eq!((links[0].from.at, links[0].to.at), (6, 40));
    }

    #[test]
    fn www_and_email_get_their_scheme() {
        assert_eq!(
            targets("(www.example.com) или help@example.com,"),
            ["https://www.example.com", "mailto:help@example.com"]
        );
    }

    #[test]
    fn versions_file_names_and_bare_domains_are_not_links() {
        assert!(targets("Версия 0.8.4 файл report.pdf сайт example.com @user a@b").is_empty());
        assert!(targets("https://").is_empty());
    }
}
