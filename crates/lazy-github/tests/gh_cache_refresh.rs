//! After one of the app's own writes, the next read of each cached request it made stale goes past gh's cache with
//! `--cache 1ns`, and later reads are cached again (#383, `infrastructure.gh-cache-after-write`).
//!
//! Drives the real `GhCli` against fake-gh, which logs each request's `--cache` value but never caches, and reads the
//! values back from its request log.
//!
//! fake-gh takes its fixtures and log from the environment, and the test may not set its own (`unsafe_code` is
//! forbidden), so [`writes_refresh_what_they_make_stale`] runs [`in_child`] again in a child test process that has
//! them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lazy_github_core::gh::GhTimeouts;
use lazy_github_core::github::{CacheTtls, GhCli, GhCliOptions, Github, RepoListQuery};
use lazy_github_core::models::{FullPullRequest, Repository};

/// Set in the child test process only.
const CHILD: &str = "LAZY_GITHUB_CACHE_REFRESH_CHILD";

/// `target/<profile>/examples/<name>`, built by `cargo test`.
fn example(name: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(Path::parent).unwrap();
    let path = profile
        .join("examples")
        .join(name)
        .with_extension(std::env::consts::EXE_EXTENSION);
    assert!(
        path.is_file(),
        "{} is missing; run `cargo build -p lazy-github --examples`",
        path.display()
    );
    path
}

fn fixtures() -> std::ffi::OsString {
    let parity = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures");
    std::env::join_paths(["pr-write", "pr-details", "base"].map(|set| parity.join(set))).unwrap()
}

#[test]
fn writes_refresh_what_they_make_stale() {
    let dir = std::env::temp_dir().join(format!("lazy-github-cache-refresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("requests.jsonl");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["in_child", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("FAKE_GH_FIXTURES", fixtures())
        .env("FAKE_GH_LOG", &log)
        .status()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "the child test failed: {status}");
}

fn options(default_ttl: u64) -> GhCliOptions {
    let ttl = Duration::from_secs(default_ttl);
    GhCliOptions {
        program: example("fake-gh"),
        hostname: None,
        ttls: CacheTtls {
            default: ttl,
            list_repos: ttl,
            list_issues: ttl,
        },
        timeouts: GhTimeouts::default(),
    }
}

/// The request log's lines so far, as `(method, path, cache)`.
fn log_lines() -> Vec<(String, String, Option<String>)> {
    let log = std::env::var_os("FAKE_GH_LOG").unwrap();
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let entry: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(entry["miss"], false, "fake-gh had no fixture for {line}");
            let text = |field: &str| entry[field].as_str().map(str::to_owned);
            (text("method").unwrap(), text("path").unwrap(), text("cache"))
        })
        .collect()
}

/// The `--cache` of each `GET` of `path` logged since line `from`, in order, `-` for none.
fn caches(from: usize, path: &str) -> Vec<String> {
    log_lines()
        .into_iter()
        .skip(from)
        .filter(|(method, logged, _)| method == "GET" && logged == path)
        .map(|(_, _, cache)| cache.unwrap_or_else(|| "-".to_owned()))
        .collect()
}

const COMMENTS: &str = "/repos/parity-bot/pr-details/issues/5/comments";
const REVIEWS: &str = "/repos/parity-bot/pr-details/pulls/5/reviews";
const PR_4: &str = "/repos/parity-bot/pr-details/pulls/4";
const README: &str = "/repos/parity-bot/pr-details/readme";

async fn setup(gh: &GhCli) -> (Repository, FullPullRequest) {
    let repos = gh.list_repositories(RepoListQuery::default()).await.unwrap();
    let repo = repos
        .into_iter()
        .find(|repo| repo.full_name == "parity-bot/pr-details")
        .unwrap();
    let pr = gh.pull_request(&repo, 5).await.unwrap();
    (repo, pr)
}

#[tokio::test]
#[ignore = "run by writes_refresh_what_they_make_stale, with fake-gh's environment"]
async fn in_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let gh = GhCli::new(options(600));
    let (repo, pr) = setup(&gh).await;
    let issue = &pr.pr.issue;

    // 1. Cached as usual.
    let start = log_lines().len();
    let before = gh.issue_comments(issue).await.unwrap();
    assert!(before.iter().all(|comment| comment.body != "Ship it."));
    assert_eq!(caches(start, COMMENTS), ["600s"]);

    // 2. The write.
    gh.create_issue_comment(&repo, 5, "Ship it.").await.unwrap();

    // 3. The next read refreshes, and shows the new comment.
    let start = log_lines().len();
    let after = gh.issue_comments(issue).await.unwrap();
    assert!(after.iter().any(|comment| comment.body == "Ship it."));
    assert_eq!(caches(start, COMMENTS), ["1ns"]);

    // 4. Once refreshed, cached again.
    let start = log_lines().len();
    gh.issue_comments(issue).await.unwrap();
    assert_eq!(caches(start, COMMENTS), ["600s"]);

    // 5. Another read under the same pull request refreshes too, once.
    let start = log_lines().len();
    gh.pull_request_reviews(&pr).await.unwrap();
    assert_eq!(caches(start, REVIEWS), ["1ns"]);
    let start = log_lines().len();
    gh.pull_request_reviews(&pr).await.unwrap();
    assert_eq!(caches(start, REVIEWS), ["600s"]);

    // 6. Reads outside the write's scope stay cached.
    let start = log_lines().len();
    gh.pull_request(&repo, 4).await.unwrap();
    gh.repo_readme(&repo.full_name).await.unwrap();
    assert_eq!(caches(start, PR_4), ["600s"]);
    assert_eq!(caches(start, README), ["600s"]);

    // 7. A reconfigure (Settings saved with new TTLs) between the write and the read keeps the mark.
    gh.create_issue_comment(&repo, 5, "Ship it.").await.unwrap();
    gh.reconfigure(options(300));
    let start = log_lines().len();
    gh.issue_comments(issue).await.unwrap();
    gh.issue_comments(issue).await.unwrap();
    assert_eq!(caches(start, COMMENTS), ["1ns", "300s"]);
}
