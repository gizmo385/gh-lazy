//! PR draft files shared with the Python app. The goldens in `tests/fixtures/pr_drafts/` are written by the Python
//! app (`generate.py` there); these tests check the Rust app names them the same, reads them as the drafts Python
//! saved, and writes those drafts as the same JSON values.

use std::path::{Path, PathBuf};

use lazy_github_core::drafts::{self, PrDraft};
use serde_json::Value;

const GENERATOR: &str = "crates/lazy-github-core/tests/fixtures/pr_drafts/generate.py";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The cache directory the goldens were generated into.
fn cache_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn pr_details() -> PrDraft {
    PrDraft {
        repo_full_name: "parity-bot/pr-details".into(),
        title: "Restored draft title".into(),
        description: "Restored draft body.\n\nSecond paragraph.".into(),
        base_ref: "main".into(),
        head_ref: "feature/restored".into(),
        is_draft: true,
        reviewers: vec!["parity-friend".into()],
    }
}

fn unicode() -> PrDraft {
    PrDraft {
        repo_full_name: "parity-bot/unicode".into(),
        title: "Ünïcödé títle — ✓".into(),
        description: "Naïve café notes: 日本語 and emoji 🎉.".into(),
        base_ref: "main".into(),
        head_ref: "feature/ünï".into(),
        is_draft: false,
        reviewers: Vec::new(),
    }
}

/// Reads the golden for `expected`'s repo as the Rust app would, checks it gives `expected`, and checks `expected`
/// written by the Rust app is the same JSON as the golden.
fn check(expected: &PrDraft) {
    let path = drafts::draft_path(&cache_dir(), &expected.repo_full_name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        &drafts::try_parse(&path, &text).unwrap(),
        expected,
        "{}",
        path.display()
    );
    let written: Value = serde_json::from_str(&drafts::to_json(expected)).unwrap();
    let golden: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written, golden, "{}", path.display());
}

#[test]
fn a_python_draft_with_every_field_reads_and_writes_back() {
    check(&pr_details());
}

#[test]
fn a_python_draft_with_non_ascii_text_reads_and_writes_back() {
    check(&unicode());
}

#[test]
fn the_pr_draft_seed_is_the_python_draft() {
    let seed =
        root().join("parity/seeds/pr-draft/home/.config/lazy-github/.cache/pr_drafts/parity-bot_pr-details.json");
    let golden = drafts::draft_path(&cache_dir(), "parity-bot/pr-details");
    assert_eq!(
        std::fs::read_to_string(seed).unwrap(),
        std::fs::read_to_string(golden).unwrap()
    );
}

/// Regenerates the goldens with the Python app and checks the committed copies match (the golden manifest is
/// `parity/goldens/pr-drafts.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn pr_drafts_goldens_are_fresh() {
    use std::process::Command;

    const GOLDENS: [&str; 2] = ["parity-bot_pr-details.json", "parity-bot_unicode.json"];
    let root = root();
    let out = tempfile::tempdir().unwrap();
    let found = Command::new("uv")
        .args(["run", "--frozen", "python", "-c", "import sys; print(sys.executable)"])
        .current_dir(&root)
        .output()
        .expect("uv is on PATH");
    assert!(
        found.status.success(),
        "uv run failed:\n{}",
        String::from_utf8_lossy(&found.stderr)
    );
    let python = String::from_utf8(found.stdout).unwrap();
    let run = Command::new(python.trim())
        .arg(GENERATOR)
        .arg("--out")
        .arg(out.path())
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{GENERATOR} failed:\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    for golden in GOLDENS {
        let committed = std::fs::read_to_string(cache_dir().join("pr_drafts").join(golden)).unwrap();
        let fresh = std::fs::read_to_string(out.path().join("pr_drafts").join(golden)).unwrap();
        if committed != fresh {
            let diff = similar::TextDiff::from_lines(&committed, &fresh)
                .unified_diff()
                .header(golden, "regenerated")
                .to_string();
            let diff: Vec<&str> = diff.lines().take(40).collect();
            panic!(
                "{golden} is stale; the Python app now writes:\n{}\n\n\
                 Regenerate it from the repository root with:\n    uv run python {GENERATOR}",
                diff.join("\n")
            );
        }
    }
}
