//! `u` from the repos, Pull Requests and Issues tables: the fetch, its errors, the dialog it opens and what a pick
//! does.

use futures_util::FutureExt;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullUser, Repository};

use super::super::pull_requests::{Kind, item};
use super::super::repos::repo;
use crate::effect::{Effect, GithubRequest};
use crate::event::Event;
use crate::modal::ModalKind;
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::{TestApp, logs};
use crate::toast::Severity;

/// The `response.body` of a `user-profile` fixture.
fn fixture(file: &str) -> serde_json::Value {
    fixture_in("user-profile", file)
}

/// The `response.body` of a fixture in the `set` fixture set.
fn fixture_in(set: &str, file: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn parity_bot() -> FullUser {
    serde_json::from_value(fixture("users/parity-bot/GET.json")).unwrap()
}

fn parity_bot_repos() -> Vec<Repository> {
    serde_json::from_value(fixture(
        "users/parity-bot/repos/GET.direction-desc_page-1_per_page-30_sort-updated.json",
    ))
    .unwrap()
}

fn parity_org() -> FullUser {
    serde_json::from_value(fixture("users/parity-org/GET.json")).unwrap()
}

/// `base`'s three repos, then `parity-org/tracked-tool`.
fn listed() -> Vec<Repository> {
    vec![
        repo("parity-bot", "dotfiles", false),
        repo("parity-bot", "hello-world", false),
        repo("parity-bot", "lazy-demo", false),
        repo("parity-org", "tracked-tool", false),
    ]
}

/// A fake that lists [`listed`] and knows both profiles.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(listed())
        .with_profile("parity-bot", Ok(parity_bot()))
        .with_user_repos("parity-bot", Ok(parity_bot_repos()))
        .with_profile("parity-org", Ok(parity_org()))
}

fn decode_error() -> GhError {
    GhError::Decode(serde_json::from_str::<u8>("\"x\"").unwrap_err())
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// Answers every queued request, and returns what that logged.
fn settle(app: &mut TestApp) -> String {
    logs(|| app.settle().now_or_never().expect("the fake answers at once"))
}

fn toasts(app: &TestApp) -> Vec<(Severity, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.message.to_string()))
        .collect()
}

fn profile_calls(app: &TestApp) -> Vec<GithubCall> {
    app.gh()
        .calls()
        .into_iter()
        .filter(|call| matches!(call, GithubCall::User(_) | GithubCall::UserRepos(_)))
        .collect()
}

// The fetch.

#[test]
fn both_answers_open_the_dialog() {
    let mut app = TestApp::with_github(Config::default(), github());
    app.press(&["u"]);
    // Both at once, and nothing opens until they're in.
    let requests: Vec<&GithubRequest> = app
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(
        requests,
        [
            &GithubRequest::User("parity-bot".into()),
            &GithubRequest::UserRepos("parity-bot".into())
        ]
    );
    assert_eq!(app.app().modals(), []);
    let out = settle(&mut app);
    assert_eq!(out, "DEBUG Opened UserProfile\n");
    assert_eq!(app.app().modals(), [ModalKind::UserProfile]);
    assert!(toasts(&app).is_empty());
    let screen = app.screen(160, 48);
    assert!(screen.contains("Parity Bot (@parity-bot) — user"), "{screen}");
    assert!(screen.contains("profile-only"), "{screen}");
}

#[test]
fn a_profile_that_does_not_decode_fails_the_fetch() {
    for gh in [
        github().with_profile("parity-bot", Err(decode_error())),
        github().with_user_repos("parity-bot", Err(decode_error())),
        // A body that doesn't decode wins over a missing profile, as Python's `gather` raises.
        github()
            .with_profile("parity-bot", Err(server_error()))
            .with_user_repos("parity-bot", Err(decode_error())),
    ] {
        let mut app = TestApp::with_github(Config::default(), gh);
        app.press(&["u"]);
        let out = settle(&mut app);
        assert!(out.starts_with("ERROR Error fetching user profile\n"), "{out}");
        assert_eq!(app.app().modals(), []);
        assert_eq!(
            toasts(&app),
            [(Severity::Error, "Failed to load profile for @parity-bot".to_owned())]
        );
    }
}

#[test]
fn a_profile_github_does_not_give_is_an_unknown_user() {
    for error in [
        GhError::Http {
            status: 404,
            message: "Not Found".into(),
            body: Vec::new(),
            headers: Vec::new(),
        },
        server_error(),
    ] {
        let mut app = TestApp::with_github(Config::default(), github().with_profile("parity-bot", Err(error)));
        app.press(&["u"]);
        let out = settle(&mut app);
        assert!(!out.contains("ERROR"), "{out}");
        assert_eq!(app.app().modals(), []);
        assert_eq!(
            toasts(&app),
            [(Severity::Error, "Could not find user @parity-bot".to_owned())]
        );
    }
}

