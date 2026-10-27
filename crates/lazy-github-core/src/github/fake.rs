//! An in-memory [`Github`] for tests.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::de::Error as _;

use super::{DiffFetchError, Github, IssueListQuery, PageQuery, RepoListQuery};
use crate::gh::GhError;
use crate::models::{FullPullRequest, FullUser, IssueListItem, Repository, User, Workflow, WorkflowJob, WorkflowRun};

/// A [`Github`] that returns canned results and records every call.
///
/// By default `check_auth` succeeds, `list_repositories` returns no repositories, `unread_notification_count` returns
/// 0, `list_issues`, `list_workflows` and `list_workflow_runs` return an empty page, `repo_readme` finds no README, `repository` finds no repository, `pull_request_diff` finds no diff, `pull_request` fails with a 404, `user` finds no user (a 404), `user_repos` returns none, `workflow_run_jobs` returns no jobs, and `current_user` fails until [`Self::with_user`] sets one. Clones share the canned results and the call log.
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
    /// [`Github::list_issues`].
    ListIssues {
        /// The repository listed.
        repo: Repository,
        /// The filters and page.
        query: IssueListQuery,
    },
    /// [`Github::repo_readme`], with the repo's full name.
    RepoReadme(String),
    /// [`Github::repository`], with the full name asked for.
    Repository(String),
    /// [`Github::list_workflows`].
    ListWorkflows {
        /// The repository listed.
        repo: Repository,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::list_workflow_runs`].
    ListWorkflowRuns {
        /// The repository listed.
        repo: Repository,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::pull_request_diff`].
    PullRequestDiff {
        /// The pull request's repository.
        repo: Repository,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::user`], with the login asked for.
    User(String),
    /// [`Github::user_repos`], with the login asked for.
    UserRepos(String),
    /// [`Github::pull_request`].
    PullRequest {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::workflow_run_jobs`], with the run's id.
    WorkflowRunJobs(u64),
}

#[derive(Debug, Default)]
struct State {
    auth_error: Option<GhError>,
    user: Option<Result<User, GhError>>,
    repositories: Option<Result<Vec<Repository>, GhError>>,
    unread_count: Option<Result<usize, GhError>>,
    /// Keyed by `full_name` and page.
    issue_pages: HashMap<(String, u32), Result<Vec<IssueListItem>, GhError>>,
    /// Keyed by `full_name`.
    readmes: HashMap<String, Result<Option<String>, GhError>>,
    /// Keyed by `full_name`.
    repository: HashMap<String, Result<Option<Repository>, GhError>>,
    /// Keyed by `full_name` and page.
    workflow_pages: HashMap<(String, u32), Result<Vec<Workflow>, GhError>>,
    /// Keyed by `full_name` and page.
    run_pages: HashMap<(String, u32), Result<Vec<WorkflowRun>, GhError>>,
    /// Keyed by `full_name` and pull request number.
    diffs: HashMap<(String, u64), Result<String, DiffFetchError>>,
    /// Keyed by login.
    profiles: HashMap<String, Result<FullUser, GhError>>,
    /// Keyed by login.
    user_repos: HashMap<String, Result<Vec<Repository>, GhError>>,
    /// Keyed by `full_name` and number.
    pull_requests: HashMap<(String, u64), Result<FullPullRequest, GhError>>,
    /// Keyed by run id.
    run_jobs: HashMap<u64, Result<Vec<WorkflowJob>, GhError>>,
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

    /// `list_issues` returns `page` for page `page_number` of the repository named `full_name`, whatever the
    /// filters. Pages not set are empty.
    #[must_use]
    pub fn with_issue_page(self, full_name: &str, page_number: u32, page: Result<Vec<IssueListItem>, GhError>) -> Self {
        self.lock()
            .issue_pages
            .insert((full_name.to_owned(), page_number), page);
        self
    }

    /// `repo_readme` returns `readme` for the repository named `full_name`. Repos not set have no README.
    #[must_use]
    pub fn with_readme(self, full_name: &str, readme: Result<Option<String>, GhError>) -> Self {
        self.lock().readmes.insert(full_name.to_owned(), readme);
        self
    }

    /// `repository` returns `repository` for the name `full_name`. Names not set find no repository.
    #[must_use]
    pub fn with_repository(self, full_name: &str, repository: Result<Option<Repository>, GhError>) -> Self {
        self.lock().repository.insert(full_name.to_owned(), repository);
        self
    }

    /// `list_workflows` returns `page` for page `page_number` of the repository named `full_name`. Pages not set are
    /// empty.
    #[must_use]
    pub fn with_workflow_page(self, full_name: &str, page_number: u32, page: Result<Vec<Workflow>, GhError>) -> Self {
        self.lock()
            .workflow_pages
            .insert((full_name.to_owned(), page_number), page);
        self
    }

