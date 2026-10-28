//! The Rust app's `GhCli` against fake-gh, the way the parity harness runs it.
//!
//! fake-gh reads its configuration from the environment, and `GhCli`'s `gh` inherits the test's environment, which a
//! test can't change safely while other threads run. So each parent test re-runs this test binary with fake-gh's
//! variables set, selecting one `#[ignore]`d child test, and then checks fake-gh's request log.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use lazy_github_core::config::{IssueOwnerFilter, IssueStateFilter};
use lazy_github_core::gh::{GhError, GhTimeouts};
use lazy_github_core::github::{GhCli, GhCliOptions, Github, IssueListQuery, NotificationQuery, RepoListQuery};
use lazy_github_core::models::{IssueListItem, Repository};
use serde_json::Value;

/// Where a child test finds the installed fake-gh.
const GH_BIN_ENV: &str = "LAZY_GITHUB_TEST_GH";

/// The unread count a notifications child test expects.
const EXPECTED_UNREAD_ENV: &str = "LAZY_GITHUB_TEST_EXPECTED_UNREAD";

/// Runs the child test `name` with fake-gh's environment, asserts it passed and returns fake-gh's request log.
fn run_child(name: &str, auth: &str, fixture_sets: &[&str]) -> Vec<Value> {
    run_child_with(name, auth, fixture_sets, &[])
}

/// [`run_child`], with extra environment variables for the child.
fn run_child_with(name: &str, auth: &str, fixture_sets: &[&str], envs: &[(&str, &str)]) -> Vec<Value> {
    let tmp = common::TempDir::new(name);
    let bin = common::install_gh(tmp.path());
    let log = tmp.path().join("fake-gh.log");
    let fixtures = common::repo_root().join("parity").join("fixtures");
    let sets: Vec<PathBuf> = fixture_sets.iter().map(|set| fixtures.join(set)).collect();

    let out = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env(GH_BIN_ENV, bin.join(format!("gh{}", std::env::consts::EXE_SUFFIX)))
        .env("FAKE_GH_FIXTURES", std::env::join_paths(&sets).unwrap())
        .env("FAKE_GH_LOG", &log)
        .env("FAKE_GH_AUTH", auth)
        .env_remove("FAKE_GH_USER")
        .env_remove("GH_HOST")
        .envs(envs.iter().copied())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "child {name} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Guards against the filter matching nothing, which also exits 0.
    assert!(
        stdout.contains("1 passed"),
        "child {name} didn't run\nstdout:\n{stdout}"
    );
    common::read_log(&log)
}

/// A client for the fake-gh a parent test installed, or `None` when a child runs on its own (`cargo test --
/// --ignored`): it then has no fake-gh to talk to and passes without checking anything. The parent's "1 passed" check
/// can't tell, but its log assertions can.
fn client() -> Option<GhCli> {
    let program = std::env::var_os(GH_BIN_ENV)?;
    Some(GhCli::new(GhCliOptions {
        program: Path::new(&program).to_path_buf(),
        ..GhCliOptions::default()
    }))
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect()
}

