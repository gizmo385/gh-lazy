//! The Confirm dialog: its answers, keys, focus, footer and layout.

use lazy_github_core::config::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::*;
use crate::components::footer::footer_items;
use crate::test_util::{TestApp, buffer_to_styled_string};
use crate::theme::{Accent, Theme};
use crate::toast::{Severity, Toast};

/// Create PR's push-branch question, with its "Warning:" and branch in yellow.
fn branch_warning() -> Line<'static> {
    let yellow = Style::new().fg(Theme::default().accent(Accent::Yellow));
    Line::from(vec![
        Span::styled("Warning:", yellow),
        Span::raw(" Branch "),
        Span::styled("feature/x", yellow),
        Span::raw(" may not exist on remote. Do you want to create it?"),
    ])
}

fn open(app: &mut TestApp, dialog: ConfirmDialog) {
    app.with_cx(|cx| {
        cx.open_dialog(dialog, |yes| {
            Action::Notify(Toast::new(Severity::Information, format!("answer: {yes}")))
        });
    });
}

fn app_with_config(config: Config) -> TestApp {
    let mut app = TestApp::new(config);
    open(&mut app, ConfirmDialog::new(branch_warning()));
    app
}

fn app() -> TestApp {
    app_with_config(Config::default())
}

/// The replies the opener got, oldest first.
fn replies(app: &TestApp) -> Vec<String> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::Confirm]
}

/// Asserts that the dialog closed with `answer`, replied once.
fn assert_answered(app: &TestApp, answer: bool, what: &str) {
    assert!(app.app().modals().is_empty(), "{what}: still open");
    assert_eq!(replies(app), [format!("answer: {answer}")], "{what}");
}

fn footer(app: &mut TestApp) -> Vec<String> {
    footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

/// Where Python draws the buttons at 160x48: Yes at x = 57 to 72 and No at x = 81 to 96, on rows 25 to 27.
const YES: (u16, u16) = (60, 26);
const NO: (u16, u16) = (85, 26);

// Answers.

#[test]
fn yes_answers_true() {
    for keys in [&["y"][..], &["enter"], &["tab", "tab", "enter"]] {
        let mut app = app();
        app.press(keys);
        assert_answered(&app, true, &format!("{keys:?}"));
    }
    let mut app = app();
    app.render(160, 48);
    app.click(YES.0, YES.1);
    assert_answered(&app, true, "a click on Yes");
}

#[test]
fn everything_else_answers_false() {
    for keys in [
        &["n"][..],
        &["q"],
        &["escape"],
        &["tab", "enter"],
        &["shift+tab", "enter"],
    ] {
        let mut app = app();
        app.press(keys);
        assert_answered(&app, false, &format!("{keys:?}"));
    }
    let mut app = app();
    app.render(160, 48);
    app.click(NO.0, NO.1);
    assert_answered(&app, false, "a click on No");
}

#[test]
fn a_click_between_the_buttons_does_nothing() {
    let mut app = app();
    app.render(160, 48);
    app.click(79, 26);
    app.click(60, 21);
    assert!(is_open(&app));
    assert!(replies(&app).is_empty());
}

#[test]
fn pressing_a_button_focuses_it() {
    // A press on No without its release: No has focus, so `enter` answers no.
    let mut app = app();
    app.render(160, 48);
    app.send(crate::event::Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: NO.0,
        row: NO.1,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }));
    assert!(is_open(&app));
    app.press(&["enter"]);
    assert_answered(&app, false, "enter after pressing No");
}

#[test]
fn overrides_move_the_keys() {
    let mut config = Config::default();
    for (id, key) in [("confirm.yes", "a"), ("confirm.no", "b"), ("modal.close", "x")] {
        config.bindings.overrides.insert(id.into(), key.into());
    }
    let mut app = app_with_config(config.clone());
    app.press(&["y", "n", "q", "escape"]);
    assert!(is_open(&app));
    assert_eq!(footer(&mut app), ["a Yes", "b No", "x Close"]);
    app.press(&["a"]);
    assert_answered(&app, true, "a");
    for key in ["b", "x"] {
        let mut app = app_with_config(config.clone());
        app.press(&[key]);
        assert_answered(&app, false, key);
    }
}

