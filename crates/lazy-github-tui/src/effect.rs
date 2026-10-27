//! Side effects as data. `App::update` returns them and only the [`Runtime`](crate::runtime::Runtime) performs them.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::git::GitCli;
use lazy_github_core::github::{DiffFetchError, Github, IssueListQuery, PageQuery, RepoListQuery};
use lazy_github_core::models::{IssueListItem, RepoName, Repository, User, Workflow, WorkflowRun};

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
    /// Run `request` in the user's working directory, in the background. The result comes back as `Event::Git` with
    /// the same `id`.
    Git {
        /// Which request this is, from the same counter as GitHub requests.
        id: RequestId,
        /// What to ask git.
        request: GitRequest,
    },
    /// Abort the GitHub or git request with this id, killing its `gh` or `git` child. Its response never arrives. A
    /// no-op once it finished.
    Cancel(RequestId),
    /// Save this snapshot of the config. Saves happen in order, and a newer pending save replaces an older one.
    SaveConfig(Box<Config>),
    /// Read a table cache file (`lazy_github_core::cache::read`) off the async threads. The text comes back as
    /// `Event::CacheRead` with the same `id`, `None` when there's no file.
    ReadCache {
        /// Which read this is, from the same counter as GitHub requests.
        id: RequestId,
        /// The file.
        path: PathBuf,
    },
    /// Write a table cache file (`lazy_github_core::cache::write`), creating its directory. Writes to one path happen
    /// in order, and a newer pending write replaces an older one. A failed write is logged as a warning.
    WriteCache {
        /// The file.
        path: PathBuf,
        /// Its new text, from `lazy_github_core::cache::to_json`.
        text: String,
    },
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
    /// [`Github::list_issues`].
    ListIssues {
        /// The repo whose issues and pull requests to list.
        repo: Box<Repository>,
        /// The filters and page.
        query: IssueListQuery,
    },
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount,
    /// [`Github::repo_readme`] of the repo with this full name.
    RepoReadme(String),
    /// [`Github::repository`] with this full name.
    Repository(String),
    /// [`Github::list_workflows`].
    ListWorkflows {
        /// The repo whose workflows to list.
        repo: Box<Repository>,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::list_workflow_runs`].
    ListWorkflowRuns {
        /// The repo whose runs to list.
        repo: Box<Repository>,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::pull_request_diff`].
    PullRequestDiff {
        /// The pull request's repo.
        repo: Box<Repository>,
        /// The pull request's number.
        number: u64,
    },
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
    /// [`Github::list_issues`].
    ListIssues(Result<Vec<IssueListItem>, GhError>),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount(Result<usize, GhError>),
    /// [`Github::repo_readme`].
    RepoReadme(Result<Option<String>, GhError>),
    /// [`Github::repository`], boxed to keep this enum small.
    Repository(Result<Option<Box<Repository>>, GhError>),
    /// [`Github::list_workflows`].
    ListWorkflows(Result<Vec<Workflow>, GhError>),
    /// [`Github::list_workflow_runs`].
    ListWorkflowRuns(Result<Vec<WorkflowRun>, GhError>),
    /// [`Github::pull_request_diff`].
    PullRequestDiff(Result<String, DiffFetchError>),
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
            GithubRequest::ListIssues { repo, query } => GithubResponse::ListIssues(gh.list_issues(&repo, query).await),
            GithubRequest::UnreadNotificationCount => {
                GithubResponse::UnreadNotificationCount(gh.unread_notification_count().await)
            }
            GithubRequest::RepoReadme(full_name) => GithubResponse::RepoReadme(gh.repo_readme(&full_name).await),
            GithubRequest::Repository(full_name) => {
                GithubResponse::Repository(gh.repository(&full_name).await.map(|found| found.map(Box::new)))
            }
            GithubRequest::ListWorkflows { repo, page } => {
                GithubResponse::ListWorkflows(gh.list_workflows(&repo, page).await)
            }
            GithubRequest::ListWorkflowRuns { repo, page } => {
                GithubResponse::ListWorkflowRuns(gh.list_workflow_runs(&repo, page).await)
            }
            GithubRequest::PullRequestDiff { repo, number } => {
                GithubResponse::PullRequestDiff(gh.pull_request_diff(&repo, number).await)
            }
        }
    }
}

/// One `git` call in the user's working directory, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRequest {
    /// [`GitCli::origin_repo`]: the repo LazyGithub was launched in.
    OriginRepo,
}

/// The result of a [`GitRequest`], in the variant of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitResponse {
    /// [`GitCli::origin_repo`]: `None` when there's no current-directory repo, whatever the reason.
    OriginRepo(Option<RepoName>),
}

impl GitRequest {
    /// Performs the request against `git`.
    pub async fn run(self, git: &GitCli) -> GitResponse {
        match self {
            GitRequest::OriginRepo => GitResponse::OriginRepo(git.origin_repo().await),
        }
    }
}