#[test]
fn logged_in_auth_user_and_two_page_repositories() {
    let log = run_child("child_logged_in", "logged-in", &["repos-two-pages", "base"]);

    assert!(
        log.iter().all(|line| line["miss"] == false && line["error"].is_null()),
        "{log:#?}"
    );
    let commands: Vec<&str> = log.iter().map(|line| line["command"].as_str().unwrap()).collect();
    // `current_user` ran gh once for three calls (two direct, one through a clone).
    assert_eq!(commands, ["auth status", "api", "api"]);

    assert_eq!(strings(&log[0]["argv"]), ["auth", "status"]);

    assert_eq!(
        strings(&log[1]["argv"]),
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
    assert_eq!(log[1]["fixture"], "base/user/GET.json");
    assert!(log[1]["cache"].is_null());

    let repos = &log[2];
    assert_eq!(
        strings(&repos["argv"]),
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
    // The overlay's page 1 (which links to page 2) won over base's single page; the child saw both pages' repos.
    assert_eq!(
        repos["fixture"],
        "repos-two-pages/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json"
    );
    assert_eq!(repos["status"], 200);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_logged_in() {
    let Some(gh) = client() else { return };
    gh.check_auth().await.unwrap();

    let user = gh.current_user().await.unwrap();
    assert_eq!(user.login, "parity-bot");
    assert_eq!(user.id, 1_000_001);
    assert_eq!(user.name.as_deref(), Some("Parity Bot"));
    assert_eq!(gh.current_user().await.unwrap().login, "parity-bot");
    assert_eq!(gh.clone().current_user().await.unwrap(), user);

    let repos = gh.list_repositories(RepoListQuery::all()).await.unwrap();
    let names: Vec<&str> = repos.iter().map(|repo| repo.full_name.as_str()).collect();
    assert_eq!(
        names,
        ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
    );
    assert!(repos.iter().all(|repo| repo.owner == user));
}

#[test]
fn logged_out_is_not_authenticated() {
    let log = run_child("child_logged_out", "logged-out", &["base"]);
    let commands: Vec<&str> = log.iter().map(|line| line["command"].as_str().unwrap()).collect();
    assert_eq!(commands, ["auth status"]);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_logged_out() {
    let Some(gh) = client() else { return };
    let error = gh.check_auth().await.unwrap_err();
    assert!(
        matches!(&error, GhError::NotAuthenticated { hostname: None, detail } if detail.contains("gh auth login")),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "To proceed, please log into the Github CLI via `gh auth login`"
    );
}

/// `unread_notification_count` against each notifications fixture set, listed before `base` as the scenarios list
/// them.
#[test]
fn unread_notification_counts() {
    for (set, expected) in [
        ("notifications-0", "0"),
        ("notifications-3", "3"),
        ("notifications-50", "50"),
    ] {
        let log = run_child_with(
            "child_unread_count",
            "logged-in",
            &[set, "base"],
            &[(EXPECTED_UNREAD_ENV, expected)],
        );
        assert_eq!(log.len(), 1, "{log:#?}");
        let line = &log[0];
        assert_eq!(
            strings(&line["argv"]),
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
        assert!(line["cache"].is_null());
        assert_eq!(line["miss"], false);
        assert_eq!(
            line["fixture"],
            format!("{set}/notifications/GET.all-false_page-1_per_page-50.json")
        );
    }
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_unread_count() {
    let Some(gh) = client() else { return };
    let expected: usize = std::env::var(EXPECTED_UNREAD_ENV).unwrap().parse().unwrap();
    assert_eq!(gh.unread_notification_count().await.unwrap(), expected);
}

#[test]
fn a_missing_notifications_fixture_is_an_error() {
    // `repos-60` alone has no /notifications fixture, so the request misses.
    let log = run_child("child_unread_count_miss", "logged-in", &["repos-60"]);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0]["miss"], true);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_unread_count_miss() {
    let Some(gh) = client() else { return };
    let error = gh.unread_notification_count().await.unwrap_err();
    assert!(matches!(error, GhError::Http { status: 404, .. }), "{error:?}");
}

/// How long fake-gh sleeps in the timeout tests: far past [`SHORT_TIMEOUTS`], and long enough that a client that
/// waited it out would fail the wall-time check.
const SLOW_DELAY_MS: u64 = 30_000;

/// The limits a timeout child test's client runs with, each different so a test can tell which one applied.
const SHORT_TIMEOUTS: GhTimeouts = GhTimeouts {
    auth_status: Duration::from_millis(200),
    request: Duration::from_millis(300),
    paginate: Duration::from_millis(400),
};

/// Copies `files` (paths relative to the fixture set `from`) into `to`, adding [`SLOW_DELAY_MS`] to the response of
/// those marked slow.
fn copy_fixtures(from: &str, to: &Path, files: &[(&str, bool)]) {
    let source = common::repo_root().join("parity").join("fixtures").join(from);
    for &(file, slow) in files {
        let text = std::fs::read_to_string(source.join(file)).unwrap();
        let mut fixture: Value = serde_json::from_str(&text).unwrap();
        if slow {
            fixture["response"]["delay_ms"] = SLOW_DELAY_MS.into();
        }
        common::write(to, file, &serde_json::to_string_pretty(&fixture).unwrap());
    }
}

/// A child's client with [`SHORT_TIMEOUTS`], or `None` as for [`client`].
fn impatient_client() -> Option<GhCli> {
    let program = std::env::var_os(GH_BIN_ENV)?;
    Some(GhCli::new(GhCliOptions {
        program: Path::new(&program).to_path_buf(),
        timeouts: SHORT_TIMEOUTS,
        ..GhCliOptions::default()
    }))
}

/// Asserts that `result` is a timeout after `limit` and that it came well before fake-gh's delay ran out.
fn assert_timed_out<T: std::fmt::Debug>(result: &Result<T, GhError>, limit: Duration, started: Instant) {
    let elapsed = started.elapsed();
    assert!(
        matches!(result, Err(GhError::Timeout { after }) if *after == limit),
        "{result:?}"
    );
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
}

#[test]
fn a_slow_notifications_count_times_out() {
    let slow = common::TempDir::new("slow-notifications");
    copy_fixtures(
        "notifications-3",
        slow.path(),
        &[("notifications/GET.all-false_page-1_per_page-50.json", true)],
    );
    let set = slow.path().to_str().unwrap();
    let log = run_child("child_unread_count_timeout", "logged-in", &[set]);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0]["path"], "/notifications");
    assert_eq!(log[0]["miss"], false);
    assert_eq!(log[0]["status"], 200);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_unread_count_timeout() {
    let Some(gh) = impatient_client() else { return };
    let started = Instant::now();
    assert_timed_out(&gh.unread_notification_count().await, SHORT_TIMEOUTS.request, started);
}

#[test]
fn a_slow_second_page_times_out_the_whole_list() {
    let slow = common::TempDir::new("slow-repos");
    copy_fixtures(
        "repos-two-pages",
        slow.path(),
        &[
            (
                "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
                false,
            ),
            (
                "user/repos/GET.direction-asc_page-2_per_page-50_sort-full_name_type-all.json",
                true,
            ),
        ],
    );
    let set = slow.path().to_str().unwrap();
    let log = run_child("child_list_repositories_timeout", "logged-in", &[set]);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0]["path"], "/user/repos");
    assert!(strings(&log[0]["argv"]).contains(&"--paginate"), "{log:#?}");
    assert_eq!(log[0]["miss"], false);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_list_repositories_timeout() {
    let Some(gh) = impatient_client() else { return };
    let started = Instant::now();
    assert_timed_out(
        &gh.list_repositories(RepoListQuery::all()).await,
        SHORT_TIMEOUTS.paginate,
        started,
    );
}

/// The issues-endpoint fixtures for `parity-bot/lazy-fixture`, relative to a fixture set.
const LAZY_FIXTURE_ISSUES: &str = "repos/parity-bot/lazy-fixture/issues";

/// `parity-bot/lazy-fixture`, as `prs-70` records it.
fn lazy_fixture() -> Repository {
    let path = common::repo_root().join("parity/fixtures/prs-70/repos/parity-bot/lazy-fixture/GET.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    serde_json::from_value(fixture["response"]["body"].clone()).unwrap()
}

fn issue_numbers(items: &[IssueListItem]) -> Vec<u64> {
    items.iter().map(|item| item.issue().number).collect()
}

#[test]
fn list_issues_fetches_one_page() {
    let log = run_child("child_list_issues_page_2", "logged-in", &["prs-70"]);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(
        strings(&log[0]["argv"]),
        [
            "api",
            "-i",
            "-X",
            "GET",
            "--cache",
            "600s",
            "-H",
            "Accept: application/vnd.github+json",
            "/repos/parity-bot/lazy-fixture/issues?state=all&page=2&per_page=30"
        ]
    );
    assert_eq!(
        log[0]["fixture"],
        format!("prs-70/{LAZY_FIXTURE_ISSUES}/GET.page-2_per_page-30_state-all.json")
    );
    assert_eq!(log[0]["status"], 200);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_list_issues_page_2() {
    let Some(gh) = client() else { return };
    let repo = lazy_fixture();
    let query = IssueListQuery {
        page: 2,
        ..IssueListQuery::first_page(IssueStateFilter::All, IssueOwnerFilter::All)
    };
    let items = gh.list_issues(&repo, query).await.unwrap();
    assert_eq!(issue_numbers(&items), (11..=40).rev().collect::<Vec<_>>());
    assert!(items.iter().all(IssueListItem::is_pull_request));
    assert!(items.iter().all(|item| item.issue().repo == repo));
}

#[test]
fn mine_fetches_the_login_once() {
    // `prs-70`'s first two pages, as answers to `state=open&creator=parity-bot`.
    let mine = common::TempDir::new("issues-mine");
    for page in [1, 2] {
        let source = common::repo_root().join(format!(
            "parity/fixtures/prs-70/{LAZY_FIXTURE_ISSUES}/GET.page-{page}_per_page-30_state-all.json"
        ));
        let mut fixture: Value = serde_json::from_str(&std::fs::read_to_string(source).unwrap()).unwrap();
        fixture["request"]["query"]["state"] = "open".into();
        fixture["request"]["query"]["creator"] = "parity-bot".into();
        common::write(
            mine.path(),
            &format!("{LAZY_FIXTURE_ISSUES}/GET.creator-parity-bot_page-{page}_per_page-30_state-open.json"),
            &serde_json::to_string_pretty(&fixture).unwrap(),
        );
    }
    let set = mine.path().to_str().unwrap();

    let log = run_child("child_list_issues_mine", "logged-in", &[set, "base"]);
    assert!(
        log.iter().all(|line| line["miss"] == false && line["error"].is_null()),
        "{log:#?}"
    );
    let paths: Vec<&str> = log.iter().map(|line| line["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        [
            "/user",
            "/repos/parity-bot/lazy-fixture/issues",
            "/repos/parity-bot/lazy-fixture/issues"
        ]
    );
    for (line, page) in log[1..].iter().zip([1, 2]) {
        let expected =
            format!("/repos/parity-bot/lazy-fixture/issues?state=open&page={page}&per_page=30&creator=parity-bot");
        assert_eq!(strings(&line["argv"]).last().copied(), Some(expected.as_str()));
    }
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_list_issues_mine() {
    let Some(gh) = client() else { return };
    let repo = lazy_fixture();
    let first = IssueListQuery::first_page(IssueStateFilter::Open, IssueOwnerFilter::Mine);
    let items = gh.list_issues(&repo, first).await.unwrap();
    assert_eq!(issue_numbers(&items), (41..=70).rev().collect::<Vec<_>>());
    let items = gh
        .list_issues(&repo, IssueListQuery { page: 2, ..first })
        .await
        .unwrap();
    assert_eq!(issue_numbers(&items), (11..=40).rev().collect::<Vec<_>>());
}

#[test]
fn a_failing_current_user_fails_mine_without_listing() {
    // `prs-70` alone has no `/user` fixture, so the login lookup misses.
    let log = run_child("child_list_issues_mine_no_user", "logged-in", &["prs-70"]);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0]["path"], "/user");
    assert_eq!(log[0]["miss"], true);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_list_issues_mine_no_user() {
    let Some(gh) = client() else { return };
    let query = IssueListQuery::first_page(IssueStateFilter::All, IssueOwnerFilter::Mine);
    let error = gh.list_issues(&lazy_fixture(), query).await.unwrap_err();
    assert!(matches!(error, GhError::Http { status: 404, .. }), "{error:?}");
}

#[test]
fn repo_readmes_are_raw_and_verbatim() {
    let log = run_child("child_repo_readmes", "logged-in", &["repo-overview"]);
    assert_eq!(log.len(), 3, "{log:#?}");
    assert!(log.iter().all(|line| line["miss"] == false), "{log:#?}");
    assert_eq!(
        strings(&log[0]["argv"]),
        [
            "api",
            "-i",
            "-X",
            "GET",
            "--cache",
            "600s",
            "-H",
            "Accept: application/vnd.github.raw",
            "/repos/parity-bot/hello-world/readme"
        ]
    );
    let statuses: Vec<&Value> = log.iter().map(|line| &line["status"]).collect();
    assert_eq!(statuses, [200, 404, 200]);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_repo_readmes() {
    let Some(gh) = client() else { return };
    let hello = gh.repo_readme("parity-bot/hello-world").await.unwrap().unwrap();
    assert!(hello.starts_with("# Hello World\n\nA first repository"), "{hello}");
    assert!(hello.contains("| `-q` | Quiet. |\n"), "{hello}");
    assert_eq!(gh.repo_readme("parity-bot/lazy-demo").await.unwrap(), None);
    // Blank lines and the header-shaped line survive (Py bug B47).
    assert_eq!(
        gh.repo_readme("parity-bot/dotfiles").await.unwrap().as_deref(),
        Some(
            "# dotfiles\n\nThe first paragraph.\n\nA second paragraph, after a blank line.\n\nNote: a line shaped like \
             an HTTP header.\n\n```sh\necho one\n\necho two\n```\n"
        )
    );
}

/// `parity-bot/pr-details`, as `pr-details` lists it.
fn pr_details_repo() -> Repository {
    let path = common::repo_root().join(
        "parity/fixtures/pr-details/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let repos: Vec<Repository> = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
    repos
        .into_iter()
        .find(|repo| repo.full_name == "parity-bot/pr-details")
        .unwrap()
}

#[test]
fn pull_request_diffs_through_gh_api() {
    let log = run_child("child_pull_request_diffs", "logged-in", &["pr-details"]);
    assert_eq!(log.len(), 4, "{log:#?}");
    assert!(log.iter().all(|line| line["miss"] == false), "{log:#?}");
    assert_eq!(
        strings(&log[0]["argv"]),
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
    let fixtures: Vec<&str> = log.iter().map(|line| line["fixture"].as_str().unwrap()).collect();
    let pulls = "pr-details/repos/parity-bot/pr-details/pulls";
    assert_eq!(
        fixtures,
        [5, 3, 9, 4].map(|number| format!("{pulls}/{number}/GET.diff.json"))
    );
    let statuses: Vec<&Value> = log.iter().map(|line| &line["status"]).collect();
    assert_eq!(statuses, [200, 200, 200, 406]);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_pull_request_diffs() {
    use lazy_github_core::diff;
    use lazy_github_core::github::DiffFetchError;

    let Some(gh) = client() else { return };
    let repo = pr_details_repo();
    let paths = |text: &str| -> Vec<String> {
        diff::parse(text)
            .unwrap()
            .files
            .into_iter()
            .map(|file| file.path)
            .collect()
    };
    let pr_5 = gh.pull_request_diff(&repo, 5).await.unwrap();
    assert!(pr_5.starts_with("diff --git a/README.md b/README.md\nindex 7777777..8888888 100644\n"));
    assert!(pr_5.ends_with("         Ok(())\n     }\n"), "{pr_5}");
    assert_eq!(
        paths(&pr_5),
        [
            "README.md",
            "assets/app.css",
            "build.rs",
            "docs/old.md",
            "src/viewer.rs"
        ]
    );
    assert_eq!(
        paths(&gh.pull_request_diff(&repo, 3).await.unwrap()),
        ["Cargo.toml", "CHANGELOG.md"]
    );
    assert_eq!(paths(&gh.pull_request_diff(&repo, 9).await.unwrap()), ["src/viewer.rs"]);
    let too_large = gh.pull_request_diff(&repo, 4).await.unwrap_err();
    assert!(matches!(too_large, DiffFetchError::TooLarge), "{too_large:?}");
    assert_eq!(too_large.message(), "Diff too large");
}

#[test]
fn pull_requests_are_cached_for_the_default_ttl() {
    let log = run_child("child_pull_requests", "logged-in", &["pr-details", "base"]);
    assert_eq!(log.len(), 5, "{log:#?}");
    assert!(log.iter().all(|line| line["miss"] == false), "{log:#?}");
    assert_eq!(
        strings(&log[0]["argv"]),
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
    let statuses: Vec<&Value> = log.iter().map(|line| &line["status"]).collect();
    assert_eq!(statuses, [200, 200, 200, 200, 500]);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_pull_requests() {
    let Some(gh) = client() else { return };
    let repo = pr_details_repo();
    let open = gh.pull_request(&repo, 5).await.unwrap();
    assert_eq!(open.pr.issue.title, "Add a cursor to the viewer");
    assert_eq!(open.pr.issue.repo, repo);
    assert_eq!((open.commits, open.changed_files), (3, 5));
    assert_eq!(open.merged_at, None);
    let closed = gh.pull_request(&repo, 4).await.unwrap();
    assert!(closed.pr.issue.closed_at.is_some() && closed.merged_at.is_none());
    let merged = gh.pull_request(&repo, 3).await.unwrap();
    assert!(merged.merged_at.is_some());
    assert_eq!(gh.pull_request(&repo, 9).await.unwrap().head.name, "clamp");
    let error = gh.pull_request(&repo, 2).await.unwrap_err();
    assert!(matches!(error, GhError::Http { status: 500, .. }), "{error:?}");
}

/// `parity-bot/run-fixture`, as `run-details` lists it.
fn run_fixture_repo() -> Repository {
    let path = common::repo_root().join(
        "parity/fixtures/run-details/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let repos: Vec<Repository> = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
    repos
        .into_iter()
        .find(|repo| repo.full_name == "parity-bot/run-fixture")
        .unwrap()
}

#[test]
fn workflow_run_jobs_are_slurped() {
    let log = run_child("child_workflow_run_jobs", "logged-in", &["run-details"]);
    assert_eq!(log.len(), 3, "{log:#?}");
    assert!(log.iter().all(|line| line["miss"] == false), "{log:#?}");
    assert_eq!(
        strings(&log[0]["argv"]),
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

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_workflow_run_jobs() {
    let Some(gh) = client() else { return };
    let repo = run_fixture_repo();
    let names = |jobs: Vec<lazy_github_core::models::WorkflowJob>| -> Vec<String> {
        jobs.into_iter().map(|job| job.name).collect()
    };
    assert_eq!(
        names(gh.workflow_run_jobs(&repo, 5_000_042).await.unwrap()),
        ["build", "test", "lint", "deploy"]
    );
    assert_eq!(
        names(gh.workflow_run_jobs(&repo, 5_000_041).await.unwrap()),
        ["build", "test"]
    );
    assert!(gh.workflow_run_jobs(&repo, 5_000_040).await.unwrap().is_empty());
}

#[test]
fn notifications_list_and_mark_read_against_the_modal_fixtures() {
    let log = run_child("child_notifications", "logged-in", &["notifications-modal", "base"]);
    assert_eq!(log.len(), 4, "{log:#?}");
    assert!(
        log.iter().all(|line| line["miss"] == false && line["cache"].is_null()),
        "{log:#?}"
    );
    let fixtures: Vec<&str> = log.iter().map(|line| line["fixture"].as_str().unwrap()).collect();
    assert_eq!(
        fixtures,
        [
            "notifications-modal/notifications/GET.all-true_page-1_per_page-50.json",
            "notifications-modal/notifications/GET.all-false_page-2_per_page-50.json",
            "notifications-modal/notifications/threads/9100001/PATCH.json",
            "notifications-modal/notifications/PUT.json",
        ]
    );
    let statuses: Vec<&Value> = log.iter().map(|line| &line["status"]).collect();
    assert_eq!(statuses, [200, 200, 205, 205]);
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_notifications() {
    let Some(gh) = client() else { return };
    let all = gh
        .list_notifications(NotificationQuery { all: true, page: 1 })
        .await
        .unwrap();
    let ids: Vec<u64> = all.iter().map(|notification| notification.id).collect();
    assert_eq!(ids, [9_100_001, 9_100_002, 9_100_003, 9_100_004, 9_100_005, 9_100_006]);
    let unread_page_2 = gh
        .list_notifications(NotificationQuery { all: false, page: 2 })
        .await
        .unwrap();
    assert!(unread_page_2.is_empty());
    gh.mark_notification_read(9_100_001).await.unwrap();
    gh.mark_all_notifications_read().await.unwrap();
}

/// A fixture answering `method path` (with `query`, if any) with a 500.
fn server_error_fixture(method: &str, path: &str, query: Option<Value>) -> String {
    let mut request = serde_json::json!({ "method": method, "path": path });
    if let Some(query) = query {
        request["query"] = query;
    }
    let fixture = serde_json::json!({
        "version": 1,
        "request": request,
        "response": {
            "status": 500,
            "headers": { "Content-Type": "application/json; charset=utf-8" },
            "body": { "message": "Server Error" },
        },
    });
    serde_json::to_string_pretty(&fixture).unwrap()
}

#[test]
fn notifications_server_errors_are_errors() {
    let failing = common::TempDir::new("failing-notifications");
    common::write(
        failing.path(),
        "notifications/GET.all-true_page-1_per_page-50.json",
        &server_error_fixture(
            "GET",
            "/notifications",
            Some(serde_json::json!({ "all": "true", "page": "1", "per_page": "50" })),
        ),
    );
    common::write(
        failing.path(),
        "notifications/threads/9100001/PATCH.json",
        &server_error_fixture("PATCH", "/notifications/threads/9100001", None),
    );
    common::write(
        failing.path(),
        "notifications/PUT.json",
        &server_error_fixture("PUT", "/notifications", None),
    );
    let set = failing.path().to_str().unwrap();
    let log = run_child("child_notifications_server_errors", "logged-in", &[set]);
    assert_eq!(log.len(), 3, "{log:#?}");
    assert!(
        log.iter().all(|line| line["miss"] == false && line["status"] == 500),
        "{log:#?}"
    );
}

#[tokio::test]
#[ignore = "run by its parent test, with fake-gh's environment set"]
async fn child_notifications_server_errors() {
    let Some(gh) = client() else { return };
    let is_500 = |error: GhError| matches!(error, GhError::Http { status: 500, .. });
    let listed = gh.list_notifications(NotificationQuery { all: true, page: 1 }).await;
    assert!(listed.is_err_and(is_500));
    assert!(gh.mark_notification_read(9_100_001).await.is_err_and(is_500));
    assert!(gh.mark_all_notifications_read().await.is_err_and(is_500));
}
