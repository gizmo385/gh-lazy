//! fake-gh replays the repo-scoped fixtures that `cargo xtask record-fixtures --set base` records (#68).

mod common;

use common::{TempDir, gh, install_gh, read_log, repo_root};

#[test]
fn recorded_issues_page_replays() {
    let tmp = TempDir::new("recorded");
    let bin = install_gh(tmp.path());
    let base = repo_root().join("parity").join("fixtures").join("base");

    let assert = assert_cmd::Command::from_std(gh(&bin, &[&base], None))
        .args([
            "api",
            "-i",
            "-X",
            "GET",
            "-F",
            "state=all",
            "-F",
            "page=1",
            "-F",
            "per_page=30",
            "/repos/gizmo385/gh-lazy/issues",
        ])
        .assert()
        .success();
    let out = &assert.get_output().stdout;
    let text = String::from_utf8_lossy(out);
    assert!(text.starts_with("HTTP/2.0 200 OK\n"), "{text}");
    let body_start = text.find("\r\n\r\n").expect("headers end") + 4;
    let issues: serde_json::Value = serde_json::from_str(&text[body_start..]).unwrap();
    assert!(issues.as_array().is_some_and(|a| !a.is_empty()), "{issues}");
}

/// `gh api --paginate` with no `page` gets page 1 (#109) and then follows each `rel="next"` link, which needs the
/// links to be at the request's path rather than GitHub's `/repositories/<id>` alias (#151).
#[test]
fn paginate_follows_links_through_prs_70() {
    let tmp = TempDir::new("recorded-paginate");
    let bin = install_gh(tmp.path());
    let fixtures = repo_root().join("parity").join("fixtures");
    let log = tmp.path().join("fake-gh.log");

    let out = gh(&bin, &[&fixtures.join("prs-70"), &fixtures.join("base")], Some(&log))
        .args([
            "api",
            "--paginate",
            "/repos/parity-bot/lazy-fixture/issues?state=all&per_page=30",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let items: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let numbers: Vec<u64> = items
        .as_array()
        .expect("one merged array")
        .iter()
        .map(|item| item["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, (1..=70).rev().collect::<Vec<_>>());

    let log = read_log(&log);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(
        log[0]["fixture"],
        "prs-70/repos/parity-bot/lazy-fixture/issues/GET.page-1_per_page-30_state-all.json"
    );
    assert!(
        log.iter().all(|line| line.get("miss").is_none_or(|miss| miss == false)),
        "{log:?}"
    );
}
