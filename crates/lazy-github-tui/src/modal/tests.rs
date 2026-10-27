//! The modal stack, against the test dialog: opening and closing, key and mouse routing, the footer, the frame and
//! the dim.

use std::collections::HashMap;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use super::{DialogAction, ModalKind, TestDialogAction};
use crate::action::Action;
use crate::components::footer::footer_items;
use crate::event::Event;
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::{TestApp, TestDialog, TestField, buffer_to_styled_string};
use crate::theme::Theme;
use crate::toast::{Severity, Toast};

/// The toast a test opener raises with dialog `id`'s output.
fn reply_toast(id: u8, output: Option<&str>) -> Action {
    Action::Notify(Toast::new(Severity::Information, format!("{id}: {output:?}")))
}

/// Opens `dialog`, whose reply raises [`reply_toast`].
fn open(app: &mut TestApp, dialog: TestDialog) {
    let ModalKind::Test(id) = super::Dialog::kind(&dialog) else {
        unreachable!("a test dialog's kind is Test")
    };
    app.with_cx(|cx| cx.open_dialog(dialog, move |output| reply_toast(id, output.as_deref())));
}

fn app_with(dialog: TestDialog) -> TestApp {
    let mut app = TestApp::new(Config::default());
    open(&mut app, dialog);
    app
}

/// The toasts' messages, oldest first.
fn replies(app: &TestApp) -> Vec<String> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

fn open_kinds(app: &TestApp) -> Vec<ModalKind> {
    app.app().modals()
}

fn main_focus(app: &TestApp) -> FocusTarget {
    app.app().main_screen().unwrap().focus()
}