#[test]
fn repos_github_does_not_give_are_none() {
    let mut app = TestApp::with_github(
        Config::default(),
        github().with_user_repos("parity-bot", Err(server_error())),
    );
    app.press(&["u"]);
    settle(&mut app);
    assert!(toasts(&app).is_empty());
    assert_eq!(app.app().modals(), [ModalKind::UserProfile]);
    assert!(app.screen(160, 48).contains("No public repositories."));
}

#[test]
fn an_empty_table_toasts_and_asks_for_nothing() {
    let mut app = TestApp::new(Config::default());
    app.press(&["u"]);
    assert_eq!(
        toasts(&app),
        [(Severity::Error, "No repository currently selected".to_owned())]
    );
    settle(&mut app);
    assert_eq!(profile_calls(&app), []);
    assert_eq!(app.app().modals(), []);
}

#[test]
fn two_fetches_in_flight_do_not_mix() {
    let mut app = TestApp::with_github(Config::default(), github());
    // parity-bot's, then parity-org's, before either answers.
    app.press(&["u", "j", "j", "j", "u"]);
    let requests: Vec<(crate::effect::RequestId, GithubRequest)> = app
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 4);
    // Answered out of order: parity-org's profile first, parity-bot's repos last.
    for index in [2, 0, 3, 1] {
        let (id, request) = requests[index].clone();
        let response = request.run(app.gh()).now_or_never().unwrap();
        app.send(Event::Github { id, response });
        let opened = app.app().modals().len();
        match index {
            2 | 0 => assert_eq!(opened, 0),
            3 => assert_eq!(opened, 1),
            _ => assert_eq!(opened, 2),
        }
    }
    // parity-bot's opened last, on top.
    assert!(app.screen(160, 48).contains("Parity Bot (@parity-bot)"));
    app.press(&["escape"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("parity-org (@parity-org) — organization"), "{screen}");
    assert!(screen.contains("No public repositories."), "{screen}");
}

// Picking.

fn current_repo(app: &TestApp) -> Option<String> {
    app.app().current_repo().map(|repo| repo.full_name.clone())
}

fn row_keys(app: &TestApp) -> Vec<String> {
    app.app()
        .main_screen()
        .unwrap()
        .repos()
        .table()
        .row_keys()
        .cloned()
        .collect()
}

fn main_focus(app: &TestApp) -> FocusTarget {
    app.app().main_screen().unwrap().focus()
}

#[test]
fn a_picked_repo_the_table_lacks_is_added_and_selected() {
    let mut app = TestApp::with_github(Config::default(), github());
    app.press(&["u"]);
    settle(&mut app);
    app.render(160, 48);
    // The second repo, `profile-only`, isn't listed.
    let out = logs(|| {
        app.press(&["tab", "j", "enter"]);
    });
    assert!(out.contains("INFO Selected repo parity-bot/profile-only"), "{out}");
    assert_eq!(app.app().modals(), []);
    assert_eq!(current_repo(&app).as_deref(), Some("parity-bot/profile-only"));
    assert_eq!(
        row_keys(&app).last().map(String::as_str),
        Some("parity-bot/profile-only")
    );
    assert_eq!(row_keys(&app).len(), 5);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
}

#[test]
fn a_picked_repo_the_table_has_is_only_selected() {
    let mut app = TestApp::with_github(Config::default(), github());
    app.press(&["u"]);
    settle(&mut app);
    app.render(160, 48);
    app.press(&["tab", "enter"]);
    assert_eq!(current_repo(&app).as_deref(), Some("parity-bot/dotfiles"));
    assert_eq!(row_keys(&app).len(), 4);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
}

#[test]
fn closing_selects_nothing() {
    let mut app = TestApp::with_github(Config::default(), github());
    app.press(&["u"]);
    settle(&mut app);
    app.press(&["escape"]);
    assert_eq!(app.app().modals(), []);
    assert_eq!(current_repo(&app), None);
    assert_eq!(row_keys(&app).len(), 4);
    assert_eq!(main_focus(&app), FocusTarget::Table(SectionId::Repos));
}

// From the Pull Requests and Issues tables.

fn parity_friend() -> FullUser {
    serde_json::from_value(fixture_in("user-profile-authors", "users/parity-friend/GET.json")).unwrap()
}

