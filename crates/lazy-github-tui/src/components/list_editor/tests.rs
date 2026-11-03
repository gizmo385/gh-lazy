//! The list editor's navigation, editing, adding, removing, moving, paste, mouse and drawing.

use super::*;
use crate::keymap::KeySpec;
use crate::test_util::{buffer_to_string, buffer_to_styled_string};

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn editor(items: &[&str]) -> ListEditor {
    ListEditor::new(items.iter().map(|&item| item.to_owned()).collect(), "owner/repo")
}

fn items(editor: &ListEditor) -> Vec<&str> {
    editor.items().iter().map(String::as_str).collect()
}

/// Types `text` into the open input, a key per char.
fn type_text(editor: &mut ListEditor, text: &str) {
    let keymap = Keymap::defaults();
    for c in text.chars() {
        let key = chord(&if c == ' ' { "space".to_owned() } else { c.to_string() });
        assert!(editor.consumes_key(&key, &keymap), "{c:?} wasn't typed");
        assert_eq!(editor.handle_key(&key, &keymap, ""), ListEditorOutcome::Editing);
    }
}

/// Clears the open input with `ctrl+u` from its end.
fn clear(editor: &mut ListEditor) {
    let keymap = Keymap::defaults();
    let _ = editor.handle_key(&chord("end"), &keymap, "");
    let _ = editor.handle_key(&chord("ctrl+u"), &keymap, "");
}

fn bind(editor: &mut ListEditor, id: BindingId) -> ListEditorOutcome {
    editor.on_binding(id)
}

#[test]
fn the_highlight_moves_without_wrapping_and_the_add_line_is_last() {
    let mut list = editor(&["a", "b"]);
    assert_eq!(list.highlighted(), 0);
    assert_eq!(
        bind(&mut list, BindingId::TextualOptionListCursorUp),
        ListEditorOutcome::Moved
    );
    assert_eq!(list.highlighted(), 0);
    let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    assert_eq!(list.highlighted(), 2, "the add line");
    let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    assert_eq!(list.highlighted(), 2);
    let _ = bind(&mut list, BindingId::TextualOptionListFirst);
    assert_eq!(list.highlighted(), 0);
    let _ = bind(&mut list, BindingId::TextualOptionListLast);
    assert_eq!(list.highlighted(), 2);
}

#[test]
fn page_keys_move_by_the_lines_in_view() {
    let names: Vec<String> = (0..20).map(|i| format!("item-{i}")).collect();
    let mut list = ListEditor::new(names, "");
    let _ = bind(&mut list, BindingId::TextualOptionListPageDown);
    assert_eq!(list.highlighted(), MAX_LINES);
    let _ = bind(&mut list, BindingId::TextualOptionListPageUp);
    assert_eq!(list.highlighted(), 0);
}

#[test]
fn an_empty_list_highlights_its_add_line() {
    let list = editor(&[]);
    assert_eq!(list.highlighted(), 0);
    assert_eq!(list.items().len(), 0);
}

#[test]
fn enter_edits_in_place_and_commits_the_stripped_text() {
    let mut list = editor(&["a", "b"]);
    let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    assert_eq!(bind(&mut list, BindingId::SettingsListEdit), ListEditorOutcome::Editing);
    assert_eq!(list.input_value(), Some("b"));
    // The text is selected: typing replaces it.
    type_text(&mut list, " c ");
    assert_eq!(list.input_value(), Some(" c "));
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Changed
    );
    assert!(!list.is_editing());
    assert_eq!(items(&list), ["a", "c"]);
    assert_eq!(list.highlighted(), 1);
}

#[test]
fn a_blank_edit_removes_the_item() {
    let mut list = editor(&["a", "b"]);
    let _ = bind(&mut list, BindingId::SettingsListEdit);
    clear(&mut list);
    type_text(&mut list, "  ");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Changed
    );
    assert_eq!(items(&list), ["b"]);
    assert_eq!(list.highlighted(), 0);
}

