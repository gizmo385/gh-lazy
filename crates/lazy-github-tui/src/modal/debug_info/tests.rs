//! The Debug Info dialog: its keys, focus, Copy, scrolling and layout, and how the palette opens it.

use crossterm::event::MouseEventKind;
use lazy_github_core::config::Config;
use lazy_github_core::debug::{DebugEnv, debug_markdown};
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};

use super::*;
use crate::components::footer::footer_items;
use crate::effect::{Effect, GithubRequest};
use crate::test_util::{TestApp, buffer_to_styled_string};
use crate::toast::Severity;

fn fixed_env() -> DebugEnv {
    DebugEnv {
        version: "0.1.0",
        rustc: "1.97.0 (fixed 2026-01-01)",
        gh: Some("2.89.0 (fake-gh)".to_owned()),
        os: "linux x86_64".to_owned(),
        terminal: "WezTerm 20240203-110809 (xterm-256color)".to_owned(),
    }
}

/// The config with fixed paths, so the text doesn't depend on the machine.
fn fixed_config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = "/home/user/.cache/lazy-github".into();
    config.core.logfile_path = "/home/user/.config/lazy-github/lazy_github.log".into();
    config
}

fn open(app: &mut TestApp) {
    app.with_cx(|cx| {
        cx.open_dialog(DebugInfoDialog::new(&fixed_env(), &fixed_config()), |()| {
            Action::Notify(Toast::info("closed"))
        });
    });
}

fn app() -> TestApp {
    let mut app = TestApp::new(Config::default());
    open(&mut app);
    app
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::DebugInfo]
}

fn dialog(app: &TestApp) -> &DebugInfoDialog {
    app.app()
        .top_dialog::<DebugInfoDialog>()
        .expect("the Debug Info dialog is open")
}

fn focused(app: &TestApp) -> Option<DebugField> {
    dialog(app).ring.focused()
}

fn toasts(app: &TestApp) -> Vec<(Severity, Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.title.clone(), toast.message.to_string()))
        .collect()
}

