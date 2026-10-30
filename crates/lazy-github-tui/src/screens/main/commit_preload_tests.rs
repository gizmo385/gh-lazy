//! Preloading the current commit's pull request when a repo loads (#198).

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullPullRequest, IssueListItem, PartialPullRequest, RepoName, Repository};
use serde_json::Value;

use super::{FocusTarget, MainScreen, SectionId};
use crate::test_util::TestApp;

const REPO: &str = "parity-bot/pr-details";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn fixture_body(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details")
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// PRs 5, 4, 3 and 2, as the issues endpoint lists them.
fn partial_prs() -> Vec<PartialPullRequest> {
    let repo = fixture_repo();
    fixture_body(&format!("repos/{REPO}/issues/GET.page-1_per_page-30_state-all.json"))
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| {
            IssueListItem::from_api_item(item.clone(), &repo)
                .unwrap()
                .into_pull_request()
        })
        .collect()
}

fn partial(number: u64) -> PartialPullRequest {
    partial_prs().into_iter().find(|pr| pr.issue.number == number).unwrap()
}

fn full(number: u64) -> FullPullRequest {
    FullPullRequest::from_api(
        fixture_body(&format!("repos/{REPO}/pulls/{number}/GET.json")),
        &fixture_repo(),
    )
    .unwrap()
}

/// The setting on; it's off by default.
fn preloading() -> Config {
    let mut config = Config::default();
    config.pull_requests.preload_pull_request_for_current_commit = true;
    config
}

fn gh(for_commit: Result<Vec<PartialPullRequest>, GhError>) -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_pull_request(REPO, 3, Ok(full(3)))
        .with_pull_requests_for_commit(REPO, SHA, for_commit)
}

/// The app launched in a clone of the repo on `branch` at [`SHA`], past the auth screen with every request answered:
/// the repo is selected as the current-directory repo.
fn launched(config: Config, gh: FakeGithub, branch: Option<&str>, sha: Option<&str>) -> TestApp {
    let mut app = TestApp::with_github(config, gh)
        .with_local_branch(branch)
        .with_local_commit(sha)
        .with_local_repo(RepoName::from_remote_url(&format!("git@github.com:{REPO}.git")));
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
    app
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

fn lookups(app: &TestApp) -> Vec<GithubCall> {
    app.gh()
        .calls()
        .into_iter()
        .filter(|call| matches!(call, GithubCall::PullRequestsForCommit { .. }))
        .collect()
}

#[test]
fn one_pull_request_for_the_commit_is_shown_without_moving_focus() {
    let app = launched(preloading(), gh(Ok(vec![partial(3)])), Some("new-feature"), Some(SHA));
    assert_eq!(app.app().current_repo().map(|repo| repo.full_name.as_str()), Some(REPO));
    assert_eq!(
        lookups(&app),
        [GithubCall::PullRequestsForCommit {
            full_name: REPO.to_owned(),
            sha: SHA.to_owned(),
        }]
    );
    assert_eq!(main(&app).details_pull_request().map(|pr| pr.pr.issue.number), Some(3));
    assert_eq!(main(&app).focus(), FocusTarget::Table(SectionId::Repos));
}

#[test]
fn nothing_is_looked_up_on_the_default_branch() {
    let app = launched(preloading(), gh(Ok(vec![partial(3)])), Some("main"), Some(SHA));
    assert_eq!(app.app().current_repo().map(|repo| repo.full_name.as_str()), Some(REPO));
    assert_eq!(lookups(&app), []);
    assert_eq!(main(&app).details_pull_request(), None);
}

#[test]
fn nothing_is_looked_up_with_the_setting_off() {
    let app = launched(
        Config::default(),
        gh(Ok(vec![partial(3)])),
        Some("new-feature"),
        Some(SHA),
    );
    assert_eq!(lookups(&app), []);
}

#[test]
fn nothing_is_looked_up_without_a_commit() {
    // Not a git repo: no branch (which isn't the default either) and no commit.
    let app = launched(preloading(), gh(Ok(vec![partial(3)])), None, None);
    assert_eq!(lookups(&app), []);
}

#[test]
fn nothing_is_looked_up_with_the_pr_and_issue_sections_hidden() {
    let mut config = preloading();
    config.appearance.show_pull_requests = false;
    config.appearance.show_issues = false;
    let app = launched(config, gh(Ok(vec![partial(3)])), Some("new-feature"), Some(SHA));
    assert_eq!(lookups(&app), []);
}

#[test]
fn several_or_no_matches_show_nothing() {
    for prs in [vec![partial(3), partial(4)], Vec::new()] {
        let app = launched(preloading(), gh(Ok(prs)), Some("new-feature"), Some(SHA));
        assert_eq!(lookups(&app).len(), 1);
        assert_eq!(main(&app).details_pull_request(), None);
    }
}

#[test]
fn a_failed_lookup_shows_nothing_and_no_toast() {
    let error = GhError::Http {
        status: 404,
        message: "Not Found".to_owned(),
        body: Vec::new(),
    };
    let app = launched(preloading(), gh(Err(error)), Some("new-feature"), Some(SHA));
    assert_eq!(lookups(&app).len(), 1);
    assert_eq!(main(&app).details_pull_request(), None);
    assert!(app.app().toasts().is_empty());
}
