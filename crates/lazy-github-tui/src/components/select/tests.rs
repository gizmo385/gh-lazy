//! The select's keys, search, mouse and outcomes, and its drawing.

use crossterm::event::KeyModifiers;
use ratatui::style::Color;
use rstest::rstest;

use super::*;
use crate::components::button::ButtonRow;
use crate::keymap::KeySpec;
use crate::test_util::buffer_to_styled_string;

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

/// Edit issue's Status.
fn status(value: Option<&str>) -> Select<&'static str> {
    Select::new(
        DEFAULT_PROMPT,
        [("Open", "open"), ("Closed", "closed")],
        value.map(|v| if v == "open" { &"open" } else { &"closed" }),
    )
}

/// Twelve options, more than the list shows.
fn many() -> Select<usize> {
    Select::new("Select a workflow", (0..12).map(|i| (format!("Workflow {i}"), i)), None)
}

struct Keys {
    keymap: Keymap,
    now: Instant,
}

impl Keys {
    fn new() -> Self {
        Self {
            keymap: Keymap::defaults(),
            now: Instant::now(),
        }
    }

    fn press<T>(&self, select: &mut Select<T>, key: &str) -> SelectOutcome {
        select.handle_key(&chord(key), &self.keymap, self.now)
    }

    fn after(&mut self, millis: u64) -> &mut Self {
        self.now += Duration::from_millis(millis);
        self
    }
}

// Opening.

#[rstest]
#[case("enter")]
#[case("space")]
#[case("down")]
#[case("up")]
fn each_opening_key_opens_the_list(#[case] key: &str) {
    let keys = Keys::new();
    let mut select = status(None);
    assert!(
        !select.consumes_key(&chord(key), &keys.keymap),
        "the box's keys go through the chain"
    );
    assert_eq!(keys.press(&mut select, key), SelectOutcome::Opened);
    assert!(select.is_open());
    assert_eq!(select.bindings(), SELECT_OVERLAY_BINDINGS);
}

#[test]
fn other_keys_dont_open_it() {
    let keys = Keys::new();
    let mut select = status(None);
    for key in ["a", "escape", "tab", "left"] {
        assert_eq!(keys.press(&mut select, key), SelectOutcome::Ignored, "{key}");
    }
    assert!(!select.is_open());
    assert_eq!(select.bindings(), SELECT_BINDINGS);
}

#[test]
fn opening_highlights_the_value_or_the_blank() {
    let mut select = status(Some("closed"));
    let _ = select.open();
    assert_eq!(select.highlighted(), Some(2));
    let mut select = status(None);
    let _ = select.open();
    assert_eq!(select.highlighted(), Some(0));
}

#[test]
fn a_click_on_the_box_toggles_the_list() {
    let mut select = status(None);
    assert_eq!(select.toggle(), SelectOutcome::Opened);
    assert_eq!(select.toggle(), SelectOutcome::Closed);
    assert!(!select.is_open());
}

// Moving.

#[test]
fn up_and_down_move_the_highlight_and_wrap_at_the_ends() {
    let keys = Keys::new();
    let mut select = status(None);
    let _ = select.open();
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert_eq!(keys.press(&mut select, "down"), SelectOutcome::Consumed);
        seen.push(select.highlighted().unwrap());
    }
    // `OptionList.action_cursor_down` wraps, as Textual 8.2.5 does.
    assert_eq!(seen, [1, 2, 0, 1]);
    let _ = keys.press(&mut select, "up");
    let _ = keys.press(&mut select, "up");
    assert_eq!(select.highlighted(), Some(2));
}

