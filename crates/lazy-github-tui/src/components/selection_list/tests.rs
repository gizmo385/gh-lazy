//! The selection list's highlight, toggles, owner changes, mouse and drawing.

use crossterm::event::KeyModifiers;
use ratatui::style::Color;
use rstest::rstest;

use super::*;
use crate::keymap::KeySpec;
use crate::test_util::buffer_to_styled_string;

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn press<T>(list: &mut SelectionList<T>, key: &str) -> ListOutcome {
    list.handle_key(&chord(key), &Keymap::defaults())
}

/// Add reaction's list for a user who already gave 👍, 🎉 and 👀.
fn reactions() -> SelectionList<&'static str> {
    SelectionList::with_options(
        [
            ("👎", "-1"),
            ("😄", "laugh"),
            ("😕", "confused"),
            ("❤️", "heart"),
            ("🚀", "rocket"),
        ]
        .map(|(label, value)| (label, value, false)),
    )
}

fn letters(n: usize) -> SelectionList<usize> {
    SelectionList::with_options((0..n).map(|i| (format!("Option {i}"), i, false)))
}

// The highlight.

#[test]
fn a_list_built_with_options_highlights_the_first() {
    assert_eq!(reactions().highlighted(), Some(0));
    let empty: SelectionList<u8> = SelectionList::with_options(std::iter::empty::<(&str, u8, bool)>());
    assert_eq!(empty.highlighted(), None);
}

#[test]
fn options_added_later_arent_highlighted_until_a_key_moves() {
    // The PR reviewers list: built empty, filled once the reviewers load.
    let mut list = SelectionList::new();
    list.add("alice", "alice", true);
    list.add("bob", "bob", true);
    assert_eq!(list.highlighted(), None);
    // `space` toggles nothing without a highlight.
    assert_eq!(press(&mut list, "space"), ListOutcome::Moved);
    assert_eq!(list.selected(), [&"alice", &"bob"]);
    assert_eq!(press(&mut list, "down"), ListOutcome::Moved);
    assert_eq!(list.highlighted(), Some(0));
}

#[rstest]
#[case::down("down", Some(0))]
#[case::up("up", Some(4))]
#[case::home("home", Some(0))]
#[case::end("end", Some(4))]
#[case::page_down("pagedown", Some(4))]
#[case::page_up("pageup", Some(0))]
fn a_key_without_a_highlight_goes_to_an_end(#[case] key: &str, #[case] highlighted: Option<usize>) {
    let mut list = SelectionList::new();
    for i in 0..5 {
        list.add(format!("{i}"), i, false);
    }
    assert_eq!(press(&mut list, key), ListOutcome::Moved);
    assert_eq!(list.highlighted(), highlighted);
}

#[test]
fn up_and_down_move_and_wrap_at_the_ends() {
    let mut list = reactions();
    for expected in [1, 2, 3, 4, 0] {
        assert_eq!(press(&mut list, "down"), ListOutcome::Moved);
        assert_eq!(list.highlighted(), Some(expected));
    }
    assert_eq!(press(&mut list, "up"), ListOutcome::Moved);
    assert_eq!(list.highlighted(), Some(4));
}

#[test]
fn home_end_and_the_page_keys_stop_at_the_ends() {
    let mut list = letters(20);
    // Before the first draw a page is every option; drawn 7 rows tall, it's the 5 in view.
    assert_eq!(press(&mut list, "pagedown"), ListOutcome::Moved);
    assert_eq!(list.highlighted(), Some(19));
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 7));
    let theme = Theme::default();
    let _ = press(&mut list, "home");
    let area = buf.area;
    list.render(&mut buf, area, &theme, theme.background(), true);
    let _ = press(&mut list, "pagedown");
    assert_eq!(list.highlighted(), Some(5));
    let _ = press(&mut list, "pagedown");
    assert_eq!(list.highlighted(), Some(10));
    let _ = press(&mut list, "pageup");
    assert_eq!(list.highlighted(), Some(5));
    let _ = press(&mut list, "pageup");
    let _ = press(&mut list, "pageup");
    assert_eq!(list.highlighted(), Some(0));
    let _ = press(&mut list, "end");
    assert_eq!(list.highlighted(), Some(19));
    let _ = press(&mut list, "pagedown");
    assert_eq!(list.highlighted(), Some(19));
}

