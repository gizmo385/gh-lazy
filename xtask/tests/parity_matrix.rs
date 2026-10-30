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

    // The coverage columns add up to each area's rows, and the areas to the status table's total.
    let coverage = matrix.coverage_counts();
    for (area, [rows, columns @ ..]) in &coverage {
        assert_eq!(columns.iter().sum::<usize>(), *rows, "{area}");
    }
    let rows: usize = coverage.iter().map(|(_, c)| c[0]).sum();
    let by_status: usize = matrix.counts().iter().map(|(_, c)| c.iter().sum::<usize>()).sum();
    assert_eq!(rows, by_status);

    // Every status row has a milestone, so the milestone table has exactly M0 to M6 and the same total.
    let by_milestone = matrix.counts_by_milestone();
    assert_eq!(by_milestone.len(), 7);
    assert_eq!(
        by_milestone.iter().map(|(_, c)| c.iter().sum::<usize>()).sum::<usize>(),
        by_status
    );
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
const GOLDEN: &str = r#"version = 1
description = "Demo golden"
generator = "gen.py"
golden = ["golden.json"]
test = "tests/demo.rs"
fresh_test = "demo_goldens_are_fresh"
parity_rows = ["demo.two"]
"#;
const GOLDEN_TEST: &str = r#"
#[test]
fn matches_the_golden() {}

#[cfg(unix)]
#[test]
#[ignore = "needs uv and the Python app"]
fn demo_goldens_are_fresh() {}
"#;

/// A repo with a valid matrix, with `edit` applied to one of its files.
fn repo_with(edit: impl FnOnce(&Path)) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let m = root.join("docs/rust-rewrite/parity");
    for sub in ["areas", "py-bugs", "decisions"] {
        fs::create_dir_all(m.join(sub)).unwrap();
    }
    fs::create_dir_all(root.join("parity/scenarios")).unwrap();
    fs::create_dir_all(root.join("parity/goldens")).unwrap();
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::write(m.join("README.md"), README).unwrap();
    fs::write(m.join("areas/demo.md"), AREA).unwrap();
    fs::write(m.join("py-bugs/B01.md"), BUG).unwrap();
    fs::write(m.join("decisions/gh.md"), DECISION).unwrap();
    fs::write(root.join("parity/scenarios/demo.toml"), SCENARIO).unwrap();
    fs::write(root.join("parity/goldens/demo.toml"), GOLDEN).unwrap();
    fs::write(root.join("gen.py"), "").unwrap();
    fs::write(root.join("golden.json"), "[]\n").unwrap();
    fs::write(root.join("tests/demo.rs"), GOLDEN_TEST).unwrap();
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
const GOLDEN_FILE: &str = "parity/goldens/demo.toml";

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
fn counts_statuses_by_milestone() {
    let dir = repo_with(|_| {});
    let matrix = check(dir.path());
    assert_eq!(matrix.row_ids().count(), 3);
    assert_eq!(matrix.areas[0].rows[0].milestone.as_deref(), Some("M1"));
    assert_eq!(
        matrix.counts_by_milestone(),
        vec![
            ("M0", [0, 0, 0, 0]),
            ("M1", [1, 0, 0, 0]),
            ("M2", [0, 0, 0, 1]),
            ("M3", [0, 0, 0, 0]),
            ("M4", [0, 0, 1, 0]),
            ("M5", [0, 0, 0, 0]),
            ("M6", [0, 0, 0, 0]),
        ]
    );

    // A row with an invalid milestone is a problem, and still counted, under `none`, so the totals agree.
    let dir = repo_with(|root| {
        let path = root.join(AREA_FILE);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text + "| `demo.four` | Four | M9 | in progress | | |\n").unwrap();
    });
    let matrix = check(dir.path());
    assert_eq!(matrix.problems.len(), 1, "{:#?}", matrix.problems);
    let by_milestone = matrix.counts_by_milestone();
    assert_eq!(by_milestone.len(), 8);
    assert_eq!(by_milestone[7], ("none", [0, 1, 0, 0]));
    let total = |counts: &[(&str, [usize; 4])]| counts.iter().flat_map(|(_, c)| c).sum::<usize>();
    assert_eq!(total(&by_milestone), total(&matrix.counts()));
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
    assert_edit(AREA_FILE, "| M4 |", "| M9 |", "milestone `M9` isn't M0–M6");
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
    // Its rows are gone too, so the py bug, decision, scenario and golden that name them also fail.
    let found = problems(|r| fs::remove_file(r.join(AREA_FILE)).unwrap());
    assert!(
        found
            .iter()
            .any(|p| p.contains("the index links areas/demo.md, which doesn't exist")),
        "{found:#?}"
    );
}