#[test]
fn an_edit_equal_to_another_item_is_refused_and_stays_open() {
    let mut list = editor(&["a", "b"]);
    let _ = bind(&mut list, BindingId::SettingsListEdit);
    type_text(&mut list, "b");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Refused { text: "b".into() }
    );
    assert!(list.is_editing());
    assert_eq!(list.input_value(), Some("b"));
    assert_eq!(items(&list), ["a", "b"]);
    // Its own text isn't a duplicate.
    clear(&mut list);
    type_text(&mut list, "a");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Changed
    );
}

#[test]
fn escape_cancels_an_edit() {
    let mut list = editor(&["a"]);
    let _ = bind(&mut list, BindingId::SettingsListEdit);
    type_text(&mut list, "zzz");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCancel),
        ListEditorOutcome::Cancelled
    );
    assert!(!list.is_editing());
    assert_eq!(items(&list), ["a"]);
    assert_eq!(list.highlighted(), 0);
}

#[test]
fn adding_goes_on_until_an_empty_enter() {
    let mut list = editor(&[]);
    assert_eq!(bind(&mut list, BindingId::SettingsListEdit), ListEditorOutcome::Editing);
    type_text(&mut list, "one");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Changed
    );
    assert!(list.is_editing(), "a fresh add input opens");
    assert_eq!(list.input_value(), Some(""));
    type_text(&mut list, "two");
    let _ = bind(&mut list, BindingId::SettingsListCommit);
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Cancelled
    );
    assert!(!list.is_editing());
    assert_eq!(items(&list), ["one", "two"]);
    assert_eq!(list.highlighted(), 2, "the add line");
}

#[test]
fn a_adds_from_anywhere_and_a_listed_item_is_refused() {
    let mut list = editor(&["a", "b"]);
    assert_eq!(bind(&mut list, BindingId::SettingsListAdd), ListEditorOutcome::Editing);
    assert_eq!(list.highlighted(), 2);
    type_text(&mut list, "a");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCommit),
        ListEditorOutcome::Refused { text: "a".into() }
    );
    assert_eq!(list.input_value(), Some("a"));
}

#[test]
fn escape_from_adding_puts_the_highlight_back() {
    let mut list = editor(&["a", "b"]);
    let _ = bind(&mut list, BindingId::SettingsListAdd);
    type_text(&mut list, "c");
    assert_eq!(
        bind(&mut list, BindingId::SettingsListCancel),
        ListEditorOutcome::Cancelled
    );
    assert_eq!(items(&list), ["a", "b"]);
    assert_eq!(list.highlighted(), 0);
}

#[test]
fn remove_keeps_the_highlights_position() {
    let mut list = editor(&["a", "b", "c"]);
    let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    assert_eq!(
        bind(&mut list, BindingId::SettingsListRemove),
        ListEditorOutcome::Changed
    );
    assert_eq!(items(&list), ["a", "c"]);
    assert_eq!(list.highlighted(), 1);
    let _ = bind(&mut list, BindingId::SettingsListRemove);
    assert_eq!(list.highlighted(), 1, "the add line");
    assert_eq!(bind(&mut list, BindingId::SettingsListRemove), ListEditorOutcome::Moved);
    assert_eq!(items(&list), ["a"]);
}

#[test]
fn moves_swap_with_the_neighbour_and_stop_at_the_ends() {
    let mut list = editor(&["a", "b", "c"]);
    assert_eq!(bind(&mut list, BindingId::SettingsListMoveUp), ListEditorOutcome::Moved);
    assert_eq!(
        bind(&mut list, BindingId::SettingsListMoveDown),
        ListEditorOutcome::Changed
    );
    assert_eq!(items(&list), ["b", "a", "c"]);
    assert_eq!(list.highlighted(), 1);
    let _ = bind(&mut list, BindingId::SettingsListMoveDown);
    assert_eq!(items(&list), ["b", "c", "a"]);
    assert_eq!(
        bind(&mut list, BindingId::SettingsListMoveDown),
        ListEditorOutcome::Moved
    );
    let _ = bind(&mut list, BindingId::SettingsListMoveUp);
    assert_eq!(items(&list), ["b", "a", "c"]);
    // Nothing on the add line.
    let _ = bind(&mut list, BindingId::TextualOptionListLast);
    assert_eq!(bind(&mut list, BindingId::SettingsListMoveUp), ListEditorOutcome::Moved);
    assert_eq!(items(&list), ["b", "a", "c"]);
}