#[test]
fn moving_scrolls_the_highlight_into_view() {
    let mut list = letters(20);
    let theme = Theme::default();
    let draw = |list: &mut SelectionList<usize>| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 7));
        let area = buf.area;
        list.render(&mut buf, area, &theme, theme.background(), true);
        crate::test_util::buffer_to_string(&buf)
    };
    draw(&mut list);
    let _ = press(&mut list, "end");
    let text = draw(&mut list);
    assert!(text.contains("Option 19") && text.contains("Option 15"), "{text}");
    assert!(!text.contains("Option 14"), "{text}");
    let _ = press(&mut list, "down");
    let text = draw(&mut list);
    assert!(text.contains("Option 0") && !text.contains("Option 5"), "{text}");
}

#[test]
fn other_keys_arent_the_lists() {
    let mut list = reactions();
    let keymap = Keymap::defaults();
    for key in ["a", "q", "tab", "left", "escape", "shift+enter"] {
        assert_eq!(list.handle_key(&chord(key), &keymap), ListOutcome::Ignored, "{key}");
    }
    assert_eq!(list.highlighted(), Some(0));
    let mut empty: SelectionList<u8> = SelectionList::new();
    assert_eq!(press(&mut empty, "down"), ListOutcome::Moved);
    assert_eq!(press(&mut empty, "enter"), ListOutcome::Moved);
    assert_eq!(empty.highlighted(), None);
}

// Toggling.

#[rstest]
fn space_and_enter_toggle_the_highlighted_option(#[values("space", "enter")] key: &str) {
    let mut list = reactions();
    let _ = press(&mut list, "down");
    assert_eq!(
        press(&mut list, key),
        ListOutcome::Toggled {
            index: 1,
            selected: true
        }
    );
    assert!(list.is_selected(1));
    assert_eq!(list.value(1), Some(&"laugh"));
    assert_eq!(
        press(&mut list, key),
        ListOutcome::Toggled {
            index: 1,
            selected: false
        }
    );
    assert!(list.selected().is_empty());
}

#[test]
fn selected_lists_the_values_in_the_order_they_were_selected() {
    // Textual keeps the selected values in a dict: checking 🚀, then 😄, lists 🚀 first; unchecking and checking 🚀
    // again moves it last.
    let mut list = reactions();
    for key in ["end", "space", "home", "down", "space"] {
        let _ = press(&mut list, key);
    }
    assert_eq!(list.selected(), [&"rocket", &"laugh"]);
    for key in ["end", "space", "space"] {
        let _ = press(&mut list, key);
    }
    assert_eq!(list.selected(), [&"laugh", &"rocket"]);
    // Options added selected count as selected when added.
    list.add("👀", "eyes", true);
    assert_eq!(list.selected(), [&"laugh", &"rocket", &"eyes"]);
}

// Owner changes.

#[test]
fn removing_the_highlighted_option_keeps_the_index() {
    let mut list = reactions();
    let _ = press(&mut list, "down");
    let _ = press(&mut list, "space");
    assert_eq!(list.remove(&"laugh"), Some("laugh"));
    assert_eq!(list.highlighted(), Some(1));
    assert_eq!(list.labels().collect::<Vec<_>>(), ["👎", "😕", "❤️", "🚀"]);
    assert!(list.selected().is_empty(), "a removed option is deselected");
    assert!(!list.contains(&"laugh"));
    assert_eq!(list.remove(&"laugh"), None);
}

