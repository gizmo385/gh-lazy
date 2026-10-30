//! GitHub's rate limits: one toast per limit window, and the notifications poll paused until the limit lifts.
//!
//! Python treats a rate-limited response as any other HTTP error (decision `rate-limit-toast`). Requests the user
//! asks for still run while limited, each failing with its own error. A `--paginate` call has no headers to say when
//! the limit lifts, so its window is a minute ([`lazy_github_core::gh::RATE_LIMIT_FALLBACK`], #333).

use jiff::Timestamp;
use lazy_github_core::gh::GhError;

use crate::component::Cx;
use crate::toast::Toast;

/// The toast's title.
pub const TOAST_TITLE: &str = "Rate limited";

/// The toast's message for a limit lifting at `at`, a local `HH:MM`.
pub fn toast_message(at: &str) -> String {
    format!("GitHub API rate limit reached. Try again at {at}")
}

/// Whether GitHub is rate limiting the app, and until when. `App` owns one for the life of the app.
#[derive(Debug, Default)]
pub struct RateLimitWatch {
    /// When the latest limit lifts. Kept after it has passed, which is harmless: [`Self::is_limited`] compares.
    until: Option<Timestamp>,
}

impl RateLimitWatch {
    /// A request failed with `error`. When that's a rate limit, records when it lifts and, unless the app was already
    /// limited, shows the toast. A second limited response in the same window only moves the end later.
    pub fn observe(&mut self, error: &GhError, cx: &mut Cx<'_>) {
        let Some(limit) = error.rate_limit() else {
            return;
        };
        let now = cx.local_now();
        let reset = limit.reset(now.timestamp());
        if let Some(until) = self.until.as_mut().filter(|until| now.timestamp() < **until) {
            *until = (*until).max(reset);
            return;
        }
        self.until = Some(reset);
        let at = reset.to_zoned(now.time_zone().clone()).strftime("%H:%M").to_string();
        cx.notify(Toast::error(toast_message(&at)).with_title(TOAST_TITLE));
    }

