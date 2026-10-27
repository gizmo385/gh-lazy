//! The binding registry matches the Python app's, dumped to `fixtures/bindings.json` by
//! `fixtures/generate_bindings.py`. `contexts.rs` and `keys.rs` check the generator's other two goldens.

mod common;

use std::collections::HashSet;

use lazy_github_tui::keymap::BindingId;
use serde_json::Value;

/// Python ids whose description intentionally differs, with the Rust value.
const DESCRIPTION_DIFFS: &[(&str, &str)] = &[
    // B14: Python's "Command Pallete" is a typo.
    ("app.command_palette", "Command Palette"),
];

/// Python ids whose `show` intentionally differs.
const SHOW_DIFFS: &[&str] = &[
    // B7: Python fixes `show` from the config at import time (false by default). Rust shows it, and the declaring
    // component hides it while notifications are disabled.
    "notifications.open",
];

fn python_bindings() -> Vec<Value> {
    let json = include_str!("fixtures/bindings.json");
    serde_json::from_str(json).expect("bindings.json is a JSON array")
}

fn text<'a>(binding: &'a Value, field: &str) -> &'a str {
    binding[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} in {binding}"))
}

#[test]
fn every_python_binding_exists_with_the_same_defaults() {
    let python = python_bindings();
    assert_eq!(python.len(), 59);
    for binding in &python {
        let id = text(binding, "id");
        let def = BindingId::from_config_id(id)
            .unwrap_or_else(|| panic!("{id} is missing from BindingId"))
            .def();
        assert_eq!(def.default_keys, text(binding, "key"), "{id} key");
        assert_eq!(def.action, text(binding, "action"), "{id} action");
        assert_eq!(def.tooltip, text(binding, "tooltip"), "{id} tooltip");
        assert_eq!(def.key_display, binding["key_display"].as_str(), "{id} key_display");
        assert_eq!(Some(def.priority), binding["priority"].as_bool(), "{id} priority");
        assert_eq!(Some(def.system), binding["system"].as_bool(), "{id} system");

        let description = DESCRIPTION_DIFFS
            .iter()
            .find(|&&(diff, _)| diff == id)
            .map_or_else(|| text(binding, "description"), |&(_, rust)| rust);
        assert_eq!(def.description, description, "{id} description");

        let show = binding["show"].as_bool().expect("show is a bool");
        if SHOW_DIFFS.contains(&id) {
            assert_ne!(def.show, show, "{id} show: drop it from SHOW_DIFFS");
        } else {
            assert_eq!(def.show, show, "{id} show");
        }
    }
}

#[test]
fn there_are_no_extra_ids() {
    let python: HashSet<String> = python_bindings()
        .iter()
        .map(|binding| text(binding, "id").to_owned())
        .collect();
    for &id in BindingId::ALL {
        let name = id.def().name;
        assert!(
            name.starts_with("textual.") || python.contains(name),
            "{name} isn't a Python binding id"
        );
    }
}

#[test]
fn the_registry_keeps_python_order() {
    let python: Vec<String> = python_bindings()
        .iter()
        .map(|binding| text(binding, "id").to_owned())
        .collect();
    let rust: Vec<&str> = BindingId::ALL.iter().filter_map(|id| id.config_id()).collect();
    assert_eq!(rust, python);
}

/// Regenerates `bindings.json`, `contexts.json` and `keys.json` with the Python app and checks the committed copies
/// match (the golden manifest is `parity/goldens/keybindings.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn bindings_goldens_are_fresh() {
    // An empty HOME, so `notifications.open` gets the default config, as the generator's docstring says.
    let home = tempfile::tempdir().unwrap();
    common::assert_goldens_fresh(
        "crates/lazy-github-tui/tests/fixtures/generate_bindings.py",
        &[
            "crates/lazy-github-tui/tests/fixtures/bindings.json",
            "crates/lazy-github-tui/tests/fixtures/contexts.json",
            "crates/lazy-github-tui/tests/fixtures/keys.json",
        ],
        home.path(),
        "HOME=$(mktemp -d) uv run python crates/lazy-github-tui/tests/fixtures/generate_bindings.py",
    );
}
