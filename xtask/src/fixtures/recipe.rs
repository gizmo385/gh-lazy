//! Recipes (`parity/fixtures/<set>/recipe.toml`): the committed list of requests a set is recorded from, plus the
//! scope rules that decide what may be recorded at all.

use std::path::{Path, PathBuf};

use fake_gh::fixture::{Accept, Query, RequestKey, normalize_path, normalize_query};
use fake_gh::log::LogEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only recipe version this tool reads.
pub const RECIPE_VERSION: u32 = 1;

/// A parsed recipe file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub version: u32,
    /// `owner/name` of each repo whose `/repos/<owner>/<name>/...` paths may be recorded.
    #[serde(default)]
    pub allowed_repos: Vec<String>,
    /// Logins whose `/users/<login>` and `/users/<login>/repos` may be recorded.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default, rename = "request")]
    pub requests: Vec<RecipeEntry>,
}

/// One `[[request]]` of a recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeEntry {
    /// Only `GET` is accepted; anything else is refused by [`Recipe::check_scope`].
    #[serde(default = "default_method")]
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Query::is_empty")]
    pub query: Query,
    #[serde(default, skip_serializing_if = "Accept::is_json")]
    pub accept: Accept,
    /// The body is stored as a `body_file` and never text-scanned.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
}

fn default_method() -> String {
    "GET".to_owned()
}

impl RecipeEntry {
    /// A plain `GET` of `path`.
    #[cfg(test)]
    pub fn get(path: &str) -> RecipeEntry {
        RecipeEntry {
            method: default_method(),
            path: path.to_owned(),
            query: Query::new(),
            accept: Accept::Json,
            binary: false,
        }
    }

