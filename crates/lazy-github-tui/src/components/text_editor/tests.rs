//! The editor's keys and outcomes, and its drawing. Textual's key-by-key behavior is checked against the
//! `text-editor-keys` golden in `tests/editor_keys.rs`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rstest::rstest;

use super::*;
use crate::keymap::KeySpec;

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn press(editor: &mut TextEditor, key: &str) -> EditorOutcome {
    editor.handle_key(&chord(key), &Keymap::defaults())
}

/// Draws `editor` into a `width` x `height` buffer at the origin.
fn draw(editor: &mut TextEditor, width: u16, height: u16, focused: bool) -> Buffer {
    let theme = Theme::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            editor.render(frame.buffer_mut(), area, &theme, theme.background(), focused);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[rstest]
#[case::tab("tab", false)]
#[case::shift_tab("shift+tab", false)]
#[case::escape("escape", false)]
#[case::shift_enter("shift+enter", false)]
#[case::q("q", true)]
#[case::space("space", true)]
#[case::enter("enter", true)]
#[case::up("up", true)]
#[case::ctrl_k("ctrl+k", true)]
// #257's keys are taken already, so they don't reach the dialog behind the editor.
#[case::undo("ctrl+z", true)]
#[case::select("shift+left", true)]
fn keys_the_editor_takes(#[case] key: &str, #[case] consumed: bool) {
    let editor = TextEditor::new(EditorVariant::Code, "");
    assert_eq!(editor.consumes_key(&chord(key), &Keymap::defaults()), consumed);
}

#[test]
fn a_paste_inserts_every_line() {
    let mut editor = TextEditor::new(EditorVariant::Plain, "xy");
    editor.set_cursor((0, 1));
    assert_eq!(editor.handle_paste("a\r\nb\nc"), EditorOutcome::Changed);
    assert_eq!(editor.text(), "xa\nb\ncy");
    assert_eq!(editor.cursor(), (2, 1));
}

#[test]
fn typing_and_enter_change_the_text() {
    let mut editor = TextEditor::new(EditorVariant::Code, "ab");
    editor.set_cursor((0, 1));
    assert_eq!(press(&mut editor, "q"), EditorOutcome::Changed);
    assert_eq!(press(&mut editor, "enter"), EditorOutcome::Changed);
    assert_eq!(press(&mut editor, "é"), EditorOutcome::Changed);
    assert_eq!(editor.text(), "aq\néb");
    assert_eq!(editor.selection(), ((1, 1), (1, 1)));
}

#[rstest]
#[case::backspace("backspace", "first\nsecond", (1, 0), "firstsecond")]
#[case::delete("delete", "first\nsecond", (0, 5), "firstsecond")]
#[case::delete_word_left("ctrl+w", "one two", (0, 7), "one ")]
#[case::delete_word_right("ctrl+f", "one two", (0, 0), " two")]
#[case::delete_to_start("ctrl+u", "one two", (0, 4), "two")]
#[case::delete_to_end("ctrl+k", "one two", (0, 3), "one")]
#[case::delete_line("ctrl+shift+k", "one\ntwo", (0, 1), "two")]
fn edits_are_changes(#[case] key: &str, #[case] text: &str, #[case] cursor: Location, #[case] after: &str) {
    let mut editor = TextEditor::new(EditorVariant::Code, text);
    editor.set_cursor(cursor);
    assert_eq!(press(&mut editor, key), EditorOutcome::Changed);
    assert_eq!(editor.text(), after);
}

#[rstest]
#[case::left("left", (0, 2))]
#[case::right("right", (0, 4))]
#[case::down("down", (1, 3))]
#[case::up("up", (0, 0))]
#[case::word_left("ctrl+left", (0, 0))]
#[case::word_right("ctrl+right", (0, 7))]
#[case::home("home", (0, 0))]
#[case::end("end", (0, 7))]
fn moves_are_moves(#[case] key: &str, #[case] cursor: Location) {
    let mut editor = TextEditor::new(EditorVariant::Code, "one two\nthree");
    editor.set_cursor((0, 3));
    assert_eq!(press(&mut editor, key), EditorOutcome::Moved);
    assert_eq!(editor.cursor(), cursor);
    assert_eq!(editor.text(), "one two\nthree");
}

#[rstest]
#[case::left_at_start("left", (0, 0))]
#[case::backspace_at_start("backspace", (0, 0))]
#[case::delete_at_end("delete", (0, 3))]
#[case::word_left_at_start("ctrl+left", (0, 0))]
#[case::word_right_at_end("ctrl+right", (0, 3))]
#[case::ctrl_u_at_start("ctrl+u", (0, 0))]
#[case::undo_until_257("ctrl+z", (0, 1))]
fn keys_that_do_nothing_are_ignored(#[case] key: &str, #[case] cursor: Location) {
    let mut editor = TextEditor::new(EditorVariant::Plain, "one");
    editor.set_cursor(cursor);
    assert_eq!(press(&mut editor, key), EditorOutcome::Ignored);
    assert_eq!((editor.text(), editor.cursor()), ("one".to_owned(), cursor));
}

#[rstest]
#[case::empty("", &[""])]
#[case::trailing_newline("a\n", &["a", ""])]
#[case::crlf("a\r\nb", &["a", "b"])]
#[case::lone_cr("a\rb\r", &["a", "b", ""])]
#[case::unicode_separator("a\u{2028}b", &["a", "b"])]
#[case::vertical_tab_isnt_trailing("a\x0b", &["a"])]
fn text_splits_into_lines_as_textual_does(#[case] text: &str, #[case] lines: &[&str]) {
    assert_eq!(insert_lines(text), lines);
    let editor = TextEditor::new(EditorVariant::Code, text);
    assert_eq!(editor.text(), lines.join("\n"));
}

#[test]
fn up_and_down_keep_the_column_of_the_last_sideways_move() {
    let mut editor = TextEditor::new(EditorVariant::Code, "abcdef\nab\nabcdef");
    editor.set_cursor((0, 5));
    let _ = press(&mut editor, "down");
    assert_eq!(editor.cursor(), (1, 2));
    let _ = press(&mut editor, "down");
    assert_eq!(editor.cursor(), (2, 5));
}

// Snapshots, textual-dark.

/// Three lines, the second wrapping at a 20-column text area (28 wide with the border, padding and gutter).
const WRAPPING: &str = "First line\nthe second line wraps onto another row\nthird";

#[test]
fn code_editor_empty_unfocused() {
    let mut editor = TextEditor::new(EditorVariant::Code, "");
    insta::assert_debug_snapshot!(draw(&mut editor, 30, 5, false));
}

#[test]
fn code_editor_focused_on_a_wrapped_line() {
    let mut editor = TextEditor::new(EditorVariant::Code, WRAPPING);
    editor.set_cursor((1, 27));
    let buffer = draw(&mut editor, 28, 7, true);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn code_editor_scrolled_with_the_scrollbar() {
    let text: Vec<String> = (1..=8).map(|i| format!("line {i}")).collect();
    let mut editor = TextEditor::new(EditorVariant::Code, &text.join("\n"));
    let _ = draw(&mut editor, 28, 6, true);
    editor.set_cursor((6, 2));
    assert_eq!(editor.scroll_offset(), 3);
    insta::assert_debug_snapshot!(draw(&mut editor, 28, 6, true));
}

#[test]
fn code_editor_ten_lines() {
    let text: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
    let mut editor = TextEditor::new(EditorVariant::Code, &text.join("\n"));
    let _ = draw(&mut editor, 30, 12, true);
    editor.set_cursor((9, 4));
    insta::assert_debug_snapshot!(draw(&mut editor, 30, 12, true));
}

#[rstest]
fn plain_editor(#[values(true, false)] focused: bool) {
    let mut editor = TextEditor::new(EditorVariant::Plain, WRAPPING).with_border(EditorBorder::Blank);
    editor.set_cursor((0, 3));
    let buffer = draw(&mut editor, 26, 6, focused);
    insta::assert_debug_snapshot!(format!("plain_editor_focused_{focused}"), buffer);
}

#[test]
fn plain_editor_with_its_tall_border() {
    let mut editor = TextEditor::new(EditorVariant::Plain, "ab\nxy");
    insta::assert_debug_snapshot!(draw(&mut editor, 25, 5, true));
}
