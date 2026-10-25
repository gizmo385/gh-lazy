//! The unread-notifications poll behind the status box: Python's `refresh_notification_count` and the
//! `set_interval` that drives it.
//!
//! Each refresh checks `gh auth status` first, then counts the first page of unread notifications, so a logged-out
//! `gh` and a failing fetch are told apart, as in Python.

use std::time::Duration;

use lazy_github_core::gh::GhError;

use crate::component::Cx;
use crate::components::status_bar::UnreadState;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId, TimerId};
use crate::toast::Toast;

/// How often the count refreshes: Python's `NOTIFICATION_REFRESH_INTERVAL`.
pub const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

/// The error toast's title when `gh` isn't logged in.
///
/// Py bug B14 (pending Chris): Python's title is "Failed to Load Notifiations". Rust fixes the typo, as recommended;
/// put the typo back here to match Python.
pub const LOGIN_TOAST_TITLE: &str = "Failed to Load Notifications";

/// The error toast's message and the command-log line when `gh` isn't logged in.
pub const LOGIN_MESSAGE: &str = "Cannot load notifications - please login to the gh CLI: gh auth login";

/// The command-log line when the count can't be fetched. Python's `lg.exception` message.
pub const FETCH_FAILED_MESSAGE: &str = "Failed to retrieve notifications from the Github API";

/// The poll's state. `App` owns one for the life of the app, as the timer outlives any one screen.
#[derive(Debug, Default)]
pub struct NotificationPoll {
    unread: UnreadState,
    in_flight: Option<InFlight>,
}

/// The step of a refresh that's running, and its request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InFlight {
    CheckingAuth(RequestId),
    Counting(RequestId),
}

impl InFlight {
    fn id(self) -> RequestId {
        match self {
            InFlight::CheckingAuth(id) | InFlight::Counting(id) => id,
        }
    }
}

impl NotificationPoll {
    /// What the status box shows.
    pub fn unread(&self) -> UnreadState {
        self.unread
    }

    /// Whether a refresh is running.
    pub fn is_refreshing(&self) -> bool {
        self.in_flight.is_some()
    }

    /// The main screen was created. When notifications are enabled, starts the interval and refreshes now; otherwise
    /// does nothing, and the count stays blank.
    pub fn start(&mut self, cx: &mut Cx<'_>) {
        if !cx.config().notifications.enabled {
            return;
        }
        cx.effect(Effect::StartInterval {
            id: TimerId::Notifications,
            every: REFRESH_INTERVAL,
        });
        self.refresh(cx);
    }

    /// Refreshes the count: blank, with no request, while notifications are disabled; nothing while a refresh is
    /// already running; otherwise checks auth, then counts.
    pub fn refresh(&mut self, cx: &mut Cx<'_>) {
        if !cx.config().notifications.enabled {
            self.unread = UnreadState::Hidden;
            return;
        }
        if self.in_flight.is_some() {
            tracing::debug!("Skipping the notifications refresh: one is already running");
            return;
        }
        self.in_flight = Some(InFlight::CheckingAuth(cx.request(GithubRequest::CheckAuth)));
    }