    /// Whether a limit is in force at `now`.
    pub fn is_limited(&self, now: Timestamp) -> bool {
        self.until.is_some_and(|until| now < until)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::FutureExt as _;
    use lazy_github_core::config::Config;
    use lazy_github_core::github::{FakeGithub, GithubCall};

    use super::*;
    use crate::action::Action;
    use crate::components::status_bar::UnreadState;
    use crate::notifications::REFRESH_INTERVAL;
    use crate::test_util::{TestApp, logs};
    use crate::toast::Severity;

    /// The fake clock's start, 2026-10-25 12:00:00 UTC, in epoch seconds.
    const START: i64 = 1_792_929_600;

    fn enabled() -> Config {
        let mut config = Config::default();
        config.notifications.enabled = true;
        config
    }

    fn http(status: u16, headers: &[(&str, String)]) -> GhError {
        GhError::Http {
            status,
            message: "API rate limit exceeded for user ID 1.".into(),
            body: Vec::new(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
        }
    }

    /// A primary-limit 403 that lifts `minutes` after the clock's start.
    fn primary(minutes: i64) -> GhError {
        http(
            403,
            &[
                ("X-RateLimit-Remaining", "0".to_owned()),
                ("X-RateLimit-Reset", (START + minutes * 60).to_string()),
            ],
        )
    }

    fn settle(app: &mut TestApp) {
        app.settle().now_or_never().expect("the fake answers at once");
    }

    fn rate_limit_toasts(app: &TestApp) -> Vec<String> {
        app.app()
            .toasts()
            .iter()
            .filter(|toast| toast.title.as_deref() == Some(TOAST_TITLE))
            .map(|toast| toast.message.to_string())
            .collect()
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
    fn a_limited_response_toasts_with_the_reset_time() {
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Err(primary(10))));
        settle(&mut app);
        let toast = app.app().toasts().iter().next().expect("the toast");
        assert_eq!(toast.severity, Severity::Error);
        assert_eq!(toast.title.as_deref(), Some("Rate limited"));
        assert_eq!(
            toast.message.to_string(),
            "GitHub API rate limit reached. Try again at 12:10"
        );
    }

    #[test]
    fn retry_after_counts_from_the_clock() {
        let limited = http(429, &[("Retry-After", "90".to_owned())]);
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Err(limited)));
        settle(&mut app);
        assert_eq!(
            rate_limit_toasts(&app),
            ["GitHub API rate limit reached. Try again at 12:01"]
        );
    }

    #[test]
    fn a_permission_403_is_only_its_own_error() {
        let forbidden = http(403, &[("X-RateLimit-Remaining", "4999".to_owned())]);
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Err(forbidden)));
        settle(&mut app);
        assert!(rate_limit_toasts(&app).is_empty());
        // The poll carries on.
        app.advance(REFRESH_INTERVAL);
        settle(&mut app);
        assert_eq!(refreshes(&app), 2);
    }

    #[test]
    fn two_limited_calls_in_one_window_toast_once() {
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Err(primary(10))));
        settle(&mut app);
        // The user's own refresh still makes its calls, and is limited too.
        app.dispatch(Action::RefreshNotifications);
        settle(&mut app);
        assert_eq!(refreshes(&app), 2);
        assert_eq!(rate_limit_toasts(&app).len(), 1);

        // Once the window has passed, a new limit toasts again.
        app.gh().set_unread_count(Err(primary(30)));
        app.advance(Duration::from_mins(11));
        settle(&mut app);
        assert_eq!(
            rate_limit_toasts(&app),
            ["GitHub API rate limit reached. Try again at 12:30"]
        );
    }

    #[test]
    fn a_later_reset_in_the_window_extends_it_without_a_toast() {
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Err(primary(2))));
        settle(&mut app);
        app.gh().set_unread_count(Err(primary(5)));
        app.dispatch(Action::RefreshNotifications);
        settle(&mut app);
        assert_eq!(rate_limit_toasts(&app).len(), 1);
        // Paused until 12:05, not 12:02.
        app.gh().set_unread_count(Ok(1));
        app.advance(Duration::from_mins(4));
        settle(&mut app);
        assert_eq!(refreshes(&app), 2);
        app.advance(Duration::from_mins(1));
        settle(&mut app);
        assert_eq!(refreshes(&app), 3);
    }

    #[test]
    fn the_poll_skips_until_the_reset_then_polls() {
        let mut app = TestApp::with_github(enabled(), FakeGithub::new().with_unread_count(Err(primary(3))));
        settle(&mut app);
        assert_eq!(refreshes(&app), 1);
        app.gh().set_unread_count(Ok(4));

        // The ticks at 12:01 and 12:02 are skipped.
        let out = logs(|| {
            app.advance(REFRESH_INTERVAL * 2);
            settle(&mut app);
        });
        assert_eq!(
            out,
            "DEBUG Skipping the notifications poll: GitHub is rate limiting requests\n".repeat(2)
        );
        assert_eq!(refreshes(&app), 1);

        // The tick at 12:03, when the limit lifts, polls again.
        app.advance(REFRESH_INTERVAL);
        settle(&mut app);
        assert_eq!(refreshes(&app), 2);
        assert_eq!(app.app().unread(), UnreadState::Count(4));
    }

    // #333: paginated calls and the PR diff.

    /// A limit as a `--paginate` call sees it: GitHub's message, no headers.
    fn paginated(status: u16, message: &str) -> GhError {
        GhError::Http {
            status,
            message: message.into(),
            body: Vec::new(),
            headers: Vec::new(),
        }
    }

    #[test]
    fn a_paginated_limit_toasts_a_minute_ahead_and_pauses_the_poll_that_long() {
        let limited = paginated(403, "API rate limit exceeded for user ID 1.");
        let gh = FakeGithub::new()
            .with_repositories_error(limited)
            .with_unread_count(Ok(2));
        let mut app = TestApp::loading(enabled(), gh);
        settle(&mut app);
        assert_eq!(
            rate_limit_toasts(&app),
            ["GitHub API rate limit reached. Try again at 12:01"]
        );
        // The poll's startup refresh ran with the list; the tick at 12:01, when the fallback window ends, polls.
        let before = refreshes(&app);
        app.advance(REFRESH_INTERVAL);
        settle(&mut app);
        assert_eq!(refreshes(&app), before + 1);
    }

    #[test]
    fn a_paginated_secondary_limit_counts_too() {
        let limited = paginated(
            403,
            "You have exceeded a secondary rate limit. Please wait a few minutes before you try again.",
        );
        let mut app = TestApp::loading(Config::default(), FakeGithub::new().with_repositories_error(limited));
        settle(&mut app);
        assert_eq!(rate_limit_toasts(&app).len(), 1);
    }

    #[test]
    fn a_paginated_permission_403_is_only_its_own_error() {
        let forbidden = paginated(403, "Resource not accessible by integration");
        let mut app = TestApp::loading(Config::default(), FakeGithub::new().with_repositories_error(forbidden));
        settle(&mut app);
        assert!(rate_limit_toasts(&app).is_empty());
    }

    #[test]
    fn a_limited_diff_toasts_from_its_headers() {
        use lazy_github_core::github::DiffFetchError;

        let error = DiffFetchError::from(primary(10));
        let response = crate::effect::GithubResponse::PullRequestDiff(Err(error));
        assert!(response.error().is_some_and(|error| error.rate_limit().is_some()));
        let mut app = TestApp::new(Config::default());
        app.send(crate::event::Event::Github {
            id: crate::effect::RequestId::new(999),
            response,
        });
        assert_eq!(
            rate_limit_toasts(&app),
            ["GitHub API rate limit reached. Try again at 12:10"]
        );
    }
}
