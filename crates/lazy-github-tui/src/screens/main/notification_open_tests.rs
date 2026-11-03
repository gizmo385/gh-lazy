//! Opening a notification chosen in the Notifications modal: the count refreshes, its repo is selected, and a pull
//! request or issue subject opens in the details. Against the `notifications-modal`, `notifications-open-issue` and
//! `pr-details` fixtures.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall, NotificationQuery};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Notification, Repository};
use serde_json::Value;

use super::*;
use crate::effect::Effect;
use crate::modal::ModalKind;
use crate::test_util::{LogCapture, TestApp};
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

fn notifications(file: &str) -> Vec<Notification> {
    serde_json::from_value(fixture_body(&format!("notifications-modal/notifications/{file}"))).unwrap()
}

/// `parity-bot/pr-details`, as its repos page lists it.
pub(super) fn pr_details_repo() -> Repository {
    let page = fixture_body("pr-details/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// `parity-bot/dotfiles`, as 9100004 names it.
pub(super) fn dotfiles_repo() -> Repository {
    notifications("GET.all-true_page-1_per_page-50.json")
        .into_iter()
        .find(|n| n.id == 9_100_004)
        .unwrap()
        .repository
}

fn pr(number: u64) -> FullPullRequest {
    let body = fixture_body(&format!(
        "pr-details/repos/parity-bot/pr-details/pulls/{number}/GET.json"
    ));
    FullPullRequest::from_api(body, &pr_details_repo()).unwrap()
}

/// Issue 7 of `parity-bot/pr-details`, as the `notifications-open-issue` overlay answers it.
pub(super) fn issue_7() -> IssueListItem {
    let body = fixture_body("notifications-open-issue/repos/parity-bot/pr-details/issues/7/GET.json");
    IssueListItem::from_api_item(body, &pr_details_repo()).unwrap()
}

/// Number 5 of `parity-bot/pr-details` from the issues endpoint: a pull request.
fn issue_5() -> IssueListItem {
    let body = fixture_body("paste-link/repos/parity-bot/pr-details/issues/5/GET.json");
    IssueListItem::from_api_item(body, &pr_details_repo()).unwrap()
}

pub(super) fn server_error() -> lazy_github_core::gh::GhError {
    lazy_github_core::gh::GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// GitHub as the `notifications-open` and `notifications-open-issue` scenarios' fixtures answer: the six threads,
/// every mark succeeding, PR 5, a failing PR 2, issue 7, and number 5 from the issues endpoint.
pub(super) fn github() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![dotfiles_repo(), pr_details_repo()])
        .with_unread_count(Ok(4))
        .with_notification_page(
            NotificationQuery { all: true, page: 1 },
            Ok(notifications("GET.all-true_page-1_per_page-50.json")),
        )
        .with_notification_page(
            NotificationQuery { all: false, page: 1 },
            Ok(notifications("GET.all-false_page-1_per_page-50.json")),
        )
        .with_notification_read(9_100_001, Ok(()))
        .with_notification_read(9_100_002, Ok(()))
        .with_notification_read(9_100_004, Ok(()))
        .with_pull_request("parity-bot/pr-details", 5, Ok(pr(5)))
        .with_pull_request("parity-bot/pr-details", 2, Err(server_error()))
        .with_issue("parity-bot/pr-details", 7, Ok(issue_7()))
        .with_issue("parity-bot/pr-details", 5, Ok(issue_5()))
}

pub(super) fn config(mark: bool) -> Config {
    let mut config = Config::default();
    config.notifications.enabled = true;
    config.notifications.mark_notification_as_read_when_selected = mark;
    config
}

/// The modal open and loaded over the main screen.
async fn opened(mark: bool) -> TestApp {
    let mut app = TestApp::with_github(config(mark), github());
    app.settle().await;
    open_modal(&mut app).await;
    app
}

async fn open_modal(app: &mut TestApp) {
    app.press(&["ctrl+n"]);
    app.settle().await;
    assert_eq!(app.app().modals(), [ModalKind::Notifications]);
}

/// Opens the modal, chooses the row `downs` below the first on the tab `tab_key` shows (`u` or `r`), and settles.
async fn choose(tab_key: &str, downs: usize) -> TestApp {
    choose_with(github(), tab_key, downs).await
}

/// [`choose`], against `gh`.
async fn choose_with(gh: FakeGithub, tab_key: &str, downs: usize) -> TestApp {
    let mut app = TestApp::with_github(config(true), gh);
    app.settle().await;
    choose_in(&mut app, tab_key, downs).await;
    app
}

/// [`choose`] in a running app.
async fn choose_in(app: &mut TestApp, tab_key: &str, downs: usize) {
    open_modal(app).await;
    app.press(&[tab_key]);
    for _ in 0..downs {
        app.press(&["j"]);
    }
    app.press(&["enter"]);
    app.settle().await;
}

pub(super) fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

pub(super) fn toasts(app: &TestApp) -> Vec<(Severity, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.message.to_string()))
        .collect()
}

