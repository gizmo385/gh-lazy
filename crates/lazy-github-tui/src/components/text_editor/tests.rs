//! The editor's keys and outcomes, and its drawing. Textual's key-by-key behavior is checked against the
//! `text-editor-keys` golden in `tests/editor_keys.rs`.

use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rstest::rstest;

use super::*;
use crate::keymap::KeySpec;
use crossterm::event::KeyModifiers;

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn press(editor: &mut TextEditor, key: &str) -> EditorOutcome {
    press_with(editor, key, "")
}

/// Presses `key` with `clipboard` as the in-app clipboard.
fn press_with(editor: &mut TextEditor, key: &str, clipboard: &str) -> EditorOutcome {
    editor.handle_key(&chord(key), &Keymap::defaults(), clipboard, Instant::now())
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
#[case::undo("ctrl+z", true)]
#[case::select("shift+left", true)]
#[case::cut("ctrl+x", true)]
#[case::paste("ctrl+v", true)]
// Copy with nothing selected falls through.
#[case::copy("ctrl+c", false)]
#[case::super_copy("super+c", false)]
fn keys_the_editor_takes(#[case] key: &str, #[case] consumed: bool) {
    let editor = TextEditor::new(EditorVariant::Code, "");
    assert_eq!(editor.consumes_key(&chord(key), &Keymap::defaults()), consumed);
}

#[test]
fn a_paste_inserts_every_line() {
    let mut editor = TextEditor::new(EditorVariant::Plain, "xy");
    editor.set_cursor((0, 1));
    assert_eq!(editor.handle_paste("a\r\nb\nc", Instant::now()), EditorOutcome::Changed);
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
#[case::undo_with_nothing_to_undo("ctrl+z", (0, 1))]
#[case::redo_with_nothing_to_redo("ctrl+y", (0, 1))]
#[case::copy_without_a_selection("ctrl+c", (0, 1))]
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

// Selection, the clipboard and undo (#257).

#[test]
fn copy_needs_a_selection() {
    let mut editor = TextEditor::new(EditorVariant::Code, "one two");
    let keymap = Keymap::defaults();
    assert!(!editor.consumes_key(&chord("ctrl+c"), &keymap));
    let _ = press(&mut editor, "ctrl+shift+right");
    assert!(editor.consumes_key(&chord("ctrl+c"), &keymap));
    assert_eq!(
        press(&mut editor, "ctrl+c"),
        EditorOutcome::Copied {
            text: "one".into(),
            changed: false
        }
    );
    assert_eq!(editor.selection(), ((0, 0), (0, 3)), "copying keeps the selection");
}

#[rstest]
#[case::middle_line((1, 2), "one\nthree", "two\n", (1, 2))]
#[case::last_line_has_no_line_break((2, 4), "one\ntwo\n", "three", (2, 0))]
#[case::first_line((0, 1), "two\nthree", "one\n", (0, 1))]
fn cut_without_a_selection_cuts_the_line(
    #[case] cursor: Location,
    #[case] after: &str,
    #[case] cut: &str,
    #[case] cursor_after: Location,
) {
    let mut editor = TextEditor::new(EditorVariant::Code, "one\ntwo\nthree");
    editor.set_cursor(cursor);
    assert_eq!(
        press(&mut editor, "ctrl+x"),
        EditorOutcome::Copied {
            text: cut.into(),
            changed: true
        }
    );
    assert_eq!((editor.text(), editor.cursor()), (after.to_owned(), cursor_after));
}

#[test]
fn cut_and_paste_a_selection() {
    let mut editor = TextEditor::new(EditorVariant::Plain, "one\ntwo");
    editor.set_cursor((0, 1));
    let _ = press(&mut editor, "shift+down");
    let EditorOutcome::Copied { text, changed: true } = press(&mut editor, "ctrl+x") else {
        panic!("ctrl+x cuts");
    };
    assert_eq!((text.as_str(), editor.text().as_str()), ("ne\nt", "owo"));
    assert_eq!(press_with(&mut editor, "ctrl+v", &text), EditorOutcome::Changed);
    assert_eq!(editor.text(), "one\ntwo");
}

#[test]
fn undo_restores_the_selection_and_redo_the_edit() {
    let mut editor = TextEditor::new(EditorVariant::Code, "one two");
    editor.set_cursor((0, 4));
    let _ = press(&mut editor, "shift+end");
    let _ = press(&mut editor, "x");
    assert_eq!(editor.text(), "one x");
    assert_eq!(press(&mut editor, "ctrl+z"), EditorOutcome::Changed);
    assert_eq!(
        (editor.text().as_str(), editor.selection()),
        ("one two", ((0, 4), (0, 7)))
    );
    assert_eq!(press(&mut editor, "ctrl+y"), EditorOutcome::Changed);
    assert_eq!(
        (editor.text().as_str(), editor.selection()),
        ("one x", ((0, 5), (0, 5)))
    );
}

#[test]
fn typing_after_a_pause_is_a_batch_of_its_own() {
    let start = Instant::now();
    let mut editor = TextEditor::new(EditorVariant::Code, "");
    let keymap = Keymap::defaults();
    for (key, after) in [("a", 0), ("b", 1000), ("c", 3500)] {
        let _ = editor.handle_key(&chord(key), &keymap, "", start + Duration::from_millis(after));
    }
    let _ = press(&mut editor, "ctrl+z");
    assert_eq!(editor.text(), "ab");
    let _ = press(&mut editor, "ctrl+z");
    assert_eq!(editor.text(), "");
}

#[test]
fn gaining_focus_ends_the_batch() {
    let mut editor = TextEditor::new(EditorVariant::Code, "");
    let _ = press(&mut editor, "a");
    editor.focus_gained();
    let _ = press(&mut editor, "b");
    let _ = press(&mut editor, "ctrl+z");
    assert_eq!(editor.text(), "a");
}

// The mouse (#257): the code editor in a 30 x 6 area at (10, 5). Its text starts at column 15 (border, padding and a
// three-cell gutter), and its first text row is row 6.

const MOUSE_TEXT: &str = "first line\nsecond line here\nthird";

fn mouse_editor() -> TextEditor {
    let mut editor = TextEditor::new(EditorVariant::Code, MOUSE_TEXT);
    let area = Rect::new(10, 5, 30, 6);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    editor.render(&mut buf, area, &theme, theme.background(), true);
    editor
}

#[rstest]
#[case::on_a_char(18, 7, (1, 3))]
#[case::in_the_gutter(12, 7, (1, 0))]
#[case::on_the_border(10, 6, (0, 0))]
#[case::past_a_rows_end(36, 8, (2, 5))]
#[case::below_the_text(20, 10, (2, 5))]
fn a_press_places_the_cursor(#[case] column: u16, #[case] row: u16, #[case] cursor: Location) {
    let mut editor = mouse_editor();
    editor.press(column, row);
    assert_eq!(editor.selection(), (cursor, cursor));
    assert!(editor.is_selecting());
}

#[test]
fn a_drag_selects_even_outside_the_editor() {
    let mut editor = mouse_editor();
    editor.press(17, 6);
    editor.drag(19, 7);
    assert_eq!(editor.selection(), ((0, 2), (1, 4)));
    // Above and left of the editor: the start of the first row.
    editor.drag(0, 0);
    assert_eq!(editor.selection(), ((0, 2), (0, 0)));
    // Below and right of it: the end of the last row.
    editor.drag(70, 30);
    assert_eq!(editor.selection(), ((0, 2), (2, 5)));
    let release = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 70,
        row: 30,
        modifiers: KeyModifiers::NONE,
    };
    assert!(editor.captured_mouse(release));
    assert!(!editor.is_selecting());
    // The release ended the drag.
    editor.drag(17, 6);
    assert_eq!(editor.selection(), ((0, 2), (2, 5)));
    assert_eq!(editor.selected_text(), "rst line\nsecond line here\nthird");
}

#[test]
fn the_wheel_scrolls_without_moving_the_cursor() {
    let text: Vec<String> = (1..=12).map(|i| format!("line {i}")).collect();
    let mut editor = TextEditor::new(EditorVariant::Code, &text.join("\n"));
    let area = Rect::new(0, 0, 30, 6);
    let theme = Theme::default();
    editor.render(&mut Buffer::empty(area), area, &theme, theme.background(), true);
    editor.set_cursor((1, 2));
    assert!(editor.scroll(2));
    assert_eq!((editor.scroll_offset(), editor.cursor()), (2, (1, 2)));
    assert!(editor.scroll(100));
    assert_eq!(editor.scroll_offset(), 8);
    assert!(editor.scroll(-3));
    assert_eq!(editor.scroll_offset(), 5);
    assert!(editor.scroll(-30));
    assert_eq!((editor.scroll_offset(), editor.cursor()), (0, (1, 2)));
    // At the top, the wheel up moves nothing, and says so.
    assert!(!editor.scroll(-2));
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

#[rstest]
fn selection_across_a_wrapped_row(#[values(EditorVariant::Code, EditorVariant::Plain)] variant: EditorVariant) {
    let mut editor = TextEditor::new(variant, WRAPPING);
    let _ = draw(&mut editor, 28, 7, true);
    editor.set_cursor((0, 6));
    for key in ["shift+down", "shift+down", "shift+right"] {
        let _ = press(&mut editor, key);
    }
    let name = match variant {
        EditorVariant::Code => "selection_across_a_wrapped_row_code",
        EditorVariant::Plain => "selection_across_a_wrapped_row_plain",
    };
    insta::assert_debug_snapshot!(name, draw(&mut editor, 28, 7, true));
}

#[test]
fn a_selected_empty_line_shows_a_marker() {
    let mut editor = TextEditor::new(EditorVariant::Code, "one\n\nthree");
    let _ = press(&mut editor, "f7");
    insta::assert_debug_snapshot!(draw(&mut editor, 20, 5, true));
}
