//! Following a link on the main screen (`Action::FollowLink`, #339): where each kind of link goes, and a click on a
//! link drawn in the last frame. Against the `link-click` scenario's fixtures.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use crossterm::event::MouseEventKind;
use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::links::GithubLink;
use lazy_github_core::models::{FullPullRequest, FullUser, IssueListItem, Repository};
use serde_json::Value;

use super::repos::repo;
use super::*;
use crate::modal::ModalKind;
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

fn issue_item(set: &str, repo: &str, number: u64) -> IssueListItem {
    let body = fixture_body(&format!("{set}/repos/parity-bot/{repo}/issues/{number}/GET.json"));
    IssueListItem::from_api_item(body, &links_repo(repo)).unwrap()
}

fn pr_5() -> FullPullRequest {
    let body = fixture_body("pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
    FullPullRequest::from_api(body, &links_repo("pr-details")).unwrap()
}

fn parity_bot() -> FullUser {
    serde_json::from_value(fixture_body("user-profile/users/parity-bot/GET.json")).unwrap()
}

/// GitHub as the `link-click` and `paste-link` fixtures answer.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![links_repo("issue-fixture")])
        .with_profile("parity-bot", Ok(parity_bot()))
        .with_repository("parity-bot/issue-fixture", Ok(Some(links_repo("issue-fixture"))))
        .with_repository("parity-bot/pr-details", Ok(Some(links_repo("pr-details"))))
        .with_repository("parity-bot/missing", Ok(None))
        .with_issue(
            "parity-bot/issue-fixture",
            7,
            Ok(issue_item("links", "issue-fixture", 7)),
        )
        .with_issue(
            "parity-bot/pr-details",
            5,
            Ok(issue_item("paste-link", "pr-details", 5)),
        )
        .with_pull_request("parity-bot/pr-details", 5, Ok(pr_5()))
}

async fn app() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), github());
    app.settle().await;
    app
}

/// Follows `target` as a click does, and settles.
async fn follow(app: &mut TestApp, target: &str) {
    app.dispatch(Action::FollowLink {
        target: Arc::from(target),
        external: false,
    });
    app.settle().await;
}

fn opened(app: &TestApp) -> Vec<String> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::OpenUrl(url) => Some(url.clone()),
            _ => None,
        })
        .collect()
}

fn resolved(app: &TestApp) -> Vec<GithubLink> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                request: GithubRequest::ResolveLink(link),
                ..
            } => Some(link.clone()),
            _ => None,
        })
        .collect()
}

fn toasts(app: &TestApp) -> Vec<(Severity, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.message.to_string()))
        .collect()
}

fn current_repo(app: &TestApp) -> Option<&str> {
    app.app().current_repo().map(|repo| repo.full_name.as_str())
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

#[tokio::test]
async fn a_user_link_opens_the_profile() {
    let mut app = app().await;
    follow(&mut app, "https://github.com/parity-bot").await;
    assert_eq!(
        resolved(&app),
        [GithubLink::User {
            login: "parity-bot".into()
        }]
    );
    assert_eq!(app.app().modals(), [ModalKind::UserProfile]);
    assert!(app.gh().calls().contains(&GithubCall::UserRepos("parity-bot".into())));
    assert!(opened(&app).is_empty());
}

#[tokio::test]
async fn an_issue_link_opens_the_issue_in_its_repo() {
    let mut app = app().await;
    follow(&mut app, "https://github.com/parity-bot/issue-fixture/issues/7").await;
    assert_eq!(current_repo(&app), Some("parity-bot/issue-fixture"));
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(7));
    assert_eq!(toasts(&app), [(Severity::Information, "Opening issue for link".into())]);
    assert!(opened(&app).is_empty());
}

#[tokio::test]
async fn a_pull_request_link_and_an_issue_link_to_one_open_the_pull_request() {
    for link in [
        "https://github.com/parity-bot/pr-details/pull/5",
        "https://github.com/parity-bot/pr-details/issues/5",
    ] {
        let mut app = app().await;
        follow(&mut app, link).await;
        assert_eq!(current_repo(&app), Some("parity-bot/pr-details"), "{link}");
        assert_eq!(
            main(&app).details_pull_request().map(FullPullRequest::number),
            Some(5),
            "{link}"
        );
        assert_eq!(
            toasts(&app),
            [(Severity::Information, "Opening pull request for link".into())],
            "{link}"
        );
        assert!(opened(&app).is_empty(), "{link}");
    }
}