/// The calls that matter here, in order: marks, the auth check each count refresh starts with, pull requests and
/// the readme each selection loads.
fn calls(app: &TestApp) -> Vec<GithubCall> {
    app.gh()
        .calls()
        .into_iter()
        .filter(|call| {
            matches!(
                call,
                GithubCall::MarkNotificationRead(_)
                    | GithubCall::CheckAuth
                    | GithubCall::PullRequest { .. }
                    | GithubCall::Issue(_)
                    | GithubCall::RepoReadme(_)
            )
        })
        .collect()
}

fn pull_request_call(number: u64) -> GithubCall {
    GithubCall::PullRequest {
        full_name: "parity-bot/pr-details".to_owned(),
        number,
    }
}

const OPENING_PR: (Severity, &str) = (Severity::Information, "Opening pull request for notification");
const OPENING_REPO: (Severity, &str) = (Severity::Information, "Opening repository for notification");
const OPENING_ISSUE: (Severity, &str) = (Severity::Information, "Opening issue for notification");

fn owned(toasts: &[(Severity, &str)]) -> Vec<(Severity, String)> {
    toasts
        .iter()
        .map(|&(severity, message)| (severity, message.to_owned()))
        .collect()
}

#[tokio::test]
async fn a_pull_request_notification_opens_the_pull_request() {
    let app = choose("u", 0).await;
    assert!(app.app().modals().is_empty());
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    let screen = main(&app);
    assert_eq!(screen.details_pull_request().map(FullPullRequest::number), Some(5));
    assert_eq!(screen.details_title(), "[5] PR #5 Details");
    assert_eq!(screen.details_tabs(), ["Overview", "Diff", "Conversation"]);
    assert_eq!(screen.focus(), FocusTarget::DetailsTabs);
    assert_eq!(toasts(&app), owned(&[OPENING_PR]));
    // The mark, then the count refresh, then the repo, then the pull request: from the notification's repository.
    let calls = calls(&app);
    let at = |call: &GithubCall| calls.iter().position(|c| c == call).unwrap();
    let mark = at(&GithubCall::MarkNotificationRead(9_100_001));
    let count = mark
        + 1
        + calls[mark + 1..]
            .iter()
            .position(|c| *c == GithubCall::CheckAuth)
            .unwrap();
    let readme = at(&GithubCall::RepoReadme("parity-bot/pr-details".to_owned()));
    let pull = at(&pull_request_call(5));
    assert!(mark < count && count < readme && readme < pull, "{calls:?}");
}

#[tokio::test]
async fn an_issue_notification_selects_its_repo_then_opens_the_issue() {
    let app = choose("u", 1).await;
    assert!(app.gh().calls().contains(&GithubCall::MarkNotificationRead(9_100_002)));
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    let screen = main(&app);
    assert_eq!(screen.details_pull_request(), None);
    assert_eq!(screen.details_issue().map(|issue| issue.number), Some(7));
    assert_eq!(screen.details_title(), "[5] Issue #7 Details");
    // Python's issue tabs (the spec's "Conversation" is the pull request's).
    assert_eq!(screen.details_tabs(), ["Overview", "Comments"]);
    assert_eq!(screen.focus(), FocusTarget::DetailsTabs);
    assert_eq!(toasts(&app), owned(&[OPENING_ISSUE]));
    // The repo first, then issue 7 from the notification's repository; no pull request.
    let calls = calls(&app);
    let at = |call: &GithubCall| calls.iter().position(|c| c == call).unwrap();
    let readme = at(&GithubCall::RepoReadme("parity-bot/pr-details".to_owned()));
    assert!(readme < at(&GithubCall::Issue(7)), "{calls:?}");
    assert!(!calls.iter().any(|call| matches!(call, GithubCall::PullRequest { .. })));
}

