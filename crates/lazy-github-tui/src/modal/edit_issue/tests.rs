//! The Edit issue dialog: its prefilled fields, focus, the footer, the Status select, saving, the answer, closing and
//! its drawing, opened with `E` on the Issues table against the `issue-write` and `issue-details` fixtures.

use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{IssueListItem, IssueState, Repository};
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

/// Issues 12, 11, 10 and 9, and pull request 8.
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

/// Issue `number` from the issues page.
fn issue(number: u64) -> Issue {
    issues_page()
        .into_iter()
        .find_map(|item| item.into_issue().filter(|issue| issue.number == number))
        .unwrap()
}

/// Issue 12 as `PATCH .../issues/12` answers: retitled and closed.
fn updated() -> Issue {
    let body = fixture_body("issue-write", &format!("repos/{REPO}/issues/12/PATCH.json"));
    Issue::from_api(body, &fixture_repo()).unwrap()
}

/// The repo and its issues; `update_issue` answers with `updated`, or fails with a 422 for `None`.
fn gh(updated: Option<Issue>) -> FakeGithub {
    let gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()));
    match updated {
        Some(issue) => gh.with_updated_issue(Ok(issue)),
        None => gh,
    }
}

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

/// The repo selected and `E` pressed on the Issues table, whose cursor is on issue 12.
fn opened_with(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "E"]);
    assert!(is_open(&app), "E opens the dialog");
    app
}

fn opened() -> TestApp {
    opened_with(gh(Some(updated())))
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::EditIssue]
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

/// The update requests the app sent.
fn updates(app: &TestApp) -> Vec<(RequestId, u64, IssueUpdate)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::UpdateIssue { repo, number, update },
            } => {
                assert_eq!(repo.full_name, REPO);
                Some((*id, *number, update.clone()))
            }
            _ => None,
        })
        .collect()
}

fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
        app.press(&[key.as_str()]);
    }
}

fn click(app: &mut TestApp, column: u16, row: u16) {
    app.send(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

// The fields.

#[test]
fn the_fields_hold_the_issue() {
    let dialog = EditIssueDialog::new(Box::new(issue(12)));
    assert_eq!(dialog.form.title(), "Crash when the config file is empty");
    assert!(dialog.form.body().starts_with("## Steps to reproduce\n"));
    assert_eq!(dialog.form.status(), Some(IssueState::Open));
    // Issue 11 is closed and has no body.
    let dialog = EditIssueDialog::new(Box::new(issue(11)));
    assert_eq!(dialog.form.body(), "");
    assert_eq!(dialog.form.status(), Some(IssueState::Closed));
}

#[test]
fn focus_walks_the_scroll_area_the_fields_and_the_buttons() {
    let mut dialog = EditIssueDialog::new(Box::new(issue(12)));
    assert_eq!(dialog.form.focused(), Some(IssueField::Scroll));
    let mut seen = Vec::new();
    for _ in 0..6 {
        dialog.move_focus(FocusMove::Next);
        seen.push(dialog.form.focused().unwrap());
    }
    assert_eq!(
        seen,
        [
            IssueField::Title,
            IssueField::Status,
            IssueField::Description,
            IssueField::Save,
            IssueField::Cancel,
            IssueField::Scroll,
        ]
    );
}

#[test]
fn the_footer_follows_the_focus_and_the_open_list() {
    let mut app = opened();
    let q = ["q Close", "shift+⏎ Submit"];
    let esc = ["esc Close", "shift+⏎ Submit"];
    assert_eq!(footer(&mut app), q);
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), esc);
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), q, "the closed select types nothing");
    app.press(&["enter"]);
    assert_eq!(footer(&mut app), ["esc Dismiss menu", "shift+⏎ Submit"]);
    // `q` searches the list instead of closing the dialog.
    app.press(&["q"]);
    assert!(is_open(&app));
    app.press(&["escape"]);
    assert!(is_open(&app), "escape only closes the list");
    assert_eq!(footer(&mut app), q);
}

#[test]
fn the_title_selects_its_text_so_typing_replaces_it() {
    let mut app = opened();
    app.press(&["tab"]);
    type_text(&mut app, "New");
    app.press(&["shift+enter"]);
    assert_eq!(updates(&app)[0].2.title, "New");
}

// The Status select.

#[test]
fn picking_closed_in_the_list_sends_it() {
    let mut app = opened();
    app.press(&["tab", "tab", "enter", "down", "enter"]);
    assert!(app.screen(160, 48).contains("▊  Closed "));
    app.press(&["shift+enter"]);
    let (_, number, update) = updates(&app).remove(0);
    assert_eq!(number, 12);
    assert_eq!(update.state, Some(IssueState::Closed));
    assert_eq!(update.title, "Crash when the config file is empty");
    assert!(update.body.starts_with("## Steps to reproduce\n"));
}

#[test]
fn the_blank_row_leaves_the_state_out() {
    let mut app = opened();
    app.press(&["tab", "tab", "enter", "home", "enter"]);
    assert!(app.screen(160, 48).contains("▊  Select "));
    app.press(&["shift+enter"]);
    assert_eq!(updates(&app)[0].2.state, None);
}

