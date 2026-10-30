//! `core::links` against the Python app's own link patterns (`github_url_patterns.json`, golden manifest
//! `parity/goldens/github-url-patterns.toml`): for every text in the corpus, the validator's answer and the link
//! looked up with and without repository links.

mod common;

use std::path::Path;

use lazy_github_core::links::{GithubLink, looks_like_github_url, parse_github_url};
use serde_json::{Value, json};

const GOLDEN: &str = "crates/lazy-github-tui/tests/fixtures/github_url_patterns.json";

fn golden() -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(GOLDEN);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A number as Python wrote it: one too long for `u64` (a JSON float here) is `u64::MAX` on the Rust side.
fn number(value: &Value) -> Value {
    json!(value.as_u64().unwrap_or(u64::MAX))
}

/// The golden's shape of a parsed link.
fn as_golden(link: Option<GithubLink>) -> Value {
    match link {
        None => Value::Null,
        Some(GithubLink::User { login }) => json!({ "kind": "user", "login": login }),
        Some(GithubLink::PullRequest { owner, repo, number }) => {
            json!({ "kind": "pull_request", "owner": owner, "repo": repo, "number": number })
        }
        Some(GithubLink::Issue { owner, repo, number }) => {
            json!({ "kind": "issue", "owner": owner, "repo": repo, "number": number })
        }
        Some(GithubLink::Repository { owner, repo }) => json!({ "kind": "repository", "owner": owner, "repo": repo }),
    }
}

fn python_link(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(n) = value.get("number").map(number) {
        value["number"] = n;
    }
    value
}

#[test]
fn links_parse_as_in_python() {
    let cases = golden();
    assert!(cases.len() > 40, "the corpus has every case");
    let mut wrong = Vec::new();
    for case in &cases {
        let text = case["text"].as_str().unwrap();
        let checks = [
            ("valid", json!(looks_like_github_url(text)), case["valid"].clone()),
            (
                "accept_repo",
                as_golden(parse_github_url(text, true)),
                python_link(&case["accept_repo"]),
            ),
            (
                "reject_repo",
                as_golden(parse_github_url(text, false)),
                python_link(&case["reject_repo"]),
            ),
        ];
        for (what, rust, python) in checks {
            if rust != python {
                wrong.push(format!("{text:?} {what}: Rust {rust}, Python {python}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Regenerates `github_url_patterns.json` with the Python app and checks the committed copy matches (the golden
/// manifest is `parity/goldens/github-url-patterns.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn github_url_patterns_golden_is_fresh() {
    let home = tempfile::tempdir().unwrap();
    common::assert_goldens_fresh(
        "crates/lazy-github-tui/tests/fixtures/generate_github_url_patterns.py",
        &[GOLDEN],
        home.path(),
        "uv run python crates/lazy-github-tui/tests/fixtures/generate_github_url_patterns.py",
    );
}