/// 9100002 with its subject URL pointing at `number` from the issues endpoint.
fn issue_notification(number: u64) -> Notification {
    let mut notification = notifications("GET.all-true_page-1_per_page-50.json")
        .into_iter()
        .find(|n| n.id == 9_100_002)
        .unwrap();
    notification.subject.url = Some(format!(
        "https://api.github.com/repos/parity-bot/pr-details/issues/{number}"
    ));
    notification
}

async fn close_with(app: &mut TestApp, notification: Notification) {
    app.dispatch(Action::Main(MainAction::NotificationsClosed(Some(Box::new(
        notification,
    )))));
    app.settle().await;
}

#[tokio::test]
async fn an_issue_number_that_is_a_pull_request_opens_the_pull_request() {
    let mut app = TestApp::with_github(config(true), github());
    app.settle().await;
    close_with(&mut app, issue_notification(5)).await;
    let screen = main(&app);
    assert_eq!(screen.details_pull_request().map(FullPullRequest::number), Some(5));
    assert_eq!(screen.details_title(), "[5] PR #5 Details");
    assert_eq!(screen.focus(), FocusTarget::DetailsTabs);
    assert_eq!(toasts(&app), owned(&[OPENING_PR]));
    let calls = calls(&app);
    let at = |call: &GithubCall| calls.iter().position(|c| c == call).unwrap();
    assert!(at(&GithubCall::Issue(5)) < at(&pull_request_call(5)), "{calls:?}");
}

#[tokio::test]
async fn an_issue_that_fails_opens_only_the_repo_without_an_error() {
    let logs = LogCapture::start();
    let gh = github().with_issue("parity-bot/pr-details", 7, Err(server_error()));
    let app = choose_with(gh, "u", 1).await;
    assert!(app.gh().calls().contains(&GithubCall::Issue(7)));
    let screen = main(&app);
    assert_eq!(screen.details_issue(), None);
    assert_eq!(
        screen.details_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    assert_eq!(toasts(&app), owned(&[OPENING_REPO]));
    let text = logs.text();
    assert!(!text.contains("ERROR") && !text.contains("WARN"), "{text}");
    assert!(text.contains("Server Error"), "the error goes to the debug log: {text}");
}

/// The id of the last `Effect::Github` asking for `request`.
pub(super) fn request_id(app: &TestApp, request: impl Fn(&GithubRequest) -> bool) -> RequestId {
    app.effects()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Github { id, request: r } if request(r) => Some(*id),
            _ => None,
        })
        .unwrap()
}

#[tokio::test]
async fn a_selection_while_the_issue_is_pending_cancels_it() {
    let mut app = opened(false).await;
    app.press(&["j", "enter"]);
    let id = request_id(&app, |r| matches!(r, GithubRequest::Issue { number: 7, .. }));
    // Before the issue answers, a repo is picked in the repos table.
    app.press(&["1", "enter"]);
    assert!(app.effects().contains(&Effect::Cancel(id)));
    app.settle().await;
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/dotfiles")
    );
    assert_eq!(main(&app).details_issue(), None);
    assert_eq!(toasts(&app), []);
}

#[tokio::test]
async fn a_notification_in_the_selected_repo_selects_it_again() {
    let mut app = TestApp::with_github(config(true), github());
    app.settle().await;
    app.dispatch(Action::SelectRepo(Box::new(pr_details_repo())));
    app.settle().await;
    let readme = GithubCall::RepoReadme("parity-bot/pr-details".to_owned());
    let before = app.gh().calls().iter().filter(|call| **call == readme).count();
    choose_in(&mut app, "u", 1).await;
    let after = app.gh().calls().iter().filter(|call| **call == readme).count();
    assert_eq!(after, before + 1, "the repo is loaded again");
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(7));
    assert_eq!(toasts(&app), owned(&[OPENING_ISSUE]));
}

