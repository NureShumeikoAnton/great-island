//! Every app the island has, in the launcher's order, and which of them are on.

use super::{Applet, Lifetime, alarm::Alarm, calculator::Calculator, prefs, prefs::AppPrefs};

pub fn all() -> Vec<Box<dyn Applet>> {
    vec![Box::new(Calculator::default()), Box::new(Alarm::default())]
}

/// The apps the user has on, in the launcher's order. One that is off is not created at all:
/// it gets no events, keeps no timers and reaches nothing.
pub fn on(prefs: &AppPrefs) -> Vec<Box<dyn Applet>> {
    all()
        .into_iter()
        .filter(|app| {
            let m = app.manifest();
            prefs::is_on(&m.kind, prefs.get(m.id))
        })
        .collect()
}

/// After its page closed: an app that lives one opening is made anew, the others go on.
pub fn after_page(app: &mut Box<dyn Applet>) {
    let manifest = app.manifest();
    if manifest.lifetime == Lifetime::Page
        && let Some(fresh) = all().into_iter().find(|a| a.manifest().id == manifest.id)
    {
        *app = fresh;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{Context, Event, Key, prefs::Values};
    use crate::lang::{self, Lang};
    use std::collections::BTreeSet;

    #[test]
    fn ids_are_unique_and_plain() {
        let ids: Vec<&str> = all().iter().map(|a| a.manifest().id).collect();
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
        for id in ids {
            assert!(!id.is_empty());
            assert!(
                id.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{id}"
            );
        }
    }

    #[test]
    fn every_app_has_a_title_in_every_language() {
        for lang in [Lang::En, Lang::Ru, Lang::Uk] {
            lang::with(lang, || {
                for app in all() {
                    let m = app.manifest();
                    assert!(
                        !m.title.is_empty() && !m.title.starts_with("app."),
                        "{}",
                        m.id
                    );
                }
            });
        }
    }

    #[test]
    fn a_page_s_actions_are_its_own() {
        for app in all() {
            let m = app.manifest();
            let values = Values::resolve(&m.fields, None);
            let page = app.page(&Context::bare(&values));
            let actions = page.actions();
            let distinct: BTreeSet<u16> = actions.iter().map(|a| a.0).collect();
            assert_eq!(distinct.len(), actions.len(), "{}", m.id);
        }
    }

    #[test]
    fn the_calculator_starts_from_zero_at_every_opening() {
        let values = Values::default();
        let cx = Context::bare(&values);
        let mut app = all().remove(0);
        assert_eq!(app.manifest().id, "calculator");
        let fresh = app.page(&cx);
        app.update(Event::Key(Key::Char('7')), &cx);
        assert_ne!(app.page(&cx), fresh);
        after_page(&mut app);
        assert_eq!(app.page(&cx), fresh);
    }

    #[test]
    fn an_app_switched_off_is_not_created() {
        let mut prefs = AppPrefs::new();
        assert!(on(&prefs).iter().any(|a| a.manifest().id == "calculator"));
        prefs.insert(
            "calculator".into(),
            prefs::Prefs {
                on: Some(false),
                ..Default::default()
            },
        );
        assert!(!on(&prefs).iter().any(|a| a.manifest().id == "calculator"));
    }
}