fn footer(app: &mut TestApp) -> Vec<String> {
    footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

fn styled(app: &mut TestApp, cols: u16, rows: u16) -> String {
    buffer_to_styled_string(&app.render(cols, rows))
}

// Focus and keys.

#[test]
fn focus_starts_on_copy() {
    assert_eq!(focused(&app()), Some(DebugField::Copy));
}

#[test]
fn tab_goes_copy_close_text_and_round() {
    let mut app = app();
    for expected in [DebugField::Close, DebugField::Body, DebugField::Copy] {
        app.press(&["tab"]);
        assert_eq!(focused(&app), Some(expected));
    }
    for expected in [DebugField::Body, DebugField::Close, DebugField::Copy] {
        app.press(&["shift+tab"]);
        assert_eq!(focused(&app), Some(expected));
    }
}

#[test]
fn the_footer_shows_submit_and_close() {
    let mut app = app();
    app.render(160, 48);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close"]);
}

#[test]
fn enter_on_close_closes() {
    let mut app = app();
    app.press(&["tab", "enter"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(toasts(&app), [(Severity::Information, None, "closed".to_owned())]);
}

#[test]
fn q_and_escape_close() {
    for key in ["q", "escape"] {
        let mut app = app();
        app.press(&[key]);
        assert!(app.app().modals().is_empty(), "{key}");
    }
    // From the text too.
    let mut app = app();
    app.press(&["tab", "tab", "q"]);
    assert!(app.app().modals().is_empty());
}

#[test]
fn shift_enter_does_nothing() {
    for keys in [
        &["shift+enter"][..],
        &["tab", "shift+enter"],
        &["tab", "tab", "shift+enter"],
    ] {
        let mut app = app();
        app.press(keys);
        assert!(is_open(&app), "{keys:?}");
        assert!(toasts(&app).is_empty(), "{keys:?}");
        assert!(app.app().clipboard().is_empty(), "{keys:?}");
    }
}

#[test]
fn enter_on_copy_copies_the_markdown_and_stays_open() {
    let mut app = app();
    app.press(&["enter"]);
    let markdown = debug_markdown(&fixed_env(), &fixed_config());
    assert!(is_open(&app));
    assert_eq!(focused(&app), Some(DebugField::Copy));
    assert_eq!(app.app().clipboard(), markdown);
    assert!(
        app.effects()
            .iter()
            .any(|effect| matches!(effect, Effect::SetClipboard(text) if *text == markdown))
    );
    assert_eq!(
        toasts(&app),
        [(
            Severity::Information,
            Some("Copied".to_owned()),
            "Debug info copied to clipboard".to_owned()
        )]
    );
}

#[test]
fn ctrl_c_does_nothing() {
    for keys in [&["ctrl+c"][..], &["enter", "ctrl+c"], &["tab", "tab", "ctrl+c"]] {
        let mut app = app();
        app.press(keys);
        let before = app.effects().len();
        app.press(&["ctrl+c"]);
        assert!(is_open(&app), "{keys:?}");
        assert_eq!(app.effects().len(), before, "{keys:?}");
        // Only the copy's own toast, never the quit hint.
        assert!(toasts(&app).len() <= 1, "{keys:?}");
    }
}

#[test]
fn clicks_press_the_buttons() {
    // At 100x30 the buttons' labels are on row 22: Copy to Clipboard at x = 32 to 50, Close at 52 to 67.
    let mut app = app();
    app.render(100, 30);
    app.click(40, 22);
    assert!(is_open(&app));
    assert_eq!(app.app().clipboard(), dialog(&app).text());
    app.click(60, 22);
    assert!(app.app().modals().is_empty());
}

// Scrolling (Py bug B95's answer).

#[test]
fn the_scroll_keys_move_the_text_once_it_has_focus() {
    let mut app = app();
    app.render(100, 30);
    // On Copy, `down` and `end` scroll nothing.
    app.press(&["down", "end"]);
    app.render(100, 30);
    assert_eq!(dialog(&app).scroll_offset(), 0);
    app.press(&["tab", "tab", "down"]);
    app.render(100, 30);
    assert_eq!(dialog(&app).scroll_offset(), 1);
    app.press(&["end"]);
    app.render(100, 30);
    let end = dialog(&app).scroll_offset();
    assert!(end > 1);
    app.press(&["home"]);
    app.render(100, 30);
    assert_eq!(dialog(&app).scroll_offset(), 0);
}

#[test]
fn the_wheel_scrolls_the_text() {
    let mut app = app();
    app.render(100, 30);
    app.mouse(MouseEventKind::ScrollDown, 40, 10);
    app.render(100, 30);
    assert!(dialog(&app).scroll_offset() > 0);
    // Not over the buttons.
    let mut app = self::app();
    app.render(100, 30);
    app.mouse(MouseEventKind::ScrollDown, 40, 22);
    app.render(100, 30);
    assert_eq!(dialog(&app).scroll_offset(), 0);
}

#[test]
fn text_that_fits_does_not_scroll() {
    let mut app = app();
    app.render(160, 100);
    app.press(&["tab", "tab", "down", "end"]);
    app.mouse(MouseEventKind::ScrollDown, 80, 40);
    app.render(160, 100);
    assert_eq!(dialog(&app).scroll_offset(), 0);
}

// Layout.

#[test]
fn debug_info_160x48() {
    insta::assert_snapshot!(styled(&mut app(), 160, 48));
}

#[test]
fn debug_info_100x30() {
    insta::assert_snapshot!(styled(&mut app(), 100, 30));
}

#[test]
fn debug_info_100x30_scrolled_to_the_end() {
    let mut app = app();
    app.render(100, 30);
    app.press(&["tab", "tab", "end"]);
    insta::assert_snapshot!(styled(&mut app, 100, 30));
}

#[test]
fn text_that_fits_sits_in_the_middle() {
    let mut app = app();
    insta::assert_snapshot!(app.screen(160, 100));
}

// Opening from the palette.

fn run_debug_info(app: &mut TestApp) {
    app.press(&["ctrl+p"]);
    for c in "debug".chars() {
        app.press(&[&c.to_string()]);
    }
    app.press(&["enter"]);
}

fn gh_version_requests(app: &TestApp) -> usize {
    app.effects()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Github {
                    request: GithubRequest::GhVersion,
                    ..
                }
            )
        })
        .count()
}

#[tokio::test]
async fn the_first_debug_info_asks_gh_once_and_opens_on_the_answer() {
    let mut app = TestApp::new(Config::default());
    run_debug_info(&mut app);
    assert!(app.app().modals().is_empty(), "waits for gh");
    assert_eq!(gh_version_requests(&app), 1);
    // A second one while waiting doesn't ask again.
    run_debug_info(&mut app);
    assert_eq!(gh_version_requests(&app), 1);
    app.settle().await;
    assert!(is_open(&app));
    assert!(dialog(&app).text().contains("- Gh Version: 2.89.0 (fake-gh)\n"));
    assert!(dialog(&app).text().contains("- Terminal: xterm-256color\n"));
    assert_eq!(
        app.gh()
            .calls()
            .iter()
            .filter(|call| **call == GithubCall::GhVersion)
            .count(),
        1
    );

    // Later ones open at once, with the config as it is then.
    app.press(&["q"]);
    app.with_cx(|cx| cx.config_mut().appearance.show_workflows = false);
    run_debug_info(&mut app);
    assert!(is_open(&app));
    assert_eq!(gh_version_requests(&app), 1);
    assert!(dialog(&app).text().contains("\"show_workflows\": false"));
}

#[tokio::test]
async fn a_failed_answer_shows_unknown_and_is_not_asked_again() {
    let gh = FakeGithub::new().with_gh_version(Err(GhError::NotInstalled));
    let mut app = TestApp::with_github(Config::default(), gh);
    run_debug_info(&mut app);
    app.settle().await;
    assert!(is_open(&app));
    assert!(dialog(&app).text().contains("- Gh Version: unknown\n"));
    app.press(&["q"]);
    run_debug_info(&mut app);
    assert!(is_open(&app));
    assert_eq!(gh_version_requests(&app), 1);
}
