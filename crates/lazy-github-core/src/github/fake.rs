//! An in-memory [`Github`] for tests.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::de::Error as _;

use super::{Github, RepoListQuery};
use crate::gh::GhError;
use crate::models::{Repository, User};

/// A [`Github`] that returns canned results and records every call.
///
/// By default `check_auth` succeeds, `list_repositories` returns no repositories, `unread_notification_count` returns
/// 0, and `current_user` fails until [`Self::with_user`] sets one. Clones share the canned results and the call log.
#[derive(Debug, Clone, Default)]
pub struct FakeGithub {
    state: Arc<Mutex<State>>,
}

/// One call made to a [`FakeGithub`], with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubCall {
    /// [`Github::check_auth`].
    CheckAuth,
    /// [`Github::current_user`].
    CurrentUser,
    /// [`Github::list_repositories`].
    ListRepositories(RepoListQuery),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount,
}

#[derive(Debug, Default)]
struct State {
    auth_error: Option<GhError>,
    user: Option<Result<User, GhError>>,
    repositories: Option<Result<Vec<Repository>, GhError>>,
    unread_count: Option<Result<usize, GhError>>,
    latency: Option<Duration>,
    calls: Vec<GithubCall>,
}

impl FakeGithub {
    /// A fake that is logged in, has no current user and no repositories.
    pub fn new() -> Self {
        Self::default()
    }

    /// `current_user` returns `user`.
    #[must_use]
    pub fn with_user(self, user: User) -> Self {
        self.lock().user = Some(Ok(user));
        self
    }

    /// `current_user` fails with `error`.
    #[must_use]
    pub fn with_user_error(self, error: GhError) -> Self {
        self.lock().user = Some(Err(error));
        self
    }

    /// `list_repositories` returns `repositories`, whatever the query.
    #[must_use]
    pub fn with_repositories(self, repositories: Vec<Repository>) -> Self {
        self.lock().repositories = Some(Ok(repositories));
        self
    }

    /// `list_repositories` fails with `error`.
    #[must_use]
    pub fn with_repositories_error(self, error: GhError) -> Self {
        self.lock().repositories = Some(Err(error));
        self
    }

    /// `unread_notification_count` returns `count`.
    #[must_use]
    pub fn with_unread_count(self, count: Result<usize, GhError>) -> Self {
        self.set_unread_count(count);
        self
    }

    /// From now on, `unread_notification_count` returns `count`. Clones see the change too.
    pub fn set_unread_count(&self, count: Result<usize, GhError>) {
        self.lock().unread_count = Some(count);
    }

    /// `check_auth` fails with `error`.
    #[must_use]
    pub fn with_auth_error(self, error: GhError) -> Self {
        self.lock().auth_error = Some(error);
        self
    }

    /// From now on, `check_auth` fails with `error`, or succeeds for `None`. Clones see the change too, so a test can
    /// log in while the app holds the fake.
    pub fn set_auth_error(&self, error: Option<GhError>) {
        self.lock().auth_error = error;
    }

    /// Every call is recorded at once but returns only after `latency`, on tokio's clock. With
    /// `tokio::time::pause()` this keeps a request in flight, so a test can cancel it.
    #[must_use]
    pub fn with_latency(self, latency: Duration) -> Self {
        self.lock().latency = Some(latency);
        self
    }

    /// Every call made so far, oldest first.
    pub fn calls(&self) -> Vec<GithubCall> {
        self.lock().calls.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking test thread can't leave `State` half-updated, so a poisoned lock is still usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, call: GithubCall) -> MutexGuard<'_, State> {
        let mut state = self.lock();
        state.calls.push(call);
        state
    }
}

impl FakeGithub {
    /// Records `call`, computes its result from the canned state, then waits out the latency. The lock is released
    /// before waiting, so the future stays `Send`.
    async fn respond<T>(&self, call: GithubCall, result: impl FnOnce(&State) -> T) -> T {
        let (result, latency) = {
            let state = self.record(call);
            (result(&state), state.latency)
        };
        if let Some(latency) = latency {
            tokio::time::sleep(latency).await;
        }
        result
    }
}

impl Github for FakeGithub {
    async fn check_auth(&self) -> Result<(), GhError> {
        self.respond(GithubCall::CheckAuth, |state| match &state.auth_error {
            Some(error) => Err(duplicate(error)),
            None => Ok(()),
        })
        .await
    }

    async fn current_user(&self) -> Result<User, GhError> {
        self.respond(GithubCall::CurrentUser, |state| match &state.user {
            Some(Ok(user)) => Ok(user.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Command {
                code: None,
                stderr: "FakeGithub has no current user; call with_user".to_owned(),
            }),
        })
        .await
    }

