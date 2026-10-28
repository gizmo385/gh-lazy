//! The [`Github`] trait: every GitHub operation the app performs, typed per endpoint.
//!
//! [`GhCli`] implements it over the `gh` runner in [`crate::gh`]. With the `test-util` feature (or under `cfg(test)`),
//! [`FakeGithub`] implements it in memory for `update` tests.
//!
//! The trait grows one typed method per action as features need them (`list_pull_requests`, `merge_pull_request`,
//! ...), each implemented on both [`GhCli`] and [`FakeGithub`]. There is deliberately no generic `get(path)`.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::OnceCell;

use crate::config::{CacheSettings, Config, IssueOwnerFilter, IssueStateFilter, MergeMethod};
use crate::gh::{self, Accept, GhError, GhRequest, GhResponse, GhTimeouts, Method};
use crate::models::{
    CheckRun, CombinedStatus, CommentRef, FullPullRequest, FullUser, Issue, IssueComment, IssueListItem, IssueState,
    ReactionKind, Reactions, Repository, Review, ReviewComment, ReviewState, User, Workflow, WorkflowJob, WorkflowRun,
};

#[cfg(any(test, feature = "test-util"))]
mod fake;

#[cfg(any(test, feature = "test-util"))]
pub use fake::{FakeGithub, GithubCall};

/// Page size for `/user/repos`. Kept at Python's 50 so both apps request the same pages and share fixtures.
const REPOS_PER_PAGE: &str = "50";

/// Page size for `/notifications`: Python's `NOTIFICATIONS_PAGE_COUNT`. Only the first page is counted, so this is
/// also the most [`Github::unread_notification_count`] returns.
pub const NOTIFICATIONS_PER_PAGE: usize = 50;

/// Page size for `/repos/{owner}/{repo}/issues`: Python's `DEFAULT_PAGE_SIZE`, so both apps request the same pages.
pub const ISSUES_PER_PAGE: u32 = 30;

/// GitHub as the app sees it.
///
/// Implementations are cheap to clone and clones share state, so the effect runner can hand a clone to each spawned
/// task. Consumers are generic over `G: Github` rather than holding a `dyn Github`.
pub trait Github: Clone + Send + Sync + 'static {
    /// Checks that `gh` is installed and logged in.
    ///
    /// Fails with [`GhError::NotInstalled`] when `gh` can't be run and [`GhError::NotAuthenticated`] when it isn't
    /// logged in.
    fn check_auth(&self) -> impl Future<Output = Result<(), GhError>> + Send;

    /// The logged-in user. Fetched once; later calls return the same value.
    fn current_user(&self) -> impl Future<Output = Result<User, GhError>> + Send;

    /// Every repository matching `query` that the logged-in user can access, across all pages.
    fn list_repositories(&self, query: RepoListQuery) -> impl Future<Output = Result<Vec<Repository>, GhError>> + Send;

    /// How many unread notifications are on the first page of `/notifications`, from 0 to
    /// [`NOTIFICATIONS_PER_PAGE`]. A full page means there may be more. Never cached.
    ///
    /// Fails on an HTTP error status or a body that isn't a JSON array.
    fn unread_notification_count(&self) -> impl Future<Output = Result<usize, GhError>> + Send;

    /// One page of `repo`'s issues and pull requests matching `query`, in API order. Each item carries `repo`.
    ///
    /// For [`IssueOwnerFilter::Mine`] the login comes from [`Github::current_user`]; if that fails, so does this,
    /// without listing anything.
    ///
    /// Fails on an HTTP error status, or with [`GhError::Decode`] when any item doesn't decode.
    fn list_issues(
        &self,
        repo: &Repository,
        query: IssueListQuery,
    ) -> impl Future<Output = Result<Vec<IssueListItem>, GhError>> + Send;

    /// The README of `full_name` as raw markdown, verbatim, or `None` when the repo has none (404).
    ///
    /// Fails on any other HTTP error status or `gh` failure.
    fn repo_readme(&self, full_name: &str) -> impl Future<Output = Result<Option<String>, GhError>> + Send;

    /// The repository named `full_name` (`owner/name`, used verbatim in the path), or `None` when there's none
    /// (404). Cached for `cache.default_ttl`.
    ///
    /// Fails on any other HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn repository(&self, full_name: &str) -> impl Future<Output = Result<Option<Repository>, GhError>> + Send;

    /// One page of `repo`'s Actions workflows, in API order. Never cached.
    ///
    /// A body without a `workflows` list (missing or `null`) is an empty page. Fails on an HTTP error status, or with
    /// [`GhError::Decode`] when the body or any workflow doesn't decode.
    fn list_workflows(
        &self,
        repo: &Repository,
        page: PageQuery,
    ) -> impl Future<Output = Result<Vec<Workflow>, GhError>> + Send;

    /// One page of `repo`'s workflow runs, newest first as the API lists them. Never cached.
    ///
    /// A body without a `workflow_runs` list (missing or `null`) is an empty page. Fails on an HTTP error status, or
    /// with [`GhError::Decode`] when the body or any run doesn't decode.
    fn list_workflow_runs(
        &self,
        repo: &Repository,
        page: PageQuery,
    ) -> impl Future<Output = Result<Vec<WorkflowRun>, GhError>> + Send;

    /// The diff of pull request `number` in `repo`, verbatim, for [`crate::diff::parse`]. Cached for
    /// `cache.default_ttl`.
    ///
    /// Fails with [`DiffFetchError::NotFound`] for a 404, [`DiffFetchError::TooLarge`] for a 406 (over GitHub's
    /// 20,000-line limit), [`DiffFetchError::Http`] for any other HTTP error status and [`DiffFetchError::Gh`] when
    /// `gh` fails without one.
    fn pull_request_diff(
        &self,
        repo: &Repository,
        number: u64,
    ) -> impl Future<Output = Result<String, DiffFetchError>> + Send;

    /// The whole profile of the user or organization `login`. Never cached.
    ///
    /// Fails on any HTTP error status (a 404 for an unknown login) or `gh` failure, or with [`GhError::Decode`] when
    /// the body doesn't decode.
    fn user(&self, login: &str) -> impl Future<Output = Result<FullUser, GhError>> + Send;

    /// The first page of `login`'s public repositories, most recently updated first ([`DEFAULT_PER_PAGE`] of them).
    /// Cached for `cache.list_repos_ttl`.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when any repository doesn't decode.
    fn user_repos(&self, login: &str) -> impl Future<Output = Result<Vec<Repository>, GhError>> + Send;

    /// Pull request `number` of `repo`, from the pull request endpoint, carrying `repo`. Cached for
    /// `cache.default_ttl`.
    ///
    /// Fails on any HTTP error status (a 404 included) or `gh` failure, or with [`GhError::Decode`] when the body
    /// doesn't decode.
    fn pull_request(
        &self,
        repo: &Repository,
        number: u64,
    ) -> impl Future<Output = Result<FullPullRequest, GhError>> + Send;

    /// Every job of run `run_id` in `repo`, in API order, from every page (Python stops at the first 30, Py bug B64).
    /// Never cached.
    ///
    /// A page without a `jobs` list (missing or `null`) adds none. Fails on any HTTP error status or `gh` failure, or
    /// with [`GhError::Decode`] when a page or any job doesn't decode.
    fn workflow_run_jobs(
        &self,
        repo: &Repository,
        run_id: u64,
    ) -> impl Future<Output = Result<Vec<WorkflowJob>, GhError>> + Send;

    /// The combined status of commit `sha` in `repo`: its first [`CHECKS_PER_PAGE`] commit statuses and GitHub's
    /// overall state. Cached for `cache.default_ttl`.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn commit_status(
        &self,
        repo: &Repository,
        sha: &str,
    ) -> impl Future<Output = Result<CombinedStatus, GhError>> + Send;

    /// The first [`CHECKS_PER_PAGE`] check runs of commit `sha` in `repo`, in API order. Cached for
    /// `cache.default_ttl`.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body or any run doesn't
    /// decode.
    fn check_runs(&self, repo: &Repository, sha: &str) -> impl Future<Output = Result<Vec<CheckRun>, GhError>> + Send;

    /// The reactions on issue or pull request `number` of `repo`: GitHub's first page of them (30, as Python asks for
    /// no more). Cached for `cache.default_ttl`. A reaction of a kind this build doesn't know is skipped.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn issue_reactions(
        &self,
        repo: &Repository,
        number: u64,
    ) -> impl Future<Output = Result<Reactions, GhError>> + Send;

    /// Issue `number` of `repo`, from the issues endpoint, carrying `repo`. Cached for `cache.default_ttl`.
    ///
    /// The endpoint answers for pull requests too; [`IssueListItem::from_api_item`] tells them apart, so the caller
    /// sees which it got.
    ///
    /// Fails on any HTTP error status (a 404 included) or `gh` failure, or with [`GhError::Decode`] when the body
    /// doesn't decode.
    fn issue(&self, repo: &Repository, number: u64) -> impl Future<Output = Result<IssueListItem, GhError>> + Send;

    /// Every comment on `issue` (an issue or a pull request), oldest first, from every page of its `comments_url`
    /// (Python stops at GitHub's first 30, Py bug B64). Cached for `cache.default_ttl`.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when a page or any comment doesn't
    /// decode.
    fn issue_comments(&self, issue: &Issue) -> impl Future<Output = Result<Vec<IssueComment>, GhError>> + Send;

    /// The first [`COMMENT_REACTIONS_PER_PAGE`] reactions on `comment` in `repo`. An issue comment's are asked for on
    /// `/issues/comments/{id}/reactions` and a review comment's on `/pulls/comments/{id}/reactions` (Python asks for
    /// both on the first, Py bug B54). Cached for `cache.default_ttl`. A reaction of a kind this build doesn't know is
    /// skipped.
    ///
    /// Fails on any HTTP error status (a 404 for an unknown comment) or `gh` failure, or with [`GhError::Decode`]
    /// when the body doesn't decode.
    fn comment_reactions(
        &self,
        repo: &Repository,
        comment: CommentRef,
    ) -> impl Future<Output = Result<Reactions, GhError>> + Send;

    /// The submitted reviews of `pr`, in API order, each with its inline comments: every page of
    /// `GET .../pulls/{n}/reviews`, pending reviews dropped, then every page of each kept review's
    /// `.../reviews/{id}/comments`, one review after another. All cached for `cache.default_ttl`.
    ///
    /// Fails as a whole, as Python's does, on any HTTP error status or `gh` failure of any of the requests, or with
    /// [`GhError::Decode`] when a review or a comment doesn't decode.
    fn pull_request_reviews(&self, pr: &FullPullRequest) -> impl Future<Output = Result<Vec<Review>, GhError>> + Send;

    /// The logs of job `job_id` in `repo`, as plain text, verbatim: GitHub answers with a redirect to the text, which
    /// `gh` follows. `None` for an empty body. Never cached.
    ///
    /// Fails on any HTTP error status (a 404 for a job without logs, such as a skipped or queued one) or `gh` failure.
    fn job_logs(&self, repo: &Repository, job_id: u64) -> impl Future<Output = Result<Option<String>, GhError>> + Send;

    /// Opens a new issue in `repo` with `issue`'s title and body: `POST /repos/{owner}/{repo}/issues`, the body over
    /// stdin. Returns the created issue, carrying `repo`. Never cached.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn create_issue(&self, repo: &Repository, issue: &NewIssue) -> impl Future<Output = Result<Issue, GhError>> + Send;

    /// Edits issue `number` of `repo`: `PATCH /repos/{owner}/{repo}/issues/{number}` with `update`, the body over
    /// stdin, sent even when nothing changed. Returns the updated issue, carrying `repo`. Never cached.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn update_issue(
        &self,
        repo: &Repository,
        number: u64,
        update: &IssueUpdate,
    ) -> impl Future<Output = Result<Issue, GhError>> + Send;

    /// Posts a comment on issue or pull request `number` of `repo`: `POST /repos/{owner}/{repo}/issues/{number}/comments`
    /// with `{"body": body}` over stdin, `body` exactly as given (an empty one too, which GitHub refuses). Returns the
    /// comment. Never cached.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn create_issue_comment(
        &self,
        repo: &Repository,
        number: u64,
        body: &str,
    ) -> impl Future<Output = Result<IssueComment, GhError>> + Send;

    /// Replies to review comment `comment_id` on pull request `pr_number` of `repo`: `POST
    /// /repos/{owner}/{repo}/pulls/{pr_number}/comments/{comment_id}/replies` with `{"body": body}` over stdin. Returns
    /// the reply, a review comment in the same thread. Never cached.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn reply_to_review_comment(
        &self,
        repo: &Repository,
        pr_number: u64,
        comment_id: u64,
        body: &str,
    ) -> impl Future<Output = Result<ReviewComment, GhError>> + Send;

    /// Submits a review of pull request `number` of `repo`: `POST /repos/{owner}/{repo}/pulls/{number}/reviews` with
    /// `review` over stdin, its comments placed by diff position. Returns the review, without its comments. Never
    /// cached.
    ///
    /// Fails on any HTTP error status or `gh` failure, or with [`GhError::Decode`] when the body doesn't decode.
    fn create_review(
        &self,
        repo: &Repository,
        number: u64,
        review: &NewReview,
    ) -> impl Future<Output = Result<Review, GhError>> + Send;

    /// Adds the logged-in user's reaction of `kind` to issue or pull request `number` of `repo`: `POST
    /// /repos/{owner}/{repo}/issues/{number}/reactions` with `{"content": kind.api_name()}` over stdin. Returns `true`
    /// when it was created (201) and `false` when the user had already given it (200). Never cached.
    ///
    /// Fails on any HTTP error status or `gh` failure.
    fn add_issue_reaction(
        &self,
        repo: &Repository,
        number: u64,
        kind: ReactionKind,
    ) -> impl Future<Output = Result<bool, GhError>> + Send;

    /// Merges pull request `number` of `repo`: `PUT /repos/{owner}/{repo}/pulls/{number}/merge` with `request` over
    /// stdin. GitHub merges only while the head is still `request.sha`. Returns its answer, which may say it didn't
    /// merge. Never cached.
    ///
    /// Fails on any HTTP error status (405 when it isn't mergeable, 409 when the head moved) or `gh` failure, or with
    /// [`GhError::Decode`] when the body doesn't decode.
    fn merge_pull_request(
        &self,
        repo: &Repository,
        number: u64,
        request: &MergeRequest,
    ) -> impl Future<Output = Result<MergeResult, GhError>> + Send;
}