    /// Handles `response` if it's to this poll's request; otherwise hands it back.
    pub fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let Some(in_flight) = self.in_flight.filter(|in_flight| in_flight.id() == id) else {
            return Some(response);
        };
        self.in_flight = None;
        match (in_flight, response) {
            (InFlight::CheckingAuth(_), GithubResponse::CheckAuth(result)) => self.auth_checked(result, cx),
            (InFlight::Counting(_), GithubResponse::UnreadNotificationCount(result)) => self.counted(result),
            (_, response) => tracing::debug!("Dropping an unexpected notifications response {response:?}"),
        }
        None
    }

    fn auth_checked(&mut self, result: Result<(), GhError>, cx: &mut Cx<'_>) {
        match result {
            Ok(()) => {
                self.in_flight = Some(InFlight::Counting(cx.request(GithubRequest::UnreadNotificationCount)));
            }
            // The count keeps its last value, as in Python.
            Err(error) => {
                tracing::debug!("Notifications auth check failed: {error}");
                cx.notify(Toast::error(LOGIN_MESSAGE).with_title(LOGIN_TOAST_TITLE));
                tracing::error!("{LOGIN_MESSAGE}");
            }
        }
    }

    fn counted(&mut self, result: Result<usize, GhError>) {
        // Python's fetch swallows every error and counts nothing, so a failure reads "No unread notifications".
        let count = result.unwrap_or_else(|error| {
            tracing::error!("{FETCH_FAILED_MESSAGE}");
            tracing::debug!("Fetching notifications failed: {error}");
            0
        });
        self.unread = UnreadState::Count(count);
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt as _;
    use lazy_github_core::config::Config;
    use lazy_github_core::github::{FakeGithub, GithubCall};
    use lazy_github_core::models::{Repository, User};

    use super::*;
    use crate::action::Action;
    use crate::test_util::{TestApp, logs};
    use crate::toast::Severity;

    fn enabled() -> Config {
        let mut config = Config::default();
        config.notifications.enabled = true;
        config
    }

    fn logged_out() -> GhError {
        GhError::NotAuthenticated {
            hostname: None,
            detail: "You are not logged into any GitHub hosts.".into(),
        }
    }

    /// Runs every queued request on the fake.
    fn settle(app: &mut TestApp) {
        app.settle().now_or_never().expect("the fake answers at once");
    }

    /// The status box's content row at 100x30.
    fn status_row(app: &mut TestApp) -> String {
        app.screen(100, 30).lines().nth(1).unwrap().to_owned()
    }

    #[test]
    fn enabled_startup_starts_the_interval_and_checks_auth() {
        let app = TestApp::new(enabled());
        let [interval, check] = app.effects() else {
            panic!("{:?}", app.effects());
        };
        assert_eq!(
            interval,
            &Effect::StartInterval {
                id: TimerId::Notifications,
                every: Duration::from_mins(1),
            }
        );
        assert!(matches!(
            check,
            Effect::Github {
                request: GithubRequest::CheckAuth,
                ..
            }
        ));
        assert!(app.app().notifications_refreshing());
        assert_eq!(app.app().unread(), UnreadState::Hidden);
    }

    #[test]
    fn the_startup_check_then_the_main_screen_start_the_poll() {
        let mut app = TestApp::on_startup(enabled(), FakeGithub::new().with_unread_count(Ok(3)));
        settle(&mut app);
        assert_eq!(
            app.gh().calls(),
            [
                GithubCall::CheckAuth,
                GithubCall::CheckAuth,
                GithubCall::UnreadNotificationCount
            ]
        );
        assert_eq!(app.app().unread(), UnreadState::Count(3));
    }

    #[test]
    fn disabled_startup_does_nothing() {
        let mut app = TestApp::new(Config::default());
        assert_eq!(app.effects(), []);
        app.advance(Duration::from_mins(10));
        assert_eq!(app.effects(), []);
        assert_eq!(app.app().unread(), UnreadState::Hidden);
        assert!(!status_row(&mut app).contains("nread"));
    }

    #[test]
    fn a_refresh_counts_and_the_bar_shows_it() {
        let gh = FakeGithub::new().with_unread_count(Ok(3));
        let mut app = TestApp::with_github(enabled(), gh);
        settle(&mut app);
        assert_eq!(
            app.gh().calls(),
            [GithubCall::CheckAuth, GithubCall::UnreadNotificationCount]
        );
        assert_eq!(app.app().unread(), UnreadState::Count(3));
        assert!(!app.app().notifications_refreshing());
        assert!(status_row(&mut app).ends_with("• Unread Notifications: 3│"));
    }

    #[test]
    fn the_interval_refreshes_every_minute() {
        let gh = FakeGithub::new().with_unread_count(Ok(0));
        let mut app = TestApp::with_github(enabled(), gh);
        settle(&mut app);
        assert!(status_row(&mut app).ends_with("No unread notifications│"));

        app.gh().set_unread_count(Ok(50));
        app.advance(Duration::from_secs(59));
        settle(&mut app);
        assert_eq!(app.gh().calls().len(), 2);
        app.advance(Duration::from_secs(1));
        settle(&mut app);
        assert_eq!(app.gh().calls().len(), 4);
        assert_eq!(app.app().unread(), UnreadState::Count(50));
        assert!(status_row(&mut app).ends_with("• Unread Notifications: 50+│"));
        app.advance(REFRESH_INTERVAL);
        settle(&mut app);
        assert_eq!(app.gh().calls().len(), 6);
    }

    #[test]
    fn an_auth_failure_toasts_and_logs_and_keeps_the_count() {
        let gh = FakeGithub::new().with_unread_count(Ok(3));
        let mut app = TestApp::with_github(enabled(), gh);
        settle(&mut app);
        assert_eq!(app.app().unread(), UnreadState::Count(3));

        app.gh().set_auth_error(Some(logged_out()));
        app.advance(REFRESH_INTERVAL);
        let out = logs(|| settle(&mut app));
        assert_eq!(
            out,
            "DEBUG Notifications auth check failed: To proceed, please log into the Github CLI via `gh auth login`\n\
             ERROR Cannot load notifications - please login to the gh CLI: gh auth login\n"
        );
        let toast = app.app().toasts().iter().next().expect("the error toast");
        assert_eq!(toast.severity, Severity::Error);
        assert_eq!(toast.title.as_deref(), Some("Failed to Load Notifications"));
        assert_eq!(
            toast.message.to_string(),
            "Cannot load notifications - please login to the gh CLI: gh auth login"
        );
        assert_eq!(app.app().unread(), UnreadState::Count(3));
        assert!(!app.app().notifications_refreshing());
        // No count was fetched after the failed check.
        assert_eq!(app.gh().calls().last(), Some(&GithubCall::CheckAuth));
    }

    #[test]
    fn a_count_error_shows_zero_and_logs() {
        let gh = FakeGithub::new().with_unread_count(Err(GhError::Http {
            status: 502,
            message: "Bad Gateway".into(),
            body: Vec::new(),
        }));
        let mut app = TestApp::with_github(enabled(), gh);
        let out = logs(|| settle(&mut app));
        assert_eq!(
            out,
            "ERROR Failed to retrieve notifications from the Github API\n\
             DEBUG Fetching notifications failed: GitHub API request failed (HTTP 502): Bad Gateway\n"
        );
        assert_eq!(app.app().unread(), UnreadState::Count(0));
        assert!(app.app().toasts().is_empty());
    }

    #[test]
    fn a_refresh_while_one_runs_is_skipped() {
        let mut app = TestApp::new(enabled());
        assert!(app.app().notifications_refreshing());
        let out = logs(|| assert_eq!(app.app_mut().update(Action::RefreshNotifications), []));
        assert_eq!(
            out,
            "DEBUG Skipping the notifications refresh: one is already running\n"
        );
        // A tick while it runs is skipped too: only the first check and its count reach the fake.
        app.advance(REFRESH_INTERVAL);
        settle(&mut app);
        assert_eq!(
            app.gh().calls(),
            [GithubCall::CheckAuth, GithubCall::UnreadNotificationCount]
        );
    }

    #[test]
    fn refreshing_while_disabled_blanks_the_count_without_calls() {
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Ok(3)));
        settle(&mut app);
        assert_eq!(app.app().unread(), UnreadState::Count(3));
        app.with_cx(|cx| cx.config_mut().notifications.enabled = false);
        settle(&mut app);

        assert_eq!(app.app_mut().update(Action::RefreshNotifications), []);
        assert_eq!(app.app().unread(), UnreadState::Hidden);
        // The interval keeps ticking, but calls nothing.
        let calls = app.gh().calls().len();
        app.advance(REFRESH_INTERVAL * 3);
        settle(&mut app);
        assert_eq!(app.gh().calls().len(), calls);
        assert!(!status_row(&mut app).contains("nread"));
    }

    #[test]
    fn responses_to_other_requests_are_not_claimed() {
        let mut app = TestApp::new(enabled());
        let other = app.with_cx(|cx| cx.request(GithubRequest::CheckAuth));
        let out = logs(|| settle(&mut app));
        assert!(
            out.contains(&format!("Dropping the response to request {other}")),
            "{out}"
        );
        assert_eq!(app.app().unread(), UnreadState::Count(0));
    }

    #[test]
    fn set_current_repo_logs_and_shows_it() {
        let mut app = TestApp::new(Config::default());
        assert!(status_row(&mut app).starts_with("│No repository selected "));
        let repo = Repository {
            name: "lazy-fixture".into(),
            full_name: "parity-bot/lazy-fixture".into(),
            default_branch: Some("main".into()),
            private: false,
            archived: Some(false),
            owner: User {
                login: "parity-bot".into(),
                id: 1_000_001,
                name: None,
                html_url: "https://github.com/parity-bot".into(),
            },
            description: None,
            permissions: None,
        };
        let out = logs(|| app.app_mut().set_current_repo(repo.clone()));
        assert_eq!(out, "INFO Selected repo parity-bot/lazy-fixture\n");
        assert_eq!(app.app().current_repo(), Some(&repo));
        assert!(status_row(&mut app).starts_with("│Current repo: parity-bot/lazy-fixture "));
    }
}
