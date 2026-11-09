//! The Rust snapshots in `parity/snapshots/` are complete and consistent, as `insta` snapshots must match their tests:
//! every scenario's capture has a `.screen` and a `.svg` at each of its sizes, plus an `effects.txt`, nothing is left
//! over, every `.screen` parses and every `.svg` is exactly the rendering of its `.screen`. A scenario edited without
//! `cargo xtask parity --accept` fails here. Whether a run still draws the same screens is the parity run's job.
//! Reads files only; nothing is built or run.

use std::path::{Path, PathBuf};

use xtask::parity::scenario::{Roots, Scenario};
use xtask::parity::snapshot::{self, Problem};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned()
}

fn roots(repo: &Path) -> Roots {
    Roots {
        fixtures: repo.join("parity/fixtures"),
        configs: repo.join("parity/configs"),
        seeds: repo.join("parity/seeds"),
    }
}

fn load(repo: &Path, name: &str) -> Scenario {
    Scenario::load(&repo.join(format!("parity/scenarios/{name}.toml")), &roots(repo)).unwrap_or_else(|e| panic!("{e}"))
}

fn lines(problems: &[Problem]) -> Vec<String> {
    problems.iter().map(ToString::to_string).collect()
}

#[test]
fn committed_snapshots_are_complete_and_consistent() {
    let repo = repo();
    let mut files: Vec<_> = std::fs::read_dir(repo.join("parity/scenarios"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    let scenarios: Vec<Scenario> = files
        .iter()
        .map(|f| Scenario::load(f, &roots(&repo)).unwrap_or_else(|e| panic!("{e}")))
        .collect();

    let problems = lines(&snapshot::check(&repo, &scenarios, true).unwrap_or_else(|e| panic!("{e}")));
    assert!(
        problems.is_empty(),
        "{} problem(s) with the Rust snapshots:\n{}",
        problems.len(),
        problems.join("\n")
    );
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

/// On a copy of `startup`'s snapshots: a deleted `.screen`, an orphan directory and an edited `.svg` are each found.
#[test]
fn the_check_finds_a_missing_screen_an_orphan_and_an_edited_svg() {
    let repo = repo();
    let startup = load(&repo, "startup");
    let tmp = tempfile::tempdir().unwrap();
    let snapshots = tmp.path().join("parity/snapshots");
    copy_dir(&repo.join("parity/snapshots/startup"), &snapshots.join("startup"));
    assert_eq!(
        snapshot::check(tmp.path(), std::slice::from_ref(&startup), true).unwrap(),
        []
    );

    std::fs::remove_file(snapshots.join("startup/160x48/main.screen")).unwrap();
    std::fs::create_dir_all(snapshots.join("gone/160x48")).unwrap();
    let svg = snapshots.join("startup/100x30/main.svg");
    let text = std::fs::read_to_string(&svg).unwrap();
    std::fs::write(&svg, text.replacen("<title>", "<title>edited ", 1)).unwrap();

    assert_eq!(
        lines(&snapshot::check(tmp.path(), std::slice::from_ref(&startup), true).unwrap()),
        [
            "orphaned snapshot: parity/snapshots/gone/ (no such scenario); cargo xtask parity --accept deletes it",
            "missing snapshot: startup/160x48/main.screen; run cargo xtask parity --accept startup",
            "stale snapshot: startup/100x30/main.svg isn't the rendering of its .screen; run cargo xtask parity \
             --accept startup",
        ]
    );
    // Checking one scenario can't tell whether another's directory is an orphan.
    assert_eq!(snapshot::check(tmp.path(), &[startup], false).unwrap().len(), 2);
}