#[test]
fn home_end_and_the_page_keys_jump_and_stop_at_the_ends() {
    let keys = Keys::new();
    let mut select = many();
    let _ = select.open();
    let _ = keys.press(&mut select, "end");
    assert_eq!(select.highlighted(), Some(12));
    let _ = keys.press(&mut select, "home");
    assert_eq!(select.highlighted(), Some(0));
    // Ten rows show.
    let _ = keys.press(&mut select, "pagedown");
    assert_eq!(select.highlighted(), Some(10));
    let _ = keys.press(&mut select, "pagedown");
    assert_eq!(select.highlighted(), Some(12));
    let _ = keys.press(&mut select, "pageup");
    assert_eq!(select.highlighted(), Some(2));
    let _ = keys.press(&mut select, "pageup");
    assert_eq!(select.highlighted(), Some(0));
}

// Picking.

#[test]
fn enter_picks_an_option_and_the_blank() {
    let keys = Keys::new();
    let mut select = status(Some("open"));
    let _ = select.open();
    let _ = keys.press(&mut select, "down");
    assert_eq!(keys.press(&mut select, "enter"), SelectOutcome::Changed);
    assert!(!select.is_open());
    assert_eq!(select.value(), Some(&"closed"));
    assert_eq!(select.selected_index(), Some(1));
    // The blank clears the value.
    let _ = keys.press(&mut select, "enter");
    let _ = keys.press(&mut select, "home");
    assert_eq!(keys.press(&mut select, "enter"), SelectOutcome::Changed);
    assert_eq!(select.value(), None);
}

#[test]
fn picking_the_current_option_closes_without_a_change() {
    let keys = Keys::new();
    let mut select = status(Some("open"));
    let _ = keys.press(&mut select, "enter");
    assert_eq!(keys.press(&mut select, "enter"), SelectOutcome::Closed);
    assert_eq!(select.value(), Some(&"open"));
}

#[test]
fn escape_closes_and_keeps_the_value() {
    let keys = Keys::new();
    let mut select = status(Some("open"));
    let _ = keys.press(&mut select, "enter");
    let _ = keys.press(&mut select, "down");
    assert!(select.consumes_key(&chord("escape"), &keys.keymap));
    assert_eq!(keys.press(&mut select, "escape"), SelectOutcome::Closed);
    assert!(!select.is_open());
    assert_eq!(select.value(), Some(&"open"));
}

#[test]
fn losing_focus_closes_the_list() {
    let mut select = status(Some("open"));
    let _ = select.open();
    select.close();
    assert!(!select.is_open());
    assert_eq!(select.value(), Some(&"open"));
}

// Searching.

#[test]
fn typing_jumps_to_the_earliest_match_with_an_accumulated_query() {
    let mut keys = Keys::new();
    let mut select = Select::new(
        DEFAULT_PROMPT,
        [("CI", 0), ("Release", 1), ("Nightly cleanup", 2), ("Clean", 3)],
        None,
    );
    let _ = select.open();
    assert!(select.consumes_key(&chord("c"), &keys.keymap));
    // "c" is earliest in "CI" and "Clean"; the first of them wins.
    assert_eq!(keys.press(&mut select, "c"), SelectOutcome::Consumed);
    assert_eq!(select.highlighted(), Some(1));
    // "cl" is at 0 in "Clean" only.
    keys.after(300);
    let _ = keys.press(&mut select, "l");
    assert_eq!(select.highlighted(), Some(4));
    // No match for "clx": the highlight stays.
    keys.after(300);
    let _ = keys.press(&mut select, "x");
    assert_eq!(select.highlighted(), Some(4));
    assert!(select.is_open());
}

#[test]
fn the_query_resets_after_0_7_seconds() {
    let mut keys = Keys::new();
    let mut select = Select::new(DEFAULT_PROMPT, [("Alpha", 0), ("Beta", 1), ("Release", 2)], None);
    let _ = select.open();
    let _ = keys.press(&mut select, "r");
    assert_eq!(select.highlighted(), Some(3));
    // Within 0.7 s "re" still matches Release.
    keys.after(700);
    let _ = keys.press(&mut select, "e");
    assert_eq!(select.highlighted(), Some(3));
    // Past 0.7 s the query starts again from "b".
    keys.after(701);
    let _ = keys.press(&mut select, "b");
    assert_eq!(select.highlighted(), Some(2));
    // Any key restarts the wait.
    keys.after(600);
    let _ = keys.press(&mut select, "down");
    keys.after(600);
    let _ = keys.press(&mut select, "e");
    assert_eq!(select.highlighted(), Some(2), "\"be\" matches Beta");
}