#[test]
fn removing_the_last_option_while_highlighted_moves_up() {
    let mut list = reactions();
    let _ = press(&mut list, "end");
    let _ = list.remove(&"rocket");
    assert_eq!(list.highlighted(), Some(3));
    // Removing one above the highlight keeps the index too, which is now the last option's.
    let _ = list.remove(&"-1");
    assert_eq!(list.highlighted(), Some(2));
    assert_eq!(list.value(2), Some(&"heart"));
}

#[test]
fn removing_the_only_option_clears_the_highlight() {
    let mut list = SelectionList::with_options([("alice", "alice", true)]);
    assert_eq!(list.highlighted(), Some(0));
    let _ = list.remove(&"alice");
    assert_eq!((list.highlighted(), list.len(), list.height()), (None, 0, 2));
    assert!(list.is_empty());
    // Added back, it isn't highlighted.
    list.add("alice", "alice", true);
    assert_eq!(list.highlighted(), None);
}

// The mouse.

fn press_at(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn a_click_on_an_option_highlights_and_toggles_it() {
    let mut list = reactions();
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 9));
    list.render(&mut buf, Rect::new(0, 1, 30, 7), &theme, theme.background(), false);
    // ❤️ is on row 5: the border, then 👎 on row 2.
    assert_eq!(
        list.handle_mouse(&press_at(10, 5)),
        ListOutcome::Toggled {
            index: 3,
            selected: true
        }
    );
    assert_eq!(list.highlighted(), Some(3));
    // The check box is part of the option.
    assert_eq!(
        list.handle_mouse(&press_at(3, 5)),
        ListOutcome::Toggled {
            index: 3,
            selected: false
        }
    );
    // The border, the padding, a release and the rows outside the list aren't options.
    for event in [press_at(10, 1), press_at(1, 5), press_at(10, 7), press_at(10, 8)] {
        assert_eq!(list.handle_mouse(&event), ListOutcome::Ignored, "{event:?}");
    }
    let release = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        ..press_at(10, 4)
    };
    assert_eq!(list.handle_mouse(&release), ListOutcome::Ignored);
    assert!(list.selected().is_empty());
}

// Drawing, textual-dark over a dialog's `$surface-lighten-3`.

fn under(theme: &Theme) -> Paint {
    theme.palette.surface_lighten_3.over(theme.background())
}

fn draw<T>(list: &mut SelectionList<T>, width: u16, focused: bool) -> String {
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, width, list.height()));
    let area = buf.area;
    list.render(&mut buf, area, &theme, under(&theme), focused);
    buffer_to_styled_string(&buf)
}

#[test]
fn selection_list_focused_first_highlighted() {
    insta::assert_snapshot!(draw(&mut letters(3), 30, true));
}

#[test]
fn selection_list_selected_and_highlighted() {
    let mut list = letters(3);
    let _ = press(&mut list, "down");
    let _ = press(&mut list, "space");
    insta::assert_snapshot!(draw(&mut list, 30, true));
}

#[test]
fn selection_list_unfocused_with_a_selection() {
    let mut list = letters(3);
    let _ = press(&mut list, "end");
    let _ = press(&mut list, "space");
    let _ = press(&mut list, "home");
    insta::assert_snapshot!(draw(&mut list, 30, false));
}

#[test]
fn selection_list_label_cut_with_an_ellipsis() {
    let mut list = SelectionList::with_options([
        ("a-reviewer-with-a-very-long-login", 0, false),
        ("exactly-fourteen", 1, true),
        ("fifteen-letters", 2, false),
    ]);
    insta::assert_snapshot!(draw(&mut list, 22, true));
}

#[test]
fn selection_list_reaction_emoji() {
    insta::assert_snapshot!(draw(&mut reactions(), 48, true));
}

#[test]
fn selection_list_empty() {
    insta::assert_snapshot!(draw(&mut SelectionList::<u8>::new(), 20, false));
}

