//! The apps' part of `settings.json`: for each app whether it is on and the values of the
//! settings it declares ([`Field`]), read so that nothing an app stores can reset the rest of the
//! settings: an entry that doesn't read is dropped alone, a value that doesn't fit its field is
//! its default.

use super::Kind;
use crate::hotkey::Hotkey;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// `None` — never switched: the kind's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

/// By app id. An app that left the island keeps its entry: it may come back.
pub type AppPrefs = BTreeMap<String, Prefs>;

/// For `#[serde(deserialize_with)]` on the settings' field.
pub fn lenient<'de, D: Deserializer<'de>>(d: D) -> Result<AppPrefs, D::Error> {
    let Value::Object(entries) = Value::deserialize(d)? else {
        return Ok(AppPrefs::new());
    };
    Ok(entries
        .into_iter()
        .filter_map(|(id, entry)| Some((id, serde_json::from_value(entry).ok()?)))
        .collect())
}

/// An app is on until the user turns it off; a connector reaches a third party and is off until
/// the user turns it on.
pub fn is_on(kind: &Kind, prefs: Option<&Prefs>) -> bool {
    prefs
        .and_then(|p| p.on)
        .unwrap_or(matches!(kind, Kind::App))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The key in [`Prefs::values`]: never renamed.
    pub key: &'static str,
    pub label: String,
    /// A line under the label; empty — none.
    pub sub: String,
    /// `Some` starts a new group of rows in the settings window, under this title (empty: no
    /// title); `None` goes on in the group above.
    pub group: Option<String>,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Toggle {
        default: bool,
    },
    /// `(value, label)`; the value is what is stored.
    Choice {
        options: Vec<(&'static str, String)>,
        default: &'static str,
    },
    Text {
        default: String,
    },
    /// One item of a long list the app searches ([`super::Applet::lookup`]); empty — none chosen.
    Lookup,
    /// A global key, unset by default. The island takes it from Windows like any key of its
    /// settings (`island/hotkeys.rs`).
    Hotkey,
}

/// An app's settings as it reads them: every declared field has a value that fits it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values(BTreeMap<&'static str, Value>);

impl Values {
    pub fn resolve(fields: &[Field], prefs: Option<&Prefs>) -> Self {
        let stored = |key: &str| prefs.and_then(|p| p.values.get(key));
        Self(
            fields
                .iter()
                .map(|field| {
                    let value = match (&field.kind, stored(field.key)) {
                        (FieldKind::Toggle { .. }, Some(v @ Value::Bool(_))) => v.clone(),
                        (FieldKind::Toggle { default }, _) => Value::Bool(*default),
                        (FieldKind::Choice { options, .. }, Some(Value::String(s)))
                            if options.iter().any(|(value, _)| value == s) =>
                        {
                            Value::String(s.clone())
                        }
                        (FieldKind::Choice { default, .. }, _) => Value::String((*default).into()),
                        (FieldKind::Text { .. }, Some(v @ Value::String(_))) => v.clone(),
                        (FieldKind::Text { default }, _) => Value::String(default.clone()),
                        (FieldKind::Lookup, Some(v @ Value::String(_))) => v.clone(),
                        (FieldKind::Lookup, _) => Value::String(String::new()),
                        (FieldKind::Hotkey, Some(v))
                            if serde_json::from_value::<Hotkey>(v.clone()).is_ok() =>
                        {
                            v.clone()
                        }
                        (FieldKind::Hotkey, _) => Value::Null,
                    };
                    (field.key, value)
                })
                .collect(),
        )
    }

    pub fn toggle(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or_default()
    }

    pub fn choice(&self, key: &str) -> &str {
        self.get(key).as_str().unwrap_or_default()
    }

    pub fn text(&self, key: &str) -> &str {
        self.choice(key)
    }

    pub fn hotkey(&self, key: &str) -> Option<Hotkey> {
        serde_json::from_value(self.get(key).clone()).ok()
    }

    fn get(&self, key: &str) -> &Value {
        let value = self.0.get(key);
        debug_assert!(
            value.is_some(),
            "the app asked for an undeclared setting {key}"
        );
        value.unwrap_or(&Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::Service;
    use serde_json::json;

    fn field(key: &'static str, kind: FieldKind) -> Field {
        Field {
            key,
            label: String::new(),
            sub: String::new(),
            group: None,
            kind,
        }
    }

    fn fields() -> Vec<Field> {
        vec![
            field("sound", FieldKind::Toggle { default: true }),
            field(
                "units",
                FieldKind::Choice {
                    options: vec![("c", String::new()), ("f", String::new())],
                    default: "c",
                },
            ),
            field(
                "city",
                FieldKind::Text {
                    default: String::new(),
                },
            ),
            field("key", FieldKind::Hotkey),
            field("place", FieldKind::Lookup),
        ]
    }

    fn prefs(values: Value) -> Prefs {
        serde_json::from_value(json!({ "values": values })).unwrap()
    }

    #[test]
    fn nothing_stored_gives_the_defaults() {
        let v = Values::resolve(&fields(), None);
        assert!(v.toggle("sound"));
        assert_eq!(v.choice("units"), "c");
        assert_eq!(v.text("city"), "");
        assert_eq!(v.hotkey("key"), None);
        assert_eq!(v.text("place"), "");
    }

    #[test]
    fn stored_values_that_fit_are_taken() {
        let p = prefs(json!({ "sound": false, "units": "f", "city": "Kyiv", "place": "31" }));
        let v = Values::resolve(&fields(), Some(&p));
        assert!(!v.toggle("sound"));
        assert_eq!(v.choice("units"), "f");
        assert_eq!(v.text("city"), "Kyiv");
        assert_eq!(v.text("place"), "31");
    }

    #[test]
    fn a_value_that_doesn_t_fit_is_its_default_alone() {
        let p = prefs(json!({ "sound": "yes", "units": "kelvin", "city": "Kyiv", "key": 5 }));
        let v = Values::resolve(&fields(), Some(&p));
        assert!(v.toggle("sound"));
        assert_eq!(v.choice("units"), "c");
        assert_eq!(v.text("city"), "Kyiv");
        assert_eq!(v.hotkey("key"), None);
    }

    #[test]
    fn apps_are_on_and_connectors_off_until_switched() {
        let connector = Kind::Connector(Service {
            provider: "",
            hosts: &[],
            site: "",
            sends: Vec::new(),
            note: String::new(),
        });
        assert!(is_on(&Kind::App, None));
        assert!(!is_on(&connector, None));
        let off = Prefs {
            on: Some(false),
            ..Prefs::default()
        };
        let on = Prefs {
            on: Some(true),
            ..Prefs::default()
        };
        assert!(!is_on(&Kind::App, Some(&off)));
        assert!(is_on(&connector, Some(&on)));
    }

    #[derive(Deserialize)]
    struct Holder {
        #[serde(deserialize_with = "lenient")]
        apps: AppPrefs,
    }

    #[test]
    fn an_entry_that_doesn_t_read_is_dropped_alone() {
        let h: Holder = serde_json::from_value(json!({ "apps": {
            "calculator": { "on": false },
            "broken": { "on": "yes" },
            "odd": 5,
            "gone": { "on": true, "values": { "x": [1, 2] } },
        }}))
        .unwrap();
        assert_eq!(h.apps.len(), 2);
        assert_eq!(h.apps["calculator"].on, Some(false));
        assert_eq!(h.apps["gone"].values["x"], json!([1, 2]));
        let h: Holder = serde_json::from_value(json!({ "apps": "nonsense" })).unwrap();
        assert!(h.apps.is_empty());
    }
}
