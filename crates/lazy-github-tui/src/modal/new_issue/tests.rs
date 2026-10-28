//! The New issue dialog: focus, the footer, submitting, the answer, closing, the mouse and its drawing, opened with
//! `I` against the `issue-write` and `issue-details` fixtures.

use std::path::Path;

use crossterm::event::{KeyModifiers, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::IssueListItem;
use serde_json::Value;

use super::*;
use crate::event::Event;
use crate::modal::issue_form::IssueField;
use crate::test_util::TestApp;

const REPO: &str = "parity-bot/issue-fixture";

fn fixture_body(set: &str, path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn fixture_repo() -> Repository {
    let page = fixture_body(
        "issue-details",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    serde_json::from_value(page[0].clone()).unwrap()
}

/// Issue 13, as `POST .../issues` answers.
fn created() -> Issue {
    let body = fixture_body("issue-write", &format!("repos/{REPO}/issues/POST.json"));
    Issue::from_api(body, &fixture_repo()).unwrap()
}

fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body(
        "issue-details",
        &format!("repos/{REPO}/issues/GET.page-1_per_page-30_state-all.json"),
    );
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

/// The repo and its issues; `create_issue` answers with `created`, or fails with a 422 for `None`.
fn gh(created: Option<Issue>) -> FakeGithub {
    let gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()));
    match created {
        Some(issue) => gh.with_created_issue(Ok(issue)),
        None => gh,
    }
}

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

/// The repo selected and `I` pressed on the Issues table.
fn opened_with(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "I"]);
    assert!(is_open(&app), "I opens the dialog");
    app
}

fn opened() -> TestApp {
    opened_with(gh(Some(created())))
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::NewIssue]
}

/// Types `text` into the focused field, a key at a time; `\n` is `enter`.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            ' ' => "space".to_owned(),
            '\n' => "enter".to_owned(),
            c => c.to_string(),
        };
        app.press(&[key.as_str()]);
    }
}

fn toasts(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

fn footer(app: &mut TestApp) -> Vec<String> {
    crate::components::footer::footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

/// The create requests the app sent.
fn creates(app: &TestApp) -> Vec<(RequestId, NewIssue)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::CreateIssue { repo, issue },
            } => {
                assert_eq!(repo.full_name, REPO);
                Some((*id, issue.clone()))
            }
            _ => None,
        })
        .collect()
}

/// A title and a two-line description typed, as the parity scenario types them, with the description focused.
fn typed() -> TestApp {
    let mut app = opened();
    app.press(&["tab"]);
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    type_text(&mut app, "First line\nSecond line");
    app
}

