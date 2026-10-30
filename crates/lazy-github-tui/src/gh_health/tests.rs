//! `gh` going missing, logging out or hanging mid-session (#323).

use std::time::Duration;

use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{Repository, User};

use super::*;
use crate::action::Action;
use crate::notifications::{LOGIN_MESSAGE, LOGIN_TOAST_TITLE, REFRESH_INTERVAL};
use crate::test_util::TestApp;
use crate::toast::Severity;

const NOT_INSTALLED: &str = "The GitHub CLI (gh) is not installed or not on your PATH. Install it from https://cli.github.com, then run `gh auth login`.";

fn enabled() -> Config {
    let mut config = Config::default();
    config.notifications.enabled = true;
    config
}

fn repo(name: &str) -> Repository {
    Repository {
        name: name.into(),
        full_name: format!("octocat/{name}"),
        default_branch: Some("main".into()),
        private: false,
        archived: Some(false),
        owner: User {
            login: "octocat".into(),
            id: 1,
            name: None,
            html_url: "https://github.com/octocat".into(),
        },
        description: None,
        permissions: None,
    }
}

fn logged_out() -> GhError {
    GhError::NotAuthenticated {
        hostname: None,
        detail: "You are not logged into any GitHub hosts.".into(),
    }
}

fn unauthorized() -> GhError {
    GhError::Http {
        status: 401,
        message: "Bad credentials".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

fn settle(app: &mut TestApp) {
    app.settle().now_or_never().expect("the fake answers at once");
}

/// The toasts up now, as `(title, message)`.
fn toasts(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

fn api_error(message: &str) -> (Option<String>, String) {
    (Some(TOAST_TITLE.to_owned()), message.to_owned())
}

/// How many notification refreshes have reached the fake: each starts with an auth check.
fn refreshes(app: &TestApp) -> usize {
    app.gh()
        .calls()
        .iter()
        .filter(|call| matches!(call, GithubCall::CheckAuth))
        .count()
}

#[test]
fn gh_removed_from_path_toasts_the_install_message_once_and_the_next_action_retries() {
    let mut app = TestApp::new(Config::default());
    settle(&mut app);
    app.gh().fail_from_now(GhError::NotInstalled);

    // Selecting a repo asks for its issues, workflows, runs and README: every one fails, one toast says why.
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    assert!(app.gh().calls().len() > 2);
    assert_eq!(toasts(&app), [api_error(NOT_INSTALLED)]);
    let toast = app.app().toasts().iter().next().unwrap();
    assert_eq!(toast.severity, Severity::Error);

    // gh comes back: the next selection spawns it again and works, with no new toast.
    app.gh().stop_failing();
    let before = app.gh().calls().len();
    app.dispatch(Action::SelectRepo(Box::new(repo("b"))));
    settle(&mut app);
    assert!(app.gh().calls().len() > before);
    assert_eq!(toasts(&app), [api_error(NOT_INSTALLED)]);

    // Missing again after working: a new outage, a new toast.
    app.gh().fail_from_now(GhError::NotInstalled);
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    assert_eq!(toasts(&app), [api_error(NOT_INSTALLED), api_error(NOT_INSTALLED)]);
}

#[test]
fn a_missing_gh_pauses_the_poll_until_a_request_succeeds() {
    let mut app = TestApp::with_github(enabled(), FakeGithub::new());
    settle(&mut app);
    assert_eq!(refreshes(&app), 1);

    app.gh().fail_from_now(GhError::NotInstalled);
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    // The poll's own check failed: it says gh is missing rather than "please login", and there's one toast.
    assert_eq!(refreshes(&app), 2);
    assert_eq!(
        toasts(&app),
        [(Some(LOGIN_TOAST_TITLE.to_owned()), NOT_INSTALLED.to_owned())]
    );

    // The next ticks don't run gh.
    app.advance(REFRESH_INTERVAL * 3);
    settle(&mut app);
    assert_eq!(refreshes(&app), 2);

    // Once a request the user makes works, the poll comes back on its next tick.
    app.gh().stop_failing();
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    assert_eq!(refreshes(&app), 3);
}

#[test]
fn logging_out_mid_session_toasts_login_once_and_stops_the_poll() {
    let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Ok(2)));
    settle(&mut app);

    // `gh auth logout`: the poll's check fails first, with its own toast.
    app.gh().set_auth_error(Some(logged_out()));
    app.gh().fail_from_now(unauthorized());
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    assert_eq!(
        toasts(&app),
        [(Some(LOGIN_TOAST_TITLE.to_owned()), LOGIN_MESSAGE.to_owned())]
    );
    let checks = refreshes(&app);

    // A request the user makes fails with a 401 too: the app already knows, so no second login toast.
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    assert_eq!(toasts(&app).len(), 1);

    // The poll stays stopped.
    app.advance(REFRESH_INTERVAL * 5);
    settle(&mut app);
    assert_eq!(refreshes(&app), checks);

    // After `gh auth login`, the next request that works brings the poll back.
    app.gh().set_auth_error(None);
    app.gh().stop_failing();
    app.dispatch(Action::SelectRepo(Box::new(repo("b"))));
    settle(&mut app);
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    assert_eq!(refreshes(&app), checks + 1);
}

#[test]
fn a_401_from_a_request_toasts_the_login_message() {
    let mut app = TestApp::new(Config::default());
    settle(&mut app);
    app.gh().fail_from_now(GhError::NotAuthenticated {
        hostname: Some("ghe.example.com".into()),
        detail: String::new(),
    });
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    assert_eq!(
        toasts(&app),
        [api_error(
            "To proceed, please log into the Github CLI via `gh auth login --hostname ghe.example.com`"
        )]
    );
}

#[test]
fn an_http_401_toasts_the_login_message_for_the_configured_host() {
    for (base_url, expected) in [
        (None, "To proceed, please log into the Github CLI via `gh auth login`"),
        (
            Some("https://ghe.example.com/api/v3"),
            "To proceed, please log into the Github CLI via `gh auth login --hostname ghe.example.com`",
        ),
    ] {
        let mut config = Config::default();
        if let Some(url) = base_url {
            config.api.base_url = url.into();
        }
        let mut app = TestApp::new(config);
        settle(&mut app);
        app.gh().fail_from_now(unauthorized());
        app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
        settle(&mut app);
        assert_eq!(toasts(&app), [api_error(expected)], "{base_url:?}");
    }
}

#[test]
fn a_timeout_toasts_that_it_timed_out_and_keeps_the_poll() {
    let mut app = TestApp::with_github(enabled(), FakeGithub::new());
    settle(&mut app);
    app.gh().fail_from_now(GhError::Timeout {
        after: Duration::from_secs(30),
    });
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    assert_eq!(toasts(&app), [api_error("gh did not respond within 30 seconds")]);
    // A hung request doesn't stop the poll: its next tick runs, and its check times out too, which the poll
    // reports as Python does (the first toast has expired by then).
    let before = refreshes(&app);
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    assert_eq!(refreshes(&app), before + 1);
    assert_eq!(
        toasts(&app),
        [(Some(LOGIN_TOAST_TITLE.to_owned()), LOGIN_MESSAGE.to_owned())]
    );
}

#[test]
fn other_failures_leave_it_to_the_request() {
    let mut app = TestApp::new(Config::default());
    settle(&mut app);
    app.gh().fail_from_now(GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    });
    app.dispatch(Action::SelectRepo(Box::new(repo("a"))));
    settle(&mut app);
    assert!(
        toasts(&app)
            .iter()
            .all(|(title, _)| title.as_deref() != Some(TOAST_TITLE)),
        "{:?}",
        toasts(&app)
    );
}

#[test]
fn gh_version_working_logged_out_doesnt_end_the_outage() {
    let mut app = TestApp::with_github(enabled(), FakeGithub::new());
    settle(&mut app);
    app.gh().set_auth_error(Some(logged_out()));
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    let checks = refreshes(&app);
    app.with_cx(|cx| {
        cx.request(crate::effect::GithubRequest::GhVersion);
    });
    settle(&mut app);
    app.advance(REFRESH_INTERVAL);
    settle(&mut app);
    assert_eq!(refreshes(&app), checks);
}
