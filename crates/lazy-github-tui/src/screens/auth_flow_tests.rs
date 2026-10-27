//! The startup flow end to end: the cached check, the startup `CheckAuth`, the authentication screen and its poll.

use std::time::Duration;

use futures_util::FutureExt as _;
use jiff::ToSpan as _;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall, RepoListQuery};

use crate::app::App;
use crate::clock::{Clock as _, FakeClock};
use crate::component::ScreenKind;
use crate::effect::{Effect, GitRequest, GithubRequest, TimerId};
use crate::screens::auth::{AUTH_POLL_INTERVAL, AuthReason};
use crate::test_util::TestApp;

const LOGIN_MESSAGE: &str = "To proceed, please log into the Github CLI via gh auth login";

fn not_logged_in() -> GhError {
    GhError::NotAuthenticated {
        hostname: None,
        detail: "You are not logged into any GitHub hosts. To log in, run: gh auth login".into(),
    }
}

/// A config whose last auth check was `ago` before the fake clock's start, or never.
fn config_checked(ago: Option<jiff::Span>) -> Config {
    let mut config = Config::default();
    let now = FakeClock::new().local_now().datetime();
    config.cache.auth_last_checked = ago.map(|ago| now - ago);
    config
}

fn settle(app: &mut TestApp) {
    // FakeGithub answers at once when it has no latency, so this never waits.
    app.settle().now_or_never().expect("FakeGithub answers without waiting");
}

fn check_auths(app: &TestApp) -> usize {
    app.effects()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Github {
                    request: GithubRequest::CheckAuth,
                    ..
                }
            )
        })
        .count()
}

fn saved_configs(app: &TestApp) -> Vec<&Config> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::SaveConfig(config) => Some(&**config),
            _ => None,
        })
        .collect()
}

/// An app that failed the startup check with `error` and shows the authentication screen.
fn reason(app: &App) -> Option<AuthReason> {
    app.auth_screen().map(crate::screens::auth::AuthScreen::reason)
}

fn on_auth_screen(error: GhError) -> TestApp {
    let mut app = TestApp::on_startup(config_checked(None), FakeGithub::new().with_auth_error(error));
    settle(&mut app);
    assert_eq!(app.app().screen(), Some(ScreenKind::Auth));
    app
}

#[test]
fn a_fresh_cache_goes_straight_to_main() {
    let mut app = TestApp::on_startup(config_checked(Some(1.hour())), FakeGithub::new());
    settle(&mut app);
    assert_eq!(app.app().screen(), Some(ScreenKind::Main));
    // Only the main screen's repos list, its `repos.json` read and write, and the current-directory repo.
    assert!(
        matches!(
            app.effects(),
            [
                Effect::Github {
                    request: GithubRequest::ListRepositories(_),
                    ..
                },
                Effect::ReadCache { .. },
                Effect::Git {
                    request: GitRequest::OriginRepo,
                    ..
                },
                Effect::WriteCache { .. },
            ]
        ),
        "{:?}",
        app.effects()
    );
    assert_eq!(app.gh().calls(), [GithubCall::ListRepositories(RepoListQuery::all())]);
}

#[test]
fn a_stale_cache_checks_then_saves_once_and_shows_main() {
    let mut app = TestApp::on_startup(config_checked(Some(2.days())), FakeGithub::new());
    // Blank until the check answers.
    assert_eq!(app.app().screen(), None);
    assert!(app.screen(100, 30).trim().is_empty());
    assert!(matches!(
        app.effects(),
        [Effect::Github {
            request: GithubRequest::CheckAuth,
            ..
        }]
    ));
    settle(&mut app);
    assert_eq!(
        app.gh().calls(),
        [
            GithubCall::CheckAuth,
            GithubCall::ListRepositories(RepoListQuery::all())
        ]
    );
    let saved = saved_configs(&app);
    assert_eq!(saved.len(), 1);
    let now = app.clock().local_now().datetime();
    assert_eq!(saved[0].cache.auth_last_checked, Some(now));
    assert!(!saved[0].core.first_start);
    // The check, the repos list, the `repos.json` read, the current-directory repo, the save and the `repos.json`
    // write.
    assert_eq!(app.effects().len(), 6, "{:?}", app.effects());
    assert_eq!(app.app().screen(), Some(ScreenKind::Main));
}

#[test]
fn no_previous_check_is_stale() {
    let mut app = TestApp::on_startup(config_checked(None), FakeGithub::new());
    settle(&mut app);
    assert_eq!(
        app.gh().calls(),
        [
            GithubCall::CheckAuth,
            GithubCall::ListRepositories(RepoListQuery::all())
        ]
    );
    assert_eq!(saved_configs(&app).len(), 1);
    assert_eq!(app.app().screen(), Some(ScreenKind::Main));
}

#[test]
fn not_logged_in_shows_the_login_message_and_starts_polling() {
    let mut app = on_auth_screen(not_logged_in());
    assert_eq!(reason(app.app()), Some(AuthReason::NotLoggedIn));
    assert!(app.screen(100, 30).contains(LOGIN_MESSAGE));
    assert!(app.effects().contains(&Effect::StartInterval {
        id: TimerId::AuthPoll,
        every: Duration::from_secs(5),
    }));
    assert_eq!(saved_configs(&app), Vec::<&Config>::new());
}

