//! End-to-end runs of `cargo xtask parity` against the Python app. They need uv and the repo's Python dependencies,
//! so they're ignored by default; run them with `cargo test -p xtask --test python_app -- --ignored` after a
//! `cargo build -p fake-gh` (CI wires them up in #70).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn parity(args: &[&std::ffi::OsStr]) -> std::process::Output {
    // --no-build: a nested `cargo build` would wait on the lock this test run holds.
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("parity")
        .args(args)
        .arg("--no-build")
        .output()
        .unwrap()
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

#[test]
#[ignore = "needs uv and the Python app"]
fn startup_passes_on_the_python_app() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let output = parity(&[
        "startup".as_ref(),
        "--python-only".as_ref(),
        "--out".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for size in ["160x48", "100x30"] {
        let dir = out.join("startup").join(size).join("python");
        let main = std::fs::read_to_string(dir.join("main.txt")).unwrap();
        assert!(main.contains("Repositories"), "{main}");
        for repo in ["dotfiles", "hello-world", "lazy-demo"] {
            assert!(main.contains(repo), "{size} is missing {repo}:\n{main}");
        }
        assert!(dir.join("main.grid.json").is_file());
        let log = std::fs::read_to_string(dir.join("fake-gh.log")).unwrap();
        assert!(log.contains(r#""command":"auth status""#), "{log}");
        assert!(log.contains(r#""path":"/user/repos""#), "{log}");
        assert_eq!(read_json(&dir.join("status.json"))["misses"], serde_json::json!([]));
    }
    assert_eq!(
        read_json(&out.join("summary.json"))["scenarios"][0]["result"],
        "python-only"
    );
}

#[test]
#[ignore = "needs uv and the Python app"]
fn a_fixture_miss_fails_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    // A copy of `base` without the /user/repos fixture.
    let fixtures = tmp.path().join("fixtures");
    copy_dir(&repo().join("parity/fixtures/base"), &fixtures.join("base"));
    std::fs::remove_dir_all(fixtures.join("base/user/repos")).unwrap();
    let scenario = tmp.path().join("startup-miss.toml");
    let source = std::fs::read_to_string(repo().join("parity/scenarios/startup.toml")).unwrap();
    std::fs::write(
        &scenario,
        source.replace("sizes = [[160, 48], [100, 30]]", "sizes = [[160, 48]]"),
    )
    .unwrap();
    let out = tmp.path().join("out");

    let output = parity(&[
        scenario.as_os_str(),
        "--python-only".as_ref(),
        "--out".as_ref(),
        out.as_os_str(),
        "--fixtures-dir".as_ref(),
        fixtures.as_os_str(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let status = read_json(&out.join("startup-miss/160x48/python/status.json"));
    assert_eq!(status["status"], "ok");
    assert_eq!(status["fixture_problems"], true);
    let misses = status["misses"].as_array().unwrap();
    assert!(
        misses
            .iter()
            .any(|m| m["method"] == "GET" && m["path"] == "/user/repos" && m["query"]["page"] == "1"),
        "{misses:?}"
    );
    assert_eq!(read_json(&out.join("summary.json"))["scenarios"][0]["result"], "fail");
}