    /// `list_workflow_runs` returns `page` for page `page_number` of the repository named `full_name`. Pages not set
    /// are empty.
    #[must_use]
    pub fn with_run_page(self, full_name: &str, page_number: u32, page: Result<Vec<WorkflowRun>, GhError>) -> Self {
        self.lock().run_pages.insert((full_name.to_owned(), page_number), page);
        self
    }

    /// `pull_request_diff` returns `diff` for pull request `number` of the repository named `full_name`. Pull requests
    /// not set have no diff ([`DiffFetchError::NotFound`]).
    #[must_use]
    pub fn with_pull_request_diff(self, full_name: &str, number: u64, diff: Result<String, DiffFetchError>) -> Self {
        self.lock().diffs.insert((full_name.to_owned(), number), diff);
        self
    }

    /// `user` returns `profile` for `login`. Logins not set get a 404.
    #[must_use]
    pub fn with_profile(self, login: &str, profile: Result<FullUser, GhError>) -> Self {
        self.lock().profiles.insert(login.to_owned(), profile);
        self
    }

    /// `user_repos` returns `repos` for `login`. Logins not set have none.
    #[must_use]
    pub fn with_user_repos(self, login: &str, repos: Result<Vec<Repository>, GhError>) -> Self {
        self.lock().user_repos.insert(login.to_owned(), repos);
        self
    }

    /// `pull_request` returns `pull_request` for number `number` of the repository named `full_name`. Numbers not set
    /// fail with a 404.
    #[must_use]
    pub fn with_pull_request(
        self,
        full_name: &str,
        number: u64,
        pull_request: Result<FullPullRequest, GhError>,
    ) -> Self {
        self.lock()
            .pull_requests
            .insert((full_name.to_owned(), number), pull_request);
        self
    }