    /// The normalized fixture key. A `per_page` query with no `page` gets `page = "1"`, as fake-gh keys it.
    pub fn key(&self) -> RequestKey {
        let method = self.method.to_ascii_uppercase();
        let mut query = self.query.clone();
        normalize_query(&method, &mut query);
        RequestKey {
            method,
            path: normalize_path(&self.path),
            query,
            accept: self.accept.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    #[error("can't read {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("{}: unsupported recipe version {version} (expected {RECIPE_VERSION})", path.display())]
    Version { path: PathBuf, version: u32 },
}

impl Recipe {
    pub fn load(path: &Path) -> Result<Recipe, RecipeError> {
        let text = std::fs::read_to_string(path).map_err(|source| RecipeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let recipe: Recipe = toml::from_str(&text).map_err(|source| RecipeError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
        if recipe.version != RECIPE_VERSION {
            return Err(RecipeError::Version {
                path: path.to_path_buf(),
                version: recipe.version,
            });
        }
        Ok(recipe)
    }

    /// Whether `key` may be recorded. The error says why not and where to go instead.
    pub fn check_scope(&self, key: &RequestKey) -> Result<(), String> {
        const BY_HAND: &str =
            "write it by hand in parity/fixtures/base/ (see parity/README.md, \"Recording fixtures\")";
        if key.method != "GET" {
            return Err(format!(
                "only GET requests are recorded; {} fixtures are always written by hand",
                key.method
            ));
        }
        let segments: Vec<&str> = key.path.split('/').skip(1).collect();
        if segments.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
            return Err(format!("path {:?} has empty or relative segments", key.path));
        }
        let repo_allowed = |owner: &str, name: &str| {
            self.allowed_repos.iter().any(|allowed| {
                allowed
                    .split_once('/')
                    .is_some_and(|(o, n)| o.eq_ignore_ascii_case(owner) && n.eq_ignore_ascii_case(name))
            })
        };
        let user_allowed = |login: &str| self.allowed_users.iter().any(|u| u.eq_ignore_ascii_case(login));
        match segments.as_slice() {
            ["emojis" | "rate_limit"] => Ok(()),
            ["repos", _, _, rest @ ..] if is_admin_only(rest) => Err(format!(
                "{} is admin-only repo data (the recorder's token can read it, the public can't); {BY_HAND}",
                key.path
            )),
            ["repos", owner, name, ..] if repo_allowed(owner, name) => Ok(()),
            ["users", login] | ["users", login, "repos"] if user_allowed(login) => Ok(()),
            ["user" | "notifications" | "orgs", ..] => Err(format!(
                "{} reads the signed-in user's private scope, so recording it would leak the recorder's data; {BY_HAND}",
                key.path
            )),
            ["repos", owner, name, ..] => Err(format!("{owner}/{name} is not in allowed_repos")),
            ["users", login, ..] => Err(format!(
                "only /users/<login> and /users/<login>/repos are recorded, for logins in allowed_users \
                 (got {login:?}); {BY_HAND}"
            )),
            _ => Err(format!(
                "{} is outside the recording scope (allowed_repos, allowed_users, /emojis, /rate_limit); {BY_HAND}",
                key.path
            )),
        }
    }
}

/// Repo sub-paths that only admins can read, even on a public repo: webhooks (whose URLs can carry tokens), deploy
/// keys, invitations, collaborators, Actions secrets and variables, environment secrets and traffic.
fn is_admin_only(rest: &[&str]) -> bool {
    matches!(
        rest,
        ["hooks" | "keys" | "invitations" | "collaborators" | "traffic", ..]
            | ["actions", "secrets" | "variables", ..]
            | ["environments", _, "secrets" | "variables", ..]
    )
}

/// A miss from a fake-gh request log.
#[derive(Debug, Clone, PartialEq)]
pub enum LoggedMiss {
    /// A `GET` that can become a recipe entry.
    Get(RecipeEntry),
    /// Anything else: listed as "write these by hand".
    WriteByHand { key: RequestKey, body: Option<Value> },
}

/// Reads the `miss: true` request lines of a fake-gh log.
pub fn read_log_misses(path: &Path) -> Result<Vec<LoggedMiss>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let mut misses = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: LogEntry = serde_json::from_str(line)
            .map_err(|e| format!("{}:{}: not a fake-gh log line: {e}", path.display(), n + 1))?;
        let (true, Some(method), Some(path)) = (entry.miss, entry.method, entry.path) else {
            continue;
        };
        let entry_out = RecipeEntry {
            method: method.to_ascii_uppercase(),
            path,
            query: entry.query.unwrap_or_default(),
            accept: entry.accept.as_deref().map(Accept::normalize).unwrap_or_default(),
            binary: false,
        };
        misses.push(if entry_out.method == "GET" {
            LoggedMiss::Get(entry_out)
        } else {
            LoggedMiss::WriteByHand {
                key: entry_out.key(),
                body: entry.body,
            }
        });
    }
    Ok(misses)
}

/// The TOML text of `entries` as `[[request]]` tables, for appending to a recipe.
pub fn entries_toml(entries: &[RecipeEntry]) -> String {
    #[derive(Serialize)]
    struct Tables<'a> {
        request: &'a [RecipeEntry],
    }
    // Serializing plain strings, string maps and bools can't fail.
    toml::to_string(&Tables { request: entries }).expect("recipe entries serialize")
}