// Options.

#[test]
fn new_options_keep_the_value_when_they_have_it() {
    let mut select = status(Some("closed"));
    select.set_options([("Closed", "closed")]);
    assert_eq!(select.selected_index(), Some(0));
    select.set_options([("Open", "open")]);
    assert_eq!(select.value(), None);
    select.set_value(Some(&"open"));
    assert_eq!(select.value(), Some(&"open"));
    select.set_value(Some(&"nope"));
    assert_eq!(select.value(), None);
    assert_eq!(select.labels().collect::<Vec<_>>(), ["Open"]);
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
fn a_click_on_a_row_picks_it() {
    let theme = Theme::default();
    let mut select = status(None);
    let _ = select.open();
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 12));
    select.render_overlay(
        &mut buf,
        Rect::new(0, 0, 40, 3),
        Rect::new(0, 0, 40, 12),
        &theme,
        theme.background(),
    );
    // The list is rows 3 to 7: its border, the blank, Open, Closed, its border.
    assert!(select.overlay_contains(5, 3));
    assert_eq!(select.handle_overlay_mouse(&press_at(5, 3)), SelectOutcome::Consumed);
    assert_eq!(select.handle_overlay_mouse(&press_at(5, 9)), SelectOutcome::Ignored);
    assert_eq!(select.handle_overlay_mouse(&press_at(5, 6)), SelectOutcome::Changed);
    assert_eq!(select.value(), Some(&"closed"));
    assert!(!select.is_open());
    assert!(!select.overlay_contains(5, 6));
}

// Drawing, textual-dark over a dialog's `$surface-lighten-3`.

fn under(theme: &Theme) -> Paint {
    theme.palette.surface_lighten_3.over(theme.background())
}

fn draw_box(select: &Select<&'static str>, focused: bool) -> String {
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, HEIGHT));
    let area = buf.area;
    select.render_box(&mut buf, area, &theme, under(&theme), focused);
    buffer_to_styled_string(&buf)
}

#[test]
fn select_closed_with_the_prompt() {
    insta::assert_snapshot!(draw_box(&status(None), false));
}

#[test]
fn select_closed_with_a_value() {
    insta::assert_snapshot!(draw_box(&status(Some("open")), false));
}

#[test]
fn select_focused() {
    insta::assert_snapshot!(draw_box(&status(Some("closed")), true));
}