    /// `workflow_run_jobs` returns `jobs` for run `run_id`, whatever the repository. Runs not set have no jobs.
    #[must_use]
    pub fn with_run_jobs(self, run_id: u64, jobs: Result<Vec<WorkflowJob>, GhError>) -> Self {
        self.lock().run_jobs.insert(run_id, jobs);
        self
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

    async fn list_issues(&self, repo: &Repository, query: IssueListQuery) -> Result<Vec<IssueListItem>, GhError> {
        let key = (repo.full_name.clone(), query.page);
        let call = GithubCall::ListIssues {
            repo: repo.clone(),
            query,
        };
        self.respond(call, |state| match state.issue_pages.get(&key) {
            Some(Ok(items)) => Ok(items.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(Vec::new()),
        })
        .await
    }

    async fn repo_readme(&self, full_name: &str) -> Result<Option<String>, GhError> {
        self.respond(GithubCall::RepoReadme(full_name.to_owned()), |state| {
            match state.readmes.get(full_name) {
                Some(Ok(readme)) => Ok(readme.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Ok(None),
            }
        })
        .await
    }

    async fn repository(&self, full_name: &str) -> Result<Option<Repository>, GhError> {
        self.respond(GithubCall::Repository(full_name.to_owned()), |state| {
            match state.repository.get(full_name) {
                Some(Ok(repository)) => Ok(repository.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Ok(None),
            }
        })
        .await
    }

    async fn list_workflows(&self, repo: &Repository, page: PageQuery) -> Result<Vec<Workflow>, GhError> {
        let key = (repo.full_name.clone(), page.page);
        let call = GithubCall::ListWorkflows {
            repo: repo.clone(),
            page,
        };
        self.respond(call, |state| canned_page(state.workflow_pages.get(&key)))
            .await
    }

    async fn list_workflow_runs(&self, repo: &Repository, page: PageQuery) -> Result<Vec<WorkflowRun>, GhError> {
        let key = (repo.full_name.clone(), page.page);
        let call = GithubCall::ListWorkflowRuns {
            repo: repo.clone(),
            page,
        };
        self.respond(call, |state| canned_page(state.run_pages.get(&key))).await
    }

    async fn pull_request_diff(&self, repo: &Repository, number: u64) -> Result<String, DiffFetchError> {
        let key = (repo.full_name.clone(), number);
        let call = GithubCall::PullRequestDiff {
            repo: repo.clone(),
            number,
        };
        self.respond(call, |state| match state.diffs.get(&key) {
            Some(Ok(diff)) => Ok(diff.clone()),
            Some(Err(error)) => Err(duplicate_diff_error(error)),
            None => Err(DiffFetchError::NotFound),
        })
        .await
    }

    async fn user(&self, login: &str) -> Result<FullUser, GhError> {
        self.respond(GithubCall::User(login.to_owned()), |state| {
            match state.profiles.get(login) {
                Some(Ok(profile)) => Ok(profile.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Err(GhError::Http {
                    status: 404,
                    message: "Not Found".to_owned(),
                    body: Vec::new(),
                }),
            }
        })
        .await
    }

    async fn user_repos(&self, login: &str) -> Result<Vec<Repository>, GhError> {
        self.respond(GithubCall::UserRepos(login.to_owned()), |state| {
            canned_page(state.user_repos.get(login))
        })
        .await
    }

    async fn pull_request(&self, repo: &Repository, number: u64) -> Result<FullPullRequest, GhError> {
        let key = (repo.full_name.clone(), number);
        let call = GithubCall::PullRequest {
            full_name: repo.full_name.clone(),
            number,
        };
        self.respond(call, |state| match state.pull_requests.get(&key) {
            Some(Ok(pull_request)) => Ok(pull_request.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Http {
                status: 404,
                message: "Not Found".to_owned(),
                body: Vec::new(),
            }),
        })
        .await
    }

    async fn workflow_run_jobs(&self, _repo: &Repository, run_id: u64) -> Result<Vec<WorkflowJob>, GhError> {
        self.respond(GithubCall::WorkflowRunJobs(run_id), |state| {
            canned_page(state.run_jobs.get(&run_id))
        })
        .await
    }
}

/// A copy of `error`, as [`duplicate`] makes for a [`GhError`].
fn duplicate_diff_error(error: &DiffFetchError) -> DiffFetchError {
    match error {
        DiffFetchError::NotFound => DiffFetchError::NotFound,
        DiffFetchError::TooLarge => DiffFetchError::TooLarge,
        DiffFetchError::Http(status) => DiffFetchError::Http(*status),
        DiffFetchError::Gh(error) => DiffFetchError::Gh(duplicate(error)),
    }
}

/// A canned page, or an empty one when none was set.
fn canned_page<T: Clone>(page: Option<&Result<Vec<T>, GhError>>) -> Result<Vec<T>, GhError> {
    match page {
        Some(Ok(items)) => Ok(items.clone()),
        Some(Err(error)) => Err(duplicate(error)),
        None => Ok(Vec::new()),
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

    fn issue_item(number: u64) -> IssueListItem {
        let item = serde_json::json!({
            "id": number,
            "number": number,
            "locked": false,
            "state": "open",
            "title": "An issue",
            "user": {"login": "octocat", "id": 1, "html_url": "https://github.com/octocat"},
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "comments_url": "https://api.github.com/repos/octocat/a/issues/1/comments",
            "html_url": "https://github.com/octocat/a/issues/1",
        });
        IssueListItem::from_api_item(item, &repo("a")).unwrap()
    }

    #[tokio::test]
    async fn issue_pages_by_repo_and_page() {
        use crate::config::{IssueOwnerFilter, IssueStateFilter};

        let github = FakeGithub::new()
            .with_issue_page("octocat/a", 1, Ok(vec![issue_item(2), issue_item(1)]))
            .with_issue_page(
                "octocat/a",
                2,
                Err(GhError::Http {
                    status: 500,
                    message: "Server Error".into(),
                    body: Vec::new(),
                }),
            );
        let first = IssueListQuery::first_page(IssueStateFilter::Open, IssueOwnerFilter::Mine);
        let second = IssueListQuery { page: 2, ..first };

        let page = github.list_issues(&repo("a"), first).await.unwrap();
        assert_eq!(page, [issue_item(2), issue_item(1)]);
        assert!(matches!(
            github.list_issues(&repo("a"), second).await,
            Err(GhError::Http { status: 500, .. })
        ));
        // Unset pages and repositories are empty.
        assert!(
            github
                .list_issues(&repo("a"), IssueListQuery { page: 3, ..first })
                .await
                .unwrap()
                .is_empty()
        );
        assert!(github.list_issues(&repo("b"), first).await.unwrap().is_empty());

        assert_eq!(
            github.calls()[..2],
            [
                GithubCall::ListIssues {
                    repo: repo("a"),
                    query: first
                },
                GithubCall::ListIssues {
                    repo: repo("a"),
                    query: second
                },
            ]
        );
        assert_eq!(github.calls().len(), 4);
    }

    #[tokio::test]
    async fn readmes_by_repo() {
        let github = FakeGithub::new()
            .with_readme("octocat/a", Ok(Some("# A\n".into())))
            .with_readme(
                "octocat/b",
                Err(GhError::Timeout {
                    after: Duration::from_secs(1),
                }),
            );
        assert_eq!(github.repo_readme("octocat/a").await.unwrap().as_deref(), Some("# A\n"));
        assert!(matches!(
            github.repo_readme("octocat/b").await,
            Err(GhError::Timeout { .. })
        ));
        assert_eq!(github.repo_readme("octocat/c").await.unwrap(), None);
        assert_eq!(github.calls()[0], GithubCall::RepoReadme("octocat/a".into()));
    }

    #[tokio::test]
    async fn pull_requests_by_repo_and_number() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let pr = FullPullRequest::from_api(fixture["response"]["body"].clone(), &repo("a")).unwrap();
        let github = FakeGithub::new()
            .with_pull_request("octocat/a", 5, Ok(pr.clone()))
            .with_pull_request("octocat/a", 2, Err(GhError::NotInstalled));
        assert_eq!(github.pull_request(&repo("a"), 5).await.unwrap(), pr);
        assert!(matches!(
            github.pull_request(&repo("a"), 2).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(
            github.pull_request(&repo("b"), 5).await,
            Err(GhError::Http { status: 404, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::PullRequest {
                full_name: "octocat/a".into(),
                number: 5
            }
        );
    }

    #[tokio::test]
    async fn run_jobs_by_run_id() {
        let job: WorkflowJob = serde_json::from_value(serde_json::json!({
            "id": 7, "run_id": 1, "name": "build", "status": "queued", "html_url": ""
        }))
        .unwrap();
        let github = FakeGithub::new()
            .with_run_jobs(1, Ok(vec![job.clone()]))
            .with_run_jobs(2, Err(GhError::NotInstalled));
        assert_eq!(github.workflow_run_jobs(&repo("a"), 1).await.unwrap(), [job]);
        assert!(matches!(
            github.workflow_run_jobs(&repo("a"), 2).await,
            Err(GhError::NotInstalled)
        ));
        assert!(github.workflow_run_jobs(&repo("a"), 3).await.unwrap().is_empty());
        assert_eq!(github.calls()[0], GithubCall::WorkflowRunJobs(1));
    }

    #[tokio::test]
    async fn repositories_by_name() {
        let github = FakeGithub::new()
            .with_repository("octocat/a", Ok(Some(repo("a"))))
            .with_repository("octocat/b", Err(GhError::NotInstalled));
        assert_eq!(github.repository("octocat/a").await.unwrap(), Some(repo("a")));
        assert!(matches!(
            github.repository("octocat/b").await,
            Err(GhError::NotInstalled)
        ));
        assert_eq!(github.repository("octocat/c").await.unwrap(), None);
        assert_eq!(github.calls()[0], GithubCall::Repository("octocat/a".into()));
    }

    #[tokio::test]
    async fn workflow_and_run_pages_by_repo_and_page() {
        let github = FakeGithub::new().with_run_page(
            "octocat/a",
            2,
            Err(GhError::Http {
                status: 500,
                message: "Server Error".into(),
                body: Vec::new(),
            }),
        );
        let second = PageQuery {
            page: 2,
            ..PageQuery::default()
        };
        // Unset pages are empty.
        assert!(
            github
                .list_workflows(&repo("a"), PageQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            github
                .list_workflow_runs(&repo("a"), PageQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            github.list_workflow_runs(&repo("a"), second).await,
            Err(GhError::Http { status: 500, .. })
        ));
        assert_eq!(
            github.calls(),
            [
                GithubCall::ListWorkflows {
                    repo: repo("a"),
                    page: PageQuery::default()
                },
                GithubCall::ListWorkflowRuns {
                    repo: repo("a"),
                    page: PageQuery::default()
                },
                GithubCall::ListWorkflowRuns {
                    repo: repo("a"),
                    page: second
                },
            ]
        );
    }

    #[tokio::test]
    async fn pull_request_diffs_by_repo_and_number() {
        let github = FakeGithub::new()
            .with_pull_request_diff("octocat/a", 5, Ok("diff --git a/f b/f\n".into()))
            .with_pull_request_diff("octocat/a", 4, Err(DiffFetchError::TooLarge))
            .with_pull_request_diff("octocat/a", 6, Err(DiffFetchError::Gh(GhError::NotInstalled)));
        for _ in 0..2 {
            assert_eq!(
                github.pull_request_diff(&repo("a"), 5).await.unwrap(),
                "diff --git a/f b/f\n"
            );
            assert!(matches!(
                github.pull_request_diff(&repo("a"), 4).await,
                Err(DiffFetchError::TooLarge)
            ));
            assert!(matches!(
                github.pull_request_diff(&repo("a"), 6).await,
                Err(DiffFetchError::Gh(GhError::NotInstalled))
            ));
        }
        // Unset pull requests have no diff.
        assert!(matches!(
            github.pull_request_diff(&repo("b"), 5).await,
            Err(DiffFetchError::NotFound)
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::PullRequestDiff {
                repo: repo("a"),
                number: 5
            }
        );
        assert_eq!(github.calls().len(), 7);
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