#[test]
fn a_multi_line_paste_adds_each_new_line_and_skips_blanks_and_duplicates() {
    let mut list = editor(&["a"]);
    let _ = bind(&mut list, BindingId::TextualOptionListLast);
    assert_eq!(list.handle_paste(" b \n\n a\nc\nb\n"), ListEditorOutcome::Changed);
    assert!(!list.is_editing());
    assert_eq!(items(&list), ["a", "b", "c"]);
    assert_eq!(list.highlighted(), 3, "the add line, still");
}

#[test]
fn a_one_line_paste_starts_adding_with_it() {
    let mut list = editor(&["a"]);
    assert_eq!(list.handle_paste("  b  \n"), ListEditorOutcome::Editing);
    assert_eq!(list.input_value(), Some("b"));
    assert_eq!(list.highlighted(), 1);
    // Into the open input, its first line.
    assert_eq!(list.handle_paste("c\nd"), ListEditorOutcome::Editing);
    assert_eq!(list.input_value(), Some("bc"));
}

#[test]
fn commit_on_blur_drops_refused_text_and_opens_no_fresh_input() {
    let mut list = editor(&["a"]);
    let _ = bind(&mut list, BindingId::SettingsListAdd);
    type_text(&mut list, "a");
    assert_eq!(list.commit(), ListEditorOutcome::Refused { text: "a".into() });
    assert!(!list.is_editing());
    assert_eq!(items(&list), ["a"]);

    let _ = bind(&mut list, BindingId::SettingsListAdd);
    type_text(&mut list, "b");
    assert_eq!(list.commit(), ListEditorOutcome::Changed);
    assert!(!list.is_editing());
    assert_eq!(items(&list), ["a", "b"]);
    assert_eq!(list.commit(), ListEditorOutcome::Ignored);
}

#[test]
fn the_open_input_leaves_commit_and_cancel_to_the_chain() {
    let keymap = Keymap::defaults();
    let mut list = editor(&["a"]);
    assert!(!list.consumes_key(&chord("q"), &keymap), "nothing while browsing");
    let _ = bind(&mut list, BindingId::SettingsListEdit);
    for key in ["q", "/", "left", "ctrl+v"] {
        assert!(list.consumes_key(&chord(key), &keymap), "{key}");
    }
    for key in ["enter", "escape", "tab"] {
        assert!(!list.consumes_key(&chord(key), &keymap), "{key}");
    }
}

#[test]
fn height_grows_with_the_items_up_to_the_cap() {
    assert_eq!(editor(&[]).height(), 3);
    assert_eq!(editor(&["a"]).height(), 4);
    let names: Vec<String> = (0..20).map(|i| format!("item-{i}")).collect();
    let list = ListEditor::new(names, "");
    assert_eq!(list.height(), 10);
}

#[test]
fn the_scroll_follows_the_highlight() {
    let names: Vec<String> = (0..20).map(|i| format!("item-{i}")).collect();
    let mut list = ListEditor::new(names, "");
    assert_eq!(list.highlighted_rows(), 1..2);
    for _ in 0..10 {
        let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    }
    // Line 10 is the last of the eight in view.
    assert_eq!(list.highlighted_rows(), 8..9);
    let _ = bind(&mut list, BindingId::TextualOptionListLast);
    assert_eq!(list.highlighted_rows(), 8..9);
    let _ = bind(&mut list, BindingId::TextualOptionListFirst);
    assert_eq!(list.highlighted_rows(), 1..2);
}

