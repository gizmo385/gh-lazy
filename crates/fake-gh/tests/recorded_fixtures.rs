//! fake-gh replays the repo-scoped fixtures that `cargo xtask record-fixtures --set base` records (#68).

mod common;

use common::{TempDir, gh, install_gh, repo_root};

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
