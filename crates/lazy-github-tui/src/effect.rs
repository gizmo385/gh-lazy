//! Side effects as data. `App::update` returns them and only the [`Runtime`](crate::runtime::Runtime) performs them.

use std::fmt;
use std::time::Duration;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{Github, RepoListQuery};
use lazy_github_core::models::{Repository, User};

/// Something the runtime does on the app's behalf.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Run `request` in the background. The result comes back as `Event::Github` with the same `id`.
    Github {
        /// Which request this is, so a stale response can be told apart from the one being waited for.
        id: RequestId,
        /// What to ask GitHub.
        request: GithubRequest,
    },
    /// Abort the request with this id, killing its `gh` child. Its response never arrives. A no-op once it finished.
    Cancel(RequestId),
    /// Save this snapshot of the config. Saves happen in order, and a newer pending save replaces an older one.
    SaveConfig(Box<Config>),
    /// Send `Event::Timer(id)` every `every`, first at `+every` (Textual's `set_interval`). A no-op if `id` is
    /// already running.
    StartInterval {
        /// The timer.
        id: TimerId,
        /// The period, which is also the delay before the first tick.
        every: Duration,
    },
    /// Send `Event::Timer(id)` once, after `after`. Replaces a running timer with the same id.
    ///
    /// A tick the replaced timer already sent may still be queued and arrive after this.
    StartTimeout {
        /// The timer.
        id: TimerId,
        /// The delay.
        after: Duration,
    },
    /// Stop the timer with this id. A no-op if it isn't running.
    ///
    /// A tick already sent may still be queued and arrive after this; owners that care check their own state.
    CancelTimer(TimerId),
    /// Ring the terminal bell (Textual's `App.bell`). Terminals may beep, flash, or do nothing.
    Bell,
    /// Stop the event loop after the current batch of events.
    Quit,
}

/// Identifies one GitHub request. Allocated by `App`, increasing monotonically, so ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(u64);

impl RequestId {
    pub(crate) fn new(n: u64) -> Self {
        Self(n)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Identifies a timer. Each id has one owner, which `App` routes `Event::Timer` to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimerId {
    /// Re-checking `gh auth status` while the authentication screen is up (#106).
    AuthPoll,
    /// Polling for notifications (#81).
    Notifications,
}

/// One [`Github`] call, as data. One variant per trait method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubRequest {
    /// [`Github::check_auth`].
    CheckAuth,
    /// [`Github::current_user`].
    CurrentUser,
    /// [`Github::list_repositories`].
    ListRepositories(RepoListQuery),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount,
}

/// The result of a [`GithubRequest`], in the variant of the same name.
#[derive(Debug)]
pub enum GithubResponse {
    /// [`Github::check_auth`].
    CheckAuth(Result<(), GhError>),
    /// [`Github::current_user`].
    CurrentUser(Result<User, GhError>),
    /// [`Github::list_repositories`].
    ListRepositories(Result<Vec<Repository>, GhError>),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount(Result<usize, GhError>),
}

impl GithubRequest {
    /// Performs the request against `gh`.
    pub async fn run<G: Github>(self, gh: &G) -> GithubResponse {
        match self {
            GithubRequest::CheckAuth => GithubResponse::CheckAuth(gh.check_auth().await),
            GithubRequest::CurrentUser => GithubResponse::CurrentUser(gh.current_user().await),
            GithubRequest::ListRepositories(query) => {
                GithubResponse::ListRepositories(gh.list_repositories(query).await)
            }
            GithubRequest::UnreadNotificationCount => {
                GithubResponse::UnreadNotificationCount(gh.unread_notification_count().await)
            }
        }
    }
}
