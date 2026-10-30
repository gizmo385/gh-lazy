//! `pull_request.edit` on the main screen: from the PR table, which fetches the pull request first and whose row the
//! updated one replaces, and from the PR Overview, which leaves the table alone. The dialog itself is tested in
//! `modal::pull_request`.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Repository};
use serde_json::Value;

use super::{FocusTarget, MainScreen};
use crate::effect::{Effect, GithubRequest};
use crate::modal::ModalKind;
use crate::test_util::TestApp;
use crate::toast::Severity;

const REPO: &str = "parity-bot/pr-details";

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
        "pr-details",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    serde_json::from_value(page[0].clone()).unwrap()
}

fn pull_request(number: u64) -> FullPullRequest {
    let body = fixture_body("pr-details", &format!("repos/{REPO}/pulls/{number}/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

/// PRs 5, 4, 3 (merged) and 2.
fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body(
        "pr-details",
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

fn gh(page: Vec<IssueListItem>) -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(page))
        .with_pull_request(REPO, 3, Ok(pull_request(3)))
        .with_pull_request(REPO, 5, Ok(pull_request(5)))
        .with_pull_request_update(5, Ok(()))
}

/// The repo selected with `gh`'s PRs listed, the PR table focused.
fn app_with(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

fn row_title(app: &TestApp, number: u64) -> String {
    main(app)
        .issue_lists
        .prs_table()
        .get(&number)
        .unwrap()
        .issue
        .title
        .clone()
}

fn toasts(app: &TestApp) -> Vec<(Severity, Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.title.clone(), toast.message.to_string()))
        .collect()
}

/// Types `text` into the focused field, a key at a time.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
        app.press(&[key.as_str()]);
    }
}

#[test]
fn e_on_an_empty_table_toasts_and_sends_nothing() {
    let mut app = app_with(gh(Vec::new()));
    app.press(&["E"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(
        toasts(&app),
        [(Severity::Error, None, "No pull request currently selected".to_owned())]
    );
    assert!(
        !app.gh()
            .calls()
            .iter()
            .any(|call| matches!(call, GithubCall::PullRequest { .. }))
    );
}

#[test]
fn a_failed_fetch_toasts_and_opens_nothing() {
    let gh = gh(issues_page()).with_pull_request(
        REPO,
        5,
        Err(GhError::Http {
            status: 500,
            message: "Server Error".into(),
            body: Vec::new(),
            headers: Vec::new(),
        }),
    );
    let mut app = app_with(gh);
    app.press(&["E"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(
        toasts(&app),
        [(
            Severity::Error,
            Some("API Error".to_owned()),
            "Error fetching pull request!".to_owned()
        )]
    );
}

#[test]
fn a_merged_pull_request_warns_from_the_table() {
    let mut app = app_with(gh(issues_page()));
    app.press(&["j", "j", "E"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(
        toasts(&app),
        [(Severity::Warning, None, "This PR has already been merged!".to_owned())]
    );
    assert_eq!(main(&app).focus(), FocusTarget::Table(super::SectionId::PullRequests));
}

#[test]
fn a_merged_pull_request_warns_from_the_overview() {
    let mut app = app_with(gh(issues_page()));
    // PR 3 shown, then focus into its Overview.
    app.press(&["j", "j", "enter"]);
    settle(&mut app);
    app.press(&["tab", "E"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(
        toasts(&app),
        [(Severity::Warning, None, "This PR has already been merged!".to_owned())]
    );
}

#[test]
fn a_second_e_while_fetching_opens_one_dialog() {
    let mut app = app_with(gh(issues_page()));
    app.press(&["E", "E"]);
    let fetches: Vec<_> = app
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::PullRequest { .. },
            } => Some(*id),
            _ => None,
        })
        .collect();
    let [first, _] = fetches[fetches.len() - 2..] else {
        panic!("two fetches: {fetches:?}");
    };
    assert!(app.effects().contains(&Effect::Cancel(first)));
    settle(&mut app);
    assert_eq!(app.app().modals(), [ModalKind::PullRequest]);
}

#[test]
fn from_the_table_the_row_is_replaced_and_the_details_show_the_edit() {
    let mut app = app_with(gh(issues_page()));
    app.press(&["E"]);
    settle(&mut app);
    type_text(&mut app, "Add a cursor to the diff viewer");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(row_title(&app, 5), "Add a cursor to the diff viewer");
    assert_eq!(
        main(&app).details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("Add a cursor to the diff viewer")
    );
    assert_eq!(main(&app).details_title(), "[5] PR #5 Details");
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    // The table cache is rewritten with it.
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::WriteCache { text, .. } if text.contains("Add a cursor to the diff viewer")
    )));
}

#[test]
fn from_the_overview_the_details_show_the_edit_and_the_table_is_untouched() {
    let mut app = app_with(gh(issues_page()));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["tab", "E"]);
    assert_eq!(app.app().modals(), [ModalKind::PullRequest]);
    // No fetch: the shown pull request is edited.
    assert_eq!(
        app.gh()
            .calls()
            .iter()
            .filter(|call| matches!(call, GithubCall::PullRequest { number: 5, .. }))
            .count(),
        1,
        "only the selection's"
    );
    settle(&mut app);
    type_text(&mut app, "Retitled");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(row_title(&app, 5), "Add a cursor to the viewer");
    assert_eq!(
        main(&app).details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("Retitled")
    );
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
}
