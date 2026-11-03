//! The shared "select the repo, then open the item" path (`MainAction::OpenInRepo`, #405), for a repo known only by
//! name, as #394's search hits carry it. A notification's known repo is covered by `notification_open_tests`.

use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::RepoName;

use super::notification_open_tests::{
    config, dotfiles_repo, github, main, pr_details_repo, request_id, server_error, toasts,
};
use super::*;
use crate::effect::Effect;
use crate::test_util::TestApp;
use crate::toast::Severity;

fn named(full_name: &str) -> RepoRef {
    RepoRef::Named(RepoName::from_remote_url(&format!("https://github.com/{full_name}")).unwrap())
}

fn search_hit(repo: RepoRef, item: RepoItem) -> Action {
    Action::Main(MainAction::OpenInRepo(Box::new(OpenInRepo {
        repo,
        item,
        origin: OpenOrigin::SearchHit,
    })))
}

/// The app with `repo` selected, against `gh`.
async fn selected(gh: FakeGithub, repo: Repository) -> TestApp {
    let mut app = TestApp::with_github(config(false), gh);
    app.settle().await;
    app.dispatch(Action::SelectRepo(Box::new(repo)));
    app.settle().await;
    app
}

fn current(app: &TestApp) -> Option<&str> {
    app.app().current_repo().map(|repo| repo.full_name.as_str())
}

fn repo_lookups(app: &TestApp) -> usize {
    app.gh()
        .calls()
        .iter()
        .filter(|call| matches!(call, GithubCall::Repository(_)))
        .count()
}

#[tokio::test]
async fn the_selected_repo_by_name_is_not_looked_up_but_is_selected_again() {
    let mut app = selected(github(), pr_details_repo()).await;
    let readme = GithubCall::RepoReadme("parity-bot/pr-details".to_owned());
    let readmes = |app: &TestApp| app.gh().calls().iter().filter(|call| **call == readme).count();
    let before = readmes(&app);
    // Names are compared ignoring case.
    app.dispatch(search_hit(named("Parity-Bot/PR-Details"), RepoItem::Issue(7)));
    app.settle().await;
    assert_eq!(repo_lookups(&app), 0);
    assert_eq!(readmes(&app), before + 1);
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(7));
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    // A search hit toasts nothing when it opens.
    assert_eq!(toasts(&app), []);
}

#[tokio::test]
async fn another_repo_by_name_is_looked_up_then_selected_then_its_item_requested() {
    let gh = github().with_repository("parity-bot/pr-details", Ok(Some(pr_details_repo())));
    let mut app = selected(gh, dotfiles_repo()).await;
    let before = app.effects().len();
    app.dispatch(search_hit(named("parity-bot/pr-details"), RepoItem::Issue(7)));
    // Nothing changes until the lookup answers.
    assert_eq!(current(&app), Some("parity-bot/dotfiles"));
    app.settle().await;
    assert_eq!(current(&app), Some("parity-bot/pr-details"));
    let requests: Vec<&GithubRequest> = app.effects()[before..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    let at = |test: fn(&GithubRequest) -> bool| requests.iter().position(|r| test(r)).unwrap();
    let lookup = at(|r| matches!(r, GithubRequest::Repository(name) if name == "parity-bot/pr-details"));
    let readme = at(|r| matches!(r, GithubRequest::RepoReadme(name) if name == "parity-bot/pr-details"));
    let issue = at(|r| matches!(r, GithubRequest::Issue { number: 7, .. }));
    assert!(lookup < readme && readme < issue, "{requests:?}");
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(7));
    assert_eq!(main(&app).details_title(), "[5] Issue #7 Details");
}

#[tokio::test]
async fn a_pull_request_by_name_opens_without_a_toast() {
    let gh = github().with_repository("parity-bot/pr-details", Ok(Some(pr_details_repo())));
    let mut app = selected(gh, dotfiles_repo()).await;
    app.dispatch(search_hit(named("parity-bot/pr-details"), RepoItem::PullRequest(5)));
    app.settle().await;
    assert_eq!(current(&app), Some("parity-bot/pr-details"));
    assert_eq!(main(&app).details_pull_request().map(FullPullRequest::number), Some(5));
    assert_eq!(toasts(&app), []);
}