/// Why [`Github::pull_request_diff`] failed. `Display` is the line the Diff tab shows in place of the diff.
#[derive(Debug, thiserror::Error)]
pub enum DiffFetchError {
    /// The pull request or its diff doesn't exist (HTTP 404).
    #[error("No diff contents found")]
    NotFound,
    /// The diff is over GitHub's 20,000-line limit (HTTP 406).
    #[error("Diff too large")]
    TooLarge,
    /// Any other HTTP error status.
    #[error("Error fetching diff ({0})")]
    Http(u16),
    /// `gh` failed without an HTTP status: not installed, logged out, timed out, offline.
    #[error("Error fetching diff (0)")]
    Gh(#[source] GhError),
}

impl DiffFetchError {
    /// The line the Diff tab shows in place of the diff: the same text as `Display`.
    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl From<GhError> for DiffFetchError {
    /// Tells apart the HTTP statuses the Diff tab words differently; any other failure keeps its [`GhError`] for the
    /// log.
    fn from(error: GhError) -> Self {
        match error {
            GhError::Http { status: 404, .. } => DiffFetchError::NotFound,
            GhError::Http { status: 406, .. } => DiffFetchError::TooLarge,
            GhError::Http { status, .. } => DiffFetchError::Http(status),
            error => DiffFetchError::Gh(error),
        }
    }
}

/// Page size for a commit's statuses and check runs: Python's `per_page` there. Only the first page is fetched.
pub const CHECKS_PER_PAGE: u32 = 100;

/// Page size for a comment's reactions: Python's `per_page` there. Only the first page is fetched.
pub const COMMENT_REACTIONS_PER_PAGE: u32 = 100;

/// Page size for the paged endpoints that take a [`PageQuery`]: Python's default `per_page`.
pub const DEFAULT_PER_PAGE: u32 = 30;

/// Which page of a paged endpoint to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageQuery {
    /// The page, from 1.
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
}

impl Default for PageQuery {
    /// Page 1 of [`DEFAULT_PER_PAGE`] items.
    fn default() -> Self {
        Self {
            page: 1,
            per_page: DEFAULT_PER_PAGE,
        }
    }
}

impl PageQuery {
    /// The `page` and `per_page` query parameters, in that order.
    fn params(self) -> Vec<(String, String)> {
        vec![
            ("page".to_owned(), self.page.to_string()),
            ("per_page".to_owned(), self.per_page.to_string()),
        ]
    }
}

/// Which repositories [`Github::list_repositories`] returns, by the user's relationship to them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RepoKind {
    /// Every repository the user can access.
    #[default]
    All,
    /// Repositories the user owns.
    Owner,
    /// Repositories the user is a member of through an organization or as a collaborator.
    Member,
}

impl RepoKind {
    /// The `type` query value.
    pub fn as_str(self) -> &'static str {
        match self {
            RepoKind::All => "all",
            RepoKind::Owner => "owner",
            RepoKind::Member => "member",
        }
    }
}

/// The field [`Github::list_repositories`] sorts by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RepoSort {
    /// `owner/name`.
    #[default]
    FullName,
    /// Creation time.
    Created,
    /// Last update time.
    Updated,
    /// Last push time.
    Pushed,
}

impl RepoSort {
    /// The `sort` query value.
    pub fn as_str(self) -> &'static str {
        match self {
            RepoSort::FullName => "full_name",
            RepoSort::Created => "created",
            RepoSort::Updated => "updated",
            RepoSort::Pushed => "pushed",
        }
    }
}

/// A sort direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// Ascending.
    #[default]
    Asc,
    /// Descending.
    Desc,
}

impl SortDirection {
    /// The `direction` query value.
    pub fn as_str(self) -> &'static str {
        match self {
            SortDirection::Asc => "asc",
            SortDirection::Desc => "desc",
        }
    }
}

/// The parameters of [`Github::list_repositories`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RepoListQuery {
    /// Which repositories to include.
    pub kind: RepoKind,
    /// The sort field.
    pub sort: RepoSort,
    /// The sort direction.
    pub direction: SortDirection,
}

impl RepoListQuery {
    /// Every accessible repository by `full_name` ascending, as the Python app's `list_all` fetches them.
    pub fn all() -> Self {
        Self {
            kind: RepoKind::All,
            sort: RepoSort::FullName,
            direction: SortDirection::Asc,
        }
    }
}

/// The parameters of [`Github::list_issues`]: the filters and which page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IssueListQuery {
    /// Which items to list, by state.
    pub state: IssueStateFilter,
    /// Which items to list, by author.
    pub owner: IssueOwnerFilter,
    /// The page, from 1.
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
}

impl IssueListQuery {
    /// Page 1, of [`ISSUES_PER_PAGE`] items, with these filters. Callers take them from `config.pull_requests` or
    /// `config.issues`.
    pub fn first_page(state: IssueStateFilter, owner: IssueOwnerFilter) -> Self {
        Self {
            state,
            owner,
            page: 1,
            per_page: ISSUES_PER_PAGE,
        }
    }
}

/// What [`Github::create_issue`] sends: the body of `POST /repos/{owner}/{repo}/issues`, serialized with its fields
/// in this order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct NewIssue {
    /// The title, as typed.
    pub title: String,
    /// The Markdown body, as typed (lines joined with `\n`). It may be empty.
    pub body: String,
}

/// What [`Github::update_issue`] sends: the body of `PATCH /repos/{owner}/{repo}/issues/{number}`, serialized with
/// its fields in this order and `state` left out when it's `None`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct IssueUpdate {
    /// The title, as edited.
    pub title: String,
    /// The Markdown body, as edited (lines joined with `\n`).
    pub body: String,
    /// The state to set, or `None` to leave it as it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<IssueState>,
}

/// What [`Github::create_review`] sends: the body of `POST /repos/{owner}/{repo}/pulls/{number}/reviews`, serialized
/// with its fields in this order. `comments` is `[]` when there are none.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct NewReview {
    /// What the review does.
    pub event: ReviewEvent,
    /// The commit reviewed: the pull request's head.
    pub commit_id: String,
    /// The review's summary, as typed. It may be empty.
    pub body: String,
    /// Its inline comments, in the order they were added.
    pub comments: Vec<DraftReviewComment>,
}

/// What [`Github::merge_pull_request`] sends: the body of `PUT /repos/{owner}/{repo}/pulls/{number}/merge`, serialized
/// with its fields in this order and the method as `merge`, `squash` or `rebase`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct MergeRequest {
    /// How to merge: `pull_requests.preferred_merge_method`.
    pub merge_method: MergeMethod,
    /// The head commit the merge is for; GitHub refuses it (409) once the branch has moved on.
    pub sha: String,
}

/// GitHub's answer to a merge it accepted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct MergeResult {
    /// Whether the pull request was merged.
    pub merged: bool,
    /// GitHub's message, such as "Pull Request successfully merged".
    #[serde(default)]
    pub message: String,
    /// The merge commit, when there is one.
    #[serde(default)]
    pub sha: Option<String>,
}

/// A review's verdict, as the reviews endpoint takes it: `APPROVE`, `REQUEST_CHANGES` or `COMMENT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent {
    /// Approve the changes.
    Approve,
    /// Ask for changes.
    RequestChanges,
    /// Only comment.
    Comment,
}

/// An inline comment of a [`NewReview`], serialized with its fields in this order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct DraftReviewComment {
    /// The file's path, as the diff names it.
    pub path: String,
    /// The comment, as typed.
    pub body: String,
    /// The line's diff position: GitHub's older way to place a review comment, which it still takes.
    pub position: u32,
}

/// How long `gh` may serve each kind of response from its cache, from the `cache` config section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTtls {
    /// For requests without a more specific TTL.
    pub default: Duration,
    /// For listing repositories.
    pub list_repos: Duration,
    /// For listing issues.
    pub list_issues: Duration,
}

impl From<&CacheSettings> for CacheTtls {
    fn from(cache: &CacheSettings) -> Self {
        Self {
            default: cache.default_ttl(),
            list_repos: cache.list_repos_ttl(),
            list_issues: cache.list_issues_ttl(),
        }
    }
}

impl Default for CacheTtls {
    /// The TTLs of a default config.
    fn default() -> Self {
        Self::from(&CacheSettings::default())
    }
}

/// How [`GhCli`] runs `gh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCliOptions {
    /// The `gh` executable. A bare name is looked up on `PATH`.
    pub program: PathBuf,
    /// The GitHub Enterprise host to pass as `--hostname`, or `None` to use gh's default (and `GH_HOST`).
    pub hostname: Option<String>,
    /// Cache lifetimes per request kind.
    pub ttls: CacheTtls,
    /// How long each kind of `gh` call may run before it's killed.
    pub timeouts: GhTimeouts,
}

impl Default for GhCliOptions {
    /// `gh` from `PATH`, github.com, and the default TTLs and time limits.
    fn default() -> Self {
        Self {
            program: PathBuf::from("gh"),
            hostname: None,
            ttls: CacheTtls::default(),
            timeouts: GhTimeouts::default(),
        }
    }
}

impl From<&Config> for GhCliOptions {
    /// The config has no time limits (Python has none to mirror), so these are always the defaults.
    fn from(config: &Config) -> Self {
        Self {
            program: PathBuf::from("gh"),
            hostname: hostname_for_base_url(&config.api.base_url),
            ttls: CacheTtls::from(&config.cache),
            timeouts: GhTimeouts::default(),
        }
    }
}

/// The `--hostname` for an `api.base_url`: `None` for github.com (so gh keeps its default and `GH_HOST`), otherwise
/// the URL's host, lowercased and with any port kept.
///
/// `https://ghe.example.com/api/v3` gives `ghe.example.com`. A URL with no host gives `None`.
pub fn hostname_for_base_url(base_url: &str) -> Option<String> {
    let url = base_url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .to_ascii_lowercase();
    let name = host.split(':').next().unwrap_or_default();
    match name {
        "" | "github.com" | "api.github.com" => None,
        _ => Some(host),
    }
}

/// The real [`Github`]: typed endpoints over the [`gh::GhCli`] runner.
///
/// Cheap to clone; clones share the runner and the memoized current user.
#[derive(Debug, Clone)]
pub struct GhCli {
    inner: Arc<GhCliInner>,
}

#[derive(Debug)]
struct GhCliInner {
    runner: gh::GhCli,
    ttls: CacheTtls,
    current_user: OnceCell<User>,
}

impl Default for GhCli {
    fn default() -> Self {
        Self::new(GhCliOptions::default())
    }
}

impl GhCli {
    /// A client that runs `gh` as `options` say.
    pub fn new(options: GhCliOptions) -> Self {
        Self {
            inner: Arc::new(GhCliInner {
                runner: gh::GhCli::new(options.program, options.hostname).with_timeouts(options.timeouts),
                ttls: options.ttls,
                current_user: OnceCell::new(),
            }),
        }
    }

    /// The runner the typed methods are built on. Test-only, so every real call site stays typed.
    #[cfg(test)]
    fn runner(&self) -> &gh::GhCli {
        &self.inner.runner
    }

    fn current_user_request() -> GhRequest {
        GhRequest::get("/user")
    }

    fn list_repositories_request(&self, query: RepoListQuery) -> GhRequest {
        let mut request = GhRequest::get("/user/repos");
        // Python's order. `page=1` is explicit, as Python sends it, so both apps hit the same fixture key; gh then
        // follows each page's `Link: rel="next"` URL.
        request.query = [
            ("type", query.kind.as_str()),
            ("direction", query.direction.as_str()),
            ("sort", query.sort.as_str()),
            ("page", "1"),
            ("per_page", REPOS_PER_PAGE),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        request.cache = Some(self.inner.ttls.list_repos);
        request
    }

    /// One page of `GET /repos/{owner}/{repo}/issues`, with `creator` set for the `mine` filter.
    fn list_issues_request(&self, repo: &Repository, query: IssueListQuery, creator: Option<&str>) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{}/{}/issues", repo.owner.login, repo.name));
        // Python's order, so both apps' request logs read the same.
        request.query = vec![
            ("state".to_owned(), query.state.as_str().to_owned()),
            ("page".to_owned(), query.page.to_string()),
            ("per_page".to_owned(), query.per_page.to_string()),
        ];
        if let Some(creator) = creator {
            request.query.push(("creator".to_owned(), creator.to_owned()));
        }
        // Python sends its default TTL here; `cache.list_issues_ttl` is never read.
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{full_name}/readme` as raw markdown, cached for the default TTL as Python does.
    fn repo_readme_request(&self, full_name: &str) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{full_name}/readme"));
        request.accept = Accept::Raw;
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{full_name}`, cached for the default TTL as Python's `github_headers()` asks.
    fn repository_request(&self, full_name: &str) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{full_name}"));
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// One page of `GET /repos/{owner}/{repo}/actions/{what}`, uncached: Python sends no `Cache-Control` here.
    fn actions_request(repo: &Repository, what: &str, page: PageQuery) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{}/{}/actions/{what}", repo.owner.login, repo.name));
        request.query = page.params();
        request
    }

