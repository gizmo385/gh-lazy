//! The Rust app's `GhCli` against fake-gh, the way the parity harness runs it.
//!
//! fake-gh reads its configuration from the environment, and `GhCli`'s `gh` inherits the test's environment, which a
//! test can't change safely while other threads run. So each parent test re-runs this test binary with fake-gh's
//! variables set, selecting one `#[ignore]`d child test, and then checks fake-gh's request log.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use lazy_github_core::gh::GhError;
use lazy_github_core::github::{GhCli, GhCliOptions, Github, RepoListQuery};
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