#[test]
fn the_labels_can_change() {
    let mut app = TestApp::new(Config::default());
    open(&mut app, ConfirmDialog::new("Push?").with_labels("Push", "Skip"));
    let screen = app.screen(160, 48);
    assert!(screen.contains("Push") && screen.contains("Skip"), "{screen}");
    app.press(&["tab", "enter"]);
    assert_answered(&app, false, "Skip");
}

// Focus and the footer.

#[test]
fn tab_and_shift_tab_toggle_between_the_buttons() {
    let mut dialog = ConfirmDialog::new("?");
    assert_eq!(dialog.ring.focused(), Some(ConfirmField::Yes));
    let mut seen = Vec::new();
    for direction in [
        FocusMove::Next,
        FocusMove::Next,
        FocusMove::Previous,
        FocusMove::Previous,
    ] {
        dialog.move_focus(direction);
        seen.push(dialog.ring.focused().unwrap());
    }
    assert_eq!(
        seen,
        [ConfirmField::No, ConfirmField::Yes, ConfirmField::No, ConfirmField::Yes]
    );
}

#[test]
fn the_footer_lists_yes_no_and_close() {
    let mut app = app();
    assert_eq!(footer(&mut app), ["y Yes", "n No", "q Close"]);
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), ["y Yes", "n No", "q Close"]);
    let screen = app.screen(160, 48);
    assert_eq!(screen.lines().last(), Some(" y Yes  n No  q Close"));
}

// Drawing.

/// Python's `run_test` capture at 160x48, columns 40 to 119 of rows 16 to 31 (rows 16, 17 and 29 to 31 are blank),
/// with the button rows as the running app draws them (the `pr-new` scenario's `push-prompt` reference): each button
/// fills its column, as `Button { width: 100% }` says. A bare `run_test` loads Button's own default CSS after the
/// dialog's, so the dialog's rule loses on order there; in the app Create PR's buttons load it first.
const PYTHON: &str = "
               █▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀█
               █                                                █
               █   Warning: Branch feature/x may not exist on   █
               █   remote. Do you want to create it?            █
               █                                                █
               █                                                █
               █                                                █
               █ ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔  ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔ █
               █          Yes                      No           █
               █ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁  ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ █
               █▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄█";

#[test]
fn the_layout_matches_pythons_capture() {
    let mut app = app();
    let screen = app.screen(160, 48);
    let rows: Vec<&str> = screen.lines().collect();
    for (offset, python) in PYTHON.lines().skip(1).enumerate() {
        let y = 18 + offset;
        // Only the frame: the capture was over a blank screen, and this one is over the main screen.
        let rust: String = rows[y].chars().skip(55).take(50).collect();
        let python: String = python.chars().skip(15).take(50).collect();
        assert_eq!(rust, python, "row {y}");
    }
    let buffer = app.render(160, 48);
    let yellow = Theme::default().accent(Accent::Yellow);
    assert_eq!(buffer[(59, 20)].fg, yellow, "Warning:");
    assert_eq!(buffer[(76, 20)].fg, yellow, "feature/x");
    assert_ne!(buffer[(68, 20)].fg, yellow, "Branch");
}

fn styled(app: &mut TestApp, cols: u16, rows: u16) -> String {
    buffer_to_styled_string(&app.render(cols, rows))
}

#[test]
fn confirm_160x48() {
    insta::assert_snapshot!(styled(&mut app(), 160, 48));
}

#[test]
fn confirm_no_focused() {
    let mut app = app();
    app.press(&["tab"]);
    insta::assert_snapshot!(styled(&mut app, 160, 48));
}

#[test]
fn confirm_long_question() {
    // Five rows exactly, then seven cut off after the fifth.
    let five = "one two three four five six seven eight nine ten ".repeat(4) + "the end of the fifth row";
    let seven = "word ".repeat(60) + "never shown";
    for question in [five, seven] {
        let mut app = TestApp::new(Config::default());
        open(&mut app, ConfirmDialog::new(question));
        insta::assert_snapshot!(styled(&mut app, 100, 30));
    }
}

#[test]
fn confirm_100x30() {
    insta::assert_snapshot!(styled(&mut app(), 100, 30));
}