#[test]
fn tab_closes_the_list_and_moves_on() {
    let mut app = opened();
    app.press(&["tab", "tab", "enter", "down", "tab"]);
    assert_eq!(
        footer(&mut app),
        ["esc Close", "shift+⏎ Submit"],
        "the description has focus"
    );
    app.press(&["shift+enter"]);
    assert_eq!(updates(&app)[0].2.state, Some(IssueState::Open));
}

#[test]
fn clicks_open_the_list_and_pick_a_row() {
    let mut app = opened();
    let screen = app.screen(160, 48);
    // The Status box's text row, and the list's rows below it: the blank, Open, Closed.
    let row = screen.lines().position(|line| line.contains("▊  Open ")).unwrap();
    let row = u16::try_from(row).unwrap();
    click(&mut app, 60, row);
    assert_eq!(footer(&mut app), ["esc Dismiss menu", "shift+⏎ Submit"]);
    // The app draws after every event, which places the list.
    app.screen(160, 48);
    click(&mut app, 60, row + 5);
    assert_eq!(footer(&mut app), ["q Close", "shift+⏎ Submit"]);
    app.press(&["shift+enter"]);
    assert_eq!(updates(&app)[0].2.state, Some(IssueState::Closed));
}

// Saving.

#[test]
fn save_reads_saving_and_is_disabled_while_in_flight() {
    for keys in [&["tab", "tab", "tab", "tab", "enter"][..], &["shift+enter"]] {
        let mut app = opened();
        app.press(keys);
        assert_eq!(updates(&app).len(), 1, "{keys:?}");
        assert_eq!(toasts(&app), [(None, "Updating issue #12...".to_owned())], "{keys:?}");
        let toast = app.app().toasts().iter().next().unwrap().clone();
        assert_eq!(toast.timeout, Duration::from_secs(1));
        assert!(app.screen(160, 48).contains("Saving..."), "{keys:?}");
        // A second submit, by key or by the disabled button, sends nothing.
        app.press(&["shift+enter"]);
        assert_eq!(updates(&app).len(), 1, "{keys:?}");
    }
}

#[test]
fn success_closes_with_the_issue() {
    let mut app = opened();
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert_eq!(
        toasts(&app),
        [
            (None, "Updating issue #12...".to_owned()),
            (None, "Successfully updated issue #12".to_owned()),
        ]
    );
}

#[test]
fn a_failure_toasts_restores_save_and_keeps_the_edits() {
    let mut app = opened_with(gh(None));
    app.press(&["tab"]);
    type_text(&mut app, "Edited");
    app.press(&["shift+enter"]);
    let out = crate::test_util::logs(|| settle(&mut app));
    assert!(out.contains("ERROR Error updating issue #12:"), "{out}");
    assert!(is_open(&app));
    assert_eq!(
        toasts(&app)[1],
        (
            Some("Error updating issue".to_owned()),
            "Could not update issue #12".to_owned()
        )
    );
    let screen = app.screen(160, 48);
    assert!(!screen.contains("Saving..."), "{screen}");
    assert!(screen.contains("▊  Edited "), "{screen}");
    app.press(&["shift+enter"]);
    assert_eq!(updates(&app).len(), 2);
}

#[test]
fn save_comes_back_after_a_failure() {
    let mut dialog = EditIssueDialog::new(Box::new(issue(12)));
    let mut app = opened();
    app.with_cx(|cx| dialog.update(EditIssueAction::Submit, cx));
    assert_eq!(dialog.form.save_state(), ("Saving...", true));
    let id = dialog.pending.unwrap();
    app.with_cx(|cx| {
        let other = dialog.on_response(id, GithubResponse::CreateIssue(Ok(Box::new(updated()))), cx);
        assert!(other.is_some(), "not an update");
        let handed_back = dialog.on_response(id, GithubResponse::UpdateIssue(Err(GhError::NotInstalled)), cx);
        assert!(handed_back.is_none());
    });
    assert_eq!(dialog.form.save_state(), ("Save", false));
    assert!(!dialog.is_pending());
    assert!(dialog.take_output().is_none());
}

// Closing.

#[test]
fn cancel_escape_and_q_close_without_a_request() {
    for keys in [
        &["tab", "tab", "tab", "tab", "tab", "enter"][..],
        &["escape"],
        &["q"],
        &["tab", "escape"],
    ] {
        let mut app = opened();
        app.press(keys);
        assert!(!is_open(&app), "{keys:?}");
        settle(&mut app);
        assert!(updates(&app).is_empty(), "{keys:?}");
    }
}

#[test]
fn closing_in_flight_cancels_the_request() {
    let mut app = opened();
    app.press(&["shift+enter", "escape"]);
    assert!(!is_open(&app));
    let (id, ..) = updates(&app)[0].clone();
    assert!(app.effects().contains(&Effect::Cancel(id)));
}

// Drawing.

#[test]
fn edit_issue_dialog_opened() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn edit_issue_dialog_status_open() {
    let mut app = opened();
    app.press(&["tab", "tab", "enter"]);
    insta::assert_snapshot!(app.screen(160, 48));
}
