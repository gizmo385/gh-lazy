//! The text editor does with its movement and editing keys what Textual's `TextArea` does:
//! `fixtures/generate_editor_keys.py` plays cases on headless Textual editors and dumps the text, the selection, the
//! scroll and the in-app clipboard after each key or paste to `fixtures/editor_keys.json`, and this replays them on
//! `TextEditor` drawn at the same size, keeping the clipboard as `App` does.

mod common;

use lazy_github_tui::components::text_editor::{EditorOutcome, EditorVariant, Location, TextEditor};
use lazy_github_tui::keymap::{KeySpec, Keymap};
use lazy_github_tui::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde::Deserialize;

const GOLDEN: &str = include_str!("fixtures/editor_keys.json");

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    variant: String,
    size: (u16, u16),
    text: String,
    cursor: Location,
    clipboard: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    key: Option<String>,
    paste: Option<String>,
    text: String,
    /// Textual's `Selection(start, end)`.
    selection: (Location, Location),
    scroll: usize,
    clipboard: String,
}

fn cases() -> Vec<Case> {
    serde_json::from_str(GOLDEN).unwrap()
}

/// Draws `editor` at `size`, as the Textual app laid it out, so it wraps and pages as there.
fn draw(editor: &mut TextEditor, (width, height): (u16, u16)) {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    editor.render(&mut buf, area, &theme, theme.background(), true);
}

#[test]
fn the_editor_matches_textual_key_by_key() {
    let keymap = Keymap::defaults();
    // One instant for every key: the generator doesn't wait between keys, so undo never batches by time here.
    let now = tokio::time::Instant::now();
    let mut failures = Vec::new();
    for case in cases() {
        let variant = match case.variant.as_str() {
            "code" => EditorVariant::Code,
            _ => EditorVariant::Plain,
        };
        let mut editor = TextEditor::new(variant, &case.text);
        draw(&mut editor, case.size);
        editor.set_cursor(case.cursor);
        let mut clipboard = case.clipboard.clone();
        for (i, step) in case.steps.iter().enumerate() {
            let label = match (&step.key, &step.paste) {
                (Some(key), _) => {
                    let KeySpec::Chord(chord) = KeySpec::parse(key) else {
                        panic!("{}: {key:?} isn't a key", case.name);
                    };
                    // `App` applies `Action::Copy` by setting its clipboard.
                    if let EditorOutcome::Copied { text, .. } = editor.handle_key(&chord, &keymap, &clipboard, now) {
                        clipboard = text;
                    }
                    key.clone()
                }
                (None, Some(text)) => {
                    let _ = editor.handle_paste(text, now);
                    format!("paste {text:?}")
                }
                (None, None) => panic!("{}: a step with neither a key nor a paste", case.name),
            };
            draw(&mut editor, case.size);
            let got = (
                editor.text(),
                editor.selection(),
                editor.scroll_offset(),
                clipboard.clone(),
            );
            let want = (step.text.clone(), step.selection, step.scroll, step.clipboard.clone());
            if got != want {
                failures.push(format!(
                    "{} step {i} ({label}):\n  got  {got:?}\n  want {want:?}",
                    case.name
                ));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} case(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Regenerates `editor_keys.json` with Textual and checks the committed copy matches (the golden manifest is
/// `parity/goldens/text-editor-keys.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn editor_keys_golden_is_fresh() {
    let home = tempfile::tempdir().unwrap();
    common::assert_goldens_fresh(
        "crates/lazy-github-tui/tests/fixtures/generate_editor_keys.py",
        &["crates/lazy-github-tui/tests/fixtures/editor_keys.json"],
        home.path(),
        "uv run python crates/lazy-github-tui/tests/fixtures/generate_editor_keys.py",
    );
}