fn mouse(app: &mut TestApp, kind: MouseEventKind, column: u16, row: u16) {
    app.send(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

// Focus.

#[test]
fn focus_walks_the_scroll_area_the_fields_and_the_buttons_and_wraps() {
    let mut dialog = NewIssueDialog::new(fixture_repo(), None);
    assert_eq!(dialog.form.focused(), Some(IssueField::Scroll));
    let mut seen = Vec::new();
    for _ in 0..5 {
        dialog.move_focus(FocusMove::Next);
        seen.push(dialog.form.focused().unwrap());
    }
    assert_eq!(
        seen,
        [
            IssueField::Title,
            IssueField::Description,
            IssueField::Save,
            IssueField::Cancel,
            IssueField::Scroll
        ]
    );
    dialog.move_focus(FocusMove::Previous);
    assert_eq!(dialog.form.focused(), Some(IssueField::Cancel));
}

#[test]
fn the_footer_offers_q_outside_the_text_fields_and_escape_in_them() {
    let mut app = opened();
    let q = ["shift+⏎ Submit", "q Close"];
    let esc = ["shift+⏎ Submit", "esc Close"];
    // The scroll area, the title, the description, Save, Cancel.
    for expected in [&q, &esc, &esc, &q, &q] {
        assert_eq!(footer(&mut app), expected);
        app.press(&["tab"]);
    }
}

#[test]
fn q_types_in_the_text_fields_and_closes_elsewhere() {
    let mut app = opened();
    app.press(&["tab", "q", "tab", "q"]);
    assert!(is_open(&app));
    app.press(&["tab", "q"]);
    assert!(!is_open(&app));
    assert!(creates(&app).is_empty());
}

#[test]
fn typing_in_the_scroll_area_does_nothing() {
    let mut app = opened();
    type_text(&mut app, "abc");
    assert!(is_open(&app));
    app.press(&["shift+enter"]);
    assert_eq!(toasts(&app), [(None, "Must have a non-empty issue title!".to_owned())]);
}

// Submitting.

#[test]
fn an_empty_title_toasts_and_sends_nothing() {
    let mut app = opened();
    // Save, with nothing typed.
    app.press(&["tab", "tab", "tab", "enter"]);
    assert_eq!(toasts(&app), [(None, "Must have a non-empty issue title!".to_owned())]);
    assert_eq!(
        app.app().toasts().iter().next().unwrap().severity,
        crate::toast::Severity::Error
    );
    assert!(creates(&app).is_empty());
    assert!(is_open(&app));
}

/// Py bug B25: Python refuses an empty body.
#[test]
fn a_blank_title_and_an_empty_description_are_sent() {
    let mut app = opened();
    app.press(&["tab", "space", "shift+enter"]);
    assert_eq!(toasts(&app), [(None, "Creating new issue...".to_owned())]);
    let sent: Vec<NewIssue> = creates(&app).into_iter().map(|(_, issue)| issue).collect();
    assert_eq!(
        sent,
        [NewIssue {
            title: " ".into(),
            body: String::new(),
        }]
    );
}

#[test]
fn the_description_is_sent_as_typed_and_success_closes_with_the_issue() {
    let mut app = typed();
    app.press(&["shift+enter"]);
    let sent: Vec<NewIssue> = creates(&app).into_iter().map(|(_, issue)| issue).collect();
    assert_eq!(
        sent,
        [NewIssue {
            title: "Parity title".into(),
            body: "First line\nSecond line".into(),
        }]
    );
    settle(&mut app);
    assert!(!is_open(&app));
    assert_eq!(
        toasts(&app),
        [
            (None, "Creating new issue...".to_owned()),
            (None, "Successfully created issue #13".to_owned()),
        ]
    );
}

#[test]
fn a_failure_toasts_and_keeps_everything_typed() {
    let mut app = TestApp::with_github(Config::default(), gh(None));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "I", "tab"]);
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    type_text(&mut app, "Body");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(is_open(&app));
    assert_eq!(
        toasts(&app),
        [
            (None, "Creating new issue...".to_owned()),
            (
                Some("Error creating issue".to_owned()),
                "Could not create the issue".to_owned()
            ),
        ]
    );
    let screen = app.screen(160, 48);
    assert!(screen.contains("Parity title"), "{screen}");
    assert!(screen.contains("1  Body"), "{screen}");
    // It can be sent again.
    app.press(&["shift+enter"]);
    assert_eq!(creates(&app).len(), 2);
}

#[test]
fn a_second_submit_while_one_is_in_flight_is_ignored() {
    let mut app = typed();
    app.press(&["shift+enter", "shift+enter"]);
    assert_eq!(creates(&app).len(), 1);
    assert_eq!(toasts(&app), [(None, "Creating new issue...".to_owned())]);
}

#[test]
fn a_failed_answer_is_logged() {
    let mut dialog = NewIssueDialog::new(fixture_repo(), None);
    let mut app = opened();
    dialog.form.set_title("T");
    let id = app.with_cx(|cx| {
        dialog.update(NewIssueAction::Submit, cx);
        dialog.pending.unwrap()
    });
    let out = crate::test_util::logs(|| {
        app.with_cx(|cx| {
            let handed_back = dialog.on_response(id, GithubResponse::CreateIssue(Err(GhError::NotInstalled)), cx);
            assert!(handed_back.is_none());
        });
    });
    assert!(out.contains("ERROR Error creating issue:"), "{out}");
    assert!(!dialog.is_pending());
    assert!(dialog.take_output().is_none());
}

