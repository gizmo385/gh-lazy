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

use crate::config::{CacheSettings, Config};
use crate::gh::{self, GhError, GhRequest, GhResponse};
use crate::models::{Repository, User};

#[cfg(any(test, feature = "test-util"))]
mod fake;

#[cfg(any(test, feature = "test-util"))]
pub use fake::{FakeGithub, GithubCall};

/// Page size for `/user/repos`. Kept at Python's 50 so both apps request the same pages and share fixtures.
const REPOS_PER_PAGE: &str = "50";

/// Page size for `/notifications`: Python's `NOTIFICATIONS_PAGE_COUNT`. Only the first page is counted, so this is
/// also the most [`Github::unread_notification_count`] returns.
pub const NOTIFICATIONS_PER_PAGE: usize = 50;

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
}

impl Default for GhCliOptions {
    /// `gh` from `PATH`, github.com, and the default TTLs.
    fn default() -> Self {
        Self {
            program: PathBuf::from("gh"),
            hostname: None,
            ttls: CacheTtls::default(),
        }
    }
}

impl From<&Config> for GhCliOptions {
    fn from(config: &Config) -> Self {
        Self {
            program: PathBuf::from("gh"),
            hostname: hostname_for_base_url(&config.api.base_url),
            ttls: CacheTtls::from(&config.cache),
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
                runner: gh::GhCli::new(options.program, options.hostname),
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
    }
}