    async fn list_repositories(&self, query: RepoListQuery) -> Result<Vec<Repository>, GhError> {
        self.respond(GithubCall::ListRepositories(query), |state| match &state.repositories {
            Some(Ok(repositories)) => Ok(repositories.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(Vec::new()),
        })
        .await
    }

    async fn unread_notification_count(&self) -> Result<usize, GhError> {
        self.respond(GithubCall::UnreadNotificationCount, |state| match &state.unread_count {
            Some(Ok(count)) => Ok(*count),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(0),
        })
        .await
    }
}

/// A copy of `error`, so a canned error can be returned on every call. `GhError` isn't `Clone` because its
/// `serde_json` and `io` sources aren't; those are rebuilt from their message (and kind).
fn duplicate(error: &GhError) -> GhError {
    match error {
        GhError::NotInstalled => GhError::NotInstalled,
        GhError::NotAuthenticated { hostname, detail } => GhError::NotAuthenticated {
            hostname: hostname.clone(),
            detail: detail.clone(),
        },
        GhError::Http { status, message, body } => GhError::Http {
            status: *status,
            message: message.clone(),
            body: body.clone(),
        },
        GhError::Command { code, stderr } => GhError::Command {
            code: *code,
            stderr: stderr.clone(),
        },
        GhError::Parse(message) => GhError::Parse(message.clone()),
        GhError::Decode(error) => GhError::Decode(serde_json::Error::custom(error)),
        GhError::Io(error) => GhError::Io(io::Error::new(error.kind(), error.to_string())),
        GhError::Timeout { after } => GhError::Timeout { after: *after },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{RepoKind, RepoSort, SortDirection};

    fn octocat() -> User {
        User {
            login: "octocat".into(),
            id: 1,
            name: Some("The Octocat".into()),
            html_url: "https://github.com/octocat".into(),
        }
    }

    fn repo(name: &str) -> Repository {
        Repository {
            name: name.into(),
            full_name: format!("octocat/{name}"),
            default_branch: Some("main".into()),
            private: false,
            archived: Some(false),
            owner: octocat(),
            description: None,
            permissions: None,
        }
    }

    #[tokio::test]
    async fn canned_results_and_call_log() {
        let github = FakeGithub::new()
            .with_user(octocat())
            .with_repositories(vec![repo("a"), repo("b")]);
        // Clones share the log, as clones handed to spawned tasks would.
        let clone = github.clone();

        clone.check_auth().await.unwrap();
        assert_eq!(clone.current_user().await.unwrap(), octocat());
        let owned = RepoListQuery {
            kind: RepoKind::Owner,
            sort: RepoSort::Updated,
            direction: SortDirection::Desc,
        };
        let repos = clone.list_repositories(owned).await.unwrap();
        assert_eq!(repos, [repo("a"), repo("b")]);
        github.current_user().await.unwrap();
        assert_eq!(
            github
                .clone()
                .with_unread_count(Ok(7))
                .unread_notification_count()
                .await
                .unwrap(),
            7
        );

        assert_eq!(
            github.calls(),
            [
                GithubCall::CheckAuth,
                GithubCall::CurrentUser,
                GithubCall::ListRepositories(owned),
                GithubCall::CurrentUser,
                GithubCall::UnreadNotificationCount,
            ]
        );
    }

    #[tokio::test]
    async fn defaults() {
        let github = FakeGithub::new();
        github.check_auth().await.unwrap();
        assert!(github.list_repositories(RepoListQuery::all()).await.unwrap().is_empty());
        assert_eq!(github.unread_notification_count().await.unwrap(), 0);
        assert!(matches!(github.current_user().await, Err(GhError::Command { .. })));
    }

    #[tokio::test]
    async fn canned_errors_repeat() {
        let github = FakeGithub::new()
            .with_auth_error(GhError::NotAuthenticated {
                hostname: None,
                detail: "logged out".into(),
            })
            .with_user_error(GhError::Io(io::Error::new(io::ErrorKind::BrokenPipe, "pipe")))
            .with_repositories_error(GhError::Http {
                status: 502,
                message: "Bad Gateway".into(),
                body: Vec::new(),
            })
            .with_unread_count(Err(GhError::Parse("no status line".into())));
        for _ in 0..2 {
            assert!(matches!(
                github.check_auth().await,
                Err(GhError::NotAuthenticated { detail, .. }) if detail == "logged out"
            ));
            assert!(matches!(
                github.current_user().await,
                Err(GhError::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe && e.to_string() == "pipe"
            ));
            assert!(matches!(
                github.list_repositories(RepoListQuery::all()).await,
                Err(GhError::Http { status: 502, .. })
            ));
            assert!(matches!(
                github.unread_notification_count().await,
                Err(GhError::Parse(_))
            ));
        }
        assert_eq!(github.calls().len(), 8);
    }

    #[test]
    fn duplicate_keeps_every_message() {
        let decode = serde_json::from_str::<u32>("\"x\"").unwrap_err();
        let errors = [
            GhError::NotInstalled,
            GhError::NotAuthenticated {
                hostname: Some("ghe.example.com".into()),
                detail: String::new(),
            },
            GhError::Http {
                status: 404,
                message: "Not Found".into(),
                body: b"{}".to_vec(),
            },
            GhError::Command {
                code: Some(1),
                stderr: "boom".into(),
            },
            GhError::Parse("no status line".into()),
            GhError::Decode(decode),
            GhError::Io(io::Error::other("broken")),
            GhError::Timeout {
                after: Duration::from_secs(30),
            },
        ];
        for error in &errors {
            assert_eq!(duplicate(error).to_string(), error.to_string());
        }
    }
}
