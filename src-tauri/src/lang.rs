//! The interface language (Auto / English / Russian) and the `tr!` lookup into `locales/`.
//! Pure logic: the one Windows call is `platform::ui_languages`.

use crate::{fault::Failed, platform, settings::Language};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ru => "ru",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|lang| lang.code() == code)
    }

    pub fn negotiate<S: AsRef<str>>(tags: &[S]) -> Option<Self> {
        tags.iter().find_map(|tag| {
            let primary = tag.as_ref().split(['-', '_']).next()?.to_ascii_lowercase();
            Self::from_code(&primary)
        })
    }
}

/// `Failed` = Windows would not say its language: English is taken and the island reports it.
fn choose(
    choice: Language,
    system: impl FnOnce() -> Result<Vec<String>, Failed>,
) -> (Lang, Option<Failed>) {
    match choice {
        Language::En => (Lang::En, None),
        Language::Ru => (Lang::Ru, None),
        Language::Auto => match system() {
            Ok(tags) => (Lang::negotiate(&tags).unwrap_or(Lang::En), None),
            Err(failed) => (Lang::En, Some(failed)),
        },
    }
}

pub fn resolve(choice: Language) -> Lang {
    match choice {
        Language::En => Lang::En,
        Language::Ru => Lang::Ru,
        Language::Auto => current(),
    }
}

pub fn installer_choice() -> Option<Language> {
    choice_of(&platform::installer_language()?)
}

fn choice_of(code: &str) -> Option<Language> {
    match code {
        "ru" => Some(Language::Ru),
        "en" => Some(Language::En),
        _ => None,
    }
}

const UNSET: u8 = 0;
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);

/// Applies the setting for the rest of the run. Restart is the only way to change it later:
/// a text already built (the island's cards, the tray menu) is not rebuilt.
pub fn select(choice: Language) -> Option<Failed> {
    let (lang, failed) = choose(choice, platform::ui_languages);
    fix(lang);
    failed
}

/// The settings window takes the language of the island that opened it, not its own choice.
pub fn fix(lang: Lang) {
    CURRENT.store(lang as u8 + 1, Ordering::Release);
}

/// Whether `choice` would give another language than the one in use.
pub fn pending(choice: Language) -> bool {
    choose(choice, platform::ui_languages).0 != current()
}

pub fn current() -> Lang {
    #[cfg(test)]
    return forced::get();
    #[cfg(not(test))]
    match CURRENT.load(Ordering::Acquire) {
        1 => Lang::En,
        2 => Lang::Ru,
        // A text is needed before the settings are read (a crash message, the first-run
        // reminder names): follow Windows without reporting anything.
        _ => {
            select(Language::Auto);
            current()
        }
    }
}

/// The form a word takes after a number: Russian has three ("1 файл, 3 файла, 5 файлов"),
/// English uses the first and the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    One,
    Few,
    Many,
}

pub fn count(n: u64) -> Count {
    match current() {
        Lang::En if n == 1 => Count::One,
        Lang::En => Count::Many,
        Lang::Ru => match (n % 10, n % 100) {
            (1, h) if h != 11 => Count::One,
            (2..=4, h) if !(12..=14).contains(&h) => Count::Few,
            _ => Count::Many,
        },
    }
}

#[cfg(test)]
mod forced {
    use super::Lang;
    use std::cell::Cell;

    // Per thread, so parallel tests can't change each other's language.
    thread_local!(static LANG: Cell<Lang> = const { Cell::new(Lang::En) });

    pub fn get() -> Lang {
        LANG.with(Cell::get)
    }

    pub fn set(lang: Lang) -> Lang {
        LANG.with(|cell| cell.replace(lang))
    }
}

#[cfg(test)]
pub fn with<R>(lang: Lang, body: impl FnOnce() -> R) -> R {
    let before = forced::set(lang);
    let result = body();
    forced::set(before);
    result
}

/// A key missing from the catalog comes back as the key itself; the tests below catch that.
macro_rules! tr {
    ($key:literal $(, $($rest:tt)*)?) => {
        ::rust_i18n::t!($key, locale = $crate::lang::current().code() $(, $($rest)*)?)
    };
}
pub(crate) use tr;

