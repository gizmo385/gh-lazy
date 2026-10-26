//! The recorded Python references in `parity/references/` match the current inputs, as `insta` snapshots must match
//! their tests: a scenario, fixture, Python-app or harness edit made without `cargo xtask parity --record` fails
//! here, and so does a new scenario without a reference. Needs only `git`; nothing is built or run.

use std::path::Path;

use xtask::parity::reference;
use xtask::parity::scenario::{Roots, Scenario};

#[test]
fn recorded_references_are_fresh() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let roots = Roots {
        fixtures: repo.join("parity/fixtures"),
        configs: repo.join("parity/configs"),
    };
    let mut files: Vec<_> = std::fs::read_dir(repo.join("parity/scenarios"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    let scenarios: Vec<Scenario> = files
        .iter()
        .map(|f| Scenario::load(f, &roots).unwrap_or_else(|e| panic!("{e}")))
        .collect();

    let report = reference::check(repo, &scenarios, &roots, true).unwrap_or_else(|e| panic!("{e}"));
    let problems: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with recorded references:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
