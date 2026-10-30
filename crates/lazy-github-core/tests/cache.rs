//! Table cache files shared with the Python app. The goldens in `tests/fixtures/cache/` are written by the Python app
//! (`generate.py` there) from the parity fixtures; these tests check the Rust app reads them as the models it builds
//! from the same fixtures, and writes those models as the same JSON values.

use std::path::{Path, PathBuf};

use lazy_github_core::cache::{self, CacheScope};
use lazy_github_core::models::{Issue, IssueListItem, Notification, PartialPullRequest, Repository, WorkflowRun};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

const USER_REPOS: &str = "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json";
const CACHE_FIXTURE_ISSUES: &str = "repos/parity-bot/cache-fixture/issues/GET.page-1_per_page-30_state-all.json";
const CACHE_FIXTURE_RUNS: &str = "repos/parity-bot/cache-fixture/actions/runs/GET.page-1_per_page-30.json";
const CACHE_FIXTURE: &str = "parity-bot/cache-fixture";
const NOTIFICATIONS_PAGE: &str = "notifications/GET.all-true_page-1_per_page-50.json";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn goldens() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cache")
}

/// The response body of a recorded parity fixture.
fn body(fixture: &str, path: &str) -> Value {
    let path = root().join("parity/fixtures").join(fixture).join(path);
    let recorded: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    recorded["response"]["body"].clone()
}

/// base's repos, as the Rust client parses `/user/repos`.
fn repos() -> Vec<Repository> {
    serde_json::from_value(body("base", USER_REPOS)).unwrap()
}

/// table-caches' first issues page of cache-fixture, as the Rust client parses it, split as the sections split it.
fn cache_fixture_items() -> (Vec<PartialPullRequest>, Vec<Issue>) {
    let repos: Vec<Repository> = serde_json::from_value(body("table-caches", USER_REPOS)).unwrap();
    let repo = repos.into_iter().find(|repo| repo.full_name == CACHE_FIXTURE).unwrap();
    let Value::Array(items) = body("table-caches", CACHE_FIXTURE_ISSUES) else {
        panic!("the issues page is an array");
    };
    let mut pull_requests = Vec::new();
    let mut issues = Vec::new();
    for item in items {
        match IssueListItem::from_api_item(item, &repo).unwrap() {
            IssueListItem::PullRequest(pr) => pull_requests.push(pr),
            IssueListItem::Issue(issue) => issues.push(issue),
        }
    }
    (pull_requests, issues)
}

/// table-caches' first runs page of cache-fixture, as the Rust client parses it.
fn cache_fixture_runs() -> Vec<WorkflowRun> {
    serde_json::from_value(body("table-caches", CACHE_FIXTURE_RUNS)["workflow_runs"].clone()).unwrap()
}

/// Reads the golden `file` as the Rust app would, checks it gives `expected`, and checks `expected` written by the
/// Rust app is the same JSON as the golden.
fn check<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(file: &str, type_name: &str, expected: &[T]) {
    let path = goldens().join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: Vec<T> = cache::try_parse(&path, &text, type_name).unwrap();
    assert!(!parsed.is_empty(), "{file} has no items");
    assert_eq!(parsed, expected, "{file}");
    let written: Value = serde_json::from_str(&cache::to_json(expected)).unwrap();
    let golden: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written, golden, "{file}");
}

#[test]
fn the_python_files_are_named_as_the_rust_app_names_them() {
    let dir = goldens();
    for (scope, name, file) in [
        (CacheScope::App, "repos", "repos.json"),
        (
            CacheScope::Repo(CACHE_FIXTURE),
            "pull_requests",
            "parity-bot_cache-fixture_pull_requests.json",
        ),
        (
            CacheScope::Repo(CACHE_FIXTURE),
            "issues",
            "parity-bot_cache-fixture_issues.json",
        ),
        (
            CacheScope::Repo(CACHE_FIXTURE),
            "workflow_runs",
            "parity-bot_cache-fixture_workflow_runs.json",
        ),
        (CacheScope::App, cache::NOTIFICATIONS, "notifications.json"),
    ] {
        let path = cache::cache_path(&dir, scope, name);
        assert_eq!(path, dir.join(file));
        assert!(path.is_file(), "{}", path.display());
    }
}

#[test]
fn repos_json_round_trips_with_python() {
    check("repos.json", "Repository", &repos());
}

#[test]
fn pull_requests_json_round_trips_with_python() {
    let (pull_requests, _) = cache_fixture_items();
    check(
        "parity-bot_cache-fixture_pull_requests.json",
        "PartialPullRequest",
        &pull_requests,
    );
}

#[test]
fn issues_json_round_trips_with_python() {
    let (_, issues) = cache_fixture_items();
    check("parity-bot_cache-fixture_issues.json", "Issue", &issues);
}

#[test]
fn workflow_runs_json_round_trips_with_python() {
    check(
        "parity-bot_cache-fixture_workflow_runs.json",
        "WorkflowRun",
        &cache_fixture_runs(),
    );
}

#[test]
fn notifications_json_round_trips_with_python() {
    // Python writes the subject's kind as `subject_type`, which the Rust app reads and writes back.
    let notifications: Vec<Notification> =
        serde_json::from_value(body("notifications-modal", NOTIFICATIONS_PAGE)).unwrap();
    check("notifications.json", "Notification", &notifications);
}

/// Regenerates the goldens with the Python app and checks the committed copies match (the golden manifest is
/// `parity/goldens/table-caches.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn cache_goldens_are_fresh() {
    use std::process::Command;

    const GENERATOR: &str = "crates/lazy-github-core/tests/fixtures/cache/generate.py";
    const GOLDENS: [&str; 5] = [
        "repos.json",
        "parity-bot_cache-fixture_pull_requests.json",
        "parity-bot_cache-fixture_issues.json",
        "parity-bot_cache-fixture_workflow_runs.json",
        "notifications.json",
    ];
    let root = root();
    let out = tempfile::tempdir().unwrap();
    let found = Command::new("uv")
        .args(["run", "--frozen", "python", "-c", "import sys; print(sys.executable)"])
        .current_dir(&root)
        .output()
        .expect("uv is on PATH");
    assert!(
        found.status.success(),
        "uv run failed:\n{}",
        String::from_utf8_lossy(&found.stderr)
    );
    let python = String::from_utf8(found.stdout).unwrap();
    let run = Command::new(python.trim())
        .arg(GENERATOR)
        .arg("--out")
        .arg(out.path())
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{GENERATOR} failed:\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    for golden in GOLDENS {
        let committed = std::fs::read_to_string(goldens().join(golden)).unwrap();
        let fresh = std::fs::read_to_string(out.path().join(golden)).unwrap();
        if committed != fresh {
            let diff = similar::TextDiff::from_lines(&committed, &fresh)
                .unified_diff()
                .header(golden, "regenerated")
                .to_string();
            let diff: Vec<&str> = diff.lines().take(40).collect();
            panic!(
                "{golden} is stale; the Python app now writes:\n{}\n\n\
                 Regenerate it from the repository root with:\n    uv run python {GENERATOR}",
                diff.join("\n")
            );
        }
    }
}