#[rstest]
#[case::fits("fifteen-letters", 15, "fifteen-letters")]
#[case::one_over("sixteen-letters!", 15, "sixteen-letter…")]
#[case::wide_at_the_cut("ab👍cd", 3, "ab…")]
#[case::wide_before_the_cut("ab👍cd", 5, "ab👍…")]
#[case::no_room("abc", 0, "")]
#[case::room_for_the_ellipsis("abc", 1, "…")]
fn labels_are_cut_with_an_ellipsis(#[case] label: &str, #[case] width: usize, #[case] cut: &str) {
    assert_eq!(fit(label, width), cut);
}

#[test]
fn the_colors_are_pythons() {
    // Add reaction's list at 160x48, from the `pr-reactions` references: 👎 highlighted, the rest not.
    let theme = Theme::default();
    let mut list = reactions();
    let mut buf = Buffer::empty(Rect::new(0, 0, 48, list.height()));
    let area = buf.area;
    list.render(&mut buf, area, &theme, under(&theme), true);
    let cell = |x: u16, y: u16| &buf[(x, y)];
    // The border: `$border` over the tinted `$surface`.
    assert_eq!(
        (cell(0, 1).fg, cell(0, 1).bg),
        (Color::Rgb(0x01, 0x78, 0xd4), Color::Rgb(0x4f, 0x4f, 0x4f))
    );
    assert_eq!(
        (cell(5, 0).fg, cell(5, 0).bg),
        (Color::Rgb(0x01, 0x78, 0xd4), Color::Rgb(0x27, 0x27, 0x27))
    );
    // The padding, then the highlighted row's button and label.
    assert_eq!(cell(1, 1).bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(
        (cell(2, 1).symbol(), cell(2, 1).fg, cell(2, 1).bg),
        ("▐", Color::Rgb(0x24, 0x2f, 0x38), Color::Rgb(0x01, 0x78, 0xd4))
    );
    assert_eq!(
        (cell(3, 1).symbol(), cell(3, 1).fg, cell(3, 1).bg),
        ("X", Color::Rgb(0x00, 0x0f, 0x18), Color::Rgb(0x24, 0x2f, 0x38))
    );
    assert!(!cell(3, 1).modifier.contains(Modifier::BOLD));
    assert_eq!(
        (cell(5, 1).fg, cell(5, 1).bg),
        (Color::Rgb(0xdd, 0xed, 0xf9), Color::Rgb(0x01, 0x78, 0xd4))
    );
    assert!(cell(5, 1).modifier.contains(Modifier::BOLD));
    assert_eq!(cell(46, 1).bg, Color::Rgb(0x27, 0x27, 0x27));
    // An option that isn't highlighted.
    assert_eq!(
        (cell(2, 2).fg, cell(2, 2).bg),
        (Color::Rgb(0x24, 0x2f, 0x38), Color::Rgb(0x27, 0x27, 0x27))
    );
    assert_eq!(
        (cell(6, 2).symbol(), cell(6, 2).fg, cell(6, 2).bg),
        ("😄", Color::Rgb(0xe0, 0xe0, 0xe0), Color::Rgb(0x27, 0x27, 0x27))
    );
    assert_eq!(cell(8, 2).bg, Color::Rgb(0x27, 0x27, 0x27));
}

#[test]
fn a_dialog_footer_shows_none_of_the_lists_keys() {
    use crate::component::{BindingState, ChainLink, Owner, ScreenKind};
    use crate::modal::{FieldKind, ModalKind};

    let kind = ModalKind::Test(0);
    let chain = vec![
        FieldKind::SelectionList.link(kind),
        ChainLink::new(Owner::Screen(ScreenKind::Modal(kind)), &[BindingId::ModalClose]),
    ];
    let active = crate::keymap::active_bindings(&chain, &Keymap::defaults(), |_, _| BindingState::Enabled);
    let footer: Vec<String> = crate::components::footer::footer_items(&active)
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect();
    assert_eq!(footer, ["q Close"]);
    let def = BindingId::TextualSelectionListSelect.def();
    assert_eq!(
        (def.name, def.default_keys, def.description),
        ("textual.selection_list.select", "space", "Toggle option")
    );
}
