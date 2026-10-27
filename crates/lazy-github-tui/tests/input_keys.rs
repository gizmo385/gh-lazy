//! The text input does with each of its keys what Textual's `Input` does: `fixtures/generate_input_keys.py` plays key
//! sequences on a headless Textual `Input` and dumps the value, cursor, selection and in-app clipboard after each key to
//! `fixtures/input_keys.json`, and this replays them on `TextInput`, keeping the clipboard as `App` does.

mod common;

use lazy_github_tui::components::text_input::{INPUT_BINDINGS, InputOutcome, TextInput};
use lazy_github_tui::keymap::{KeySpec, Keymap};
use serde::Deserialize;

const GOLDEN: &str = include_str!("fixtures/input_keys.json");

#[derive(Debug, Deserialize)]
struct Sequence {
    name: String,
    value: String,
    cursor: usize,
    clipboard: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    key: String,
    value: String,
    cursor: usize,
    /// Textual's `Selection(start, end)`: the anchor, then the cursor.
    selection: (usize, usize),
    clipboard: String,
}

fn sequences() -> Vec<Sequence> {
    serde_json::from_str(GOLDEN).unwrap()
}

#[test]
fn the_input_matches_textual_key_by_key() {
    let keymap = Keymap::defaults();
    for sequence in sequences() {
        let mut input = TextInput::new("");
        input.set_value(sequence.value.as_str());
        input.set_cursor(sequence.cursor);
        let mut clipboard = sequence.clipboard.clone();
        for (i, step) in sequence.steps.iter().enumerate() {
            let KeySpec::Chord(chord) = KeySpec::parse(&step.key) else {
                panic!("{}: {:?} isn't a key", sequence.name, step.key);
            };
            // `App` applies `Action::Copy` by setting its clipboard.
            if let InputOutcome::Copied(text) = input.handle_key(&chord, &keymap, &clipboard) {
                clipboard = text;
            }
            assert_eq!(
                (
                    input.value(),
                    input.cursor(),
                    (input.anchor(), input.cursor()),
                    clipboard.as_str()
                ),
                (
                    step.value.as_str(),
                    step.cursor,
                    step.selection,
                    step.clipboard.as_str()
                ),
                "{} step {i} ({})",
                sequence.name,
                step.key
            );
        }
    }
}

#[test]
fn the_golden_presses_every_input_key() {
    let pressed: Vec<KeySpec> = sequences()
        .iter()
        .flat_map(|sequence| &sequence.steps)
        .map(|step| KeySpec::parse(&step.key))
        .collect();
    for id in INPUT_BINDINGS {
        for (token, spec) in KeySpec::parse_list(id.def().default_keys) {
            assert!(pressed.contains(&spec), "{} ({token}) is never pressed", id.def().name);
        }
    }
}

/// Regenerates `input_keys.json` with Textual and checks the committed copy matches (the golden manifest is
/// `parity/goldens/text-input-keys.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn input_keys_golden_is_fresh() {
    let home = tempfile::tempdir().unwrap();
    common::assert_goldens_fresh(
        "crates/lazy-github-tui/tests/fixtures/generate_input_keys.py",
        &["crates/lazy-github-tui/tests/fixtures/input_keys.json"],
        home.path(),
        "uv run python crates/lazy-github-tui/tests/fixtures/generate_input_keys.py",
    );
}