#[test]
fn golden_parity_rows_must_be_row_ids() {
    assert_edit(
        GOLDEN_FILE,
        "\"demo.two\"",
        "\"demo.gone\"",
        "parity/goldens/demo.toml:7: parity_rows names `demo.gone`, which isn't a row id",
    );
}

#[test]
fn golden_manifests_reject_unknown_keys() {
    assert_edit(
        GOLDEN_FILE,
        "parity_rows =",
        "parity_row =",
        "parity/goldens/demo.toml:7: unknown field `parity_row`",
    );
}

#[test]
fn golden_manifests_are_version_1() {
    assert_edit(
        GOLDEN_FILE,
        "version = 1",
        "version = 2",
        "parity/goldens/demo.toml:1: version 2 isn't supported (only 1 is)",
    );
}

#[test]
fn golden_manifest_paths_must_exist() {
    assert_one(
        |r| fs::remove_file(r.join("gen.py")).unwrap(),
        "parity/goldens/demo.toml:3: generator file `gen.py` doesn't exist",
    );
    assert_one(
        |r| fs::remove_file(r.join("golden.json")).unwrap(),
        "parity/goldens/demo.toml:4: golden file `golden.json` doesn't exist",
    );
    assert_one(
        |r| fs::remove_file(r.join("tests/demo.rs")).unwrap(),
        "parity/goldens/demo.toml:5: test file `tests/demo.rs` doesn't exist",
    );
}

#[test]
fn golden_test_must_run_in_plain_cargo_test() {
    assert_edit(
        "tests/demo.rs",
        "#[test]\nfn matches_the_golden",
        "#[test]\n#[ignore]\nfn matches_the_golden",
        "parity/goldens/demo.toml:5: tests/demo.rs has no #[test] that isn't #[ignore]d",
    );
}

#[test]
fn fresh_test_must_exist() {
    assert_edit(
        GOLDEN_FILE,
        "\"demo_goldens_are_fresh\"",
        "\"demo_golden_is_fresh\"",
        "parity/goldens/demo.toml:6: fresh_test `demo_golden_is_fresh` isn't a #[test] fn in tests/demo.rs",
    );
}

#[test]
fn fresh_test_must_be_ignored() {
    assert_edit(
        "tests/demo.rs",
        "#[ignore = \"needs uv and the Python app\"]\n",
        "",
        "parity/goldens/demo.toml:6: fresh_test `demo_goldens_are_fresh` in tests/demo.rs isn't #[ignore]d",
    );
}

/// The fixture plus rows in every coverage column: the scenario alone names `demo.one`, the golden alone names
/// `demo.two`, a second scenario and the golden name `demo.four`, and nothing names `demo.three` or `demo.five` (an
/// intentional diff).
fn coverage_fixture() -> tempfile::TempDir {
    repo_with(|r| {
        let area = r.join(AREA_FILE);
        let rows = "| `demo.four` | Four | M1 | in progress | | |\n\
                    | `demo.five` | Five | M1 | intentional diff | | Decision: [gh](../decisions/gh.md). |\n";
        fs::write(&area, fs::read_to_string(&area).unwrap() + rows).unwrap();
        fs::write(
            r.join("parity/scenarios/both.toml"),
            "version = 1\nparity_rows = [\"demo.four\"]\n",
        )
        .unwrap();
        let golden = r.join(GOLDEN_FILE);
        let text = fs::read_to_string(&golden).unwrap();
        fs::write(&golden, text.replace("[\"demo.two\"]", "[\"demo.two\", \"demo.four\"]")).unwrap();
    })
}

#[test]
fn coverage_summary_counts_rows_per_column() {
    let dir = coverage_fixture();
    let matrix = check(dir.path());
    assert!(matrix.problems.is_empty(), "{:#?}", matrix.problems);
    assert_eq!(matrix.coverage_counts(), vec![("demo", [5, 1, 1, 1, 2])]);
    insta::assert_snapshot!("coverage_text", matrix.coverage_summary(false));
    insta::assert_snapshot!("coverage_markdown", matrix.coverage_summary(true));
}

#[test]
fn uncovered_lists_neither_rows_except_intentional_diffs() {
    let dir = coverage_fixture();
    let matrix = check(dir.path());
    let ids: Vec<&str> = matrix.uncovered().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["demo.three"]);
    assert_eq!(matrix.uncovered_report(), "demo.three  parity\n");
}