#[test]
fn the_colors_are_pythons() {
    // Edit issue's Status at 160x48, from the `issue-edit` references.
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 9));
    let mut select = status(Some("open"));
    select.render_box(&mut buf, Rect::new(0, 0, 30, 3), &theme, under(&theme), true);
    assert_eq!(buf[(3, 1)].fg, Color::Rgb(0xe0, 0xe0, 0xe0));
    assert_eq!(buf[(3, 1)].bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(buf[(26, 1)].symbol(), "▼");
    assert_eq!(buf[(26, 1)].fg, Color::Rgb(0x83, 0x83, 0x83));
    assert_eq!(buf[(1, 0)].fg, Color::Rgb(0x01, 0x78, 0xd4));
    let _ = select.open();
    select.render_box(&mut buf, Rect::new(0, 0, 30, 3), &theme, under(&theme), false);
    assert_eq!(buf[(26, 1)].symbol(), "▲");
    assert_eq!(buf[(26, 1)].fg, Color::Rgb(0x7f, 0x7f, 0x7f));
    assert_eq!(buf[(1, 0)].fg, Color::Rgb(0x19, 0x19, 0x19));
    select.render_overlay(
        &mut buf,
        Rect::new(0, 0, 30, 3),
        Rect::new(0, 0, 30, 9),
        &theme,
        under(&theme),
    );
    // The blank, dim; Open, highlighted across the row; Closed.
    assert_eq!(buf[(3, 4)].symbol(), "S");
    assert_eq!(buf[(3, 4)].fg, Color::Rgb(0xa1, 0xa1, 0xa1));
    assert_eq!(buf[(2, 5)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    assert_eq!(buf[(3, 5)].fg, Color::Rgb(0xdd, 0xed, 0xf9));
    assert_eq!(buf[(27, 5)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    assert_eq!(buf[(28, 5)].bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(buf[(3, 6)].fg, Color::Rgb(0xe0, 0xe0, 0xe0));
    assert_eq!(buf[(1, 3)].fg, Color::Rgb(0x01, 0x78, 0xd4));
}

/// On the ANSI themes Textual's `$foreground 50%` is the terminal's default color, so the prompt and the arrow are
/// plain; only the open list's blank row is Rich `dim`, SGR 2 there.
#[test]
fn the_ansi_box_has_no_dim_but_the_blank_row_does() {
    let theme = Theme::by_name("ansi-dark").unwrap();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 9));
    let mut select = status(None);
    select.render_box(&mut buf, Rect::new(0, 0, 30, 3), &theme, under(&theme), true);
    assert_eq!((buf[(3, 1)].symbol(), buf[(26, 1)].symbol()), ("S", "▼"));
    for x in [3, 26] {
        assert!(!buf[(x, 1)].modifier.contains(Modifier::DIM), "{:?}", buf[(x, 1)]);
    }
    let _ = select.open();
    select.render_box(&mut buf, Rect::new(0, 0, 30, 3), &theme, under(&theme), true);
    select.render_overlay(
        &mut buf,
        Rect::new(0, 0, 30, 3),
        Rect::new(0, 0, 30, 9),
        &theme,
        under(&theme),
    );
    assert_eq!(buf[(26, 1)].symbol(), "▲");
    assert!(!buf[(26, 1)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(3, 4)].symbol(), "S");
    assert!(buf[(3, 4)].modifier.contains(Modifier::DIM), "{:?}", buf[(3, 4)]);
    insta::assert_snapshot!(buffer_to_styled_string(&buf));
}

/// The box on rows 0 to 2 and its open list, with a button row on rows 4 to 6 underneath, on a screen `rows` tall.
fn draw_open(mut select: Select<&'static str>, rows: u16) -> String {
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, rows));
    let paint = under(&theme);
    let area = buf.area;
    buf.set_style(area, Style::new().bg(theme.paint(paint)));
    let _ = select.open();
    select.render_box(&mut buf, Rect::new(0, 0, 40, 3), &theme, paint, false);
    ButtonRow::flush("Save").render(&mut buf, Rect::new(0, 4, 40, 3), &theme, None);
    select.render_overlay(&mut buf, Rect::new(0, 0, 40, 3), area, &theme, paint);
    buffer_to_styled_string(&buf)
}

#[test]
fn select_open_on_the_blank() {
    insta::assert_snapshot!(draw_open(status(None), 9));
}

#[test]
fn select_open_on_an_option_over_a_button_row() {
    insta::assert_snapshot!(draw_open(status(Some("closed")), 9));
}

#[test]
fn select_open_moved_up_to_fit_the_screen() {
    // A six-row screen: the five-row list goes up over the box, as `constrain: none inside` moves it.
    insta::assert_snapshot!(draw_open(status(Some("open")), 6));
}

#[test]
fn a_long_list_scrolls_to_its_highlight() {
    let keys = Keys::new();
    let mut select = many();
    let _ = select.open();
    let _ = keys.press(&mut select, "end");
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 16));
    select.render_overlay(
        &mut buf,
        Rect::new(0, 0, 30, 3),
        Rect::new(0, 0, 30, 16),
        &theme,
        theme.background(),
    );
    let text = crate::test_util::buffer_to_string(&buf);
    assert!(text.contains("Workflow 11"), "{text}");
    assert!(!text.contains("Workflow 1 "), "{text}");
    assert!(!text.contains("Select a workflow"), "{text}");
}

