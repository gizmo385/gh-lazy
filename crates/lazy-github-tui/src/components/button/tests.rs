use crossterm::event::KeyModifiers;
use ratatui::style::Color;

use super::*;
use crate::clock::{Clock as _, FakeClock};
use crate::component::Owner;
use crate::components::footer::footer_items;
use crate::keymap::{ActiveBinding, KeySpec, Keymap};
use crate::modal::{FieldKind, ModalKind};
use crate::test_util::buffer_to_styled_string;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// A Lookup button drawn at (10, 5).
fn drawn() -> Button {
    let mut button = Button::new("Lookup", ButtonVariant::Success);
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    button.render(&mut buf, Rect::new(10, 5, 16, 3), &Theme::default(), false);
    button
}

fn click(button: &mut Button, now: Instant, column: u16, row: u16) -> ButtonMouse {
    let down = button.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), column, row), now);
    if down == ButtonMouse::Missed {
        return down;
    }
    button.handle_mouse(&mouse(MouseEventKind::Up(MouseButton::Left), column, row), now)
}

#[test]
fn a_press_counts_once_per_debounce() {
    let clock = FakeClock::new();
    let mut button = Button::new("Lookup", ButtonVariant::Success);
    assert!(button.press(clock.now()));
    clock.advance(Duration::from_millis(199));
    assert!(!button.press(clock.now()));
    // The ignored press doesn't restart the debounce.
    clock.advance(Duration::from_millis(1));
    assert!(button.press(clock.now()));
    clock.advance(Duration::from_millis(250));
    assert!(button.press(clock.now()));
}

#[test]
fn a_left_click_on_it_presses_it() {
    let clock = FakeClock::new();
    let mut button = drawn();
    let now = clock.now();
    assert_eq!(
        button.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), 12, 6), now),
        ButtonMouse::Focus
    );
    assert_eq!(
        button.handle_mouse(&mouse(MouseEventKind::Up(MouseButton::Left), 12, 6), now),
        ButtonMouse::Pressed
    );
    // A second click inside the debounce doesn't count; one after it does.
    assert_eq!(click(&mut button, now, 25, 7), ButtonMouse::Missed);
    clock.advance(DEBOUNCE);
    assert_eq!(click(&mut button, clock.now(), 25, 7), ButtonMouse::Pressed);
    // Outside it, or with another button, nothing.
    clock.advance(DEBOUNCE);
    assert_eq!(click(&mut button, clock.now(), 26, 6), ButtonMouse::Missed);
    assert_eq!(click(&mut button, clock.now(), 12, 8), ButtonMouse::Missed);
    assert_eq!(
        button.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Right), 12, 6), clock.now()),
        ButtonMouse::Missed
    );
    // A press that's released elsewhere isn't a click.
    let _ = button.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), 12, 6), clock.now());
    assert_eq!(
        button.handle_mouse(&mouse(MouseEventKind::Up(MouseButton::Left), 30, 6), clock.now()),
        ButtonMouse::Missed
    );
}

#[test]
fn a_disabled_button_cant_be_pressed() {
    let clock = FakeClock::new();
    let mut button = drawn();
    button.set_disabled(true);
    assert!(!button.press(clock.now()));
    assert_eq!(click(&mut button, clock.now(), 12, 6), ButtonMouse::Missed);
    button.set_disabled(false);
    assert!(button.press(clock.now()));
}

#[test]
fn press_button_is_never_in_the_footer() {
    let def = BindingId::TextualButtonPress.def();
    assert!(!def.show);
    assert_eq!(
        (def.default_keys, def.action, def.description),
        ("enter", "press", "Press button")
    );
    let keymap = Keymap::defaults();
    let KeySpec::Chord(enter) = KeySpec::parse("enter") else {
        unreachable!()
    };
    assert_eq!(
        keymap.match_in(BUTTON_BINDINGS, &enter),
        Some(BindingId::TextualButtonPress)
    );
    let active = vec![ActiveBinding {
        owner: Owner::DialogField(ModalKind::Test(1), FieldKind::Button),
        id: BindingId::TextualButtonPress,
        key: "enter".into(),
        enabled: true,
    }];
    assert!(footer_items(&active).is_empty());
}

#[test]
fn the_width_is_the_label_and_padding_at_least_16() {
    let theme = Theme::default();
    assert_eq!(Button::new("Lookup", ButtonVariant::Success).width(&theme), 16);
    assert_eq!(
        Button::new("Create pull request", ButtonVariant::Success).width(&theme),
        21
    );
    let ansi = Theme::by_name("ansi-dark").unwrap();
    assert_eq!(
        Button::new("Create pull request", ButtonVariant::Success).width(&ansi),
        23
    );
}

