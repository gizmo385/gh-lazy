//! The binding registry matches the Python app's, dumped to `fixtures/bindings.json` by
//! `fixtures/generate_bindings.py`.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

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

/// Regenerates `bindings.json` with the Python app and checks the committed copy matches (the golden manifest is
/// `parity/goldens/keybindings.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn bindings_goldens_are_fresh() {
    // An empty HOME, so `notifications.open` gets the default config, as the generator's docstring says.
    let home = tempfile::tempdir().unwrap();
    assert_goldens_fresh(
        "crates/lazy-github-tui/tests/fixtures/generate_bindings.py",
        &["crates/lazy-github-tui/tests/fixtures/bindings.json"],
        home.path(),
        "HOME=$(mktemp -d) uv run python crates/lazy-github-tui/tests/fixtures/generate_bindings.py",
    );
}

/// Runs `generator` (relative to the repository root) with `--out <tempdir>` and `HOME` set to `home`, and fails with
/// a diff unless each of `goldens` is byte for byte the file it wrote. `lazy-github-core/tests/config.rs` has the same
/// helper: two short copies are simpler than a shared test crate.
fn assert_goldens_fresh(generator: &str, goldens: &[&str], home: &Path, regenerate: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = tempfile::tempdir().unwrap();
    // `uv run` finds (and syncs) the project's interpreter under the real HOME, so uv's cache and Python installs are
    // the usual ones; only the generator sees `home`.
    let found = Command::new("uv")
        .args(["run", "--frozen", "python", "-c", "import sys; print(sys.executable)"])
        .current_dir(&root)
        .output()
        .expect("uv is on PATH");
    assert!(
        found.status.success(),
        "uv run failed:\n{}",
        String::from_utf8_lossy(&found.stderr)
    );
    let python = String::from_utf8(found.stdout).unwrap();
    let run = Command::new(python.trim())
        .arg(generator)
        .arg("--out")
        .arg(out.path())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{generator} failed:\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    for golden in goldens {
        let committed = std::fs::read(root.join(golden)).unwrap();
        let fresh = std::fs::read(out.path().join(Path::new(golden).file_name().unwrap())).unwrap();
        if committed != fresh {
            let (committed, fresh) = (String::from_utf8_lossy(&committed), String::from_utf8_lossy(&fresh));
            let diff = similar::TextDiff::from_lines(&*committed, &*fresh)
                .unified_diff()
                .header(golden, "regenerated")
                .to_string();
            let diff: Vec<&str> = diff.lines().take(40).collect();
            panic!(
                "{golden} is stale; the Python app now writes:\n{}\n\n\
                 Regenerate it from the repository root with:\n    {regenerate}",
                diff.join("\n")
            );
        }
    }
}