macro_rules! tr_for {
    ($lang:expr, $key:literal) => {
        ::rust_i18n::t!($key, locale = $lang.code())
    };
}
pub(crate) use tr_for;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::Cause;
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn the_users_first_supported_language_wins() {
        assert_eq!(Lang::negotiate(&["ru-RU", "en-US"]), Some(Lang::Ru));
        assert_eq!(Lang::negotiate(&["en-GB"]), Some(Lang::En));
        assert_eq!(Lang::negotiate(&["de-DE", "ru-RU"]), Some(Lang::Ru));
        assert_eq!(Lang::negotiate(&["RU"]), Some(Lang::Ru));
        assert_eq!(Lang::negotiate(&["ru_RU"]), Some(Lang::Ru));
        assert_eq!(Lang::negotiate(&["de-DE", "fr-FR"]), None);
        assert_eq!(Lang::negotiate::<&str>(&[]), None);
    }

    #[test]
    fn a_code_names_one_language() {
        assert_eq!(Lang::from_code("ru"), Some(Lang::Ru));
        assert_eq!(Lang::from_code("en"), Some(Lang::En));
        assert_eq!(Lang::from_code("RU"), None);
        assert_eq!(Lang::from_code(""), None);
    }

    #[test]
    fn the_installers_word_becomes_a_choice() {
        assert_eq!(choice_of("ru"), Some(Language::Ru));
        assert_eq!(choice_of("en"), Some(Language::En));
        assert_eq!(choice_of(""), None);
        assert_eq!(choice_of("de"), None);
    }

    #[test]
    fn a_chosen_language_ignores_windows() {
        let never = || -> Result<Vec<String>, Failed> { panic!("Windows was asked") };
        assert_eq!(choose(Language::Ru, never), (Lang::Ru, None));
        assert_eq!(choose(Language::En, never), (Lang::En, None));
    }

    #[test]
    fn auto_follows_windows_and_falls_back_to_english() {
        let ru = || Ok(vec!["ru-RU".to_string()]);
        assert_eq!(choose(Language::Auto, ru), (Lang::Ru, None));
        let other = || Ok(vec!["ja-JP".to_string()]);
        assert_eq!(choose(Language::Auto, other), (Lang::En, None));
        let broken = || Err(Failed::new(Cause::Unknown));
        assert_eq!(
            choose(Language::Auto, broken),
            (Lang::En, Some(Failed::new(Cause::Unknown)))
        );
    }

    fn catalog(lang: Lang) -> serde_json::Map<String, serde_json::Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("locales")
            .join(format!("{}.json", lang.code()));
        let text = fs::read_to_string(&path).expect("catalog is readable");
        serde_json::from_str(&text).expect("catalog is JSON")
    }

    fn texts(lang: Lang) -> Vec<(String, String)> {
        catalog(lang)
            .into_iter()
            .filter(|(key, _)| !key.starts_with('_'))
            .map(|(key, value)| {
                let text = value.as_str().expect("every text is a string").to_string();
                (key, text)
            })
            .collect()
    }

    fn placeholders(text: &str) -> BTreeSet<String> {
        text.split("%{")
            .skip(1)
            .filter_map(|rest| rest.split('}').next())
            .map(String::from)
            .collect()
    }

    #[test]
    fn every_language_has_the_same_keys_and_values() {
        let english = texts(Lang::En);
        for lang in Lang::ALL {
            let other = texts(lang);
            let keys = |list: &[(String, String)]| -> BTreeSet<String> {
                list.iter().map(|(key, _)| key.clone()).collect()
            };
            assert_eq!(keys(&english), keys(&other), "{}", lang.code());
            for ((key, en), (_, text)) in english.iter().zip(&other) {
                assert!(!text.trim().is_empty(), "{} {key}", lang.code());
                assert_eq!(
                    placeholders(en),
                    placeholders(text),
                    "{} {key}",
                    lang.code()
                );
            }
        }
    }

    fn walk(dir: &Path, found: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).expect("source folder") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, found);
            // This file spells the macro out in its own text.
            } else if path.extension().is_some_and(|ext| ext == "rs") && !path.ends_with("lang.rs")
            {
                let source = fs::read_to_string(&path).expect("source file");
                for (opener, in_second_place) in [("tr!(", false), ("tr_for!(", true)] {
                    let mut rest = source.as_str();
                    while let Some(at) = rest.find(opener) {
                        rest = &rest[at + opener.len()..];
                        // `tr_for!` has the language first: the key is the first quoted text.
                        let key = if in_second_place {
                            rest.find('"').map(|quote| &rest[quote..])
                        } else {
                            Some(rest.trim_start())
                        };
                        if let Some(key) = key.and_then(|key| key.strip_prefix('"'))
                            && let Some(end) = key.find('"')
                        {
                            found.insert(key[..end].to_string());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_text_the_code_asks_for_exists_in_every_catalog() {
        let mut asked = BTreeSet::new();
        walk(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut asked,
        );
        assert!(asked.len() > 50, "the scan found only {}", asked.len());
        for lang in Lang::ALL {
            let have: BTreeSet<String> = texts(lang).into_iter().map(|(key, _)| key).collect();
            for key in &asked {
                assert!(have.contains(key), "{}: no text for {key}", lang.code());
            }
        }
    }

    #[test]
    fn a_count_takes_its_language_s_form() {
        with(Lang::Ru, || {
            let forms: Vec<Count> = [1, 2, 5, 11, 12, 21, 22, 25, 101, 112]
                .into_iter()
                .map(count)
                .collect();
            use Count::*;
            assert_eq!(
                forms,
                [One, Few, Many, Many, Many, One, Few, Many, One, Many]
            );
        });
        with(Lang::En, || {
            assert_eq!(count(1), Count::One);
            assert_eq!(count(3), Count::Many);
            assert_eq!(count(21), Count::Many);
        });
    }

    #[test]
    fn a_text_is_looked_up_in_the_current_language() {
        assert_eq!(with(Lang::En, || tr!("tray.pause").to_string()), "Pause");
        assert_eq!(with(Lang::Ru, || tr!("tray.pause").to_string()), "Пауза");
    }
}