#[test]
fn the_poll_rechecks_and_moves_on_without_saving() {
    let mut app = on_auth_screen(not_logged_in());
    app.advance(AUTH_POLL_INTERVAL);
    assert_eq!(check_auths(&app), 2);

    // Still logged out: stay.
    settle(&mut app);
    assert_eq!(app.app().screen(), Some(ScreenKind::Auth));

    app.gh().set_auth_error(None);
    app.advance(AUTH_POLL_INTERVAL);
    assert_eq!(check_auths(&app), 3);
    settle(&mut app);
    assert_eq!(app.app().screen(), Some(ScreenKind::Main));
    assert_eq!(saved_configs(&app), Vec::<&Config>::new());
    let last = &app.effects()[app.effects().len() - 5..];
    assert!(
        matches!(
            last,
            [
                Effect::CancelTimer(TimerId::AuthPoll),
                Effect::Github {
                    request: GithubRequest::ListRepositories(_),
                    ..
                },
                Effect::ReadCache { .. },
                Effect::Git {
                    request: GitRequest::OriginRepo,
                    ..
                },
                Effect::WriteCache { .. },
            ]
        ),
        "{last:?}"
    );

    // The timer is gone, so nothing checks again.
    app.advance(AUTH_POLL_INTERVAL * 3);
    assert_eq!(check_auths(&app), 3);
}

#[test]
fn a_check_in_flight_is_not_doubled() {
    let mut app = on_auth_screen(not_logged_in());
    app.advance(AUTH_POLL_INTERVAL * 3);
    assert_eq!(check_auths(&app), 2);
    settle(&mut app);
    app.advance(AUTH_POLL_INTERVAL);
    assert_eq!(check_auths(&app), 3);
}

#[test]
fn gh_missing_shows_the_install_message() {
    let mut app = on_auth_screen(GhError::NotInstalled);
    assert_eq!(reason(app.app()), Some(AuthReason::NotInstalled));
    let screen = app.screen(160, 48);
    assert!(screen.contains(&GhError::NotInstalled.to_string()), "{screen}");
    assert!(!screen.contains(LOGIN_MESSAGE));
}

#[test]
fn installing_gh_while_waiting_shows_the_login_message() {
    let mut app = on_auth_screen(GhError::NotInstalled);
    app.gh().set_auth_error(Some(not_logged_in()));
    app.advance(AUTH_POLL_INTERVAL);
    settle(&mut app);
    assert_eq!(reason(app.app()), Some(AuthReason::NotLoggedIn));
    assert!(app.screen(100, 30).contains(LOGIN_MESSAGE));
}

#[test]
fn q_quits_from_the_auth_screen() {
    let mut app = on_auth_screen(not_logged_in());
    app.press(&["q"]);
    assert_eq!(app.effects().last(), Some(&Effect::Quit));
    assert!(app.app().should_quit());
}

#[test]
fn ctrl_q_still_quits_from_the_auth_screen() {
    let mut app = on_auth_screen(not_logged_in());
    app.press(&["ctrl+q"]);
    assert!(app.app().should_quit());
}

#[test]
fn only_quit_is_bound_on_the_auth_screen() {
    let mut app = on_auth_screen(not_logged_in());
    let before = app.effects().len();
    app.press(&["M", "question_mark", "ctrl+p", "ctrl+c"]);
    assert_eq!(app.effects().len(), before);
    assert!(!app.app().should_quit());
    assert!(app.app().toasts().is_empty());
}

#[test]
fn auth_not_logged_in_100x30() {
    insta::assert_snapshot!(on_auth_screen(not_logged_in()).screen(100, 30));
}

#[test]
fn auth_not_logged_in_160x48() {
    insta::assert_snapshot!(on_auth_screen(not_logged_in()).screen(160, 48));
}

#[test]
fn auth_gh_missing_100x30() {
    insta::assert_snapshot!(on_auth_screen(GhError::NotInstalled).screen(100, 30));
}

#[test]
fn the_auth_screen_colors() {
    use ratatui::style::{Color, Modifier};

    let buffer = on_auth_screen(not_logged_in()).render(100, 30);
    let red = Color::Rgb(0xFF, 0, 0);
    for (x, y) in [(0, 0), (99, 0), (0, 15), (99, 29), (5, 0)] {
        assert_eq!(buffer[(x, y)].fg, red, "border at {x},{y}");
    }
    // `gh auth login` in Textual's `green`.
    let at = LOGIN_MESSAGE.find("gh auth").unwrap();
    let x = 1 + u16::try_from(at).unwrap();
    assert_eq!(buffer[(x, 1)].fg, Color::Rgb(0x00, 0x80, 0x00));
    assert_ne!(buffer[(x - 2, 1)].fg, Color::Rgb(0x00, 0x80, 0x00));
    // The footer key is bold.
    assert_eq!(buffer[(2, 28)].symbol(), "q");
    assert!(buffer[(2, 28)].modifier.contains(Modifier::BOLD));
}