#[test]
fn the_link_while_editing_is_the_inputs_without_submit() {
    let expected: Vec<BindingId> = EDIT_KEYS
        .iter()
        .chain(super::super::text_input::INPUT_BINDINGS)
        .copied()
        .filter(|&id| id != BindingId::TextualInputSubmit)
        .collect();
    assert_eq!(LIST_EDITOR_INPUT_BINDINGS, expected.as_slice());
}

// Drawing, textual-dark over a dialog's `$surface-lighten-3`.

fn under(theme: &Theme) -> Paint {
    theme.roles().surface.dialog.over(theme.background())
}

fn draw(list: &mut ListEditor, width: u16, focused: bool) -> Buffer {
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, width, list.height()));
    let area = buf.area;
    list.render(&mut buf, area, &theme, under(&theme), focused);
    buf
}

fn press(list: &mut ListEditor, column: u16, row: u16, focused: bool) -> ListEditorOutcome {
    let event = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    list.handle_mouse(&event, focused)
}

#[test]
fn list_editor_empty() {
    insta::assert_snapshot!(buffer_to_string(&draw(&mut editor(&[]), 30, true)));
}

#[test]
fn list_editor_at_its_cap_with_a_scrollbar() {
    let names: Vec<String> = (0..20).map(|i| format!("parity-bot/repo-{i}")).collect();
    let mut list = ListEditor::new(names, "");
    for _ in 0..10 {
        let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    }
    insta::assert_snapshot!(buffer_to_string(&draw(&mut list, 40, true)));
}

#[test]
fn list_editor_at_its_cap_styled() {
    let names: Vec<String> = (0..20).map(|i| format!("parity-bot/repo-{i}")).collect();
    let mut list = ListEditor::new(names, "");
    let _ = bind(&mut list, BindingId::TextualOptionListCursorDown);
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&mut list, 40, true)));
}

#[test]
fn list_editor_unfocused_draws_no_highlight() {
    let focused = draw(&mut editor(&["a/one"]), 20, true);
    let blurred = draw(&mut editor(&["a/one"]), 20, false);
    assert_eq!(buffer_to_string(&focused), buffer_to_string(&blurred));
    assert_ne!(focused[(2, 1)].bg, blurred[(2, 1)].bg);
    assert_eq!(blurred[(2, 1)].bg, blurred[(2, 2)].bg, "no highlight");
}

#[test]
fn presses_highlight_then_edit_and_the_add_line_adds() {
    let mut list = editor(&["a/one", "a/two"]);
    let _ = draw(&mut list, 30, true);
    // The second item, from another field: highlighted only.
    assert_eq!(press(&mut list, 4, 2, false), ListEditorOutcome::Moved);
    assert_eq!(list.highlighted(), 1);
    assert_eq!(press(&mut list, 4, 2, true), ListEditorOutcome::Editing);
    assert_eq!(list.input_value(), Some("a/two"));
    // A press on another line commits and highlights it.
    assert_eq!(press(&mut list, 4, 1, true), ListEditorOutcome::Changed);
    assert!(!list.is_editing());
    assert_eq!(list.highlighted(), 0);
    let _ = draw(&mut list, 30, true);
    assert_eq!(press(&mut list, 4, 3, true), ListEditorOutcome::Editing);
    assert_eq!(list.highlighted(), 2);
    assert_eq!(list.input_value(), Some(""));
    // The border isn't a line.
    assert_eq!(press(&mut list, 0, 1, true), ListEditorOutcome::Ignored);
}

#[test]
fn the_wheel_scrolls_without_moving_the_highlight() {
    let names: Vec<String> = (0..20).map(|i| format!("item-{i}")).collect();
    let mut list = ListEditor::new(names, "");
    list.wheel(Wheel::Down);
    assert_eq!(list.highlighted(), 0);
    let buf = draw(&mut list, 30, true);
    assert!(buffer_to_string(&buf).contains("item-2"));
    assert!(!buffer_to_string(&buf).contains("item-1 "));
    for _ in 0..20 {
        list.wheel(Wheel::Down);
    }
    assert!(buffer_to_string(&draw(&mut list, 30, true)).contains("+ Add item..."));
}
