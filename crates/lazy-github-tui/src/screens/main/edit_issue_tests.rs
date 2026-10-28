//! `issue.edit` on the main screen: from the Issues table, whose row the saved issue replaces, and from the Issue
//! Overview, which nothing refreshes. The dialog itself is tested in `modal::edit_issue`.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{Issue, IssueListItem, IssueState, Repository};
use serde_json::Value;

use super::{FocusTarget, MainScreen, SectionId};
use crate::effect::Effect;
use crate::modal::ModalKind;
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

/// Issue 12 as `PATCH .../issues/12` answers: retitled and closed.
fn updated() -> Issue {
    let body = fixture_body("issue-write", &format!("repos/{REPO}/issues/12/PATCH.json"));
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

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

/// The repo selected with `page` as its issues, the Issues table focused.
fn app_with(page: Vec<IssueListItem>) -> TestApp {
    let gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(page))
        .with_updated_issue(Ok(updated()));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3"]);
    app
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

fn row(app: &TestApp, number: u64) -> Issue {
    main(app).issue_lists.issues_table().get(&number).unwrap().clone()
}

fn rows(app: &TestApp) -> Vec<u64> {
    main(app).issue_lists.issues_table().row_keys().copied().collect()
}

fn toasts(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

#[test]
fn e_on_an_empty_issues_table_toasts() {
    let mut app = app_with(Vec::new());
    app.press(&["E"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(toasts(&app), [(None, "No issue currently selected".to_owned())]);
    assert_eq!(
        app.app().toasts().iter().next().unwrap().severity,
        crate::toast::Severity::Error
    );
}

#[test]
fn the_updated_issue_replaces_its_row_and_focus_stays_on_the_table() {
    let mut app = app_with(issues_page());
    app.press(&["E"]);
    assert_eq!(app.app().modals(), [ModalKind::EditIssue]);
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(rows(&app), [12, 11, 10, 9]);
    let issue = row(&app, 12);
    assert_eq!(issue.title, "Crash when the config file is blank");
    assert_eq!(issue.state, IssueState::Closed);
    assert_eq!(main(&app).focus(), FocusTarget::Table(SectionId::Issues));
    assert_eq!(
        main(&app)
            .issue_lists
            .issues_table()
            .selected()
            .map(|issue| issue.number),
        Some(12)
    );
    // The table cache is rewritten with it.
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::WriteCache { text, .. } if text.contains("Crash when the config file is blank")
    )));
    // Nothing opens in the details.
    assert_eq!(main(&app).details_issue(), None);
}

#[test]
fn e_from_the_overview_edits_the_shown_issue_and_changes_nothing() {
    let mut app = app_with(issues_page());
    // Issue 12 shown, then focus into its Overview.
    app.press(&["enter", "tab", "E"]);
    assert_eq!(app.app().modals(), [ModalKind::EditIssue]);
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(
        toasts(&app).last().map(|(_, message)| message.as_str()),
        Some("Successfully updated issue #12")
    );
    assert_eq!(row(&app, 12).title, "Crash when the config file is empty");
    assert_eq!(
        main(&app).details_issue().map(|issue| issue.title.as_str()),
        Some("Crash when the config file is empty")
    );
    assert_eq!(main(&app).focus(), FocusTarget::DetailsContent);
}
