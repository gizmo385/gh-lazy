//! The Paste link dialog: focus, the input's checks, submitting, the lookup in flight, its answer, the prefilled
//! dialog a paste opens, and its drawing.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::links::GithubLink;
use lazy_github_core::models::{FullPullRequest, Repository};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::Value;

use super::*;
use crate::event::Event;
use crate::modal::DialogFrame;
use crate::test_util::{TestApp, buffer_to_styled_string};
use crate::theme::Theme;
use crate::toast::Severity;

/// The response body of the parity fixture at `path`.
fn fixture_body(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn pr_details_repo() -> Repository {
    serde_json::from_value(fixture_body("links/repos/parity-bot/pr-details/GET.json")).unwrap()
}

fn pr_5() -> FullPullRequest {
    let body = fixture_body("pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
    FullPullRequest::from_api(body, &pr_details_repo()).unwrap()
}

/// GitHub with PR 5 of `parity-bot/pr-details`, a missing `parity-bot/missing` and a failing `parity-bot/broken`.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_repository("parity-bot/pr-details", Ok(Some(pr_details_repo())))
        .with_repository("parity-bot/missing", Ok(None))
        .with_repository(
            "parity-bot/broken",
            Err(GhError::Http {
                status: 500,
                message: "Server Error".into(),
                body: Vec::new(),
                headers: Vec::new(),
            }),
        )
        .with_pull_request("parity-bot/pr-details", 5, Ok(pr_5()))
}

const PR_LINK: &str = "https://github.com/parity-bot/pr-details/pull/5";

/// The main screen with `ctrl+v` pressed.
fn opened() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), github());
    app.press(&["ctrl+v"]);
    assert!(is_open(&app), "ctrl+v opens the dialog");
    app
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::PasteLink]
}

fn dialog(app: &TestApp) -> &PasteLinkDialog {
    app.app().top_dialog::<PasteLinkDialog>().expect("Paste link is on top")
}

/// Pastes `text` into the focused input.
fn paste(app: &mut TestApp, text: &str) {
    app.send(Event::Paste(text.into()));
}

/// `(severity, title, message)` of each toast, oldest first.
fn toasts(app: &TestApp) -> Vec<(Severity, Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.title.clone(), toast.message.to_string()))
        .collect()
}

/// The links the app asked GitHub to resolve, with each request's id.
fn resolves(app: &TestApp) -> Vec<(RequestId, GithubLink)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::ResolveLink(link),
            } => Some((*id, link.clone())),
            _ => None,
        })
        .collect()
}

fn pr_link() -> GithubLink {
    GithubLink::PullRequest {
        owner: "parity-bot".into(),
        repo: "pr-details".into(),
        number: 5,
    }
}

fn focused(app: &TestApp) -> Option<PasteLinkField> {
    dialog(app).ring.focused()
}

#[test]
fn tab_cycles_the_input_open_and_cancel() {
    let mut app = opened();
    assert_eq!(focused(&app), Some(PasteLinkField::Input));
    let mut seen = Vec::new();
    for _ in 0..3 {
        app.press(&["tab"]);
        seen.push(focused(&app));
    }
    assert_eq!(
        seen,
        [
            Some(PasteLinkField::Open),
            Some(PasteLinkField::Cancel),
            Some(PasteLinkField::Input)
        ]
    );
    app.press(&["shift+tab"]);
    assert_eq!(focused(&app), Some(PasteLinkField::Cancel));
}

#[test]
fn q_types_in_the_input_and_closes_from_a_button() {
    let mut app = opened();
    app.press(&["q"]);
    assert!(is_open(&app));
    assert_eq!(dialog(&app).text(), "q");
    app.press(&["tab", "q"]);
    assert!(!is_open(&app));
    // escape closes from the input too.
    app.press(&["ctrl+v", "escape"]);
    assert!(!is_open(&app));
}

#[test]
fn enter_in_the_input_checks_it_and_doesnt_open() {
    let mut app = opened();
    assert!(!dialog(&app).is_invalid(), "nothing checked yet");
    app.press(&["enter"]);
    assert!(dialog(&app).is_invalid(), "an empty input is invalid once checked");
    paste(&mut app, PR_LINK);
    app.press(&["enter"]);
    assert!(!dialog(&app).is_invalid());
    assert!(resolves(&app).is_empty());
    assert!(is_open(&app));
}

#[test]
fn each_change_and_blur_checks_the_input() {
    let mut app = opened();
    app.press(&["x"]);
    assert!(dialog(&app).is_invalid());
    app.press(&["backspace"]);
    paste(&mut app, "github.com/parity-bot");
    assert!(!dialog(&app).is_invalid());
    app.press(&["backspace", "backspace", "backspace"]);
    // `github.com/parity-` isn't a login.
    assert!(dialog(&app).is_invalid());
    app.press(&["ctrl+u"]);
    paste(&mut app, "o/r");
    assert!(dialog(&app).is_invalid());
    // Blurring checks it again, whatever it holds.
    app.press(&["tab"]);
    assert!(dialog(&app).is_invalid());
}

#[test]
fn an_unknown_link_toasts_and_keeps_the_dialog() {
    let mut app = opened();
    paste(&mut app, "not a link");
    app.press(&["tab", "enter"]);
    assert_eq!(toasts(&app), [(Severity::Information, None, "Unknown link".into())]);
    assert!(resolves(&app).is_empty());
    assert!(is_open(&app));
    assert_eq!(dialog(&app).text(), "not a link");
    assert!(!dialog(&app).buttons_disabled());
    // Python disables the buttons while it parses, which blurs the pressed Open.
    assert_eq!(focused(&app), None);
}

