//! The parity matrix check: the committed matrix is valid, and each rule catches what it should.

use std::fs;
use std::path::Path;

use xtask::matrix::{Problem, check};
use xtask::parity::cli::repo_root;

#[test]
fn committed_matrix_is_valid() {
    let matrix = check(&repo_root());
    let problems: Vec<String> = matrix.problems.iter().map(Problem::to_string).collect();
    assert!(
        problems.is_empty(),
        "fix the parity matrix (rules in docs/rust-rewrite/parity/README.md):\n{}",
        problems.join("\n")
    );
    assert!(matrix.row_ids().count() > 300);
    assert!(matrix.summary(true).contains("| Total |"));
}

const README: &str =
    "# Parity Matrix\n\n## Index\n\n| Area file | Covers |\n| --- | --- |\n| [demo](./areas/demo.md) | Demo |\n";
const AREA: &str = "\
# Demo

| Id | Item | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| `demo.one` | One \\| pipe | M1 | not started | #1 | Py bug B1. |
| `demo.two` | Two | M2 | intentional diff (gh only) | #2, #3 | Decision: [gh](../decisions/gh.md). |
| `demo.three` | Three | M4 | parity | | |
";
const BUG: &str = "\
# B1: A bug

- Id: B1
- Status: open
- Python reference: `x.py`
- Recommendation: Fix it.
- Rows: `demo.one`

Prose.
";
const DECISION: &str = "\
# gh only

- Decision: Do it differently.
- Decided by: Chris
- Link: #2
- Rows: `demo.two`
";
const SCENARIO: &str = "version = 1\nparity_rows = [\"demo.one\"]\n";

/// A repo with a valid matrix, with `edit` applied to one of its files.
fn repo_with(edit: impl FnOnce(&Path)) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let m = root.join("docs/rust-rewrite/parity");
    for sub in ["areas", "py-bugs", "decisions"] {
        fs::create_dir_all(m.join(sub)).unwrap();
    }
    fs::create_dir_all(root.join("parity/scenarios")).unwrap();
    fs::write(m.join("README.md"), README).unwrap();
    fs::write(m.join("areas/demo.md"), AREA).unwrap();
    fs::write(m.join("py-bugs/B01.md"), BUG).unwrap();
    fs::write(m.join("decisions/gh.md"), DECISION).unwrap();
    fs::write(root.join("parity/scenarios/demo.toml"), SCENARIO).unwrap();
    edit(root);
    dir
}

fn problems(edit: impl FnOnce(&Path)) -> Vec<String> {
    let dir = repo_with(edit);
    check(dir.path()).problems.iter().map(Problem::to_string).collect()
}

fn assert_one(edit: impl FnOnce(&Path), want: &str) {
    let found = problems(edit);
    assert!(
        found.len() == 1 && found[0].contains(want),
        "expected one problem containing {want:?}, got {found:#?}"
    );
}

/// Replaces the first `from` in `file` with `to` and expects exactly one problem, containing `want`.
fn assert_edit(file: &str, from: &str, to: &str, want: &str) {
    assert_one(
        |root| {
            let path = root.join(file);
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains(from), "{from:?} not in {file}");
            fs::write(&path, text.replacen(from, to, 1)).unwrap();
        },
        want,
    );
}

const AREA_FILE: &str = "docs/rust-rewrite/parity/areas/demo.md";
const BUG_FILE: &str = "docs/rust-rewrite/parity/py-bugs/B01.md";
const DECISION_FILE: &str = "docs/rust-rewrite/parity/decisions/gh.md";

#[test]
fn valid_fixture_has_no_problems_and_counts_statuses() {
    let dir = repo_with(|_| {});
    let matrix = check(dir.path());
    assert!(matrix.problems.is_empty(), "{:#?}", matrix.problems);
    assert_eq!(matrix.counts(), vec![("demo", [1, 0, 1, 1])]);
    assert_eq!(
        matrix.summary(true),
        "| Area | not started | in progress | parity | intentional diff | Rows |\n\
         | --- | --- | --- | --- | --- | --- |\n\
         | demo | 1 | 0 | 1 | 1 | 3 |\n\
         | Total | 1 | 0 | 1 | 1 | 3 |\n"
    );
}

