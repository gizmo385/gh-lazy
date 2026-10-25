//! The structural check end to end against the Python app: a deliberately changed pane title fails `[expect]`. It
//! needs uv and the repo's Python dependencies, so it's ignored by default; run it with
//! `cargo build -p fake-gh && cargo test -p xtask --test structural_check -- --ignored` (CI wires it up in #70).

use std::path::Path;
use std::process::Command;

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

#[test]
#[ignore = "needs uv and the Python app"]
fn a_rebound_pane_key_fails_the_expected_titles() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let tmp = tempfile::tempdir().unwrap();
    let scenario = tmp.path().join("startup.toml");
    let mut source = std::fs::read_to_string(repo.join("parity/scenarios/startup.toml")).unwrap();
    source.push_str("\n[env.config_overrides]\nbindings = { overrides = { \"main.repos.focus\" = \"9\" } }\n");
    std::fs::write(&scenario, source).unwrap();
    let out = tmp.path().join("out");

    // --no-build: a nested `cargo build` would wait on the lock this test run holds.
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("parity")
        .arg(&scenario)
        .args(["--python-only", "--size", "160x48", "--no-build", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("python expect titles"), "{stdout}");

    let check = read_json(&out.join("startup/160x48/main.check.json"));
    let titles: Vec<_> = check["expect"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["check"] == "titles")
        .collect();
    assert_eq!(titles.len(), 1, "{check:#}");
    assert!(
        titles[0]["python"]
            .as_array()
            .unwrap()
            .contains(&"[9] Repositories".into()),
        "{check:#}"
    );
    assert!(titles[0]["message"].as_str().unwrap().contains("[9] Repositories"));

    let summary = read_json(&out.join("summary.json"));
    let scenario = &summary["scenarios"][0];
    assert_eq!(scenario["result"], "fail");
    assert_eq!(scenario["sizes"][0]["captures"][0]["check"], "fail");
    assert_eq!(
        scenario["sizes"][0]["captures"][0]["against"],
        serde_json::json!(["python"])
    );
}
