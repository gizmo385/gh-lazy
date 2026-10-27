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

use crate::config::{CacheSettings, Config, IssueOwnerFilter, IssueStateFilter};
use crate::gh::{self, Accept, GhError, GhRequest, GhResponse, GhTimeouts};
use crate::models::{FullUser, IssueListItem, Repository, User, Workflow, WorkflowRun};

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
}

/// A diff response as the diff text, verbatim, or the error the Diff tab shows.
fn diff_text(response: Result<GhResponse, GhError>) -> Result<String, DiffFetchError> {
    Ok(response?.text())
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

    /// The `response.body` of a recorded parity fixture, as `gh api -i` would print it.
    fn parity_response(set: &str, file: &str) -> GhResponse {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures")
            .join(set)
            .join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
        api_response(&fixture["response"]["body"].to_string())
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
}
