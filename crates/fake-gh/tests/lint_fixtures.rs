//! `fake-gh lint parity/fixtures/*` as part of `cargo test`.

mod common;

use std::path::PathBuf;

#[test]
fn committed_fixture_sets_lint_clean() {
    let fixtures = common::repo_root().join("parity").join("fixtures");
    let mut sets: Vec<PathBuf> = std::fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    sets.sort();
    assert!(!sets.is_empty(), "no fixture sets under {}", fixtures.display());

    let report = fake_gh::lint::lint_sets(&sets);
    let problems: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "fixture problems:\n{}", problems.join("\n"));
    assert!(report.fixtures >= 3);
}

#[test]
fn lint_cli_reports_problems() {
    let tmp = common::TempDir::new("lint-cli");
    let set = tmp.path().join("bad");
    common::write(
        &set,
        "misplaced.json",
        r#"{"version":1,"request":{"method":"GET","path":"/user"},"response":{"status":200}}"#,
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fake-gh"))
        .arg("lint")
        .arg(&set)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.contains("bad/misplaced.json: non-canonical file name; expected user/GET.json"),
        "{err}"
    );

    let good = common::repo_root().join("parity/fixtures/base");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fake-gh"))
        .arg("lint")
        .arg(good)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}