fn parity_friend_repos() -> Vec<Repository> {
    serde_json::from_value(fixture_in(
        "user-profile-authors",
        "users/parity-friend/repos/GET.direction-desc_page-1_per_page-30_sort-updated.json",
    ))
    .unwrap()
}

/// [`github`], with `parity-bot/dotfiles` listing PR 2 by parity-bot and issue 1 by parity-friend.
fn authors_github() -> FakeGithub {
    let dotfiles = repo("parity-bot", "dotfiles", false);
    let mut issue = item(&dotfiles, 1, Kind::Issue);
    if let lazy_github_core::models::IssueListItem::Issue(issue) = &mut issue {
        issue.user.login = "parity-friend".into();
    }
    github()
        .with_issue_page(
            "parity-bot/dotfiles",
            1,
            Ok(vec![item(&dotfiles, 2, Kind::OpenPr), issue]),
        )
        .with_profile("parity-friend", Ok(parity_friend()))
        .with_user_repos("parity-friend", Ok(parity_friend_repos()))
}

/// The app with `parity-bot/dotfiles` selected and its lists loaded, the profile calls so far forgotten.
fn authors_app(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app
}

fn requested_profiles(app: &TestApp) -> Vec<GithubRequest> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. }
                if matches!(request, GithubRequest::User(_) | GithubRequest::UserRepos(_)) =>
            {
                Some(request.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_pull_request_table_asks_for_the_highlighted_author() {
    let mut app = authors_app(authors_github());
    app.press(&["2", "u"]);
    assert_eq!(
        requested_profiles(&app),
        [
            GithubRequest::User("parity-bot".into()),
            GithubRequest::UserRepos("parity-bot".into())
        ]
    );
    settle(&mut app);
    assert_eq!(app.app().modals(), [ModalKind::UserProfile]);
    assert!(app.screen(160, 48).contains("Parity Bot (@parity-bot) — user"));
}

#[test]
fn the_issue_table_asks_for_the_highlighted_author() {
    let mut app = authors_app(authors_github());
    app.press(&["3", "u"]);
    assert_eq!(
        requested_profiles(&app),
        [
            GithubRequest::User("parity-friend".into()),
            GithubRequest::UserRepos("parity-friend".into())
        ]
    );
    settle(&mut app);
    assert_eq!(app.app().modals(), [ModalKind::UserProfile]);
    assert!(app.screen(160, 48).contains("Parity Friend (@parity-friend) — user"));
}

#[test]
fn empty_pull_request_and_issue_tables_toast_and_ask_for_nothing() {
    // dotfiles has no PRs or issues here.
    let mut app = authors_app(github());
    app.press(&["2", "u"]);
    app.press(&["3", "u"]);
    assert_eq!(
        toasts(&app),
        [
            (Severity::Error, "No pull request currently selected".to_owned()),
            (Severity::Error, "No issue currently selected".to_owned()),
        ]
    );
    settle(&mut app);
    assert_eq!(profile_calls(&app), []);
    assert_eq!(app.app().modals(), []);
}

#[test]
fn a_repo_picked_from_an_author_is_selected_without_joining_the_repos_table() {
    let picks: [(&str, &[&str], &str, SectionId); 2] = [
        // parity-bot's second repo isn't listed either.
        (
            "2",
            &["tab", "j", "enter"],
            "parity-bot/profile-only",
            SectionId::PullRequests,
        ),
        ("3", &["tab", "enter"], "parity-friend/friend-tools", SectionId::Issues),
    ];
    for (section, keys, picked, focus) in picks {
        let mut app = authors_app(authors_github());
        app.press(&[section, "u"]);
        settle(&mut app);
        app.render(160, 48);
        let out = logs(|| {
            app.press(keys);
        });
        assert!(out.contains(&format!("INFO Selected repo {picked}")), "{out}");
        assert_eq!(app.app().modals(), []);
        assert_eq!(current_repo(&app).as_deref(), Some(picked));
        assert_eq!(row_keys(&app).len(), 4);
        assert!(!row_keys(&app).iter().any(|key| key == picked));
        assert_eq!(main_focus(&app), FocusTarget::Table(focus));
    }
}

#[test]
fn closing_an_author_profile_returns_to_its_table() {
    for (section, focus) in [("2", SectionId::PullRequests), ("3", SectionId::Issues)] {
        let mut app = authors_app(authors_github());
        app.press(&[section, "u"]);
        settle(&mut app);
        app.press(&["escape"]);
        assert_eq!(app.app().modals(), []);
        assert_eq!(current_repo(&app).as_deref(), Some("parity-bot/dotfiles"));
        assert_eq!(main_focus(&app), FocusTarget::Table(focus));
    }
}