#[tokio::test]
async fn a_repo_the_table_does_not_list_is_not_added() {
    let gh = github().with_repositories(vec![dotfiles_repo()]);
    let mut app = TestApp::with_github(config(true), gh);
    app.settle().await;
    let rows = |app: &TestApp| main(app).repos.table.row_keys().cloned().collect::<Vec<_>>();
    let before = rows(&app);
    let cursor = main(&app).repos.table.cursor();
    choose_in(&mut app, "u", 1).await;
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    assert_eq!(rows(&app), before);
    assert_eq!(main(&app).repos.table.cursor(), cursor);
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(7));
}

#[tokio::test]
async fn an_issue_opened_from_another_repo_renders() {
    let mut app = TestApp::with_github(config(true), github());
    app.settle().await;
    app.dispatch(Action::SelectRepo(Box::new(dotfiles_repo())));
    app.settle().await;
    choose_in(&mut app, "u", 1).await;
    insta::assert_snapshot!("notification_issue_opened", app.screen(160, 48));
}

#[tokio::test]
async fn a_pull_request_that_fails_opens_only_the_repo_without_an_error() {
    let logs = LogCapture::start();
    // 9100005: read, so nothing is marked; PR 2 answers a 500.
    let app = choose("r", 1).await;
    assert!(!app.gh().calls().contains(&GithubCall::MarkNotificationRead(9_100_005)));
    assert!(app.gh().calls().contains(&pull_request_call(2)));
    let screen = main(&app);
    assert_eq!(screen.details_pull_request(), None);
    assert_eq!(
        screen.details_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    assert_eq!(toasts(&app), owned(&[OPENING_REPO]));
    let text = logs.text();
    assert!(!text.contains("ERROR") && !text.contains("WARN"), "{text}");
}

#[tokio::test]
async fn a_subject_without_a_url_opens_only_its_repo() {
    let app = choose("u", 2).await;
    assert!(app.gh().calls().contains(&GithubCall::MarkNotificationRead(9_100_004)));
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/dotfiles")
    );
    assert_eq!(main(&app).details_title(), "[5] parity-bot/dotfiles Details");
    assert_eq!(toasts(&app), owned(&[OPENING_REPO]));
}

#[tokio::test]
async fn closing_dispatches_the_refresh_then_the_repo_then_the_subject() {
    let mut app = opened(false).await;
    let before = app.effects().len();
    app.press(&["enter"]);
    // With the setting off the modal closes at once, and the pull request is asked for after the repo's requests.
    assert!(app.app().modals().is_empty());
    let requests: Vec<&GithubRequest> = app.effects()[before..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    let count = requests
        .iter()
        .position(|r| matches!(r, GithubRequest::CheckAuth))
        .unwrap();
    let readme = requests
        .iter()
        .position(|r| matches!(r, GithubRequest::RepoReadme(_)))
        .unwrap();
    let pull = requests
        .iter()
        .position(|r| matches!(r, GithubRequest::PullRequest { number: 5, .. }))
        .unwrap();
    assert!(count < readme && readme < pull, "{requests:?}");
    assert!(
        !requests
            .iter()
            .any(|r| matches!(r, GithubRequest::MarkNotificationRead(_)))
    );
}

#[tokio::test]
async fn a_selection_before_the_pull_request_answers_drops_it() {
    let mut app = opened(false).await;
    app.press(&["enter"]);
    // Before the pull request answers, a repo is picked in the repos table.
    app.press(&["1", "enter"]);
    app.settle().await;
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/dotfiles")
    );
    assert_eq!(main(&app).details_pull_request(), None);
    assert_eq!(toasts(&app), []);
}

#[tokio::test]
async fn a_click_on_the_cursors_row_chooses_it() {
    let mut app = opened(false).await;
    app.render(160, 48);
    // The first Unread row, under the cursor.
    app.click(30, 13);
    assert!(app.app().modals().is_empty());
    app.settle().await;
    assert_eq!(main(&app).details_pull_request().map(FullPullRequest::number), Some(5));
}