    /// `GET /repos/{owner}/{repo}/pulls/{number}` as a diff, through `gh api` rather than `gh pr diff` so the HTTP
    /// status is known (Py bug B56). Cached for the default TTL.
    fn pull_request_diff_request(&self, repo: &Repository, number: u64) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{}/{}/pulls/{number}", repo.owner.login, repo.name));
        request.accept = Accept::Diff;
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /users/{login}`, uncached: Python sends no `Cache-Control` here.
    fn user_request(login: &str) -> GhRequest {
        GhRequest::get(format!("/users/{login}"))
    }

    /// `GET /users/{login}/repos`, one page, cached for `cache.list_repos_ttl` as Python's `list_repos_for_user`.
    fn user_repos_request(&self, login: &str) -> GhRequest {
        let mut request = GhRequest::get(format!("/users/{login}/repos"));
        // Python's order, so both apps' request logs read the same.
        request.query = vec![
            ("sort".to_owned(), RepoSort::Updated.as_str().to_owned()),
            ("direction".to_owned(), SortDirection::Desc.as_str().to_owned()),
            ("per_page".to_owned(), DEFAULT_PER_PAGE.to_string()),
        ];
        request.cache = Some(self.inner.ttls.list_repos);
        request
    }

    /// `GET /repos/{owner}/{repo}/pulls/{number}`, cached for the default TTL as Python's `github_headers()` asks.
    fn pull_request_request(&self, repo: &Repository, number: u64) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{}/{}/pulls/{number}", repo.owner.login, repo.name));
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{owner}/{repo}/actions/runs/{run_id}/jobs`, uncached and without a query: Python sends neither, so
    /// the first page's fixture key is the one Python hits.
    fn workflow_run_jobs_request(repo: &Repository, run_id: u64) -> GhRequest {
        GhRequest::get(format!(
            "/repos/{}/{}/actions/runs/{run_id}/jobs",
            repo.owner.login, repo.name
        ))
    }

