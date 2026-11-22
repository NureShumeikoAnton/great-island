//! Look-alike letters put back into their script.
//!
//! A Cyrillic engine reads Latin words well but sometimes gives a Latin "a" as the Cyrillic
//! "а" (they look the same), and a copied "а restart" then breaks a search. A word that
//! mixes the scripts takes its majority's letters; a word of look-alikes only, among Latin
//! neighbours, becomes Latin. The other way round is left alone: "OK" in a Russian
//! sentence is meant as Latin.

use crate::text::Page;

const PAIRS: [(char, char); 26] = [
    ('а', 'a'),
    ('е', 'e'),
    ('о', 'o'),
    ('р', 'p'),
    ('с', 'c'),
    ('у', 'y'),
    ('х', 'x'),
    ('і', 'i'),
    ('ј', 'j'),
    ('ѕ', 's'),
    ('А', 'A'),
    ('В', 'B'),
    ('Е', 'E'),
    ('К', 'K'),
    ('М', 'M'),
    ('Н', 'H'),
    ('О', 'O'),
    ('Р', 'P'),
    ('С', 'C'),
    ('Т', 'T'),
    ('Х', 'X'),
    ('І', 'I'),
    ('Ј', 'J'),
    ('Ѕ', 'S'),
    ('ԁ', 'd'),
    ('һ', 'h'),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Latin,
    Cyrillic,
}

fn script(c: char) -> Option<Script> {
    if c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c) {
        Some(Script::Latin)
    } else if ('\u{0400}'..='\u{052F}').contains(&c) {
        Some(Script::Cyrillic)
    } else {
        None
    }
}

fn counts(word: &str) -> (usize, usize) {
    word.chars().fold((0, 0), |(l, c), ch| match script(ch) {
        Some(Script::Latin) => (l + 1, c),
        Some(Script::Cyrillic) => (l, c + 1),
        None => (l, c),
    })
}

fn to_latin(c: char) -> Option<char> {
    PAIRS.iter().find(|p| p.0 == c).map(|p| p.1)
}

fn to_cyrillic(c: char) -> Option<char> {
    PAIRS.iter().find(|p| p.1 == c).map(|p| p.0)
}

/// Every letter of `script` other than `target` swapped, or `None` when one has no twin.
fn swapped(word: &str, target: Script) -> Option<String> {
    word.chars()
        .map(|c| match (script(c), target) {
            (Some(Script::Cyrillic), Script::Latin) => to_latin(c),
            (Some(Script::Latin), Script::Cyrillic) => to_cyrillic(c),
            _ => Some(c),
        })
        .collect()
}

pub fn tidy(page: &mut Page) {
    for line in &mut page.lines {
        let scripts: Vec<(usize, usize)> = line.words.iter().map(|w| counts(&w.text)).collect();
        for i in 0..line.words.len() {
            let (latin, cyrillic) = scripts[i];
            let target = if latin > 0 && cyrillic > 0 {
                match latin.cmp(&cyrillic) {
                    std::cmp::Ordering::Greater => Script::Latin,
                    std::cmp::Ordering::Less => Script::Cyrillic,
                    std::cmp::Ordering::Equal => continue,
                }
            } else if cyrillic > 0 && latin_neighbours(&scripts, i) {
                Script::Latin
            } else {
                continue;
            };
            if let Some(text) = swapped(&line.words[i].text, target) {
                line.words[i].text = text;
            }
        }
    }
}

/// The nearest word with letters on each side is Latin, and there is at least one.
fn latin_neighbours(scripts: &[(usize, usize)], i: usize) -> bool {
    let lettered = |&&(l, c): &&(usize, usize)| l + c > 0;
    let before = scripts[..i].iter().rev().find(lettered);
    let after = scripts[i + 1..].iter().find(lettered);
    let latin = |n: Option<&(usize, usize)>| n.map(|&(l, c)| l > 0 && c == 0);
    match (latin(before), latin(after)) {
        (None, None) => false,
        (a, b) => a.unwrap_or(true) && b.unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Line, Rect, Word};

    fn page(words: &[&str]) -> Page {
        Page {
            lines: vec![Line {
                words: words
                    .iter()
                    .enumerate()
                    .map(|(i, w)| Word {
                        text: (*w).into(),
                        rect: Rect::default(),
                        spaced: i > 0,
                    })
                    .collect(),
            }],
        }
    }

    fn tidied(words: &[&str]) -> String {
        let mut page = page(words);
        tidy(&mut page);
        page.lines[0].text()
    }

    #[test]
    fn a_cyrillic_look_alike_among_latin_words_becomes_latin() {
        assert_eq!(tidied(&["Update:", "а", "restart"]), "Update: a restart");
        assert_eq!(
            tidied(&["let", "х", "="]),
            "let x =",
            "a sign has no script"
        );
    }

    #[test]
    fn a_russian_one_letter_word_stays_among_russian_words() {
        assert_eq!(tidied(&["Иван", "а", "также"]), "Иван а также");
        assert_eq!(
            tidied(&["Откройте", "Settings", "и", "Display"]),
            "Откройте Settings и Display"
        );
    }

    #[test]
    fn a_mixed_word_takes_its_majority() {
        assert_eq!(
            tidied(&["Аккумулятoр"]),
            "Аккумулятор",
            "a Latin o in Russian"
        );
        assert_eq!(tidied(&["Windоws"]), "Windows", "a Cyrillic о in English");
    }

    #[test]
    fn latin_look_alikes_in_russian_text_stay_latin() {
        assert_eq!(tidied(&["Нажмите", "OK", "внизу"]), "Нажмите OK внизу");
    }

    #[test]
    fn a_letter_without_a_twin_keeps_the_word() {
        assert_eq!(tidied(&["fпx"]), "fпx", "the п has no Latin twin");
        assert_eq!(tidied(&["ok", "ж", "ok"]), "ok ж ok");
    }
}