#[tokio::test]
async fn bare_repos_and_other_links_open_in_the_browser_at_once() {
    for link in [
        "https://github.com/parity-bot/issue-fixture",
        "https://example.com/docs",
        "https://github.com/parity-bot/issue-fixture/actions/runs/1",
    ] {
        let mut app = app().await;
        follow(&mut app, link).await;
        assert_eq!(opened(&app), [link], "{link}");
        assert!(resolved(&app).is_empty(), "{link}: nothing is looked up");
        assert_eq!(current_repo(&app), None, "{link}");
        assert!(toasts(&app).is_empty(), "{link}");
    }
}

#[tokio::test]
async fn a_link_that_doesnt_resolve_opens_in_the_browser() {
    for link in [
        "https://github.com/parity-bot/missing/pull/1",
        "https://github.com/parity-bot/issue-fixture/issues/99",
        "https://github.com/nobody",
    ] {
        let mut app = app().await;
        follow(&mut app, link).await;
        assert_eq!(resolved(&app).len(), 1, "{link}");
        assert_eq!(opened(&app), [link], "{link}");
        assert_eq!(current_repo(&app), None, "{link}");
        assert!(app.app().modals().is_empty(), "{link}");
    }
}

#[tokio::test]
async fn an_external_follow_opens_the_browser_with_a_toast() {
    let mut app = app().await;
    app.dispatch(Action::FollowLink {
        target: Arc::from("https://github.com/parity-bot"),
        external: true,
    });
    assert_eq!(opened(&app), ["https://github.com/parity-bot"]);
    assert!(resolved(&app).is_empty());
    assert_eq!(
        toasts(&app),
        [(
            Severity::Information,
            "Opening https://github.com/parity-bot externally".into()
        )]
    );
}

/// A README listing 40 links, one per row.
fn readme_repo() -> (Repository, String) {
    let readme = (1..=40).fold(String::new(), |mut readme, n| {
        // Writing to a `String` can't fail.
        let _ = writeln!(readme, "- [link {n}](https://example.com/{n})");
        readme
    });
    (repo("parity-bot", "linked", false), readme)
}

#[tokio::test]
async fn a_click_follows_the_link_drawn_under_the_pointer_now() {
    let (linked, readme) = readme_repo();
    let gh = FakeGithub::new()
        .with_repositories(vec![linked.clone()])
        .with_readme("parity-bot/linked", Ok(Some(readme)));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.settle().await;
    app.press(&["enter"]);
    app.settle().await;
    app.render(160, 48);
    let first = app
        .app()
        .links()
        .iter()
        .find(|link| link.target.starts_with("https://example.com/"))
        .cloned()
        .expect("the README's links are drawn");
    let (x, y) = (first.x, first.y);

    // Scrolled down a notch, another link is under the same cell.
    app.mouse(MouseEventKind::ScrollDown, x, y);
    app.render(160, 48);
    let now = app
        .app()
        .links()
        .at(x, y)
        .cloned()
        .expect("a link is still drawn there");
    assert_ne!(now.target, first.target);
    app.click(x, y);
    assert_eq!(opened(&app), [now.target.to_string()]);
    // The click also focused the content, as any click there does.
    assert_eq!(main(&app).focus(), FocusTarget::DetailsContent);
}

#[tokio::test]
async fn a_press_and_a_release_on_different_cells_follow_nothing() {
    let (linked, readme) = readme_repo();
    let gh = FakeGithub::new()
        .with_repositories(vec![linked])
        .with_readme("parity-bot/linked", Ok(Some(readme)));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.settle().await;
    app.press(&["enter"]);
    app.settle().await;
    app.render(160, 48);
    let links: Vec<_> = app.app().links().iter().cloned().collect();
    let (a, b) = (&links[links.len() - 2], &links[links.len() - 1]);
    app.mouse(MouseEventKind::Down(crossterm::event::MouseButton::Left), a.x, a.y);
    app.mouse(MouseEventKind::Up(crossterm::event::MouseButton::Left), b.x, b.y);
    assert!(opened(&app).is_empty());
    // Off any link, a click is an ordinary click.
    app.click(0, 47);
    assert!(opened(&app).is_empty());
}