#[test]
fn a_long_list_has_a_scrollbar_that_picks_nothing() {
    let theme = Theme::default();
    let mut select = many();
    let _ = select.open();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 16));
    select.render_overlay(
        &mut buf,
        Rect::new(0, 0, 30, 3),
        Rect::new(0, 0, 30, 16),
        &theme,
        theme.background(),
    );
    // Rows 3 to 14: thirteen rows in a ten-row window, so a scrollbar in columns 26-27, between the options and the
    // list's right padding, its thumb at the top.
    assert_eq!(buf[(26, 4)].modifier, Modifier::REVERSED);
    assert_eq!(buf[(26, 13)].bg, Color::Rgb(0, 0, 0));
    assert_eq!(buf[(25, 4)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    assert_eq!(select.handle_overlay_mouse(&press_at(26, 6)), SelectOutcome::Consumed);
    assert!(select.is_open());
    assert_eq!(select.handle_overlay_mouse(&press_at(25, 6)), SelectOutcome::Changed);
    assert_eq!(select.value(), Some(&1));
}

// In a dialog's chain.

/// The footer of a dialog with Edit issue's screen bindings while a select's box or its open list has focus.
fn dialog_footer(field: crate::modal::FieldKind) -> Vec<String> {
    use crate::component::{BindingState, ChainLink, Owner, ScreenKind};
    use crate::modal::ModalKind;

    let kind = ModalKind::Test(0);
    let mut chain = vec![
        field.link(kind),
        ChainLink::new(
            Owner::Screen(ScreenKind::Modal(kind)),
            &[BindingId::ModalClose, BindingId::ModalSubmit],
        ),
    ];
    ChainLink::filter_behind_inputs(&mut chain);
    let active = crate::keymap::active_bindings(&chain, &Keymap::defaults(), |_, _| BindingState::Enabled);
    crate::components::footer::footer_items(&active)
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

#[test]
fn the_open_list_takes_q_and_escape_from_the_dialog() {
    use crate::modal::FieldKind;

    assert_eq!(dialog_footer(FieldKind::Select), ["q Close", "shift+⏎ Submit"]);
    assert_eq!(
        dialog_footer(FieldKind::SelectOverlay),
        ["esc Dismiss menu", "shift+⏎ Submit"]
    );
}

#[test]
fn a_disabled_select_never_opens_and_draws_faded() {
    let mut select = status(Some("open"));
    let _ = select.open();
    select.set_disabled(true);
    assert!(select.is_disabled());
    assert!(!select.is_open(), "disabling closes the list");
    assert_eq!(select.open(), SelectOutcome::Ignored);
    assert_eq!(select.toggle(), SelectOutcome::Ignored);
    assert_eq!(
        select.on_binding(BindingId::TextualSelectShowOverlay),
        SelectOutcome::Ignored
    );
    assert!(!select.is_open());
    let theme = Theme::default();
    let under = theme.palette.surface.over(theme.background());
    let area = Rect::new(0, 0, 20, HEIGHT);
    let draw = |select: &Select<&'static str>| {
        let mut buf = Buffer::empty(area);
        select.render_box(&mut buf, area, &theme, under, true);
        buf
    };
    let enabled = draw(&status(Some("open")));
    let disabled = draw(&select);
    let symbols = |buf: &Buffer| {
        buf.content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>()
    };
    // Same cells, faded, and drawn unfocused whatever the owner says.
    assert_eq!(symbols(&enabled), symbols(&disabled));
    assert_ne!(enabled[(3, 1)].fg, disabled[(3, 1)].fg);
    select.set_disabled(false);
    assert_eq!(select.open(), SelectOutcome::Opened);
}