#[test]
fn rejects_duplicate_and_malformed_ids() {
    assert_edit(
        AREA_FILE,
        "`demo.three`",
        "`demo.one`",
        "areas/demo.md:7: duplicate row id `demo.one` (also at docs/rust-rewrite/parity/areas/demo.md:5)",
    );
    assert_edit(
        AREA_FILE,
        "`demo.three`",
        "`demo.Three`",
        "isn't `<area>.<name>` in kebab-case",
    );
    assert_edit(
        AREA_FILE,
        "`demo.three`",
        "`other.three`",
        "must start with this file's area, `demo.`",
    );
}

#[test]
fn rejects_unknown_statuses_and_qualifiers() {
    assert_edit(AREA_FILE, "| parity |", "| done |", "status `done` isn't one of");
    assert_edit(
        AREA_FILE,
        "| parity |",
        "| parity (mostly) |",
        "only `intentional diff` takes a `(qualifier)`",
    );
}

#[test]
fn rejects_bad_milestones_issues_and_cell_counts() {
    assert_edit(AREA_FILE, "| M4 |", "| M9 |", "milestone `M9` isn't M0–M5");
    assert_edit(AREA_FILE, "| #2, #3 |", "| 2 |", "issue `2` isn't `#n` or `#n, #m`");
    assert_edit(
        AREA_FILE,
        "| Three |",
        "| Three | extra |",
        "7 cells, but the header has 6",
    );
}

#[test]
fn py_bug_notes_need_a_file() {
    assert_edit(
        AREA_FILE,
        "Py bug B1.",
        "Py bug B1, B12.",
        "`demo.one` notes Py bug B12, but py-bugs/B12.md doesn't exist",
    );
}

#[test]
fn py_bug_files_are_checked() {
    assert_one(
        |r| {
            fs::copy(r.join(BUG_FILE), r.join("docs/rust-rewrite/parity/py-bugs/B001.md")).unwrap();
        },
        "duplicate py-bug number B1",
    );
    assert_edit(
        BUG_FILE,
        "`demo.one`",
        "`demo.gone`",
        "B01.md:7: unknown row id `demo.gone`",
    );
    assert_edit(
        BUG_FILE,
        "Status: open",
        "Status: fixed",
        "status `fixed` isn't one of open, answered, withdrawn",
    );
    assert_edit(
        BUG_FILE,
        "Status: open",
        "Status: answered",
        "an answered py bug needs an `Answer:` line",
    );
    assert_edit(
        BUG_FILE,
        "- Id: B1",
        "- Id: B2",
        "`Id: B2` doesn't match the file name (B1)",
    );
    assert_edit(
        BUG_FILE,
        "- Recommendation: Fix it.\n",
        "",
        "missing the `- Recommendation: …` line",
    );
}

#[test]
fn intentional_diffs_link_an_existing_decision() {
    assert_edit(
        AREA_FILE,
        "Decision: [gh](../decisions/gh.md).",
        "",
        "`demo.two` is an intentional diff but its Notes don't link a decision",
    );
    assert_edit(
        AREA_FILE,
        "../decisions/gh.md",
        "../decisions/nope.md",
        "`demo.two` links decisions/nope.md, which doesn't exist",
    );
    assert_edit(
        DECISION_FILE,
        "`demo.two`",
        "`demo.four`",
        "gh.md:6: unknown row id `demo.four`",
    );
}

#[test]
fn scenario_parity_rows_must_be_row_ids() {
    assert_edit(
        "parity/scenarios/demo.toml",
        "\"demo.one\"",
        "\"Main layout\"",
        "parity/scenarios/demo.toml:2: parity_rows names `Main layout`, which isn't a row id",
    );
}

#[test]
fn every_area_file_is_in_the_index() {
    assert_one(
        |r| fs::write(r.join("docs/rust-rewrite/parity/areas/extra.md"), "# Extra\n").unwrap(),
        "areas/extra.md is missing from the index",
    );
    // Its rows are gone too, so the py bug, decision and scenario that name them also fail.
    let found = problems(|r| fs::remove_file(r.join(AREA_FILE)).unwrap());
    assert!(
        found
            .iter()
            .any(|p| p.contains("the index links areas/demo.md, which doesn't exist")),
        "{found:#?}"
    );
}