    /// `GET /repos/{owner}/{repo}/commits/{sha}/{what}`, the first page of [`CHECKS_PER_PAGE`], cached for the default
    /// TTL as Python's `github_headers()` asks.
    fn commit_checks_request(&self, repo: &Repository, sha: &str, what: &str) -> GhRequest {
        let mut request = GhRequest::get(format!(
            "/repos/{}/{}/commits/{sha}/{what}",
            repo.owner.login, repo.name
        ));
        request.query = PageQuery {
            page: 1,
            per_page: CHECKS_PER_PAGE,
        }
        .params();
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{owner}/{repo}/actions/jobs/{job_id}/logs`, uncached: Python sends no `Cache-Control` here.
    fn job_logs_request(repo: &Repository, job_id: u64) -> GhRequest {
        GhRequest::get(format!(
            "/repos/{}/{}/actions/jobs/{job_id}/logs",
            repo.owner.login, repo.name
        ))
    }

    /// `GET /repos/{owner}/{repo}/issues/{number}/reactions`, without page parameters as Python sends it, cached for
    /// the default TTL.
    fn issue_reactions_request(&self, repo: &Repository, number: u64) -> GhRequest {
        let mut request = GhRequest::get(format!(
            "/repos/{}/{}/issues/{number}/reactions",
            repo.owner.login, repo.name
        ));
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{owner}/{repo}/issues/{number}`, cached for the default TTL as Python's `get_issue_by_number` asks.
    fn issue_request(&self, repo: &Repository, number: u64) -> GhRequest {
        let mut request = GhRequest::get(format!("/repos/{}/{}/issues/{number}", repo.owner.login, repo.name));
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET` the path of `issue.comments_url`, without page parameters as Python sends it (gh then follows each
    /// page's `Link: rel="next"`), cached for the default TTL.
    fn issue_comments_request(&self, issue: &Issue) -> GhRequest {
        let path = api_path(&issue.comments_url).unwrap_or_else(|| {
            format!(
                "/repos/{}/{}/issues/{}/comments",
                issue.repo.owner.login, issue.repo.name, issue.number
            )
        });
        let mut request = GhRequest::get(path);
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{owner}/{repo}/{issues|pulls}/comments/{id}/reactions?page=1&per_page=100`, cached for the default
    /// TTL.
    fn comment_reactions_request(&self, repo: &Repository, comment: CommentRef) -> GhRequest {
        let kind = match comment {
            CommentRef::Issue(_) => "issues",
            CommentRef::Review(_) => "pulls",
        };
        let mut request = GhRequest::get(format!(
            "/repos/{}/{}/{kind}/comments/{}/reactions",
            repo.owner.login,
            repo.name,
            comment.id()
        ));
        request.query = PageQuery {
            page: 1,
            per_page: COMMENT_REACTIONS_PER_PAGE,
        }
        .params();
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{owner}/{repo}/pulls/{number}/reviews`, without page parameters as Python sends it (gh then
    /// follows each page's `Link: rel="next"`), cached for the default TTL.
    fn pull_request_reviews_request(&self, repo: &Repository, number: u64) -> GhRequest {
        let mut request = GhRequest::get(format!(
            "/repos/{}/{}/pulls/{number}/reviews",
            repo.owner.login, repo.name
        ));
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `GET /repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}/comments`, without page parameters as Python
    /// sends it, cached for the default TTL.
    fn review_comments_request(&self, repo: &Repository, number: u64, review_id: u64) -> GhRequest {
        let mut request = GhRequest::get(format!(
            "/repos/{}/{}/pulls/{number}/reviews/{review_id}/comments",
            repo.owner.login, repo.name
        ));
        request.cache = Some(self.inner.ttls.default);
        request
    }

    /// `POST /repos/{owner}/{repo}/issues` with `issue` as the JSON body.
    fn create_issue_request(repo: &Repository, issue: &NewIssue) -> Result<GhRequest, GhError> {
        let mut request = GhRequest::new(
            Method::Post,
            format!("/repos/{}/{}/issues", repo.owner.login, repo.name),
        );
        request.body = Some(serde_json::to_value(issue)?);
        Ok(request)
    }

    /// `PATCH /repos/{owner}/{repo}/issues/{number}` with `update` as the JSON body.
    fn update_issue_request(repo: &Repository, number: u64, update: &IssueUpdate) -> Result<GhRequest, GhError> {
        let mut request = GhRequest::new(
            Method::Patch,
            format!("/repos/{}/{}/issues/{number}", repo.owner.login, repo.name),
        );
        request.body = Some(serde_json::to_value(update)?);
        Ok(request)
    }

    /// `POST /repos/{owner}/{repo}/issues/{number}/comments` with `{"body": body}`.
    fn create_issue_comment_request(repo: &Repository, number: u64, body: &str) -> GhRequest {
        let mut request = GhRequest::new(
            Method::Post,
            format!("/repos/{}/{}/issues/{number}/comments", repo.owner.login, repo.name),
        );
        request.body = Some(serde_json::json!({ "body": body }));
        request
    }

    /// `POST /repos/{owner}/{repo}/pulls/{pr_number}/comments/{comment_id}/replies` with `{"body": body}`.
    fn reply_to_review_comment_request(repo: &Repository, pr_number: u64, comment_id: u64, body: &str) -> GhRequest {
        let mut request = GhRequest::new(
            Method::Post,
            format!(
                "/repos/{}/{}/pulls/{pr_number}/comments/{comment_id}/replies",
                repo.owner.login, repo.name
            ),
        );
        request.body = Some(serde_json::json!({ "body": body }));
        request
    }

    /// `POST /repos/{owner}/{repo}/issues/{number}/reactions` with `{"content": kind}`.
    fn add_issue_reaction_request(repo: &Repository, number: u64, kind: ReactionKind) -> GhRequest {
        let mut request = GhRequest::new(
            Method::Post,
            format!("/repos/{}/{}/issues/{number}/reactions", repo.owner.login, repo.name),
        );
        request.body = Some(serde_json::json!({ "content": kind.api_name() }));
        request
    }

    /// `PUT /repos/{owner}/{repo}/pulls/{number}/merge` with `merge` as the JSON body.
    fn merge_pull_request_request(repo: &Repository, number: u64, merge: &MergeRequest) -> Result<GhRequest, GhError> {
        let mut request = GhRequest::new(
            Method::Put,
            format!("/repos/{}/{}/pulls/{number}/merge", repo.owner.login, repo.name),
        );
        request.body = Some(serde_json::to_value(merge)?);
        Ok(request)
    }

    /// `POST /repos/{owner}/{repo}/pulls/{number}/reviews` with `review` as the JSON body.
    fn create_review_request(repo: &Repository, number: u64, review: &NewReview) -> Result<GhRequest, GhError> {
        let mut request = GhRequest::new(
            Method::Post,
            format!("/repos/{}/{}/pulls/{number}/reviews", repo.owner.login, repo.name),
        );
        request.body = Some(serde_json::to_value(review)?);
        Ok(request)
    }

    fn unread_notifications_request() -> GhRequest {
        let mut request = GhRequest::get("/notifications");
        // Python's query, in its order, so both apps hit the same fixture key. No `cache`: the count must be fresh.
        request.query = vec![
            ("all".to_owned(), "false".to_owned()),
            ("page".to_owned(), "1".to_owned()),
            ("per_page".to_owned(), NOTIFICATIONS_PER_PAGE.to_string()),
        ];
        request
    }
}

/// The number of items in a JSON array body, without decoding the items.
fn count_items(response: &GhResponse) -> Result<usize, GhError> {
    Ok(response.json::<Vec<serde::de::IgnoredAny>>()?.len())
}

/// The items of one issues-endpoint page, each tagged with `repo`. One bad item fails the page.
fn issue_list_items(response: &GhResponse, repo: &Repository) -> Result<Vec<IssueListItem>, GhError> {
    response
        .json::<Vec<serde_json::Value>>()?
        .into_iter()
        .map(|item| Ok(IssueListItem::from_api_item(item, repo)?))
        .collect()
}

/// The issue a create or update answered with, tagged with `repo`.
fn written_issue(response: &GhResponse, repo: &Repository) -> Result<Issue, GhError> {
    Ok(Issue::from_api(response.json()?, repo)?)
}

/// One issues-endpoint item, an issue or a pull request, tagged with `repo`.
fn issue_list_item(response: &GhResponse, repo: &Repository) -> Result<IssueListItem, GhError> {
    Ok(IssueListItem::from_api_item(response.json()?, repo)?)
}

impl Github for GhCli {
    async fn check_auth(&self) -> Result<(), GhError> {
        self.inner.runner.auth_status().await
    }

    async fn current_user(&self) -> Result<User, GhError> {
        // Only a success is memoized, so a failed fetch is retried on the next call. Concurrent first calls share
        // one `gh` run.
        self.inner
            .current_user
            .get_or_try_init(|| async { self.inner.runner.request(Self::current_user_request()).await?.json() })
            .await
            .cloned()
    }

    async fn list_repositories(&self, query: RepoListQuery) -> Result<Vec<Repository>, GhError> {
        self.inner.runner.paginate(self.list_repositories_request(query)).await
    }

    async fn unread_notification_count(&self) -> Result<usize, GhError> {
        let response = self.inner.runner.request(Self::unread_notifications_request()).await?;
        count_items(&response)
    }

    async fn list_issues(&self, repo: &Repository, query: IssueListQuery) -> Result<Vec<IssueListItem>, GhError> {
        let creator = match query.owner {
            IssueOwnerFilter::Mine => Some(self.current_user().await?.login),
            IssueOwnerFilter::All => None,
        };
        // One page per call, never `--paginate`: tables load further pages as they scroll.
        let request = self.list_issues_request(repo, query, creator.as_deref());
        let response = self.inner.runner.request(request).await?;
        issue_list_items(&response, repo)
    }

    async fn repo_readme(&self, full_name: &str) -> Result<Option<String>, GhError> {
        readme_text(self.inner.runner.request(self.repo_readme_request(full_name)).await)
    }

    async fn repository(&self, full_name: &str) -> Result<Option<Repository>, GhError> {
        found_repository(self.inner.runner.request(self.repository_request(full_name)).await)
    }

    async fn list_workflows(&self, repo: &Repository, page: PageQuery) -> Result<Vec<Workflow>, GhError> {
        let response = self
            .inner
            .runner
            .request(Self::actions_request(repo, "workflows", page))
            .await?;
        workflows(&response)
    }

    async fn list_workflow_runs(&self, repo: &Repository, page: PageQuery) -> Result<Vec<WorkflowRun>, GhError> {
        let response = self
            .inner
            .runner
            .request(Self::actions_request(repo, "runs", page))
            .await?;
        workflow_runs(&response)
    }

    async fn pull_request_diff(&self, repo: &Repository, number: u64) -> Result<String, DiffFetchError> {
        let request = self.pull_request_diff_request(repo, number);
        diff_text(self.inner.runner.request(request).await)
    }

    async fn user(&self, login: &str) -> Result<FullUser, GhError> {
        self.inner.runner.request(Self::user_request(login)).await?.json()
    }

    async fn user_repos(&self, login: &str) -> Result<Vec<Repository>, GhError> {
        self.inner.runner.request(self.user_repos_request(login)).await?.json()
    }

    async fn pull_request(&self, repo: &Repository, number: u64) -> Result<FullPullRequest, GhError> {
        let response = self
            .inner
            .runner
            .request(self.pull_request_request(repo, number))
            .await?;
        full_pull_request(&response, repo)
    }

    async fn workflow_run_jobs(&self, repo: &Repository, run_id: u64) -> Result<Vec<WorkflowJob>, GhError> {
        let pages = self
            .inner
            .runner
            .paginate_slurp(Self::workflow_run_jobs_request(repo, run_id))
            .await?;
        Ok(workflow_jobs(pages))
    }

    async fn commit_status(&self, repo: &Repository, sha: &str) -> Result<CombinedStatus, GhError> {
        let request = self.commit_checks_request(repo, sha, "status");
        self.inner.runner.request(request).await?.json()
    }

    async fn check_runs(&self, repo: &Repository, sha: &str) -> Result<Vec<CheckRun>, GhError> {
        let request = self.commit_checks_request(repo, sha, "check-runs");
        check_run_list(&self.inner.runner.request(request).await?)
    }

    async fn issue_reactions(&self, repo: &Repository, number: u64) -> Result<Reactions, GhError> {
        let response = self
            .inner
            .runner
            .request(self.issue_reactions_request(repo, number))
            .await?;
        reactions(&response)
    }

    async fn issue(&self, repo: &Repository, number: u64) -> Result<IssueListItem, GhError> {
        let response = self.inner.runner.request(self.issue_request(repo, number)).await?;
        issue_list_item(&response, repo)
    }

    async fn issue_comments(&self, issue: &Issue) -> Result<Vec<IssueComment>, GhError> {
        self.inner.runner.paginate(self.issue_comments_request(issue)).await
    }

    async fn comment_reactions(&self, repo: &Repository, comment: CommentRef) -> Result<Reactions, GhError> {
        let response = self
            .inner
            .runner
            .request(self.comment_reactions_request(repo, comment))
            .await;
        reactions(&response?)
    }

    async fn pull_request_reviews(&self, pr: &FullPullRequest) -> Result<Vec<Review>, GhError> {
        let repo = &pr.pr.issue.repo;
        let number = pr.number();
        let runner = &self.inner.runner;
        let reviews = runner.paginate(self.pull_request_reviews_request(repo, number)).await?;
        with_review_comments(reviews, |review_id| {
            runner.paginate(self.review_comments_request(repo, number, review_id))
        })
        .await
    }

    async fn job_logs(&self, repo: &Repository, job_id: u64) -> Result<Option<String>, GhError> {
        job_log_text(self.inner.runner.request(Self::job_logs_request(repo, job_id)).await)
    }

    async fn create_issue(&self, repo: &Repository, issue: &NewIssue) -> Result<Issue, GhError> {
        let response = self
            .inner
            .runner
            .request(Self::create_issue_request(repo, issue)?)
            .await?;
        written_issue(&response, repo)
    }

    async fn update_issue(&self, repo: &Repository, number: u64, update: &IssueUpdate) -> Result<Issue, GhError> {
        let response = self
            .inner
            .runner
            .request(Self::update_issue_request(repo, number, update)?)
            .await?;
        written_issue(&response, repo)
    }

    async fn create_issue_comment(&self, repo: &Repository, number: u64, body: &str) -> Result<IssueComment, GhError> {
        let request = Self::create_issue_comment_request(repo, number, body);
        self.inner.runner.request(request).await?.json()
    }

    async fn reply_to_review_comment(
        &self,
        repo: &Repository,
        pr_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment, GhError> {
        let request = Self::reply_to_review_comment_request(repo, pr_number, comment_id, body);
        self.inner.runner.request(request).await?.json()
    }

    async fn create_review(&self, repo: &Repository, number: u64, review: &NewReview) -> Result<Review, GhError> {
        let request = Self::create_review_request(repo, number, review)?;
        self.inner.runner.request(request).await?.json()
    }

    async fn add_issue_reaction(&self, repo: &Repository, number: u64, kind: ReactionKind) -> Result<bool, GhError> {
        let request = Self::add_issue_reaction_request(repo, number, kind);
        reaction_created(self.inner.runner.request(request).await)
    }

    async fn merge_pull_request(
        &self,
        repo: &Repository,
        number: u64,
        request: &MergeRequest,
    ) -> Result<MergeResult, GhError> {
        let request = Self::merge_pull_request_request(repo, number, request)?;
        merge_result(self.inner.runner.request(request).await)
    }
}

/// What a merge request answered: GitHub's result for a 2xx, the error for anything else (a 405 or a 409).
fn merge_result(response: Result<GhResponse, GhError>) -> Result<MergeResult, GhError> {
    response?.json()
}

/// Whether an add-reaction request created the reaction: 201 for a new one, 200 when the user already had it
/// (Python's `add_reaction_on_issue` counts only a 201).
fn reaction_created(response: Result<GhResponse, GhError>) -> Result<bool, GhError> {
    Ok(response?.status == 201)
}

/// `reviews` without the pending ones, each with the comments `fetch` gets for its id, one review after another. The
/// first failure fails the whole list, and the reviews after it aren't asked about.
async fn with_review_comments<F, Fut>(reviews: Vec<Review>, mut fetch: F) -> Result<Vec<Review>, GhError>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = Result<Vec<ReviewComment>, GhError>>,
{
    let mut kept = Vec::with_capacity(reviews.len());
    for mut review in reviews {
        if review.state == ReviewState::Pending {
            continue;
        }
        review.comments = fetch(review.id).await?;
        kept.push(review);
    }
    Ok(kept)
}

/// A job logs response as the logs: the body verbatim, `None` when it's empty.
fn job_log_text(response: Result<GhResponse, GhError>) -> Result<Option<String>, GhError> {
    let text = response?.text();
    Ok((!text.is_empty()).then_some(text))
}

/// The REST path of an API URL such as an issue's `comments_url`: what follows the API root, which is the host on
/// github.com (`https://api.github.com/repos/...`) and `/api/v3` on GitHub Enterprise Server. `None` when the URL
/// has no `/repos/` path.
fn api_path(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let at = after_scheme.find("/repos/")?;
    Some(after_scheme[at..].to_owned())
}

/// The `check_runs` body of `GET .../commits/{sha}/check-runs`.
#[derive(serde::Deserialize)]
struct CheckRunsBody {
    check_runs: Vec<CheckRun>,
}

fn check_run_list(response: &GhResponse) -> Result<Vec<CheckRun>, GhError> {
    Ok(response.json::<CheckRunsBody>()?.check_runs)
}

/// A reactions endpoint body, its unknown kinds skipped.
fn reactions(response: &GhResponse) -> Result<Reactions, GhError> {
    Ok(Reactions::from_api(response.json()?)?)
}

/// A diff response as the diff text, verbatim, or the error the Diff tab shows.
fn diff_text(response: Result<GhResponse, GhError>) -> Result<String, DiffFetchError> {
    Ok(response?.text())
}

/// A pull request endpoint body, carrying `repo`.
fn full_pull_request(response: &GhResponse, repo: &Repository) -> Result<FullPullRequest, GhError> {
    Ok(FullPullRequest::from_api(response.json()?, repo)?)
}

/// The `workflows` body of `GET .../actions/workflows`.
#[derive(serde::Deserialize)]
struct WorkflowsBody {
    #[serde(default)]
    workflows: Option<Vec<Workflow>>,
}

/// The `workflow_runs` body of `GET .../actions/runs`.
#[derive(serde::Deserialize)]
struct WorkflowRunsBody {
    #[serde(default)]
    workflow_runs: Option<Vec<WorkflowRun>>,
}

/// One page of `GET .../actions/runs/{id}/jobs`.
#[derive(serde::Deserialize)]
struct JobsPage {
    #[serde(default)]
    jobs: Option<Vec<WorkflowJob>>,
}

/// The jobs of every slurped page, in order.
fn workflow_jobs(pages: Vec<JobsPage>) -> Vec<WorkflowJob> {
    pages
        .into_iter()
        .flat_map(|page| page.jobs.unwrap_or_default())
        .collect()
}

fn workflows(response: &GhResponse) -> Result<Vec<Workflow>, GhError> {
    Ok(response.json::<WorkflowsBody>()?.workflows.unwrap_or_default())
}

fn workflow_runs(response: &GhResponse) -> Result<Vec<WorkflowRun>, GhError> {
    Ok(response.json::<WorkflowRunsBody>()?.workflow_runs.unwrap_or_default())
}

/// A README response as the README: the body verbatim, `None` for a 404.
fn readme_text(response: Result<GhResponse, GhError>) -> Result<Option<String>, GhError> {
    match response {
        Ok(response) => Ok(Some(response.text())),
        Err(GhError::Http { status: 404, .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// A repository response as the repository: `None` for a 404.
fn found_repository(response: Result<GhResponse, GhError>) -> Result<Option<Repository>, GhError> {
    match response {
        Ok(response) => Ok(Some(response.json()?)),
        Err(GhError::Http { status: 404, .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RepositoryPermission;

    fn strings(args: Vec<std::ffi::OsString>) -> Vec<String> {
        args.into_iter().map(|arg| arg.into_string().unwrap()).collect()
    }

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/github")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn hostname_derivation() {
        assert_eq!(hostname_for_base_url(&Config::default().api.base_url), None);
        assert_eq!(hostname_for_base_url("https://api.github.com"), None);
        assert_eq!(hostname_for_base_url("https://api.github.com/"), None);
        assert_eq!(hostname_for_base_url("https://github.com"), None);
        assert_eq!(hostname_for_base_url("HTTPS://API.GitHub.com"), None);
        assert_eq!(hostname_for_base_url(""), None);
        assert_eq!(
            hostname_for_base_url("https://ghe.example.com/api/v3"),
            Some("ghe.example.com".into())
        );
        assert_eq!(
            hostname_for_base_url("https://GHE.example.com:8443/api/v3?x=1"),
            Some("ghe.example.com:8443".into())
        );
        assert_eq!(hostname_for_base_url("ghe.example.com"), Some("ghe.example.com".into()));
    }

    #[test]
    fn options_from_config() {
        let mut config = Config::default();
        assert_eq!(GhCliOptions::from(&config), GhCliOptions::default());
        config.api.base_url = "https://ghe.example.com/api/v3".into();
        config.cache.list_repos_ttl = 5;
        config.cache.default_ttl = 6;
        config.cache.list_issues_ttl = 7;
        let options = GhCliOptions::from(&config);
        assert_eq!(options.program, PathBuf::from("gh"));
        assert_eq!(options.hostname.as_deref(), Some("ghe.example.com"));
        assert_eq!(
            options.ttls,
            CacheTtls {
                default: Duration::from_secs(6),
                list_repos: Duration::from_secs(5),
                list_issues: Duration::from_secs(7),
            }
        );
        assert_eq!(options.timeouts, GhTimeouts::default());
    }

    /// A negative TTL, which Python loads and keeps, means no HTTP cache, as a TTL of 0 does.
    #[test]
    fn negative_ttls_disable_the_cache() {
        let mut config = Config::default();
        config.cache.default_ttl = -1;
        config.cache.list_repos_ttl = -600;
        config.cache.list_issues_ttl = -5;
        assert_eq!(
            GhCliOptions::from(&config).ttls,
            CacheTtls {
                default: Duration::ZERO,
                list_repos: Duration::ZERO,
                list_issues: Duration::ZERO,
            }
        );
    }

    #[test]
    fn check_auth_argv() {
        assert_eq!(
            strings(GhCli::default().runner().auth_status_argv()),
            ["auth", "status"]
        );
        let ghe = GhCli::new(GhCliOptions {
            hostname: Some("ghe.example.com".into()),
            ..GhCliOptions::default()
        });
        assert_eq!(
            strings(ghe.runner().auth_status_argv()),
            ["auth", "status", "--hostname", "ghe.example.com"]
        );
    }

    #[test]
    fn current_user_argv() {
        let gh = GhCli::default();
        assert_eq!(
            strings(GhCli::current_user_request().argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/user"
            ]
        );
    }

    #[test]
    fn list_repositories_argv() {
        let gh = GhCli::default();
        let request = gh.list_repositories_request(RepoListQuery::all());
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), true)),
            [
                "api",
                "-X",
                "GET",
                "--cache",
                "86400s",
                "--paginate",
                "-H",
                "Accept: application/vnd.github+json",
                "/user/repos?type=all&direction=asc&sort=full_name&page=1&per_page=50"
            ]
        );

        let gh = GhCli::new(GhCliOptions {
            hostname: Some("ghe.example.com".into()),
            ttls: CacheTtls {
                list_repos: Duration::from_secs(30),
                ..CacheTtls::default()
            },
            ..GhCliOptions::default()
        });
        let query = RepoListQuery {
            kind: RepoKind::Member,
            sort: RepoSort::Pushed,
            direction: SortDirection::Desc,
        };
        assert_eq!(
            strings(gh.list_repositories_request(query).argv(gh.runner().hostname(), true)),
            [
                "api",
                "-X",
                "GET",
                "--hostname",
                "ghe.example.com",
                "--cache",
                "30s",
                "--paginate",
                "-H",
                "Accept: application/vnd.github+json",
                "/user/repos?type=member&direction=desc&sort=pushed&page=1&per_page=50"
            ]
        );
    }

    #[test]
    fn unread_notification_count_argv() {
        let gh = GhCli::default();
        assert_eq!(
            strings(GhCli::unread_notifications_request().argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/notifications?all=false&page=1&per_page=50"
            ]
        );
        let ghe = GhCli::new(GhCliOptions {
            hostname: Some("ghe.example.com".into()),
            ..GhCliOptions::default()
        });
        assert_eq!(
            strings(GhCli::unread_notifications_request().argv(ghe.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--hostname",
                "ghe.example.com",
                "-H",
                "Accept: application/vnd.github+json",
                "/notifications?all=false&page=1&per_page=50"
            ]
        );
    }

    #[test]
    fn unread_count_is_the_array_length() {
        let response = |body: &str| {
            let stdout = format!("HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{body}");
            gh::interpret(Some(0), stdout.as_bytes(), b"", false).unwrap()
        };
        assert_eq!(count_items(&response("[]")).unwrap(), 0);
        assert_eq!(
            count_items(&response(r#"[{"id":"1"},{"id":"2","x":[1,{}]},3]"#)).unwrap(),
            3
        );
        assert!(matches!(
            count_items(&response(r#"{"message":"hi"}"#)),
            Err(GhError::Decode(_))
        ));
        assert!(matches!(count_items(&response("")), Err(GhError::Decode(_))));
    }

    #[test]
    fn repo_query_values() {
        assert_eq!(RepoListQuery::all(), RepoListQuery::default());
        assert_eq!(
            [RepoKind::All, RepoKind::Owner, RepoKind::Member].map(RepoKind::as_str),
            ["all", "owner", "member"]
        );
        assert_eq!(
            [
                RepoSort::FullName,
                RepoSort::Created,
                RepoSort::Updated,
                RepoSort::Pushed
            ]
            .map(RepoSort::as_str),
            ["full_name", "created", "updated", "pushed"]
        );
        assert_eq!(
            [SortDirection::Asc, SortDirection::Desc].map(SortDirection::as_str),
            ["asc", "desc"]
        );
    }

    #[test]
    fn user_from_recorded_response() {
        let user: User = serde_json::from_str(&fixture("user.json")).unwrap();
        assert_eq!(user.login, "parity-bot");
        assert_eq!(user.id, 1_000_001);
        assert_eq!(user.name.as_deref(), Some("Parity Bot"));
        assert_eq!(user.html_url, "https://github.com/parity-bot");
    }

    #[test]
    fn repositories_from_recorded_page() {
        let repos: Vec<Repository> = serde_json::from_str(&fixture("user_repos_page.json")).unwrap();
        let names: Vec<&str> = repos.iter().map(|r| r.full_name.as_str()).collect();
        assert_eq!(
            names,
            [
                "octo-org/archived-tool",
                "parity-bot/dotfiles",
                "parity-bot/hello-world"
            ]
        );

        let archived = &repos[0];
        assert_eq!(archived.name, "archived-tool");
        assert!(archived.private);
        assert_eq!(archived.archived, Some(true));
        assert_eq!(archived.description, None);
        assert_eq!(archived.default_branch.as_deref(), Some("master"));
        assert_eq!(archived.owner.login, "octo-org");
        assert_eq!(archived.owner.name, None);
        assert_eq!(
            archived.permissions,
            Some(RepositoryPermission {
                admin: false,
                maintain: false,
                push: false,
                triage: true,
                pull: true,
            })
        );

        let dotfiles = &repos[1];
        assert!(!dotfiles.private);
        assert_eq!(dotfiles.archived, Some(false));
        assert_eq!(dotfiles.description.as_deref(), Some("Configuration files"));
        assert_eq!(dotfiles.default_branch.as_deref(), Some("main"));
        assert_eq!(dotfiles.owner.id, 1_000_001);

        // Optional fields missing entirely.
        let minimal = &repos[2];
        assert_eq!(minimal.permissions, None);
        assert_eq!(minimal.archived, None);
        assert_eq!(minimal.default_branch, None);
        assert_eq!(minimal.description, None);
    }

    #[test]
    fn repository_round_trips_in_python_shape() {
        let repos: Vec<Repository> = serde_json::from_str(&fixture("user_repos_page.json")).unwrap();
        let json = serde_json::to_value(&repos[2]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "hello-world",
                "full_name": "parity-bot/hello-world",
                "default_branch": null,
                "private": false,
                "archived": null,
                "owner": {
                    "login": "parity-bot",
                    "id": 1_000_001,
                    "name": null,
                    "html_url": "https://github.com/parity-bot"
                },
                "description": null,
                "permissions": null
            })
        );
        let back: Repository = serde_json::from_value(json).unwrap();
        assert_eq!(back, repos[2]);
    }

    #[tokio::test]
    async fn missing_gh_is_not_installed() {
        let gh = GhCli::new(GhCliOptions {
            program: PathBuf::from("/nonexistent/gh"),
            ..GhCliOptions::default()
        });
        assert!(matches!(gh.check_auth().await, Err(GhError::NotInstalled)));
        assert!(matches!(gh.current_user().await, Err(GhError::NotInstalled)));
        assert!(matches!(
            gh.list_repositories(RepoListQuery::all()).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(
            gh.unread_notification_count().await,
            Err(GhError::NotInstalled)
        ));
        let query = IssueListQuery::first_page(IssueStateFilter::All, IssueOwnerFilter::All);
        assert!(matches!(
            gh.list_issues(&lazy_fixture(), query).await,
            Err(GhError::NotInstalled)
        ));
    }

    /// A recorded parity fixture, whole.
    fn parity_fixture(set: &str, file: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures")
            .join(set)
            .join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    /// The `response.body` of a recorded parity fixture, as `gh api -i` would print it.
    fn parity_response(set: &str, file: &str) -> GhResponse {
        api_response(&parity_fixture(set, file)["response"]["body"].to_string())
    }

    fn api_response(body: &str) -> GhResponse {
        let stdout = format!("HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{body}");
        gh::interpret(Some(0), stdout.as_bytes(), b"", false).unwrap()
    }

    fn parity_repo(set: &str, full_name: &str) -> Repository {
        parity_response(set, &format!("repos/{full_name}/GET.json"))
            .json()
            .unwrap()
    }

    fn lazy_fixture() -> Repository {
        parity_repo("prs-70", "parity-bot/lazy-fixture")
    }

    fn list_issues_argv(config: &Config, query: IssueListQuery, creator: Option<&str>) -> Vec<String> {
        let gh = GhCli::new(GhCliOptions::from(config));
        let request = gh.list_issues_request(&lazy_fixture(), query, creator);
        strings(request.argv(gh.runner().hostname(), false))
    }

    #[test]
    fn list_issues_argv_with_default_config() {
        let query = IssueListQuery::first_page(IssueStateFilter::All, IssueOwnerFilter::All);
        let defaults = Config::default();
        assert_eq!(
            query,
            IssueListQuery::first_page(defaults.pull_requests.state_filter, defaults.pull_requests.owner_filter)
        );
        assert_eq!(
            list_issues_argv(&defaults, query, None),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/lazy-fixture/issues?state=all&page=1&per_page=30"
            ]
        );
    }

    #[test]
    fn list_issues_argv_for_mine() {
        let query = IssueListQuery {
            page: 2,
            ..IssueListQuery::first_page(IssueStateFilter::Open, IssueOwnerFilter::Mine)
        };
        assert_eq!(
            list_issues_argv(&Config::default(), query, Some("parity-bot")),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/lazy-fixture/issues?state=open&page=2&per_page=30&creator=parity-bot"
            ]
        );
    }

    #[test]
    fn list_issues_argv_for_enterprise_without_cache() {
        let mut config = Config::default();
        config.api.base_url = "https://ghe.example.com/api/v3".into();
        config.cache.default_ttl = 0;
        // Only the default TTL applies to this endpoint, as in Python.
        config.cache.list_issues_ttl = 900;
        let query = IssueListQuery::first_page(IssueStateFilter::Closed, IssueOwnerFilter::All);
        assert_eq!(
            list_issues_argv(&config, query, None),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--hostname",
                "ghe.example.com",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/lazy-fixture/issues?state=closed&page=1&per_page=30"
            ]
        );
    }

    #[test]
    fn prs_70_first_page_is_thirty_pull_requests() {
        let repo = lazy_fixture();
        let response = parity_response(
            "prs-70",
            "repos/parity-bot/lazy-fixture/issues/GET.page-1_per_page-30_state-all.json",
        );
        let items = issue_list_items(&response, &repo).unwrap();
        let numbers: Vec<u64> = items.iter().map(|item| item.issue().number).collect();
        assert_eq!(numbers, (41..=70).rev().collect::<Vec<_>>());
        for item in items {
            let pr = item.into_pull_request().unwrap();
            assert!(!pr.draft);
            assert_eq!(pr.issue.repo, repo);
        }
    }

    #[test]
    fn base_first_page_mixes_issues_and_pull_requests_in_api_order() {
        let repo = parity_repo("base", "gizmo385/gh-lazy");
        let response = parity_response(
            "base",
            "repos/gizmo385/gh-lazy/issues/GET.page-1_per_page-30_state-all.json",
        );
        let items = issue_list_items(&response, &repo).unwrap();
        assert_eq!(items.len(), 30);
        assert_eq!(items.iter().filter(|item| item.is_pull_request()).count(), 12);
        assert!(!items[0].is_pull_request());
        assert_eq!(items[0].issue().number, 113);
        assert!(items[1].is_pull_request());
        assert_eq!(items[1].issue().number, 112);
        // API order is newest first.
        let numbers: Vec<u64> = items.iter().map(|item| item.issue().number).collect();
        assert!(numbers.is_sorted_by(|a, b| a > b), "{numbers:?}");
        assert!(items.iter().all(|item| item.issue().repo == repo));
    }

    #[test]
    fn one_bad_item_fails_the_page() {
        let repo = lazy_fixture();
        let response = parity_response(
            "prs-70",
            "repos/parity-bot/lazy-fixture/issues/GET.page-3_per_page-30_state-all.json",
        );
        assert_eq!(issue_list_items(&response, &repo).unwrap().len(), 10);

        let mut items: Vec<serde_json::Value> = response.json().unwrap();
        items[4].as_object_mut().unwrap().remove("number");
        let bad = api_response(&serde_json::Value::from(items).to_string());
        assert!(matches!(issue_list_items(&bad, &repo), Err(GhError::Decode(_))));
        assert!(matches!(
            issue_list_items(&api_response(r#"{"message":"hi"}"#), &repo),
            Err(GhError::Decode(_))
        ));
        assert!(issue_list_items(&api_response("[]"), &repo).unwrap().is_empty());
    }

    #[test]
    fn repo_readme_argv() {
        let gh = GhCli::default();
        assert_eq!(
            strings(
                gh.repo_readme_request("parity-bot/dotfiles")
                    .argv(gh.runner().hostname(), false)
            ),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github.raw",
                "/repos/parity-bot/dotfiles/readme"
            ]
        );
    }

    #[test]
    fn list_workflows_and_runs_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        let argv = |what: &str, page: PageQuery| {
            strings(GhCli::actions_request(&lazy_fixture(), what, page).argv(gh.runner().hostname(), false))
        };
        // No `--cache` and no `--paginate`: Python sends no `Cache-Control`, and tables page themselves.
        assert_eq!(
            argv("workflows", PageQuery::default()),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/lazy-fixture/actions/workflows?page=1&per_page=30"
            ]
        );
        assert_eq!(
            argv("runs", PageQuery { page: 2, per_page: 30 }),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/lazy-fixture/actions/runs?page=2&per_page=30"
            ]
        );
    }

    #[test]
    fn empty_missing_and_null_workflow_lists_are_empty() {
        let runs = parity_response(
            "prs-70",
            "repos/parity-bot/lazy-fixture/actions/runs/GET.page-1_per_page-30.json",
        );
        assert!(workflow_runs(&runs).unwrap().is_empty());
        let listed = parity_response(
            "prs-70",
            "repos/parity-bot/lazy-fixture/actions/workflows/GET.page-1_per_page-30.json",
        );
        assert!(workflows(&listed).unwrap().is_empty());
        for body in [r#"{"total_count": 0}"#, r#"{"workflow_runs": null, "workflows": null}"#] {
            assert!(workflow_runs(&api_response(body)).unwrap().is_empty(), "{body}");
            assert!(workflows(&api_response(body)).unwrap().is_empty(), "{body}");
        }
        assert!(matches!(
            workflow_runs(&api_response(r#""text""#)),
            Err(GhError::Decode(_))
        ));
        assert!(matches!(
            workflow_runs(&api_response(r#"{"workflow_runs": [{"id": 1}]}"#)),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn workflow_runs_page_decodes() {
        let path = "repos/parity-bot/ci-fixture/actions";
        let runs = workflow_runs(&parity_response(
            "workflow-runs",
            &format!("{path}/runs/GET.page-1_per_page-30.json"),
        ))
        .unwrap();
        assert_eq!(runs.len(), 30);
        let newest = &runs[0];
        assert_eq!(newest.display_title, "Ship runs table");
        assert_eq!(newest.name, "CI");
        assert_eq!(newest.status, "in_progress");
        assert_eq!(newest.conclusion, None);
        assert_eq!(newest.run_attempt, Some(1));
        assert_eq!(newest.actor.login, "parity-bot");
        assert_eq!(newest.repository.full_name, "parity-bot/ci-fixture");
        assert_eq!(newest.created_at.to_string(), "2026-10-22T18:30:00Z");
        assert_eq!(runs[1].status, "queued");
        assert_eq!(runs[2].conclusion.as_deref(), Some("success"));
        let page_2 = workflow_runs(&parity_response(
            "workflow-runs",
            &format!("{path}/runs/GET.page-2_per_page-30.json"),
        ))
        .unwrap();
        assert_eq!(page_2.len(), 8);
        let page_3 = workflow_runs(&parity_response(
            "workflow-runs",
            &format!("{path}/runs/GET.page-3_per_page-30.json"),
        ))
        .unwrap();
        assert!(page_3.is_empty());

        let listed = workflows(&parity_response(
            "workflow-runs",
            &format!("{path}/workflows/GET.page-1_per_page-30.json"),
        ))
        .unwrap();
        let names: Vec<&str> = listed.iter().map(|workflow| workflow.name.as_str()).collect();
        assert_eq!(names, ["CI", "Lint", "Release"]);
        assert_eq!(listed[0].state, crate::models::WorkflowState::Active);
        assert_eq!(listed[0].path, ".github/workflows/ci.yml");
    }

    #[test]
    fn workflow_states_use_the_api_spelling() {
        use crate::models::WorkflowState;
        let states: Vec<WorkflowState> = serde_json::from_str(
            r#"["active", "deleted", "disabled_fork", "disabled_inactivity", "disabled_manually"]"#,
        )
        .unwrap();
        assert_eq!(
            states,
            [
                WorkflowState::Active,
                WorkflowState::Deleted,
                WorkflowState::DisabledFork,
                WorkflowState::DisabledInactivity,
                WorkflowState::DisabledManually,
            ]
        );
    }

    fn raw_response(status: &str, body: &str) -> Result<GhResponse, GhError> {
        let stdout = format!("HTTP/2.0 {status}\r\nContent-Type: application/vnd.github.raw\r\n\r\n{body}");
        let exit = i32::from(!status.starts_with('2'));
        gh::interpret(Some(exit), stdout.as_bytes(), b"", false)
    }

    #[test]
    fn a_missing_readme_is_none_and_other_failures_are_errors() {
        let not_found = raw_response("404 Not Found", r#"{"message":"Not Found"}"#);
        assert_eq!(readme_text(not_found).unwrap(), None);
        let server = raw_response("500 Internal Server Error", r#"{"message":"boom"}"#);
        assert!(matches!(readme_text(server), Err(GhError::Http { status: 500, .. })));
        assert!(matches!(
            readme_text(Err(GhError::NotInstalled)),
            Err(GhError::NotInstalled)
        ));
    }

    #[test]
    fn a_readme_comes_back_verbatim() {
        // Blank lines and a header-shaped line survive (Py bug B47).
        let body = "# Title\n\n\nNote: kept\n\nlast\n";
        assert_eq!(
            readme_text(raw_response("200 OK", body)).unwrap().as_deref(),
            Some(body)
        );
        assert_eq!(readme_text(raw_response("200 OK", "")).unwrap().as_deref(), Some(""));
    }

    #[test]
    fn repository_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(gh.repository_request("o/r").argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/o/r"
            ]
        );
    }

    #[test]
    fn a_found_repository_decodes_and_a_missing_one_is_none() {
        let found = found_repository(Ok(parity_response(
            "repos-tracked",
            "repos/parity-org/tracked-tool/GET.json",
        )))
        .unwrap()
        .unwrap();
        assert_eq!(found.full_name, "parity-org/tracked-tool");

        let not_found = raw_response("404 Not Found", r#"{"message":"Not Found"}"#);
        assert_eq!(found_repository(not_found).unwrap(), None);
        let server = raw_response("500 Internal Server Error", r#"{"message":"boom"}"#);
        assert!(matches!(
            found_repository(server),
            Err(GhError::Http { status: 500, .. })
        ));
        assert!(matches!(
            found_repository(Ok(api_response(r#"{"name": 1}"#))),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn pull_request_diff_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        let repos: Vec<Repository> = parity_response(
            "pr-details",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )
        .json()
        .unwrap();
        let repo = &repos[0];
        assert_eq!(repo.full_name, "parity-bot/pr-details");
        assert_eq!(
            strings(
                gh.pull_request_diff_request(repo, 5)
                    .argv(gh.runner().hostname(), false)
            ),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github.diff",
                "/repos/parity-bot/pr-details/pulls/5"
            ]
        );
    }

    #[test]
    fn user_argv_has_no_cache() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(GhCli::user_request("parity-bot").argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/users/parity-bot"
            ]
        );
    }

    #[test]
    fn user_repos_argv_in_python_order_with_the_list_repos_ttl() {
        let mut config = Config::default();
        config.cache.list_repos_ttl = 120;
        let gh = GhCli::new(GhCliOptions::from(&config));
        assert_eq!(
            strings(gh.user_repos_request("parity-bot").argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "120s",
                "-H",
                "Accept: application/vnd.github+json",
                "/users/parity-bot/repos?sort=updated&direction=desc&per_page=30"
            ]
        );
    }

    fn diff_response(status: &str, body: &str) -> Result<GhResponse, GhError> {
        let stdout = format!("HTTP/2.0 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}");
        let exit = i32::from(!status.starts_with('2'));
        gh::interpret(Some(exit), stdout.as_bytes(), b"", false)
    }

    fn json_response(status: &str, body: &str) -> Result<GhResponse, GhError> {
        let stdout = format!("HTTP/2.0 {status}\r\nContent-Type: application/json\r\n\r\n{body}");
        let exit = i32::from(!status.starts_with('2'));
        gh::interpret(Some(exit), stdout.as_bytes(), b"", false)
    }

    #[test]
    fn a_diff_comes_back_verbatim() {
        // Blank lines and a header-shaped line survive (Py bug B47).
        let body = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n \n-Note: x\n+Note: y\n \n";
        assert_eq!(diff_text(diff_response("200 OK", body)).unwrap(), body);
    }

    #[test]
    fn diff_errors_read_as_the_tab_shows_them() {
        let error = |status: &str| diff_text(diff_response(status, r#"{"message":"x"}"#)).unwrap_err();
        let not_found = error("404 Not Found");
        assert!(matches!(not_found, DiffFetchError::NotFound));
        assert_eq!(not_found.message(), "No diff contents found");
        let too_large = error("406 Not Acceptable");
        assert!(matches!(too_large, DiffFetchError::TooLarge));
        assert_eq!(too_large.message(), "Diff too large");
        let server = error("500 Internal Server Error");
        assert!(matches!(server, DiffFetchError::Http(500)));
        assert_eq!(server.message(), "Error fetching diff (500)");
        let offline = diff_text(Err(GhError::Command {
            code: Some(1),
            stderr: "error connecting to api.github.com".into(),
        }))
        .unwrap_err();
        assert!(matches!(offline, DiffFetchError::Gh(GhError::Command { .. })));
        assert_eq!(offline.message(), "Error fetching diff (0)");
        assert!(matches!(
            diff_text(Err(GhError::NotInstalled)),
            Err(DiffFetchError::Gh(GhError::NotInstalled))
        ));
    }

    #[test]
    fn user_responses() {
        let decode = |response: Result<GhResponse, GhError>| response.and_then(|response| response.json::<FullUser>());
        let user = decode(Ok(parity_response("user-profile", "users/parity-bot/GET.json"))).unwrap();
        assert_eq!(user.user.login, "parity-bot");
        assert!(matches!(
            decode(json_response("404 Not Found", r#"{"message":"Not Found"}"#)),
            Err(GhError::Http { status: 404, .. })
        ));
        assert!(matches!(
            decode(json_response("500 Internal Server Error", r#"{"message":"boom"}"#)),
            Err(GhError::Http { status: 500, .. })
        ));
        assert!(matches!(
            decode(json_response("200 OK", r#"{"login": 1}"#)),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn user_repos_responses() {
        let decode =
            |response: Result<GhResponse, GhError>| response.and_then(|response| response.json::<Vec<Repository>>());
        let repos = decode(Ok(parity_response(
            "user-profile",
            "users/parity-bot/repos/GET.direction-desc_page-1_per_page-30_sort-updated.json",
        )))
        .unwrap();
        let names: Vec<&str> = repos.iter().map(|repo| repo.name.as_str()).collect();
        assert_eq!(names, ["dotfiles", "profile-only", "hello-world", "lazy-demo"]);
        assert!(matches!(
            decode(json_response("404 Not Found", r#"{"message":"Not Found"}"#)),
            Err(GhError::Http { status: 404, .. })
        ));
        assert!(matches!(
            decode(json_response("500 Internal Server Error", "{}")),
            Err(GhError::Http { status: 500, .. })
        ));
        assert!(matches!(
            decode(json_response("200 OK", r#"[{"name": 1}]"#)),
            Err(GhError::Decode(_))
        ));
        assert!(matches!(decode(json_response("200 OK", "{}")), Err(GhError::Decode(_))));
    }

    /// `parity-bot/pr-details`, first on the `pr-details` set's repos page.
    fn pr_details_repo() -> Repository {
        let page: Vec<Repository> = parity_response(
            "pr-details",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )
        .json()
        .unwrap();
        page.into_iter().next().unwrap()
    }

    #[test]
    fn pull_request_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(
                gh.pull_request_request(&pr_details_repo(), 5)
                    .argv(gh.runner().hostname(), false)
            ),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/pr-details/pulls/5"
            ]
        );
    }

    #[test]
    fn pull_requests_decode_with_their_repo() {
        let repo = pr_details_repo();
        let pr = |number: u64| {
            let response = parity_response(
                "pr-details",
                &format!("repos/parity-bot/pr-details/pulls/{number}/GET.json"),
            );
            full_pull_request(&response, &repo).unwrap()
        };
        let open = pr(5);
        assert_eq!(open.number(), 5);
        assert_eq!(open.pr.issue.repo, repo);
        assert_eq!(open.pr.issue.state, crate::models::IssueState::Open);
        assert_eq!(open.merged_at, None);
        let closed = pr(4);
        assert_eq!(closed.pr.issue.state, crate::models::IssueState::Closed);
        assert!(closed.pr.issue.closed_at.is_some());
        assert_eq!(closed.merged_at, None);
        let merged = pr(3);
        assert!(merged.merged_at.is_some());
        assert_eq!(merged.pr.issue.body, None);
        assert_eq!(pr(9).head.name, "clamp");
        assert!(matches!(
            full_pull_request(&api_response(r#"{"number": 5}"#), &repo),
            Err(GhError::Decode(_))
        ));
    }

    /// `parity-bot/issue-fixture`, first on the `issue-details` set's repos page.
    fn issue_fixture_repo() -> Repository {
        let page: Vec<Repository> = parity_response(
            "issue-details",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )
        .json()
        .unwrap();
        page.into_iter().next().unwrap()
    }

    #[test]
    fn issue_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(
                gh.issue_request(&issue_fixture_repo(), 3)
                    .argv(gh.runner().hostname(), false)
            ),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/issue-fixture/issues/3"
            ]
        );
    }

    #[test]
    fn an_issue_decodes_as_an_issue_or_a_pull_request() {
        let repo = issue_fixture_repo();
        let item = |number: u64| {
            let response = parity_response(
                "issue-details",
                &format!("repos/parity-bot/issue-fixture/issues/{number}/GET.json"),
            );
            issue_list_item(&response, &repo).unwrap()
        };
        let IssueListItem::Issue(issue) = item(3) else {
            panic!("issue 3 is an issue");
        };
        assert_eq!(issue.number, 3);
        assert_eq!(issue.repo, repo);
        let IssueListItem::PullRequest(pr) = item(8) else {
            panic!("8 is a pull request");
        };
        assert_eq!(pr.issue.number, 8);
        let not_found = json_response("404 Not Found", r#"{"message":"Not Found"}"#)
            .and_then(|response| issue_list_item(&response, &repo));
        assert!(matches!(not_found, Err(GhError::Http { status: 404, .. })));
        assert!(matches!(
            issue_list_item(&api_response(r#"{"number": 3}"#), &repo),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn create_issue_argv_posts_the_body_over_stdin_without_a_cache() {
        let issue = NewIssue {
            title: "Parity title".into(),
            body: "First line\nSecond line".into(),
        };
        let request = GhCli::create_issue_request(&issue_fixture_repo(), &issue).unwrap();
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "POST",
                "-H",
                "Accept: application/vnd.github+json",
                "--input",
                "-",
                "/repos/parity-bot/issue-fixture/issues"
            ]
        );
        // Title, then body, both always there, the body as typed.
        assert_eq!(
            request.body.unwrap().to_string(),
            r#"{"title":"Parity title","body":"First line\nSecond line"}"#
        );
        let empty = NewIssue {
            title: " ".into(),
            body: String::new(),
        };
        let request = GhCli::create_issue_request(&issue_fixture_repo(), &empty).unwrap();
        assert_eq!(request.body.unwrap().to_string(), r#"{"title":" ","body":""}"#);
    }

    #[test]
    fn update_issue_argv_patches_the_body_over_stdin_without_a_cache() {
        let update = IssueUpdate {
            title: "T".into(),
            body: "B\n".into(),
            state: Some(IssueState::Closed),
        };
        let request = GhCli::update_issue_request(&issue_fixture_repo(), 12, &update).unwrap();
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "PATCH",
                "-H",
                "Accept: application/vnd.github+json",
                "--input",
                "-",
                "/repos/parity-bot/issue-fixture/issues/12"
            ]
        );
        assert_eq!(
            request.body.unwrap().to_string(),
            r#"{"title":"T","body":"B\n","state":"closed"}"#
        );
        let without_state = IssueUpdate { state: None, ..update };
        let request = GhCli::update_issue_request(&issue_fixture_repo(), 12, &without_state).unwrap();
        assert_eq!(request.body.unwrap().to_string(), r#"{"title":"T","body":"B\n"}"#);
    }

    #[test]
    fn an_updated_issue_decodes_tagged_with_its_repo() {
        let repo = issue_fixture_repo();
        let response = parity_response("issue-write", "repos/parity-bot/issue-fixture/issues/12/PATCH.json");
        let issue = written_issue(&response, &repo).unwrap();
        assert_eq!(issue.number, 12);
        assert_eq!(issue.title, "Crash when the config file is blank");
        assert_eq!(issue.state, IssueState::Closed);
        assert_eq!(issue.repo, repo);
    }

    #[test]
    fn create_issue_comment_argv_posts_the_body_over_stdin() {
        let request = GhCli::create_issue_comment_request(&issue_fixture_repo(), 12, "A **new** comment");
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "POST",
                "-H",
                "Accept: application/vnd.github+json",
                "--input",
                "-",
                "/repos/parity-bot/issue-fixture/issues/12/comments"
            ]
        );
        assert_eq!(request.body.unwrap().to_string(), r#"{"body":"A **new** comment"}"#);
        let empty = GhCli::create_issue_comment_request(&issue_fixture_repo(), 12, "");
        assert_eq!(empty.body.unwrap().to_string(), r#"{"body":""}"#);
    }

    #[test]
    fn reply_to_review_comment_argv_posts_the_body_over_stdin() {
        let repo = issue_fixture_repo();
        let request = GhCli::reply_to_review_comment_request(&repo, 5, 9001, "It stops, yes.");
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "POST",
                "-H",
                "Accept: application/vnd.github+json",
                "--input",
                "-",
                "/repos/parity-bot/issue-fixture/pulls/5/comments/9001/replies"
            ]
        );
        assert_eq!(request.body.unwrap().to_string(), r#"{"body":"It stops, yes."}"#);
    }

    #[test]
    fn add_issue_reaction_argv_posts_each_kind_over_stdin() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        for kind in ReactionKind::ALL {
            let request = GhCli::add_issue_reaction_request(&pr_details_repo(), 5, kind);
            assert_eq!(
                strings(request.argv(gh.runner().hostname(), false)),
                [
                    "api",
                    "-i",
                    "-X",
                    "POST",
                    "-H",
                    "Accept: application/vnd.github+json",
                    "--input",
                    "-",
                    "/repos/parity-bot/pr-details/issues/5/reactions"
                ]
            );
            assert_eq!(
                request.body.unwrap().to_string(),
                format!(r#"{{"content":"{}"}}"#, kind.api_name())
            );
        }
    }

    #[test]
    fn an_added_reaction_is_created_by_a_201_only() {
        let answer = |status: &str, exit: i32| {
            let stdout = format!("HTTP/2.0 {status}\r\nContent-Type: application/json\r\n\r\n{{}}");
            reaction_created(gh::interpret(Some(exit), stdout.as_bytes(), b"", false))
        };
        assert!(answer("201 Created", 0).unwrap());
        assert!(!answer("200 OK", 0).unwrap());
        assert!(matches!(
            answer("422 Unprocessable Entity", 1),
            Err(GhError::Http { status: 422, .. })
        ));
        assert!(matches!(
            answer("404 Not Found", 1),
            Err(GhError::Http { status: 404, .. })
        ));
    }

    #[test]
    fn merge_pull_request_argv_puts_each_method_and_the_head_sha_over_stdin() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        for (method, name) in [
            (MergeMethod::Merge, "merge"),
            (MergeMethod::Squash, "squash"),
            (MergeMethod::Rebase, "rebase"),
        ] {
            let merge = MergeRequest {
                merge_method: method,
                sha: "c0ffee5555555555555555555555555555555555".into(),
            };
            let request = GhCli::merge_pull_request_request(&pr_details_repo(), 5, &merge).unwrap();
            assert_eq!(
                strings(request.argv(gh.runner().hostname(), false)),
                [
                    "api",
                    "-i",
                    "-X",
                    "PUT",
                    "-H",
                    "Accept: application/vnd.github+json",
                    "--input",
                    "-",
                    "/repos/parity-bot/pr-details/pulls/5/merge"
                ]
            );
            assert_eq!(
                request.body.unwrap().to_string(),
                format!(r#"{{"merge_method":"{name}","sha":"c0ffee5555555555555555555555555555555555"}}"#)
            );
        }
    }

    #[test]
    fn merge_results_decode_and_a_refused_merge_is_an_error() {
        let merged = parity_fixture("pr-write", "repos/parity-bot/pr-details/pulls/5/merge/PUT.json");
        let merged = merge_result(json_response("200 OK", &merged["response"]["body"].to_string())).unwrap();
        assert_eq!(
            merged,
            MergeResult {
                merged: true,
                message: "Pull Request successfully merged".into(),
                sha: Some("5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed".into()),
            }
        );
        let not_merged = merge_result(json_response("200 OK", r#"{"merged":false,"message":"Nope"}"#)).unwrap();
        assert!(!not_merged.merged);
        assert_eq!(not_merged.sha, None);
        let refused = parity_fixture("pr-write", "repos/parity-bot/pr-details/pulls/4/merge/PUT.json");
        assert!(matches!(
            merge_result(json_response(
                "405 Method Not Allowed",
                &refused["response"]["body"].to_string()
            )),
            Err(GhError::Http { status: 405, .. })
        ));
    }

    #[test]
    fn create_review_argv_posts_the_review_over_stdin() {
        let review = NewReview {
            event: ReviewEvent::Approve,
            commit_id: "c0ffee5555555555555555555555555555555555".into(),
            body: "Looks good.".into(),
            comments: vec![DraftReviewComment {
                path: "README.md".into(),
                body: "Mention the arrow keys too.".into(),
                position: 4,
            }],
        };
        let request = GhCli::create_review_request(&pr_details_repo(), 5, &review).unwrap();
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "POST",
                "-H",
                "Accept: application/vnd.github+json",
                "--input",
                "-",
                "/repos/parity-bot/pr-details/pulls/5/reviews"
            ]
        );
        // The scenario's body, keys in Python's order.
        let fixture = parity_fixture("pr-write", "repos/parity-bot/pr-details/pulls/5/reviews/POST.json");
        assert_eq!(
            request.body.unwrap().to_string(),
            fixture["request"]["body"].to_string()
        );
    }

    #[test]
    fn each_review_event_is_sent_by_its_api_name_and_no_comments_is_an_empty_list() {
        for (event, name) in [
            (ReviewEvent::Approve, "APPROVE"),
            (ReviewEvent::RequestChanges, "REQUEST_CHANGES"),
            (ReviewEvent::Comment, "COMMENT"),
        ] {
            let review = NewReview {
                event,
                commit_id: "abc".into(),
                body: String::new(),
                comments: Vec::new(),
            };
            let request = GhCli::create_review_request(&pr_details_repo(), 5, &review).unwrap();
            assert_eq!(
                request.body.unwrap().to_string(),
                format!(r#"{{"event":"{name}","commit_id":"abc","body":"","comments":[]}}"#)
            );
        }
    }

    #[test]
    fn a_created_review_decodes() {
        let response = parity_response("pr-write", "repos/parity-bot/pr-details/pulls/5/reviews/POST.json");
        let review: Review = response.json().unwrap();
        assert_eq!(review.id, 508);
        assert_eq!(review.state, ReviewState::Approved);
        assert_eq!(review.body, "Looks good.");
    }

    #[test]
    fn a_review_reply_decodes() {
        let response = parity_response(
            "pr-write",
            "repos/parity-bot/pr-details/pulls/5/comments/9001/replies/POST.json",
        );
        let reply: ReviewComment = response.json().unwrap();
        assert_eq!(reply.comment.id, 9004);
        assert_eq!(reply.comment.body, "It stops, yes.");
        assert_eq!(reply.path, "src/viewer.rs");
    }

    #[test]
    fn a_posted_comment_decodes() {
        let response = parity_response(
            "issue-write",
            "repos/parity-bot/issue-fixture/issues/12/comments/POST.json",
        );
        let comment: IssueComment = response.json().unwrap();
        assert_eq!(comment.body, "A **new** comment");
        assert_eq!(comment.user.map(|user| user.login).as_deref(), Some("parity-bot"));
    }

    #[test]
    fn a_created_issue_decodes_tagged_with_its_repo() {
        let repo = issue_fixture_repo();
        let response = parity_response("issue-write", "repos/parity-bot/issue-fixture/issues/POST.json");
        let issue = written_issue(&response, &repo).unwrap();
        assert_eq!(issue.number, 13);
        assert_eq!(issue.title, "Parity title");
        assert_eq!(issue.body.as_deref(), Some("First line\nSecond line"));
        assert_eq!(issue.repo, repo);
        assert!(matches!(
            written_issue(&api_response(r#"{"number": 13}"#), &repo),
            Err(GhError::Decode(_))
        ));
    }

    /// `parity-bot/run-fixture`, first on the `run-details` set's repos page.
    fn run_fixture_repo() -> Repository {
        let page: Vec<Repository> = parity_response(
            "run-details",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )
        .json()
        .unwrap();
        page.into_iter().next().unwrap()
    }

    /// The jobs `gh api --paginate --slurp` prints for pages with these bodies.
    fn slurped(bodies: &[String]) -> Result<Vec<WorkflowJob>, GhError> {
        let pages: Vec<JobsPage> = api_response(&format!("[{}]", bodies.join(","))).json()?;
        Ok(workflow_jobs(pages))
    }

    /// The `response.body` of a `run-details` jobs fixture.
    fn run_jobs_body(run_id: u64) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../parity/fixtures/run-details/repos/parity-bot/run-fixture/actions/runs/{run_id}/jobs/GET.json"
        ));
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        fixture["response"]["body"].to_string()
    }

    #[test]
    fn workflow_run_jobs_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        let request = GhCli::workflow_run_jobs_request(&run_fixture_repo(), 5_000_042);
        // Every page, slurped; no `--cache` and no query, as Python sends neither. `--paginate` drops `-i`.
        assert_eq!(
            strings(request.slurp_argv(gh.runner().hostname())),
            [
                "api",
                "-X",
                "GET",
                "--paginate",
                "--slurp",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/run-fixture/actions/runs/5000042/jobs"
            ]
        );
    }

    #[test]
    fn workflow_run_jobs_decode_from_the_fixtures() {
        use crate::models::{JobConclusion, JobStatus};

        let jobs = slurped(&[run_jobs_body(5_000_042)]).unwrap();
        let names: Vec<&str> = jobs.iter().map(|job| job.name.as_str()).collect();
        assert_eq!(names, ["build", "test", "lint", "deploy"]);
        let build = &jobs[0];
        assert_eq!(
            (build.id, build.run_id, build.run_attempt),
            (6_000_001, 5_000_042, Some(2))
        );
        assert_eq!(build.status, JobStatus::Completed);
        assert_eq!(build.conclusion, Some(JobConclusion::Success));
        assert_eq!(build.labels, ["ubuntu-latest"]);
        assert_eq!(build.steps.len(), 3);
        assert_eq!(build.steps[1].name, "Run cargo build");
        assert_eq!(build.steps[1].number, 2);
        assert!(jobs[1].failed());
        assert!(!build.failed());
        assert_eq!(jobs[2].conclusion, Some(JobConclusion::Skipped));
        assert_eq!(jobs[3].conclusion, Some(JobConclusion::Cancelled));

        let in_progress = slurped(&[run_jobs_body(5_000_041)]).unwrap();
        assert_eq!(in_progress[0].status, JobStatus::InProgress);
        assert_eq!(in_progress[0].completed_at, None);
        assert_eq!(in_progress[0].steps[2].status, JobStatus::Pending);
        assert_eq!(in_progress[1].status, JobStatus::Queued);
        assert_eq!(in_progress[1].started_at, None);

        assert!(slurped(&[run_jobs_body(5_000_040)]).unwrap().is_empty());
    }

    #[test]
    fn job_logs_argv() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        let request = GhCli::job_logs_request(&run_fixture_repo(), 6_000_001);
        // No `--cache`: Python sends no `Cache-Control`.
        assert_eq!(
            strings(request.argv(gh.runner().hostname(), false)),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/run-fixture/actions/jobs/6000001/logs"
            ]
        );
    }

    fn text_response(status: &str, body: &str) -> Result<GhResponse, GhError> {
        let stdout = format!("HTTP/2.0 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}");
        let exit = i32::from(!status.starts_with('2'));
        gh::interpret(Some(exit), stdout.as_bytes(), b"", false)
    }

    #[test]
    fn job_logs_are_verbatim_none_when_empty_and_a_404_is_an_error() {
        // A blank line and a header-shaped line survive.
        let body = "2026-03-04T05:06:10.0000000Z ##[group]Run cargo build\n\nWord: value\nlast\n";
        assert_eq!(
            job_log_text(text_response("200 OK", body)).unwrap().as_deref(),
            Some(body)
        );
        assert_eq!(job_log_text(text_response("200 OK", "")).unwrap(), None);
        let not_found = text_response("404 Not Found", r#"{"message":"Not Found"}"#);
        assert!(matches!(
            job_log_text(not_found),
            Err(GhError::Http { status: 404, .. })
        ));
        assert!(matches!(
            job_log_text(Err(GhError::NotInstalled)),
            Err(GhError::NotInstalled)
        ));
    }

    #[test]
    fn missing_and_null_jobs_are_empty_pages() {
        assert!(slurped(&[r#"{"total_count": 0}"#.into()]).unwrap().is_empty());
        assert!(
            slurped(&[r#"{"total_count": 0, "jobs": null}"#.into()])
                .unwrap()
                .is_empty()
        );
        assert!(slurped(&[]).unwrap().is_empty());
        assert!(matches!(
            slurped(&[r#"{"jobs": [{"id": 1}]}"#.into()]),
            Err(GhError::Decode(_))
        ));
    }

    /// One page's body with a job per `(name, status, conclusion)`, spelled as the API does.
    fn job_page(jobs: &[(&str, &str, Option<&str>)]) -> String {
        let jobs: Vec<serde_json::Value> = jobs
            .iter()
            .enumerate()
            .map(|(i, (name, status, conclusion))| {
                serde_json::json!({
                    "id": i, "run_id": 1, "name": name, "status": status, "conclusion": conclusion,
                    "html_url": "https://github.com/o/r/actions/runs/1/job/1",
                })
            })
            .collect();
        serde_json::json!({ "total_count": jobs.len(), "jobs": jobs }).to_string()
    }

    #[test]
    fn unknown_statuses_and_conclusions_decode_to_other() {
        use crate::models::{JobConclusion, JobStatus};

        let jobs = slurped(&[job_page(&[("a", "stalled", Some("exploded")), ("b", "waiting", None)])]).unwrap();
        assert_eq!(jobs[0].status, JobStatus::Other("stalled".into()));
        assert_eq!(jobs[0].conclusion, Some(JobConclusion::Other("exploded".into())));
        assert_eq!(jobs[0].status.as_str(), "stalled");
        assert_eq!(jobs[1].status, JobStatus::Waiting);
        assert!(jobs[1].steps.is_empty());
        assert!(jobs[1].labels.is_empty());
        // They serialize back as the API spells them.
        let json = serde_json::to_value(&jobs[0]).unwrap();
        assert_eq!(
            (json["status"].as_str(), json["conclusion"].as_str()),
            (Some("stalled"), Some("exploded"))
        );
        assert_eq!(serde_json::to_value(JobStatus::InProgress).unwrap(), "in_progress");
        assert_eq!(
            serde_json::to_value(JobConclusion::ActionRequired).unwrap(),
            "action_required"
        );
    }

    #[test]
    fn slurped_pages_flatten_in_order() {
        let first = job_page(&[("a", "completed", Some("success")), ("b", "queued", None)]);
        let second = job_page(&[("c", "in_progress", None)]);
        let names: Vec<String> = slurped(&[first, r#"{"total_count": 3}"#.into(), second])
            .unwrap()
            .into_iter()
            .map(|job| job.name)
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn commit_checks_argv_ask_for_the_first_hundred_with_the_default_ttl() {
        let mut config = Config::default();
        config.cache.default_ttl = 42;
        let gh = GhCli::new(GhCliOptions::from(&config));
        let sha = "c0ffee5555555555555555555555555555555555";
        for what in ["status", "check-runs"] {
            assert_eq!(
                strings(
                    gh.commit_checks_request(&pr_details_repo(), sha, what)
                        .argv(gh.runner().hostname(), false)
                ),
                [
                    "api",
                    "-i",
                    "-X",
                    "GET",
                    "--cache",
                    "42s",
                    "-H",
                    "Accept: application/vnd.github+json",
                    &format!("/repos/parity-bot/pr-details/commits/{sha}/{what}?page=1&per_page=100"),
                ]
            );
        }
    }

    #[test]
    fn issue_reactions_argv_has_no_page_parameters() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        assert_eq!(
            strings(
                gh.issue_reactions_request(&pr_details_repo(), 5)
                    .argv(gh.runner().hostname(), false)
            ),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/pr-details/issues/5/reactions"
            ]
        );
    }

    fn commit_fixture(number: u64, what: &str) -> GhResponse {
        let sha = format!("c0ffee{}", number.to_string().repeat(34));
        parity_response(
            "pr-details",
            &format!("repos/parity-bot/pr-details/commits/{sha}/{what}/GET.page-1_per_page-100.json"),
        )
    }

    #[test]
    fn commit_checks_decode_from_the_pr_details_fixtures() {
        use crate::models::{CheckConclusion, CheckRunStatus, CheckState};

        let status: CombinedStatus = commit_fixture(5, "status").json().unwrap();
        assert_eq!(status.state, CheckState::Pending);
        let statuses: Vec<(&str, CheckState, Option<&str>)> = status
            .statuses
            .iter()
            .map(|s| (s.context.as_str(), s.state, s.target_url.as_deref()))
            .collect();
        assert_eq!(
            statuses,
            [
                ("ci/lint", CheckState::Success, Some("https://ci.example.com/lint/5")),
                ("ci/docs", CheckState::Pending, None),
            ]
        );
        let runs = check_run_list(&commit_fixture(5, "check-runs")).unwrap();
        let runs: Vec<(&str, CheckRunStatus, Option<CheckConclusion>)> = runs
            .iter()
            .map(|run| (run.name.as_str(), run.status, run.conclusion))
            .collect();
        assert_eq!(
            runs,
            [
                ("build", CheckRunStatus::Completed, Some(CheckConclusion::Success)),
                ("test", CheckRunStatus::Completed, Some(CheckConclusion::Failure)),
                ("deploy", CheckRunStatus::InProgress, None),
            ]
        );

        let none: CombinedStatus = commit_fixture(4, "status").json().unwrap();
        assert!(none.statuses.is_empty());
        assert!(check_run_list(&commit_fixture(4, "check-runs")).unwrap().is_empty());

        let merged: CombinedStatus = commit_fixture(3, "status").json().unwrap();
        assert!(merged.statuses.is_empty());
        let runs = check_run_list(&commit_fixture(3, "check-runs")).unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.state() == CheckState::Success));

        assert!(matches!(
            check_run_list(&api_response(r#"{"check_runs": [{"id": 1}]}"#)),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn issue_reactions_decode_from_the_pr_details_fixtures() {
        use crate::models::ReactionKind;

        let decode = |number: u64| {
            reactions(&parity_response(
                "pr-details",
                &format!("repos/parity-bot/pr-details/issues/{number}/reactions/GET.json"),
            ))
            .unwrap()
        };
        let open = decode(5);
        let got: Vec<(ReactionKind, Vec<&str>)> = open
            .by_kind()
            .map(|(kind, users)| (kind, users.iter().map(|u| u.login.as_str()).collect()))
            .collect();
        assert_eq!(
            got,
            [
                (ReactionKind::ThumbsUp, vec!["parity-bot", "parity-friend"]),
                (ReactionKind::Hooray, vec!["parity-bot"]),
                (
                    ReactionKind::Eyes,
                    vec!["parity-bot", "parity-reviewer", "parity-carol", "parity-dave"]
                ),
            ]
        );
        assert!(decode(4).is_empty());
        let merged: Vec<(ReactionKind, usize)> = decode(3).by_kind().map(|(kind, users)| (kind, users.len())).collect();
        assert_eq!(merged, [(ReactionKind::Hooray, 2)]);
        assert!(matches!(
            reactions(&api_response(r#"{"message": "x"}"#)),
            Err(GhError::Decode(_))
        ));
    }

    /// Issue or pull request `number` of `parity-bot/pr-details`, from the `pr-details` set's first issues page.
    fn pr_details_issue(number: u64) -> Issue {
        let page: Vec<serde_json::Value> = parity_response(
            "pr-details",
            "repos/parity-bot/pr-details/issues/GET.page-1_per_page-30_state-all.json",
        )
        .json()
        .unwrap();
        page.into_iter()
            .map(|item| IssueListItem::from_api_item(item, &pr_details_repo()).unwrap())
            .find(|item| item.issue().number == number)
            .unwrap()
            .issue()
            .clone()
    }

    #[test]
    fn issue_comments_argv_pages_the_comments_url_with_the_default_ttl() {
        let mut config = Config::default();
        config.cache.default_ttl = 42;
        let gh = GhCli::new(GhCliOptions::from(&config));
        let issue = pr_details_issue(5);
        assert_eq!(
            issue.comments_url,
            "https://api.github.com/repos/parity-bot/pr-details/issues/5/comments"
        );
        // Every page (B64), the first one keyed as Python asks for it: no query.
        assert_eq!(
            strings(gh.issue_comments_request(&issue).argv(gh.runner().hostname(), true)),
            [
                "api",
                "-X",
                "GET",
                "--cache",
                "42s",
                "--paginate",
                "-H",
                "Accept: application/vnd.github+json",
                "/repos/parity-bot/pr-details/issues/5/comments"
            ]
        );
    }

    #[test]
    fn api_paths_drop_the_api_root() {
        assert_eq!(
            api_path("https://api.github.com/repos/o/r/issues/5/comments").as_deref(),
            Some("/repos/o/r/issues/5/comments")
        );
        assert_eq!(
            api_path("https://ghe.example.com/api/v3/repos/o/r/issues/5/comments").as_deref(),
            Some("/repos/o/r/issues/5/comments")
        );
        assert_eq!(
            api_path("/repos/o/r/issues/5/comments").as_deref(),
            Some("/repos/o/r/issues/5/comments")
        );
        assert_eq!(api_path("https://api.github.com/user"), None);

        // A `comments_url` without a repos path falls back to the issue's own.
        let gh = GhCli::default();
        let mut issue = pr_details_issue(5);
        issue.comments_url = "not a url".into();
        assert_eq!(
            gh.issue_comments_request(&issue).path,
            "/repos/parity-bot/pr-details/issues/5/comments"
        );
    }

    #[test]
    fn comment_reactions_argv_by_kind() {
        let gh = GhCli::new(GhCliOptions::from(&Config::default()));
        for (comment, path) in [
            (CommentRef::Issue(7001), "issues/comments/7001"),
            (CommentRef::Review(9001), "pulls/comments/9001"),
        ] {
            assert_eq!(
                strings(
                    gh.comment_reactions_request(&pr_details_repo(), comment)
                        .argv(gh.runner().hostname(), false)
                ),
                [
                    "api",
                    "-i",
                    "-X",
                    "GET",
                    "--cache",
                    "600s",
                    "-H",
                    "Accept: application/vnd.github+json",
                    &format!("/repos/parity-bot/pr-details/{path}/reactions?page=1&per_page=100"),
                ]
            );
        }
    }

    #[test]
    fn issue_comments_decode_from_the_pr_details_fixtures() {
        let comments: Vec<IssueComment> =
            parity_response("pr-details", "repos/parity-bot/pr-details/issues/5/comments/GET.json")
                .json()
                .unwrap();
        let got: Vec<(u64, Option<&str>, &str)> = comments
            .iter()
            .map(|c| (c.id, c.user.as_ref().map(|u| u.login.as_str()), c.body.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    7001,
                    Some("parity-friend"),
                    "Rebased on main. @parity-bot ready for another look."
                ),
                (7002, None, "Is this related to #4?"),
                (7003, Some("parity-carol"), ""),
            ]
        );
        assert_eq!(comments[0].author_association, "CONTRIBUTOR");
        assert_eq!(
            comments[1].created_at,
            "2026-02-03T13:30:00Z".parse::<jiff::Timestamp>().unwrap()
        );
        assert!(matches!(
            json_response("404 Not Found", r#"{"message":"Not Found"}"#),
            Err(GhError::Http { status: 404, .. })
        ));
        assert!(matches!(
            api_response(r#"[{"id": "x"}]"#).json::<Vec<IssueComment>>(),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn comment_reactions_decode_from_the_pr_details_fixtures() {
        use crate::models::ReactionKind;

        let decode = |file: &str| {
            reactions(&parity_response(
                "pr-details",
                &format!("repos/parity-bot/pr-details/{file}/reactions/GET.page-1_per_page-100.json"),
            ))
            .unwrap()
        };
        let kinds = |reactions: &Reactions| -> Vec<(ReactionKind, Vec<String>)> {
            reactions
                .by_kind()
                .map(|(kind, users)| (kind, users.iter().map(|u| u.login.clone()).collect()))
                .collect()
        };
        assert_eq!(
            kinds(&decode("issues/comments/7001")),
            [
                (ReactionKind::ThumbsUp, vec!["parity-bot".to_owned()]),
                (ReactionKind::Rocket, vec!["parity-reviewer".to_owned()]),
            ]
        );
        assert_eq!(
            kinds(&decode("pulls/comments/9001")),
            [(ReactionKind::Eyes, vec!["parity-friend".to_owned()])]
        );
        // Python's B54 request: the review comment's id on the issue-comment endpoint is a 404, an error.
        let not_found = json_response("404 Not Found", r#"{"message":"Not Found"}"#).and_then(|r| reactions(&r));
        assert!(matches!(not_found, Err(GhError::Http { status: 404, .. })));
    }

    /// Pull request `number` of `parity-bot/pr-details`.
    fn pr_details_pr(number: u64) -> FullPullRequest {
        let body: serde_json::Value = parity_response(
            "pr-details",
            &format!("repos/parity-bot/pr-details/pulls/{number}/GET.json"),
        )
        .json()
        .unwrap();
        FullPullRequest::from_api(body, &pr_details_repo()).unwrap()
    }

    #[test]
    fn pull_request_reviews_argv_pages_with_the_default_ttl() {
        let mut config = Config::default();
        config.cache.default_ttl = 42;
        let gh = GhCli::new(GhCliOptions::from(&config));
        let argv = |request: GhRequest| strings(request.argv(gh.runner().hostname(), true));
        let expected = |path: &str| {
            [
                "api",
                "-X",
                "GET",
                "--cache",
                "42s",
                "--paginate",
                "-H",
                "Accept: application/vnd.github+json",
                path,
            ]
            .map(str::to_owned)
        };
        // Every page, the first one keyed as Python asks for it: no query.
        assert_eq!(
            argv(gh.pull_request_reviews_request(&pr_details_repo(), 5)),
            expected("/repos/parity-bot/pr-details/pulls/5/reviews")
        );
        assert_eq!(
            argv(gh.review_comments_request(&pr_details_repo(), 5, 501)),
            expected("/repos/parity-bot/pr-details/pulls/5/reviews/501/comments")
        );
    }

    /// The `pr-details` reviews of PR `number`, through [`with_review_comments`] over the fixtures, and the reviews
    /// whose comments were asked for.
    async fn fixture_reviews(number: u64) -> (Result<Vec<Review>, GhError>, Vec<u64>) {
        let reviews: Vec<Review> = parity_response(
            "pr-details",
            &format!("repos/parity-bot/pr-details/pulls/{number}/reviews/GET.json"),
        )
        .json()
        .unwrap();
        let mut asked = Vec::new();
        let result = with_review_comments(reviews, |id| {
            asked.push(id);
            let comments = parity_response(
                "pr-details",
                &format!("repos/parity-bot/pr-details/pulls/{number}/reviews/{id}/comments/GET.json"),
            )
            .json::<Vec<ReviewComment>>();
            std::future::ready(comments)
        })
        .await;
        (result, asked)
    }

    /// A review's id, author, state and comments (each its id and what it answers).
    type ReviewOutline<'a> = (u64, &'a str, ReviewState, Vec<(u64, Option<u64>)>);

    #[tokio::test]
    async fn pull_request_reviews_decode_from_the_pr_details_fixtures() {
        assert_eq!(pr_details_pr(5).number(), 5);
        let (reviews, asked) = fixture_reviews(5).await;
        let reviews = reviews.unwrap();
        // The pending 506 is dropped, and its comments never asked for.
        assert_eq!(asked, [501, 502, 503, 504, 505]);
        let got: Vec<ReviewOutline<'_>> = reviews
            .iter()
            .map(|review| {
                (
                    review.id,
                    review.user.login.as_str(),
                    review.state,
                    review
                        .comments
                        .iter()
                        .map(|c| (c.comment.id, c.in_reply_to_id))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (501, "parity-bot", ReviewState::Commented, vec![(9001, None)]),
                (502, "parity-friend", ReviewState::Commented, vec![(9002, Some(9001))]),
                (503, "parity-reviewer", ReviewState::Approved, vec![]),
                (504, "parity-carol", ReviewState::Dismissed, vec![]),
                (505, "parity-bot", ReviewState::ChangesRequested, vec![(9003, None)]),
            ]
        );
        assert_eq!(reviews[0].body, "Looks close. One question inline.");
        assert_eq!(
            reviews[0].comments[0].comment.body,
            "Should the cursor wrap at the end?"
        );
        assert_eq!(reviews[0].comments[0].pull_request_review_id, 501);
        assert_eq!(reviews[0].comments[0].path, "src/viewer.rs");

        let (reviews, asked) = fixture_reviews(3).await;
        let reviews = reviews.unwrap();
        assert_eq!(asked, [301]);
        assert_eq!(
            (reviews[0].state, reviews[0].body.as_str()),
            (ReviewState::Approved, "Ship it.")
        );

        let (reviews, asked) = fixture_reviews(4).await;
        assert!(reviews.unwrap().is_empty());
        assert!(asked.is_empty());
    }

    #[tokio::test]
    async fn a_failed_comments_request_fails_the_reviews() {
        let reviews: Vec<Review> =
            parity_response("pr-details", "repos/parity-bot/pr-details/pulls/5/reviews/GET.json")
                .json()
                .unwrap();
        let mut asked = Vec::new();
        let result = with_review_comments(reviews, |id| {
            asked.push(id);
            std::future::ready(if id == 502 {
                json_response("500 Internal Server Error", r#"{"message":"boom"}"#).and_then(|r| r.json())
            } else {
                Ok(Vec::new())
            })
        })
        .await;
        assert!(matches!(result, Err(GhError::Http { status: 500, .. })));
        assert_eq!(asked, [501, 502]);
    }
}