#[test]
fn submitting_disables_the_buttons_and_blurs_open() {
    let mut app = opened();
    paste(&mut app, PR_LINK);
    app.press(&["tab", "enter"]);
    assert_eq!(
        resolves(&app).iter().map(|(_, link)| link.clone()).collect::<Vec<_>>(),
        [pr_link()]
    );
    assert!(dialog(&app).is_pending());
    assert!(dialog(&app).buttons_disabled());
    assert_eq!(focused(&app), None, "nothing has focus");
    // The next tab goes to the input: the buttons are skipped.
    app.press(&["tab"]);
    assert_eq!(focused(&app), Some(PasteLinkField::Input));
    app.press(&["tab"]);
    assert_eq!(focused(&app), Some(PasteLinkField::Input));
}

#[test]
fn a_second_submit_while_pending_is_ignored() {
    let mut app = opened();
    paste(&mut app, PR_LINK);
    app.press(&["shift+enter", "shift+enter"]);
    assert_eq!(resolves(&app).len(), 1);
    // Typing still works meanwhile.
    app.press(&["x"]);
    assert_eq!(dialog(&app).text(), format!("{PR_LINK}x"));
}

#[tokio::test]
async fn closing_while_pending_cancels_and_drops_the_answer() {
    let mut app = opened();
    paste(&mut app, PR_LINK);
    app.press(&["shift+enter"]);
    let (id, _) = resolves(&app)[0].clone();
    app.press(&["escape"]);
    assert!(!is_open(&app));
    assert!(app.effects().contains(&Effect::Cancel(id)));
    // A late answer opens nothing.
    app.send(Event::Github {
        id,
        response: GithubResponse::ResolveLink(Ok(Some(LinkSubject::PullRequest(Box::new(pr_5()))))),
    });
    app.settle().await;
    assert_eq!(app.app().current_repo(), None);
    assert!(toasts(&app).is_empty());
}

#[tokio::test]
async fn a_failed_lookup_toasts_reenables_the_buttons_and_keeps_the_text() {
    for link in [
        "https://github.com/parity-bot/missing/pull/1",
        "https://github.com/parity-bot/broken",
        "https://github.com/parity-bot/pr-details/issues/77",
        "https://github.com/nobody",
    ] {
        let mut app = opened();
        paste(&mut app, link);
        app.press(&["shift+enter"]);
        app.settle().await;
        assert_eq!(
            toasts(&app),
            [(
                Severity::Error,
                Some("GitHub lookup failed".into()),
                "Could not resolve link!".into()
            )],
            "{link}"
        );
        assert!(is_open(&app), "{link}");
        assert!(!dialog(&app).buttons_disabled(), "{link}");
        assert!(!dialog(&app).is_pending(), "{link}");
        assert_eq!(dialog(&app).text(), link);
    }
}

#[tokio::test]
async fn a_resolved_link_closes_the_dialog_with_its_subject() {
    let mut app = opened();
    paste(&mut app, PR_LINK);
    app.press(&["tab", "enter"]);
    app.settle().await;
    assert!(!is_open(&app));
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
}

#[test]
fn with_text_keeps_the_first_line_selected_and_checked() {
    let mut dialog = PasteLinkDialog::with_text("not a link\nsecond line");
    assert_eq!(dialog.text(), "not a link");
    assert!(dialog.is_invalid());
    assert_eq!(dialog.input.input().selected_text(), Some("not a link"));
    let valid = PasteLinkDialog::with_text(PR_LINK);
    assert!(!valid.is_invalid());
    assert!(valid.ring.is_focused(PasteLinkField::Input));
    // Typing replaces the selection.
    let chord = KeyChord::new(
        crossterm::event::KeyCode::Char('x'),
        crossterm::event::KeyModifiers::NONE,
    );
    let _ = dialog
        .input
        .edit(|input| input.handle_key(&chord, &Keymap::defaults(), ""));
    assert_eq!(dialog.text(), "x");
}

// Drawing.

/// The dialog on its own, framed, at its 80 x 20.
fn draw(dialog: &mut PasteLinkDialog) -> String {
    let theme = Theme::default();
    let keymap = Keymap::defaults();
    let mut terminal = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let background = dialog.background(theme.palette);
            frame.render_widget(
                DialogFrame {
                    theme: &theme,
                    background,
                    border: dialog.border(theme.palette),
                },
                area,
            );
            let cx = RenderCx {
                theme: &theme,
                keymap: &keymap,
                now: tokio::time::Instant::now(),
                focused: true,
                focus_within: true,
            };
            dialog.render(frame, DialogFrame::interior(area), &cx);
        })
        .unwrap();
    buffer_to_styled_string(terminal.backend().buffer())
}

#[test]
fn paste_link_open_160x48() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn paste_link_invalid() {
    let mut dialog = PasteLinkDialog::new();
    dialog.input.paste("not a link");
    insta::assert_snapshot!(draw(&mut dialog));
}

#[test]
fn paste_link_resolving() {
    // As a submit from Open leaves it: the buttons disabled and nothing focused.
    let mut dialog = PasteLinkDialog::new();
    dialog.input.paste(PR_LINK);
    dialog.move_focus(FocusMove::Next);
    dialog.set_buttons_disabled(true);
    dialog.pending = Some(RequestId::new(1));
    insta::assert_snapshot!(draw(&mut dialog));
}

#[test]
fn paste_link_prefilled() {
    insta::assert_snapshot!(draw(&mut PasteLinkDialog::with_text(PR_LINK)));
}