#[rstest::rstest]
#[case::missing(Ok(None))]
#[case::failed(Err(server_error()))]
#[tokio::test]
async fn a_repo_lookup_that_finds_nothing_changes_nothing(
    #[case] lookup: Result<Option<Repository>, lazy_github_core::gh::GhError>,
) {
    let gh = github().with_repository("parity-bot/missing", lookup);
    let mut app = selected(gh, dotfiles_repo()).await;
    let title = main(&app).details_title().to_owned();
    app.dispatch(search_hit(named("parity-bot/missing"), RepoItem::Issue(3)));
    app.settle().await;
    assert_eq!(current(&app), Some("parity-bot/dotfiles"));
    assert_eq!(main(&app).details_title(), title);
    assert!(!app.gh().calls().contains(&GithubCall::Issue(3)));
    assert_eq!(
        toasts(&app),
        [(Severity::Error, "Could not open parity-bot/missing#3".to_owned())]
    );
}

#[tokio::test]
async fn a_hit_whose_item_fails_keeps_the_repo_with_an_error() {
    let gh = github().with_issue("parity-bot/pr-details", 7, Err(server_error()));
    let mut app = selected(gh, pr_details_repo()).await;
    app.dispatch(search_hit(named("parity-bot/pr-details"), RepoItem::Issue(7)));
    app.settle().await;
    assert_eq!(current(&app), Some("parity-bot/pr-details"));
    assert_eq!(main(&app).details_issue(), None);
    assert_eq!(
        toasts(&app),
        [(Severity::Error, "Could not open parity-bot/pr-details#7".to_owned())]
    );
}

#[tokio::test]
async fn a_selection_while_the_repo_lookup_is_pending_cancels_it() {
    let gh = github().with_repository("parity-bot/pr-details", Ok(Some(pr_details_repo())));
    let mut app = selected(gh, dotfiles_repo()).await;
    app.dispatch(search_hit(named("parity-bot/pr-details"), RepoItem::Issue(7)));
    let id = request_id(&app, |r| matches!(r, GithubRequest::Repository(_)));
    // Before the lookup answers, another repo is selected.
    app.dispatch(Action::SelectRepo(Box::new(dotfiles_repo())));
    assert!(app.effects().contains(&Effect::Cancel(id)));
    app.settle().await;
    assert_eq!(current(&app), Some("parity-bot/dotfiles"));
    assert!(!app.gh().calls().contains(&GithubCall::Issue(7)));
    assert_eq!(main(&app).details_issue(), None);
    assert_eq!(toasts(&app), []);
}

#[test]
fn a_notification_opens_its_subject_in_its_repository() {
    let notification = |url: Option<&str>| {
        let mut notification: lazy_github_core::models::Notification = serde_json::from_value(serde_json::json!({
            "id": "1",
            "unread": true,
            "reason": "mention",
            "updated_at": "2026-10-24T22:00:00Z",
            "last_read_at": null,
            "subject": {"title": "t", "url": null, "latest_comment_url": null, "type": "Issue"},
            "repository": pr_details_repo(),
        }))
        .unwrap();
        notification.subject.url = url.map(str::to_owned);
        OpenInRepo::notification(&notification)
    };
    let opened = notification(Some("https://api.github.com/repos/someone/else/issues/7"));
    assert_eq!(opened.repo, RepoRef::Known(Box::new(pr_details_repo())));
    assert_eq!(opened.item, RepoItem::Issue(7));
    assert_eq!(opened.origin, OpenOrigin::Notification);
    let pull = notification(Some("https://api.github.com/repos/o/r/pulls/5"));
    assert_eq!(pull.item, RepoItem::PullRequest(5));
    assert_eq!(notification(None).item, RepoItem::Repo);
    let release = notification(Some("https://api.github.com/repos/o/r/releases/1"));
    assert_eq!(release.item, RepoItem::Repo);
}