/// A stable order for entries appended from logs: by path, then accept, then query.
pub fn sort_entries(entries: &mut [RecipeEntry]) {
    entries.sort_by(|a, b| {
        let query = |e: &RecipeEntry| serde_json::to_string(&e.query).unwrap_or_default();
        (&a.path, a.accept.as_str(), query(a)).cmp(&(&b.path, b.accept.as_str(), query(b)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake_gh::fixture::query_from_pairs;

    fn recipe() -> Recipe {
        toml::from_str(
            r#"
            version = 1
            allowed_repos = ["gizmo385/gh-lazy"]
            allowed_users = ["gizmo385"]

            [[request]]
            path = "/repos/gizmo385/gh-lazy/issues"
            query = { state = "all", page = "1", per_page = "30" }

            [[request]]
            path = "/repos/gizmo385/gh-lazy/readme"
            accept = "raw"

            [[request]]
            path = "/repos/gizmo385/gh-lazy/actions/runs/123/logs"
            binary = true
            "#,
        )
        .unwrap()
    }

    fn key(method: &str, path: &str) -> RequestKey {
        RequestKey::from_url(method, path, &[], Accept::Json).unwrap()
    }

    #[test]
    fn parses_the_spec_example() {
        let r = recipe();
        assert_eq!(r.requests.len(), 3);
        assert_eq!(
            r.requests[0].query,
            query_from_pairs([("state", "all"), ("page", "1"), ("per_page", "30")])
        );
        assert_eq!(r.requests[0].method, "GET");
        assert_eq!(r.requests[1].accept, Accept::Raw);
        assert!(r.requests[2].binary);
    }

    #[test]
    fn scope_allows_listed_repos_users_and_globals() {
        let r = recipe();
        for path in [
            "/repos/gizmo385/gh-lazy",
            "/repos/Gizmo385/GH-Lazy/pulls/12",
            "/users/gizmo385",
            "/users/gizmo385/repos",
            "/emojis",
            "/rate_limit",
        ] {
            assert_eq!(r.check_scope(&key("GET", path)), Ok(()), "{path}");
        }
    }

    #[test]
    fn scope_refuses_user_scope_other_repos_and_writes() {
        let r = recipe();
        let refused = |method: &str, path: &str| r.check_scope(&key(method, path)).unwrap_err();
        assert!(refused("GET", "/user/repos").contains("parity/fixtures/base/"));
        assert!(refused("GET", "/user").contains("private scope"));
        assert!(refused("GET", "/notifications").contains("private scope"));
        assert!(refused("GET", "/orgs/acme/repos").contains("private scope"));
        assert_eq!(
            refused("GET", "/repos/acme/secret/issues"),
            "acme/secret is not in allowed_repos"
        );
        assert!(refused("GET", "/users/someone-else").contains("allowed_users"));
        assert!(refused("GET", "/users/gizmo385/starred").contains("allowed_users"));
        assert!(refused("GET", "/search/issues").contains("outside the recording scope"));
        assert!(refused("GET", "/repos/gizmo385/gh-lazy/../../user").contains("relative segments"));
        assert!(refused("PUT", "/repos/gizmo385/gh-lazy/pulls/1/merge").contains("only GET"));
    }

    #[test]
    fn scope_refuses_admin_only_repo_paths() {
        let r = recipe();
        for path in [
            "/repos/gizmo385/gh-lazy/hooks",
            "/repos/gizmo385/gh-lazy/hooks/1/config",
            "/repos/gizmo385/gh-lazy/keys",
            "/repos/gizmo385/gh-lazy/invitations",
            "/repos/gizmo385/gh-lazy/collaborators",
            "/repos/gizmo385/gh-lazy/collaborators/someone/permission",
            "/repos/gizmo385/gh-lazy/actions/secrets",
            "/repos/gizmo385/gh-lazy/actions/secrets/public-key",
            "/repos/gizmo385/gh-lazy/actions/variables",
            "/repos/gizmo385/gh-lazy/environments/prod/secrets",
            "/repos/gizmo385/gh-lazy/traffic/views",
        ] {
            let err = r.check_scope(&key("GET", path)).unwrap_err();
            assert!(err.contains("admin-only"), "{path}: {err}");
        }
        // Neighbours stay allowed.
        for path in [
            "/repos/gizmo385/gh-lazy/actions/runs",
            "/repos/gizmo385/gh-lazy/environments",
        ] {
            assert_eq!(r.check_scope(&key("GET", path)), Ok(()), "{path}");
        }
    }

    #[test]
    fn per_page_without_page_keys_as_page_one() {
        let entry = RecipeEntry {
            query: query_from_pairs([("per_page", "30"), ("state", "all")]),
            ..RecipeEntry::get("/repos/gizmo385/gh-lazy/issues")
        };
        assert_eq!(
            entry.key().query,
            query_from_pairs([("page", "1"), ("per_page", "30"), ("state", "all")])
        );
    }

    #[test]
    fn entries_round_trip_through_toml() {
        let mut entries = recipe().requests;
        sort_entries(&mut entries);
        let text = format!("version = 1\n\n{}", entries_toml(&entries));
        let back: Recipe = toml::from_str(&text).unwrap();
        assert_eq!(back.requests, entries);
        assert_eq!(
            entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            [
                "/repos/gizmo385/gh-lazy/actions/runs/123/logs",
                "/repos/gizmo385/gh-lazy/issues",
                "/repos/gizmo385/gh-lazy/readme"
            ]
        );
    }
}
