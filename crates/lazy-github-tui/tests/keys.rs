//! Key strings parse into the keys Textual makes of them, and display as its footer shows them. `fixtures/keys.json`
//! (from `fixtures/generate_bindings.py`) has every default key string of `bindings.json` and `contexts.json`, and
//! the override spellings the parity matrix and scenarios use, applied to `app.open_help` as `App.set_keymap` does.

use indexmap::IndexMap;
use lazy_github_tui::keymap::{BindingId, KeySpec, Keymap};
use serde_json::Value;

fn python_keys() -> Value {
    let json = include_str!("fixtures/keys.json");
    serde_json::from_str(json).expect("keys.json is a JSON object")
}

fn strings<'a>(case: &'a Value, field: &str) -> Vec<&'a str> {
    case[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field} in {case}"))
        .iter()
        .map(|value| value.as_str().unwrap_or_else(|| panic!("{field} in {case}")))
        .collect()
}

fn cases<'a>(keys: &'a Value, list: &str) -> &'a [Value] {
    keys[list].as_array().unwrap_or_else(|| panic!("{list} in keys.json"))
}

/// Each of Python's key names is the key Rust parsed at the same position.
fn assert_same_keys(raw: &str, python: &[&str], rust: &[(String, KeySpec)]) {
    assert_eq!(
        python.len(),
        rust.len(),
        "{raw:?}: Python keys {python:?}, Rust {rust:?}"
    );
    for (name, (token, spec)) in python.iter().zip(rust) {
        assert_eq!(
            &KeySpec::from_event_name(name),
            spec,
            "{raw:?}: Python's {name:?} isn't Rust's {token:?}"
        );
    }
}

#[test]
fn default_keys_parse_and_display_as_in_textual() {
    let keys = python_keys();
    let defaults = cases(&keys, "defaults");
    assert!(defaults.len() > 50, "{} default key strings", defaults.len());
    for case in defaults {
        let raw = case["raw"].as_str().expect("raw is a string");
        let rust = KeySpec::parse_list(raw);
        assert_same_keys(raw, &strings(case, "keys"), &rust);
        let display: Vec<String> = rust.iter().map(|(token, _)| KeySpec::display(token)).collect();
        assert_eq!(display, strings(case, "display"), "{raw:?} display");
    }
}

#[test]
fn overrides_apply_and_display_as_in_textual() {
    let keys = python_keys();
    let overrides = cases(&keys, "overrides");
    assert_eq!(overrides.len(), 23);
    for case in overrides {
        let raw = case["raw"].as_str().expect("raw is a string");
        let keymap = Keymap::new(&IndexMap::from([("app.open_help".to_owned(), raw.to_owned())]));
        assert_same_keys(raw, &strings(case, "keys"), keymap.keys(BindingId::AppOpenHelp));
        assert_eq!(
            keymap.key_displays(BindingId::AppOpenHelp),
            strings(case, "display"),
            "{raw:?} display"
        );
    }
}
