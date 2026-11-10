//! The fuzzy matcher scores and highlights as Textual's does: `fixtures/generate_fuzzy.py` dumps
//! `FuzzySearch.match` and `Matcher.highlight` for each query and name in its corpus to `fixtures/fuzzy.json`, and
//! this runs `fuzzy_match` on every pair and compares the score, the offsets and the highlighted offsets.
//!
//! Frozen from the Python app (lazy-github 0.6.6) by `crates/lazy-github-tui/tests/fixtures/generate_fuzzy.py`, removed
//! at cutover (#398; `git log --diff-filter=D -- crates/lazy-github-tui/tests/fixtures/generate_fuzzy.py` finds its
//! last version). Edit by hand when the Rust behavior changes on purpose.

use lazy_github_tui::fuzzy::fuzzy_match;
use serde::Deserialize;

const GOLDEN: &str = include_str!("fixtures/fuzzy.json");

#[derive(Debug, Deserialize)]
struct Case {
    query: String,
    name: String,
    /// 0 for no match.
    score: f64,
    offsets: Vec<usize>,
    highlights: Vec<usize>,
}

#[test]
fn fuzzy_match_reproduces_textual() {
    let cases: Vec<Case> = serde_json::from_str(GOLDEN).unwrap();
    assert!(cases.iter().any(|case| case.score > 0.0), "the golden has matches");
    let mut failures = Vec::new();
    for case in &cases {
        let got = fuzzy_match(&case.query, &case.name);
        let (score, offsets, highlights) = match &got {
            Some(hit) => (hit.score, hit.offsets.clone(), hit.highlights(&case.name)),
            None => (0.0, Vec::new(), Vec::new()),
        };
        if (score - case.score).abs() > 1e-9 || offsets != case.offsets || highlights != case.highlights {
            failures.push(format!(
                "{:?} in {:?}: Textual {} {:?} highlights {:?}, Rust {} {:?} highlights {:?}",
                case.query, case.name, case.score, case.offsets, case.highlights, score, offsets, highlights
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} pairs differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
