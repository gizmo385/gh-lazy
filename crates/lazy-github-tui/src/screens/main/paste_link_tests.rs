//! Paste link on the main screen: `ctrl+v` and a bracketed paste open it, and what a link points at opens (Python's
//! `_open_link_subject`). Against the `paste-link` scenario's fixtures.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::links::LinkSubject;
use lazy_github_core::models::{FullPullRequest, FullUser, Issue, IssueListItem, Repository};
use serde_json::Value;

use super::*;
use crate::event::Event;
use crate::modal::{ModalKind, PasteLinkDialog};
use crate::test_util::TestApp;
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

fn links_repo(name: &str) -> Repository {
    serde_json::from_value(fixture_body(&format!("links/repos/parity-bot/{name}/GET.json"))).unwrap()
}

fn pr_5() -> FullPullRequest {
    let body = fixture_body("pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
    FullPullRequest::from_api(body, &links_repo("pr-details")).unwrap()
}

fn issue_12() -> Issue {
    let body = fixture_body("issue-details/repos/parity-bot/issue-fixture/issues/12/GET.json");
    match IssueListItem::from_api_item(body, &links_repo("issue-fixture")).unwrap() {
        IssueListItem::Issue(issue) => issue,
        IssueListItem::PullRequest(_) => panic!("issue 12 is an issue"),
    }
}

fn parity_bot() -> FullUser {
    serde_json::from_value(fixture_body("user-profile/users/parity-bot/GET.json")).unwrap()
}

fn parity_bot_repos() -> Vec<Repository> {
    serde_json::from_value(fixture_body(
        "user-profile/users/parity-bot/repos/GET.direction-desc_page-1_per_page-30_sort-updated.json",
    ))
    .unwrap()
}

/// The repos table lists `issue-fixture` only.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![links_repo("issue-fixture")])
        .with_profile("parity-bot", Ok(parity_bot()))
        .with_user_repos("parity-bot", Ok(parity_bot_repos()))
}

async fn app() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), github());
    app.settle().await;
    app
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

fn current_repo(app: &TestApp) -> Option<&str> {
    app.app().current_repo().map(|repo| repo.full_name.as_str())
}

fn table_keys(app: &TestApp) -> Vec<String> {
    main(app).repos().table().row_keys().cloned().collect()
}

fn toasts(app: &TestApp) -> Vec<(Severity, Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.title.clone(), toast.message.to_string()))
        .collect()
}

fn info(message: &str) -> (Severity, Option<String>, String) {
    (Severity::Information, None, message.into())
}

async fn open(app: &mut TestApp, subject: LinkSubject) {
    app.dispatch(Action::Main(MainAction::OpenLinkSubject(Box::new(subject))));
    app.settle().await;
}

#[tokio::test]
async fn a_pull_request_selects_its_repo_and_shows_with_the_tabs_focused() {
    let mut app = app().await;
    open(&mut app, LinkSubject::PullRequest(Box::new(pr_5()))).await;
    assert_eq!(current_repo(&app), Some("parity-bot/pr-details"));
    assert_eq!(main(&app).details_title(), "[5] PR #5 Details");
    assert_eq!(main(&app).details_pull_request().map(FullPullRequest::number), Some(5));
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    assert_eq!(toasts(&app), [info("Opening pull request for link")]);
    assert_eq!(table_keys(&app), ["parity-bot/issue-fixture"]);
}

#[tokio::test]
async fn an_issue_selects_its_repo_and_shows_with_the_tabs_focused() {
    let mut app = app().await;
    open(&mut app, LinkSubject::Issue(Box::new(issue_12()))).await;
    assert_eq!(current_repo(&app), Some("parity-bot/issue-fixture"));
    assert_eq!(main(&app).details_title(), "[5] Issue #12 Details");
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(12));
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    assert_eq!(toasts(&app), [info("Opening issue for link")]);
    // The repo was in the table: its cursor stays where it was.
    assert_eq!(table_keys(&app), ["parity-bot/issue-fixture"]);
    assert_eq!(main(&app).repos().table().cursor(), 0);
}

#[tokio::test]
async fn a_repository_is_selected_and_focus_stays() {
    let mut app = app().await;
    open(&mut app, LinkSubject::Repository(Box::new(links_repo("pr-details")))).await;
    assert_eq!(current_repo(&app), Some("parity-bot/pr-details"));
    assert_eq!(
        main(&app).details_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    assert_eq!(main(&app).focus(), FocusTarget::Table(SectionId::Repos));
    assert_eq!(toasts(&app), [info("Opening repository for link")]);
    assert_eq!(table_keys(&app), ["parity-bot/issue-fixture"]);
}

#[tokio::test]
async fn a_user_opens_the_profile_and_a_picked_repo_is_selected_not_added() {
    let mut app = app().await;
    open(&mut app, LinkSubject::User(Box::new(parity_bot()))).await;
    assert!(app.gh().calls().contains(&GithubCall::User("parity-bot".into())));
    assert!(app.gh().calls().contains(&GithubCall::UserRepos("parity-bot".into())));
    assert_eq!(app.app().modals(), [ModalKind::UserProfile]);
    assert!(toasts(&app).is_empty());
    app.press(&["tab", "enter"]);
    app.settle().await;
    let picked = parity_bot_repos()[0].full_name.clone();
    assert_eq!(current_repo(&app), Some(picked.as_str()));
    assert_eq!(table_keys(&app), ["parity-bot/issue-fixture"]);
}

#[tokio::test]
async fn ctrl_v_opens_the_dialog_empty() {
    let mut app = app().await;
    app.dispatch(Action::Copy("https://github.com/parity-bot".into()));
    app.press(&["ctrl+v"]);
    assert_eq!(app.app().modals(), [ModalKind::PasteLink]);
    assert_eq!(app.app().top_dialog::<PasteLinkDialog>().unwrap().text(), "");
}

#[tokio::test]
async fn ctrl_v_in_a_search_box_pastes_there() {
    let mut app = app().await;
    app.dispatch(Action::Copy("fixture".into()));
    app.press(&["slash", "ctrl+v"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(main(&app).repos().table().search_text(), "fixture");
}

#[tokio::test]
async fn a_bracketed_paste_opens_the_dialog_holding_it() {
    let mut app = app().await;
    app.send(Event::Paste("not a link\nmore".into()));
    assert_eq!(app.app().modals(), [ModalKind::PasteLink]);
    let dialog = app.app().top_dialog::<PasteLinkDialog>().unwrap();
    assert_eq!(dialog.text(), "not a link");
    assert!(dialog.is_invalid(), "checked at once");
    assert!(!dialog.is_pending(), "nothing submitted");
    // Typing replaces the selected text.
    app.press(&["x"]);
    assert_eq!(app.app().top_dialog::<PasteLinkDialog>().unwrap().text(), "x");
}

#[tokio::test]
async fn a_bracketed_paste_with_a_search_box_focused_goes_there() {
    let mut app = app().await;
    app.press(&["slash"]);
    app.send(Event::Paste("fixture".into()));
    assert!(app.app().modals().is_empty());
    assert_eq!(main(&app).repos().table().search_text(), "fixture");
}

#[tokio::test]
async fn a_bracketed_paste_with_a_dialog_open_goes_to_the_dialog() {
    let mut app = app().await;
    app.press(&["ctrl+v"]);
    app.send(Event::Paste("github.com/parity-bot".into()));
    assert_eq!(app.app().modals(), [ModalKind::PasteLink]);
    assert_eq!(
        app.app().top_dialog::<PasteLinkDialog>().unwrap().text(),
        "github.com/parity-bot"
    );
}
