use std::path::Path;

use rstest::rstest;
use serde_json::Value;

use super::*;
use crate::github::{FakeGithub, GithubCall};

#[allow(clippy::unnecessary_wraps, reason = "the cases compare with `Option`s")]
fn pr(owner: &str, repo: &str, number: u64) -> Option<GithubLink> {
    Some(GithubLink::PullRequest {
        owner: owner.into(),
        repo: repo.into(),
        number,
    })
}

#[allow(clippy::unnecessary_wraps, reason = "the cases compare with `Option`s")]
fn issue(owner: &str, repo: &str, number: u64) -> Option<GithubLink> {
    Some(GithubLink::Issue {
        owner: owner.into(),
        repo: repo.into(),
        number,
    })
}

#[allow(clippy::unnecessary_wraps, reason = "the cases compare with `Option`s")]
fn repository(owner: &str, repo: &str) -> Option<GithubLink> {
    Some(GithubLink::Repository {
        owner: owner.into(),
        repo: repo.into(),
    })
}

#[allow(clippy::unnecessary_wraps, reason = "the cases compare with `Option`s")]
fn user(login: &str) -> Option<GithubLink> {
    Some(GithubLink::User { login: login.into() })
}

#[rstest]
#[case::user("https://github.com/parity-bot", user("parity-bot"))]
#[case::user_without_scheme("github.com/parity-bot", user("parity-bot"))]
#[case::user_trailing_slash("https://github.com/parity-bot/", user("parity-bot"))]
#[case::user_one_letter("github.com/a", user("a"))]
#[case::user_inner_hyphens("github.com/a-b-c", user("a-b-c"))]
#[case::user_on_another_host("example.com/user", None)]
#[case::user_leading_hyphen("github.com/-x", None)]
#[case::user_trailing_hyphen("github.com/x-", None)]
#[case::pull_request("https://github.com/o/r/pull/5", pr("o", "r", 5))]
#[case::pull_request_without_scheme("github.com/o/r/pull/5", pr("o", "r", 5))]
#[case::pull_request_files("https://github.com/o/r/pull/5/files", pr("o", "r", 5))]
#[case::pull_request_digits_then_text("https://github.com/o/r/pull/5abc", pr("o", "r", 5))]
#[case::pull_request_too_long("github.com/o/r/pull/99999999999999999999999", pr("o", "r", u64::MAX))]
#[case::issue("https://github.com/o/r/issues/12", issue("o", "r", 12))]
#[case::issue_without_scheme("github.com/o/r/issues/12", issue("o", "r", 12))]
#[case::issue_comment("https://github.com/o/r/issues/5#issuecomment-1", issue("o", "r", 5))]
#[case::repository("https://github.com/o/r", repository("o", "r"))]
#[case::repository_trailing_slash("https://github.com/o/r/", repository("o", "r"))]
#[case::pulls_is_the_repository("https://github.com/o/r/pulls", repository("o", "r"))]
#[case::pull_without_number("https://github.com/o/r/pull/abc", repository("o", "r"))]
#[case::actions_run_is_the_repository("https://github.com/o/r/actions/runs/1", repository("o", "r"))]
#[case::query_stays_in_the_repo("https://github.com/o/r?tab=readme", repository("o", "r?tab=readme"))]
#[case::http_is_not_a_link("http://github.com/o/r", None)]
#[case::http_pull_request("http://github.com/o/r/pull/5", None)]
#[case::other_host_pull_request("https://gitlab.com/o/r/pull/5", pr("o", "r", 5))]
#[case::empty("", None)]
#[case::owner_only("https://example.com/o/", None)]
#[case::two_segments("o/r", None)]
#[case::empty_owner("https://github.com//r", None)]
fn parses_as_python_does(#[case] text: &str, #[case] expected: Option<GithubLink>) {
    assert_eq!(parse_github_url(text, true), expected);
    assert_eq!(looks_like_github_url(text), expected.is_some());
}

#[rstest]
#[case::repository("https://github.com/o/r", None)]
#[case::pulls("https://github.com/o/r/pulls", None)]
#[case::pull_request("https://github.com/o/r/pull/5", pr("o", "r", 5))]
#[case::issue("https://github.com/o/r/issues/5", issue("o", "r", 5))]
#[case::user("https://github.com/o", user("o"))]
fn without_repositories_only_repository_links_drop(#[case] text: &str, #[case] expected: Option<GithubLink>) {
    assert_eq!(parse_github_url(text, false), expected);
}