#[test]
fn another_response_is_handed_back() {
    let mut dialog = NewIssueDialog::new(fixture_repo(), None);
    let mut app = opened();
    dialog.form.set_title("T");
    app.with_cx(|cx| dialog.update(NewIssueAction::Submit, cx));
    app.with_cx(|cx| {
        let other = dialog.on_response(
            RequestId::new(u64::MAX),
            GithubResponse::CreateIssue(Ok(Box::new(created()))),
            cx,
        );
        assert!(other.is_some(), "not its request");
        assert!(dialog.is_pending());
    });
}

// Closing.

#[test]
fn cancel_and_escape_close_without_a_request() {
    for keys in [
        &["tab", "tab", "tab", "tab", "enter"][..],
        &["tab", "escape"],
        &["escape"],
    ] {
        let mut app = opened();
        app.press(keys);
        assert!(!is_open(&app), "{keys:?}");
        settle(&mut app);
        assert!(creates(&app).is_empty(), "{keys:?}");
        assert!(toasts(&app).is_empty(), "{keys:?}");
    }
}

#[test]
fn closing_in_flight_cancels_the_request() {
    let mut app = typed();
    app.press(&["shift+enter", "escape"]);
    assert!(!is_open(&app));
    let (id, _) = creates(&app)[0].clone();
    assert!(app.effects().contains(&Effect::Cancel(id)));
}

// Text.

#[test]
fn a_prefilled_description_is_in_the_editor() {
    let dialog = NewIssueDialog::new(fixture_repo(), Some("## Steps\n"));
    assert_eq!(dialog.description(), "## Steps\n");
    assert_eq!(dialog.title(), "");
}

#[test]
fn a_paste_goes_to_the_focused_text_field() {
    let mut app = opened();
    app.send(Event::Paste("ignored".into()));
    app.press(&["tab"]);
    app.send(Event::Paste("Pasted title\nsecond".into()));
    app.press(&["tab"]);
    app.send(Event::Paste("One\nTwo".into()));
    let screen = app.screen(160, 48);
    assert!(screen.contains("Pasted title "), "{screen}");
    assert!(!screen.contains("ignored"), "{screen}");
    assert!(screen.contains(" 2  Two"), "{screen}");
}

// The scroll area.

#[test]
fn focusing_the_description_scrolls_it_into_view() {
    let mut app = opened();
    app.screen(160, 48);
    // The description's 21 rows don't fit under the title in the 28-row area.
    app.press(&["tab"]);
    assert!(app.screen(160, 48).contains("█Title "));
    app.press(&["tab"]);
    let screen = app.screen(160, 48);
    assert!(!screen.contains("█Title "), "{screen}");
    assert!(screen.contains("█Description "), "{screen}");
}

#[test]
fn the_scroll_keys_and_the_wheel_scroll_the_area() {
    let mut app = opened();
    app.screen(160, 48);
    app.press(&["down"]);
    assert!(!app.screen(160, 48).contains("█Title "));
    app.press(&["up"]);
    assert!(app.screen(160, 48).contains("█Title "));
    // Over the empty editor, which can't scroll, the wheel scrolls the area.
    mouse(&mut app, MouseEventKind::ScrollDown, 60, 20);
    assert!(!app.screen(160, 48).contains("█Title "));
    mouse(&mut app, MouseEventKind::ScrollUp, 60, 8);
    assert!(app.screen(160, 48).contains("█Title "));
}

#[test]
fn a_click_focuses_the_field_under_it() {
    let mut app = opened();
    app.screen(160, 48);
    // The title's input is on rows 7 to 9, the description from row 14, the area's label rows between.
    app.click(60, 8);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "esc Close"]);
    type_text(&mut app, "Clicked");
    app.click(60, 6);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close"]);
    app.click(60, 16);
    type_text(&mut app, "Body");
    let screen = app.screen(160, 48);
    assert!(screen.contains("▊  Clicked "), "{screen}");
    assert!(screen.contains("1  Body"), "{screen}");
}

// Drawing.

#[test]
fn new_issue_dialog_opened() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_issue_dialog_typed() {
    let mut app = typed();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_issue_dialog_opened_100x30() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn new_issue_dialog_typed_100x30() {
    let mut app = opened();
    app.screen(100, 30);
    app.press(&["tab"]);
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    type_text(&mut app, "First line\nSecond line");
    insta::assert_snapshot!(app.screen(100, 30));
}