/// `"<key> <description>"` for each footer item of the current focus.
fn footer(app: &mut TestApp) -> Vec<String> {
    footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

// Opening and closing.

#[test]
fn opening_pushes_and_submitting_pops_with_the_result() {
    let mut app = app_with(TestDialog::new(1));
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    app.press(&["x", "y"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert!(replies(&app).is_empty());
    app.press(&["shift+enter"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), [r#"1: Some("xy")"#]);
}

#[test]
fn closing_gives_the_cancelled_result() {
    let mut app = app_with(TestDialog::new(1));
    app.press(&["escape"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["1: None"]);
}

#[test]
fn a_reply_can_target_a_dialog_lower_in_the_stack() {
    // Create PR waiting on Confirm: the top dialog's reply is an action for the one below it.
    let mut app = app_with(TestDialog::new(1));
    app.with_cx(|cx| {
        cx.open_dialog(TestDialog::new(2), |output| {
            Action::Dialog(DialogAction::Test(1, TestDialogAction::Replied(output)))
        });
    });
    assert_eq!(open_kinds(&app), [ModalKind::Test(1), ModalKind::Test(2)]);
    app.press(&["a", "b", "c", "shift+enter"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert!(replies(&app).is_empty());
    app.press(&["shift+enter"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), [r#"1: Some("abc")"#]);
}

#[test]
fn stacked_dialogs_close_top_first() {
    let mut app = app_with(TestDialog::new(1));
    open(&mut app, TestDialog::new(2).focused_on_switch());
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert_eq!(replies(&app), ["2: None"]);
    // The one below kept its focus, on its input, so `q` types there.
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    app.press(&["escape"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["2: None", "1: None"]);
}

#[test]
fn an_action_for_a_closed_dialog_is_dropped() {
    let mut app = TestApp::new(Config::default());
    let effects = app
        .app_mut()
        .update(Action::Dialog(DialogAction::Test(9, TestDialogAction::Close)));
    assert_eq!(effects, []);
    assert_eq!(open_kinds(&app), []);
}

// Keys.

#[test]
fn q_closes_from_the_switch_and_types_in_an_input() {
    let mut app = app_with(TestDialog::new(1).focused_on_switch());
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["1: None"]);

    let mut app = app_with(TestDialog::new(1));
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    app.press(&["shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("q")"#]);
    assert!(!app.app().should_quit());
}

#[test]
fn escape_closes_from_both_fields() {
    for dialog in [TestDialog::new(1), TestDialog::new(1).focused_on_switch()] {
        let mut app = app_with(dialog);
        app.press(&["escape"]);
        assert_eq!(open_kinds(&app), []);
        assert_eq!(replies(&app), ["1: None"]);
    }
}

#[test]
fn tab_moves_focus_inside_the_dialog() {
    let mut app = app_with(TestDialog::new(1));
    app.press(&["tab", "q"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
}

#[test]
fn an_override_moves_modal_close() {
    let mut config = Config::default();
    config.bindings.overrides.insert("modal.close".into(), "x".into());

    let mut app = TestApp::new(config.clone());
    open(&mut app, TestDialog::new(1).focused_on_switch());
    app.press(&["q", "escape"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    app.press(&["x"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["1: None"]);

    // A focused input types the key it was moved to.
    let mut app = TestApp::new(config);
    open(&mut app, TestDialog::new(1));
    app.press(&["x", "shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("x")"#]);
}

#[test]
fn ctrl_q_quits_from_a_modal() {
    for dialog in [TestDialog::new(1), TestDialog::new(1).focused_on_switch()] {
        let mut app = app_with(dialog);
        app.press(&["ctrl+q"]);
        assert_eq!(app.effects().last(), Some(&crate::effect::Effect::Quit));
        assert!(app.app().should_quit());
    }
}

#[test]
fn app_keys_do_nothing_in_a_modal() {
    let mut app = app_with(TestDialog::new(1).focused_on_switch());
    let before = app.effects().len();
    app.press(&["question_mark", "M", "ctrl+p", "1", "2", "3", "4", "5", "6", "ctrl+c"]);
    assert_eq!(app.effects().len(), before);
    assert!(app.app().toasts().is_empty());
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
    assert!(!app.app().help_open());
    assert_eq!(app.app().maximized(), None);
    assert!(!app.app().should_quit());

    // From the input the printable ones type, and the rest still do nothing.
    let mut app = app_with(TestDialog::new(1));
    app.press(&["ctrl+p", "ctrl+c", "question_mark", "1"]);
    assert!(app.app().toasts().is_empty());
    app.press(&["shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("?1")"#]);
}

#[test]
fn escape_closes_the_modal_and_keeps_the_maximize() {
    let mut app = TestApp::new(Config::default());
    app.press(&["M"]);
    let maximized = app.app().maximized();
    assert!(maximized.is_some());
    open(&mut app, TestDialog::new(1));
    app.press(&["escape"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(app.app().maximized(), maximized);
    // With the modal gone, escape minimizes again.
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
}

#[test]
fn a_paste_goes_to_the_dialog() {
    let mut app = app_with(TestDialog::new(1));
    app.send(Event::Paste("pasted".into()));
    app.press(&["shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("pasted")"#]);
}

// The footer.

#[test]
fn the_footer_lists_only_the_dialogs_bindings() {
    let mut app = app_with(TestDialog::new(1));
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "esc Close"]);
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close"]);
}

#[test]
fn the_footer_is_drawn_on_the_bottom_row() {
    let mut app = app_with(TestDialog::new(1));
    let screen = app.screen(160, 48);
    assert_eq!(screen.lines().last(), Some(" shift+⏎ Submit  esc Close"));
    let mut app = app_with(TestDialog::new(1).without_footer());
    let screen = app.screen(160, 48);
    // The main screen's footer stays, dimmed, under a dialog without one.
    assert!(screen.lines().last().unwrap().contains(" q Quit "), "{screen}");
}

#[test]
fn clicking_the_footers_close_closes_the_dialog() {
    let mut app = app_with(TestDialog::new(1));
    let screen = app.screen(160, 48);
    let footer = screen.lines().last().unwrap();
    let column = footer.chars().take_while(|&c| c != 'C').count();
    app.click(u16::try_from(column).unwrap(), 47);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["1: None"]);
}

// The mouse.

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn a_click_outside_the_frame_does_nothing() {
    let mut app = app_with(TestDialog::new(1).focused_on_switch());
    app.render(160, 48);
    let before = app.effects().len();
    // The details tabs, right of the frame, and the repos table, left of it.
    app.click(120, 5);
    app.click(10, 20);
    assert_eq!(app.effects().len(), before);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
    // The dialog didn't take focus from a click either: its switch still has it, so `q` closes.
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), []);
}

#[test]
fn a_click_inside_the_frame_reaches_the_dialog() {
    let mut app = app_with(TestDialog::new(1).focused_on_switch());
    app.render(160, 48);
    // The interior starts at (51, 12): the input's rows are 4 to 6 below that.
    app.click(60, 17);
    app.press(&["q", "shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("q")"#]);
}

#[test]
fn wheel_and_move_events_dont_reach_the_screen_below() {
    let mut config = Config::default();
    config.appearance.show_command_log = true;
    let mut app = TestApp::new(config);
    app.render(160, 48);
    // The command log is at the bottom right: the wheel over it scrolls it.
    let over_log = |kind| mouse(kind, 150, 44);
    let Event::Mouse(wheel) = over_log(MouseEventKind::ScrollDown) else {
        unreachable!()
    };
    assert!(app.app_mut().action_for(Event::Mouse(wheel)).is_some());

    open(&mut app, TestDialog::new(1));
    app.render(160, 48);
    let before = app.effects().len();
    for kind in [
        MouseEventKind::ScrollDown,
        MouseEventKind::ScrollUp,
        MouseEventKind::Moved,
        MouseEventKind::Down(MouseButton::Left),
    ] {
        assert!(app.app_mut().action_for(over_log(kind)).is_none(), "{kind:?}");
        app.send(over_log(kind));
    }
    assert_eq!(app.effects().len(), before);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
}

// Closing restores the screen below.

#[test]
fn closing_restores_the_main_screen_exactly() {
    let mut app = TestApp::new(Config::default());
    app.press(&["question_mark", "5", "M"]);
    app.advance(std::time::Duration::from_secs(10)); // The maximize toast times out.
    let (focus, maximized) = (main_focus(&app), app.app().maximized());
    assert_eq!(focus, FocusTarget::DetailsTabs);
    assert!(maximized.is_some());
    assert!(app.app().help_open());
    let before = app.render(160, 48);

    open(&mut app, TestDialog::new(1));
    assert_ne!(app.render(160, 48), before);
    app.press(&["escape"]);
    app.advance(std::time::Duration::from_secs(10)); // The reply's toast times out.
    assert_eq!(main_focus(&app), focus);
    assert_eq!(app.app().maximized(), maximized);
    assert!(app.app().help_open());
    assert_eq!(app.render(160, 48), before);
}

// Drawing.

fn styled(app: &mut TestApp, cols: u16, rows: u16) -> String {
    buffer_to_styled_string(&app.render(cols, rows))
}

#[test]
fn dialog_frame_160x48() {
    let mut app = app_with(TestDialog::new(1));
    insta::assert_snapshot!(styled(&mut app, 160, 48));
}

#[test]
fn dialog_frame_100x30() {
    let mut app = app_with(TestDialog::new(1));
    insta::assert_snapshot!(styled(&mut app, 100, 30));
}

#[test]
fn dialog_frame_ansi_dark() {
    let mut app = TestApp::with_theme(Config::default(), Theme::by_name("ansi-dark").unwrap());
    let under = app.render(100, 30);
    open(&mut app, TestDialog::new(1));
    let over = app.render(100, 30);
    // Nothing is dimmed: outside the frame and the footer row, every cell is as it was.
    for y in 0..29 {
        for x in (0..20).chain(80..100) {
            assert_eq!(over[(x, y)], under[(x, y)], "({x}, {y})");
        }
    }
    insta::assert_snapshot!(buffer_to_styled_string(&over));
}

#[test]
fn dialog_frame_clipped_50x20() {
    let mut app = app_with(TestDialog::new(1));
    insta::assert_snapshot!(styled(&mut app, 50, 20));
}

#[test]
fn dialog_stacked() {
    let mut app = app_with(TestDialog::new(1));
    open(&mut app, TestDialog::new(2).with_size(30, 11).focused_on_switch());
    insta::assert_snapshot!(styled(&mut app, 100, 30));
}

#[test]
fn the_frame_resizes_with_the_screen() {
    let mut app = app_with(TestDialog::new(1));
    for (cols, rows, x, y) in [(160, 48, 50, 11), (100, 30, 20, 2)] {
        let buffer = app.render(cols, rows);
        assert_eq!(buffer[(x, y)].symbol(), "█");
        assert_eq!(buffer[(x + 1, y)].symbol(), "▀");
        assert_eq!(buffer[(x + 59, y + 24)].symbol(), "█");
        assert_eq!(buffer[(x + 1, y + 24)].symbol(), "▄");
        assert_ne!(buffer[(x - 1, y)].symbol(), "█");
    }
}

// The dim against Python's capture.

/// The colors of each cell of a parity `.screen` capture: `(fg, bg)` as `#rrggbb`, by `(x, y)`, and its text rows.
struct Capture {
    text: Vec<Vec<char>>,
    styles: Vec<Vec<char>>,
    legend: HashMap<char, (Option<String>, Option<String>)>,
    /// The legend letters whose style is reversed.
    inverse: Vec<char>,
}

impl Capture {
    fn read(path: &str) -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        let file = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = file.lines().collect();
        let section = |name: &str| lines.iter().position(|line| *line == name).unwrap();
        let (text, styles, legend) = (section("--- text"), section("--- styles"), section("--- legend"));
        let legend_lines = || lines[legend + 1..].iter().take_while(|line| !line.starts_with("---"));
        let inverse = legend_lines()
            .filter(|line| line.split(' ').any(|word| word == "inverse"))
            .filter_map(|line| line.chars().next())
            .collect();
        let legend = legend_lines()
            .filter_map(|line| {
                let mut words = line.split(' ');
                let letter = words.next()?.chars().next()?;
                let mut fg = None;
                let mut bg = None;
                for word in words {
                    if let Some(color) = word.strip_prefix("fg=") {
                        fg = Some(color.to_owned());
                    } else if let Some(color) = word.strip_prefix("bg=") {
                        bg = Some(color.to_owned());
                    }
                }
                Some((letter, (fg, bg)))
            })
            .collect();
        Self {
            text: lines[text + 1..styles]
                .iter()
                .map(|line| line.chars().collect())
                .collect(),
            styles: lines[styles + 1..].iter().map(|line| line.chars().collect()).collect(),
            legend,
            inverse,
        }
    }

    fn symbol(&self, x: u16, y: u16) -> char {
        self.text[usize::from(y)].get(usize::from(x)).copied().unwrap_or(' ')
    }

    fn colors(&self, x: u16, y: u16) -> &(Option<String>, Option<String>) {
        &self.legend[&self.styles[usize::from(y)][usize::from(x)]]
    }

    fn reversed(&self, x: u16, y: u16) -> bool {
        self.inverse.contains(&self.styles[usize::from(y)][usize::from(x)])
    }
}

fn hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("{other:?}"),
    }
}

/// The first cell of `needle` on row `y` of `buffer`.
fn find(buffer: &Buffer, y: u16, needle: &str) -> u16 {
    let row: String = (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_owned())
        .collect();
    let byte = row
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} isn't on row {y}: {row}"));
    u16::try_from(row[..byte].chars().count()).unwrap()
}

#[test]
fn the_dim_matches_pythons_colors() {
    let python = Capture::read("parity/references/modal-frame/160x48/open.screen");
    let mut app = app_with(TestDialog::new(1));
    let rust = app.render(160, 48);
    let cells = [
        // The intro text, #e0e0e0 dimmed to #646464.
        (find(&rust, 10, "LazyGithub is"), 10),
        // The details tab bar: the active tab's label and its underline, and the rest of the underline.
        (find(&rust, 4, "Introduction"), 4),
        (65, 5),
        (100, 5),
        // The status box, the focused repos box's border and title, and the details box's border.
        (1, 1),
        (0, 3),
        (3, 3),
        (63, 3),
        // The frame's border on each side, and its interior.
        (50, 11),
        (51, 11),
        (50, 20),
        (109, 20),
        (51, 35),
        (60, 13),
        // The modal footer's key and description.
        (1, 47),
        (10, 47),
    ];
    for (x, y) in cells {
        assert_eq!(
            rust[(x, y)].symbol().chars().next(),
            Some(python.symbol(x, y)),
            "the cell at ({x}, {y}) shows something else"
        );
        let (fg, bg) = python.colors(x, y);
        let cell = &rust[(x, y)];
        // The capture leaves out the foreground of a blank cell.
        if let Some(fg) = fg
            && cell.symbol() != " "
        {
            assert_eq!(&hex(cell.fg), fg, "fg at ({x}, {y})");
        }
        assert_eq!(bg.as_deref(), Some(hex(cell.bg).as_str()), "bg at ({x}, {y})");
    }
}

// Focus, buttons and `modal.submit` (#213).

#[test]
fn tab_walks_lookups_order_and_wraps() {
    // Input, the other field, Submit, Cancel: the footer shows which has focus, as the modal-frame captures check.
    let mut app = app_with(TestDialog::new(1));
    let mut footers = vec![footer(&mut app)];
    for _ in 0..4 {
        app.press(&["tab"]);
        footers.push(footer(&mut app));
    }
    let typing = ["shift+⏎ Submit", "esc Close"];
    let other = ["shift+⏎ Submit", "q Close"];
    assert_eq!(footers, [&typing[..], &other, &other, &other, &typing]);
    // Back on the input after the wrap: it types.
    app.press(&["q", "shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("q")"#]);

    // shift+tab from the input goes to Cancel, which enter presses.
    let mut app = app_with(TestDialog::new(1));
    app.press(&["shift+tab", "enter"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["1: None"]);
}

#[test]
fn shift_enter_submits_from_every_field() {
    for field in [
        TestField::Input,
        TestField::Switch,
        TestField::Submit,
        TestField::Cancel,
    ] {
        let mut app = app_with(TestDialog::new(1).focused_on(field));
        app.press(&["shift+enter"]);
        assert_eq!(replies(&app), [r#"1: Some("")"#], "{field:?}");
    }
}

#[test]
fn enter_presses_the_focused_button_and_does_nothing_in_the_input() {
    let mut app = app_with(TestDialog::new(1).focused_on(TestField::Submit));
    app.press(&["enter"]);
    assert_eq!(replies(&app), [r#"1: Some("")"#]);

    let mut app = app_with(TestDialog::new(1).focused_on(TestField::Cancel));
    app.press(&["enter"]);
    assert_eq!(replies(&app), ["1: None"]);

    let mut app = app_with(TestDialog::new(1));
    app.press(&["a", "enter", "enter"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert!(replies(&app).is_empty());
    app.press(&["shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("a")"#]);
}

#[test]
fn clicking_a_button_presses_it() {
    // Lookup's buttons at 160x48: Submit at x = 63 to 78 and Cancel at x = 80 to 95, on rows 31 to 33.
    let mut app = app_with(TestDialog::new(1));
    app.render(160, 48);
    app.click(85, 32);
    assert_eq!(replies(&app), ["1: None"]);

    let mut app = app_with(TestDialog::new(1));
    app.press(&["x"]);
    app.render(160, 48);
    app.click(64, 31);
    assert_eq!(replies(&app), [r#"1: Some("x")"#]);

    // Between the buttons is neither.
    let mut app = app_with(TestDialog::new(1));
    app.render(160, 48);
    app.click(79, 32);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
}

#[test]
fn a_disabled_button_is_skipped_and_cant_be_pressed() {
    let mut app = app_with(TestDialog::new(1).with_disabled(TestField::Submit));
    app.render(160, 48);
    app.click(64, 32);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    // The click didn't focus it: the input still types. Then the switch, then Cancel: enter there cancels.
    app.press(&["q", "tab", "tab", "enter"]);
    assert_eq!(replies(&app), ["1: None"]);
}

#[test]
fn disabling_the_focused_button_leaves_nothing_focused() {
    let mut dialog = TestDialog::new(1).focused_on(TestField::Submit);
    dialog.set_disabled(TestField::Submit, true);
    let mut app = app_with(dialog);
    // Nothing focused: enter does nothing, and the footer is the screen's alone.
    app.press(&["enter"]);
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close"]);
    // shift+tab, like tab, goes to the first field.
    app.press(&["shift+tab"]);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "esc Close"]);
}

#[test]
fn dialog_focus_input() {
    let mut app = app_with(TestDialog::new(1));
    insta::assert_snapshot!(styled(&mut app, 160, 48));
}

#[test]
fn dialog_focus_button() {
    let mut app = app_with(TestDialog::new(1));
    app.press(&["tab", "tab"]);
    insta::assert_snapshot!(styled(&mut app, 160, 48));
}

#[test]
fn the_buttons_match_pythons_colors() {
    for (capture, presses) in [("open", &[][..]), ("submit-focused", &["tab", "tab"][..])] {
        let python = Capture::read(&format!("parity/references/modal-frame/160x48/{capture}.screen"));
        let mut app = app_with(TestDialog::new(1));
        app.press(presses);
        let rust = app.render(160, 48);
        for y in 31..=33 {
            for x in (63..79).chain(80..96) {
                let (fg, bg) = python.colors(x, y);
                let cell = &rust[(x, y)];
                assert_eq!(
                    cell.symbol().chars().next(),
                    Some(python.symbol(x, y)),
                    "{capture} ({x}, {y})"
                );
                assert_eq!(bg.as_deref(), Some(hex(cell.bg).as_str()), "{capture} bg at ({x}, {y})");
                if let Some(fg) = fg
                    && cell.symbol() != " "
                {
                    assert_eq!(&hex(cell.fg), fg, "{capture} fg at ({x}, {y})");
                }
            }
        }
    }
}

// The switch.

/// Whether the test dialog's switch is drawn on at 160x48: the right half of its slider, at (57, 25) and (58, 25), is
/// the thumb, a reversed blank.
fn switch_drawn_on(app: &mut TestApp) -> bool {
    let buffer = app.render(160, 48);
    let thumb = |x: u16| buffer[(x, 25)].modifier.contains(Modifier::REVERSED);
    assert_eq!(thumb(55), !thumb(57), "the thumb is one half of the slider");
    thumb(57)
}

#[test]
fn enter_and_space_toggle_the_focused_switch() {
    let mut app = app_with(TestDialog::new(1).focused_on_switch());
    assert!(!switch_drawn_on(&mut app));
    app.press(&["space"]);
    assert!(switch_drawn_on(&mut app));
    app.press(&["enter"]);
    assert!(!switch_drawn_on(&mut app));
    app.press(&["enter", "space", "space"]);
    assert!(switch_drawn_on(&mut app));
    assert_eq!(open_kinds(&app), [ModalKind::Test(1)]);
    // The toggle is hidden: the footer is the dialog's alone.
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close"]);
    // q isn't the switch's, so it closes the dialog from there.
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), []);
    assert_eq!(replies(&app), ["1: None"]);
}

#[test]
fn space_types_in_the_input_and_leaves_the_switch() {
    let mut app = app_with(TestDialog::new(1));
    app.press(&["a", "space", "b"]);
    assert!(!switch_drawn_on(&mut app));
    app.press(&["shift+enter"]);
    assert_eq!(replies(&app), [r#"1: Some("a b")"#]);
}

#[test]
fn clicking_the_switch_toggles_and_focuses_it() {
    let mut app = app_with(TestDialog::new(1));
    app.render(160, 48);
    // Its corners: the switch is (51, 24) to (60, 26).
    app.click(51, 26);
    assert!(switch_drawn_on(&mut app));
    app.click(60, 24);
    assert!(!switch_drawn_on(&mut app));
    // Right of it is the interior, not the switch.
    app.click(61, 25);
    assert!(!switch_drawn_on(&mut app));
    // The clicks focused it: space toggles and q closes.
    app.press(&["space"]);
    assert!(switch_drawn_on(&mut app));
    app.press(&["q"]);
    assert_eq!(open_kinds(&app), []);
}

#[test]
fn the_switch_matches_pythons_cells() {
    for (capture, presses) in [
        ("modal-frame/160x48/open", &[][..]),
        ("modal-frame/160x48/switch-focused", &["tab"][..]),
        ("repo-lookup/160x48/switch-on", &["tab", "space"][..]),
    ] {
        let python = Capture::read(&format!("parity/references/{capture}.screen"));
        let mut app = app_with(TestDialog::new(1));
        app.press(presses);
        let rust = app.render(160, 48);
        for y in 24..27 {
            for x in 51..61 {
                let (fg, bg) = python.colors(x, y);
                let cell = &rust[(x, y)];
                assert_eq!(
                    cell.symbol().chars().next(),
                    Some(python.symbol(x, y)),
                    "{capture} ({x}, {y})"
                );
                assert_eq!(bg.as_deref(), Some(hex(cell.bg).as_str()), "{capture} bg at ({x}, {y})");
                if let Some(fg) = fg {
                    assert_eq!(&hex(cell.fg), fg, "{capture} fg at ({x}, {y})");
                }
                assert_eq!(
                    cell.modifier.contains(Modifier::REVERSED),
                    python.reversed(x, y),
                    "{capture} reversed at ({x}, {y})"
                );
            }
        }
    }
}