/// The response body of the parity fixture at `path`.
fn fixture_body(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn fixture_repo(name: &str) -> Repository {
    serde_json::from_value(fixture_body(&format!("links/repos/parity-bot/{name}/GET.json"))).unwrap()
}

fn fixture_pr() -> FullPullRequest {
    let body = fixture_body("pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
    FullPullRequest::from_api(body, &fixture_repo("pr-details")).unwrap()
}

fn fixture_issue() -> IssueListItem {
    let body = fixture_body("issue-details/repos/parity-bot/issue-fixture/issues/12/GET.json");
    IssueListItem::from_api_item(body, &fixture_repo("issue-fixture")).unwrap()
}

/// Pull request 5 from the issues endpoint.
fn fixture_pr_as_issue() -> IssueListItem {
    let body = fixture_body("paste-link/repos/parity-bot/pr-details/issues/5/GET.json");
    IssueListItem::from_api_item(body, &fixture_repo("pr-details")).unwrap()
}

fn fixture_user() -> FullUser {
    serde_json::from_value(fixture_body("user-profile/users/parity-bot/GET.json")).unwrap()
}

fn http(status: u16) -> GhError {
    GhError::Http {
        status,
        message: "error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// GitHub as the `paste-link` scenario's fixtures answer, plus a repo that fails with a 500.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_profile("parity-bot", Ok(fixture_user()))
        .with_repository("parity-bot/pr-details", Ok(Some(fixture_repo("pr-details"))))
        .with_repository("parity-bot/issue-fixture", Ok(Some(fixture_repo("issue-fixture"))))
        .with_repository("parity-bot/missing", Ok(None))
        .with_repository("parity-bot/broken", Err(http(500)))
        .with_pull_request("parity-bot/pr-details", 5, Ok(fixture_pr()))
        .with_issue("parity-bot/pr-details", 5, Ok(fixture_pr_as_issue()))
        .with_issue("parity-bot/issue-fixture", 12, Ok(fixture_issue()))
}

async fn resolve(gh: &FakeGithub, text: &str) -> Result<Option<LinkSubject>, GhError> {
    let link = parse_github_url(text, true).unwrap();
    resolve_github_link(gh, &link).await
}

#[tokio::test]
async fn a_user_link_fetches_the_profile() {
    let gh = github();
    let found = resolve(&gh, "https://github.com/parity-bot").await.unwrap();
    assert_eq!(found, Some(LinkSubject::User(Box::new(fixture_user()))));
    assert_eq!(gh.calls(), [GithubCall::User("parity-bot".into())]);
    let missing = resolve(&gh, "https://github.com/nobody").await;
    assert!(matches!(missing, Err(GhError::Http { status: 404, .. })), "{missing:?}");
}

#[tokio::test]
async fn a_repository_link_fetches_the_repository() {
    let gh = github();
    let found = resolve(&gh, "https://github.com/parity-bot/pr-details").await.unwrap();
    assert_eq!(
        found,
        Some(LinkSubject::Repository(Box::new(fixture_repo("pr-details"))))
    );
    assert_eq!(gh.calls(), [GithubCall::Repository("parity-bot/pr-details".into())]);
    assert_eq!(
        resolve(&gh, "https://github.com/parity-bot/missing").await.unwrap(),
        None
    );
    let broken = resolve(&gh, "https://github.com/parity-bot/broken").await;
    assert!(matches!(broken, Err(GhError::Http { status: 500, .. })), "{broken:?}");
}

#[tokio::test]
async fn a_pull_request_link_fetches_the_repository_then_the_pull_request() {
    let gh = github();
    let found = resolve(&gh, "https://github.com/parity-bot/pr-details/pull/5")
        .await
        .unwrap();
    assert_eq!(found, Some(LinkSubject::PullRequest(Box::new(fixture_pr()))));
    assert_eq!(
        gh.calls(),
        [
            GithubCall::Repository("parity-bot/pr-details".into()),
            GithubCall::PullRequest {
                full_name: "parity-bot/pr-details".into(),
                number: 5
            },
        ]
    );
    // The repository is there, the pull request isn't.
    let missing = resolve(&gh, "https://github.com/parity-bot/pr-details/pull/6").await;
    assert!(matches!(missing, Err(GhError::Http { status: 404, .. })), "{missing:?}");
    // No repository, no pull request request.
    let gh = github();
    assert_eq!(
        resolve(&gh, "github.com/parity-bot/missing/pull/1").await.unwrap(),
        None
    );
    assert_eq!(gh.calls(), [GithubCall::Repository("parity-bot/missing".into())]);
}

#[tokio::test]
async fn an_issue_link_fetches_the_issue() {
    let gh = github();
    let found = resolve(&gh, "https://github.com/parity-bot/issue-fixture/issues/12")
        .await
        .unwrap();
    let IssueListItem::Issue(expected) = fixture_issue() else {
        panic!("issue 12 is an issue");
    };
    assert_eq!(found, Some(LinkSubject::Issue(Box::new(expected))));
    assert_eq!(
        gh.calls(),
        [
            GithubCall::Repository("parity-bot/issue-fixture".into()),
            GithubCall::Issue(12),
        ]
    );
}

#[tokio::test]
async fn an_issue_link_to_a_pull_request_fetches_the_pull_request() {
    let gh = github();
    let found = resolve(&gh, "https://github.com/parity-bot/pr-details/issues/5#issuecomment-1")
        .await
        .unwrap();
    assert_eq!(found, Some(LinkSubject::PullRequest(Box::new(fixture_pr()))));
    assert_eq!(
        gh.calls(),
        [
            GithubCall::Repository("parity-bot/pr-details".into()),
            GithubCall::Issue(5),
            GithubCall::PullRequest {
                full_name: "parity-bot/pr-details".into(),
                number: 5
            },
        ]
    );
}