/// Each variant in a column, unfocused, focused and disabled, on the test dialog's background.
fn variants(theme: &Theme) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 54, 15));
    let under = theme.palette.surface_lighten_3.over(theme.background());
    for cell in &mut buf.content {
        cell.set_bg(theme.paint(under));
    }
    for (row, variant) in ButtonVariant::ALL.into_iter().enumerate() {
        let y = u16::try_from(row).unwrap() * 3;
        for (column, (focused, disabled)) in [(false, false), (true, false), (false, true)].into_iter().enumerate() {
            let mut button = Button::new(format!("{variant:?}"), variant);
            button.set_disabled(disabled);
            let x = u16::try_from(column).unwrap() * 18;
            button.render(&mut buf, Rect::new(x, y, 16, 3), theme, focused);
        }
    }
    buffer_to_styled_string(&buf)
}

#[test]
fn button_variants() {
    insta::assert_snapshot!(variants(&Theme::default()));
}

#[test]
fn button_variants_ansi_dark() {
    insta::assert_snapshot!(variants(&Theme::by_name("ansi-dark").unwrap()));
}

#[test]
fn button_row_lookup() {
    // Lookup's interior is 58 wide; at 60 columns the row is centered the same way.
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, ButtonRow::HEIGHT));
    let mut row = ButtonRow::new("Lookup");
    let area = buf.area;
    row.render(&mut buf, area, &theme, Some(Pressed::Submit));
    insta::assert_snapshot!(buffer_to_styled_string(&buf));
}

#[test]
fn the_row_matches_lookups_layout() {
    // Python at 160x48: the interior is x = 51 to 108, the row's buttons are 16 wide at x = 63 and x = 80, on rows
    // 31 to 33, with the labels on row 32.
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 48));
    let mut row = ButtonRow::new("Lookup");
    row.render(&mut buf, Rect::new(51, 30, 58, ButtonRow::HEIGHT), &theme, None);
    assert_eq!(row.button(Pressed::Submit).area, Rect::new(63, 31, 16, 3));
    assert_eq!(row.button(Pressed::Cancel).area, Rect::new(80, 31, 16, 3));
    let label: String = (68..74).map(|x| buf[(x, 32)].symbol().to_owned()).collect();
    assert_eq!(label, "Lookup");
    let label: String = (85..91).map(|x| buf[(x, 32)].symbol().to_owned()).collect();
    assert_eq!(label, "Cancel");
    // Python's colors: the success and error backgrounds, and the label's.
    assert_eq!(buf[(63, 32)].bg, Color::Rgb(0x4e, 0xbf, 0x71));
    assert_eq!(buf[(68, 32)].fg, Color::Rgb(0x0a, 0x18, 0x0e));
    assert_eq!(buf[(63, 31)].fg, Color::Rgb(0x7a, 0xe9, 0x98));
    assert_eq!(buf[(63, 33)].fg, Color::Rgb(0x00, 0x81, 0x39));
    assert_eq!(buf[(80, 32)].bg, Color::Rgb(0xb9, 0x3c, 0x5b));
    assert_eq!(buf[(85, 32)].fg, Color::Rgb(0xf5, 0xe5, 0xe9));
}

#[test]
fn a_focused_button_is_reversed_on_a_lighter_background() {
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 48));
    let mut row = ButtonRow::new("Lookup");
    row.render(
        &mut buf,
        Rect::new(51, 30, 58, ButtonRow::HEIGHT),
        &theme,
        Some(Pressed::Submit),
    );
    // Python's submit-focused capture: #4ebf71 tinted to #55c076, the label bold and reversed.
    assert_eq!(buf[(63, 31)].bg, Color::Rgb(0x55, 0xc0, 0x76));
    assert_eq!(buf[(63, 32)].bg, Color::Rgb(0x55, 0xc0, 0x76));
    assert_eq!(buf[(68, 32)].fg, Color::Rgb(0x0b, 0x18, 0x0f));
    assert!(buf[(67, 32)].modifier.contains(Modifier::REVERSED | Modifier::BOLD));
    assert!(!buf[(66, 32)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buf[(63, 31)].fg, Color::Rgb(0x7a, 0xe9, 0x98));
}

#[test]
fn the_row_reports_which_button_a_click_pressed() {
    let clock = FakeClock::new();
    let theme = Theme::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 48));
    let mut row = ButtonRow::new("Lookup");
    row.render(&mut buf, Rect::new(51, 30, 58, ButtonRow::HEIGHT), &theme, None);
    let now = clock.now();
    let down = mouse(MouseEventKind::Down(MouseButton::Left), 85, 32);
    let up = mouse(MouseEventKind::Up(MouseButton::Left), 85, 32);
    assert_eq!(
        row.handle_mouse(&down, now),
        Some((Pressed::Cancel, ButtonMouse::Focus))
    );
    assert_eq!(
        row.handle_mouse(&up, now),
        Some((Pressed::Cancel, ButtonMouse::Pressed))
    );
    // The gap between the buttons and the margins are no button's.
    let gap = mouse(MouseEventKind::Down(MouseButton::Left), 79, 32);
    assert_eq!(row.handle_mouse(&gap, now), None);
    let margin = mouse(MouseEventKind::Down(MouseButton::Left), 70, 30);
    assert_eq!(row.handle_mouse(&margin, now), None);
    assert_eq!(row.press(Pressed::Submit, now), Some(Pressed::Submit));
    assert_eq!(row.press(Pressed::Submit, now), None);
}
