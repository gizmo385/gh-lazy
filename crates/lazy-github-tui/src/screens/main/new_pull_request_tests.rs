//! `pull_request.new` on the main screen: the created pull request added to the PR table and opened. The dialog
//! itself is tested in `modal::pull_request`.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{FullPullRequest, IssueListItem, Repository};
use serde_json::Value;

use super::{FocusTarget, MainAction, MainScreen};
use crate::action::Action;
use crate::effect::Effect;
use crate::test_util::TestApp;

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

/// PR 10, as Create PR closes with it.
fn created() -> FullPullRequest {
    let body = fixture_body("pr-create", &format!("repos/{REPO}/pulls/10/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

/// PRs 5, 4, 3 and 2.
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

fn app(config: Config) -> TestApp {
    let gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()));
    let mut app = TestApp::with_github(config, gh);
    app.press(&["enter"]);
    settle(&mut app);
    app
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

#[test]
fn the_created_pull_request_is_sorted_in_widened_cached_and_opened() {
    let mut app = app(Config::default());
    app.dispatch(Action::Main(MainAction::PullRequestCreated(Box::new(created()))));
    settle(&mut app);
    let rows: Vec<u64> = main(&app).issue_lists.prs_table().row_keys().copied().collect();
    assert_eq!(rows, [10, 5, 4, 3, 2]);
    // The number column widens to fit 10 (Py bug B73's recommendation: Python cuts it to "1").
    let screen = app.screen(160, 48);
    assert!(screen.contains("│ 10  open    parity-bot     Parity PR"), "{screen}");
    assert!(screen.contains("│ 5   open    parity-friend"), "{screen}");
    assert_eq!(main(&app).details_title(), "[5] PR #10 Details");
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    assert_eq!(main(&app).details_pull_request().map(FullPullRequest::number), Some(10));
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::WriteCache { text, .. } if text.contains("Parity PR")
    )));
}

#[test]
fn with_the_pull_requests_hidden_it_still_opens() {
    let mut config = Config::default();
    config.appearance.show_pull_requests = false;
    let mut app = app(config);
    app.dispatch(Action::Main(MainAction::PullRequestCreated(Box::new(created()))));
    settle(&mut app);
    assert_eq!(main(&app).details_title(), "[5] PR #10 Details");
}
