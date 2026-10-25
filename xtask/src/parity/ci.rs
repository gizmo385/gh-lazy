//! `--ci` output: GitHub Actions workflow commands (`::error` and `::warning`) for the results of a run.
//!
//! Errors mark what fails the job: every mismatch that counts against the gate, and every app run that fails it.
//! Warnings mark what doesn't: a `rust-not-ready` scenario (one annotation per scenario) and a run that only passed on
//! retry.

use std::fmt;
use std::path::Path;

use super::check::Mismatch;
use super::scenario::App;
use super::summary::{ScenarioResult, ScenarioSummary};

/// A structural mismatch from one capture at one size, and the app it counts against.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub size: String,
    pub capture: String,
    pub against: App,
    pub mismatch: Mismatch,
}

impl Finding {
    pub fn new(size: &str, capture: &str, against: App, mismatch: &Mismatch) -> Self {
        Self {
            size: size.to_owned(),
            capture: capture.to_owned(),
            against,
            mismatch: mismatch.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

/// One workflow command, attached to a scenario file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub level: Level,
    /// Repo-relative, with `/` separators, so GitHub can attach it to the file in the PR.
    pub file: String,
    pub message: String,
}

impl fmt::Display for Annotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
        };
        write!(
            f,
            "::{level} file={}::{}",
            escape_property(&self.file),
            escape_data(&self.message)
        )
    }
}

/// Escapes a workflow command's message, as `@actions/core` does.
fn escape_data(s: &str) -> String {
    s.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

/// Escapes a workflow command property value, which also can't hold `:` or `,`.
fn escape_property(s: &str) -> String {
    escape_data(s).replace(':', "%3A").replace(',', "%2C")
}

/// The scenario file as GitHub wants it: relative to the repo root when it's inside it, with `/` separators.
pub fn scenario_file(repo: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(repo).unwrap_or(path);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether a failure of `app` fails the job: the Python side always does, the Rust side only in a gated scenario.
pub fn gates(gate: bool, app: App) -> bool {
    gate || app == App::Python
}

/// The annotations for one scenario's results.
pub fn annotations(file: &str, summary: &ScenarioSummary, findings: &[Finding]) -> Vec<Annotation> {
    let name = &summary.name;
    let at = |level, message| Annotation {
        level,
        file: file.to_owned(),
        message,
    };
    let mut out = Vec::new();
    if let Some(error) = &summary.error {
        out.push(at(Level::Error, format!("{name}: harness error: {error}")));
    }

    let mut not_ready = Vec::new();
    for size in &summary.sizes {
        for (&app, status) in &size.apps {
            let where_ = format!("{name}/{}/{app}", size.size);
            if !status.passed() {
                if gates(summary.gate, app) {
                    out.push(at(Level::Error, format!("{where_}: {}", status.describe())));
                } else {
                    not_ready.push(format!("{} {app}: {}", size.size, status.describe()));
                }
            } else if status.flaky {
                out.push(at(Level::Warning, format!("{where_}: flaky, passed on retry")));
            }
        }
    }

    let mut ungated_mismatches = 0;
    for f in findings {
        if gates(summary.gate, f.against) {
            let m = &f.mismatch;
            let target = match &m.in_box {
                Some(b) => format!("{} {b}", m.check.as_str()),
                None => m.check.as_str().to_owned(),
            };
            out.push(at(
                Level::Error,
                format!("{name}/{}/{}: {target}: {}", f.size, f.capture, m.message),
            ));
        } else {
            ungated_mismatches += 1;
        }
    }

    if summary.result == ScenarioResult::RustNotReady {
        if ungated_mismatches > 0 {
            not_ready.push(format!("{ungated_mismatches} structural mismatch(es)"));
        }
        let detail = if not_ready.is_empty() {
            String::new()
        } else {
            format!(" ({})", not_ready.join("; "))
        };
        out.push(at(
            Level::Warning,
            format!("{name}: rust-not-ready, gate = false{detail}"),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use super::*;
    use crate::parity::check::Kind;
    use crate::parity::runner::{AppRunStatus, RunState};
    use crate::parity::summary::SizeSummary;

    fn status(state: RunState) -> AppRunStatus {
        serde_json::from_value(serde_json::json!({
            "status": state, "step": 0, "exit_code": null, "duration_ms": 15000, "flaky": false,
            "fixture_problems": false, "misses": [], "errors": []
        }))
        .unwrap()
    }

    fn summary(gate: bool, rust: RunState, result: ScenarioResult) -> ScenarioSummary {
        ScenarioSummary {
            name: "startup".into(),
            description: String::new(),
            gate,
            parity_rows: Vec::new(),
            result,
            sizes: vec![SizeSummary {
                size: "160x48".into(),
                apps: BTreeMap::from([(App::Python, status(RunState::Ok)), (App::Rust, status(rust))]),
                captures: Vec::new(),
            }],
            error: None,
        }
    }

    fn finding(against: App) -> Finding {
        Finding {
            size: "160x48".into(),
            capture: "main".into(),
            against,
            mismatch: Mismatch {
                check: Kind::Titles,
                in_box: Some("[5] Details".into()),
                python: Value::Null,
                rust: Value::Null,
                expected: None,
                message: "expected \"Overview\", got \"Details\"".into(),
            },
        }
    }

    const FILE: &str = "parity/scenarios/startup.toml";

    fn lines(annotations: &[Annotation]) -> Vec<String> {
        annotations.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_gated_mismatch_is_an_error() {
        let s = summary(true, RunState::Ok, ScenarioResult::Fail);
        assert_eq!(
            lines(&annotations(FILE, &s, &[finding(App::Rust)])),
            [
                "::error file=parity/scenarios/startup.toml::startup/160x48/main: titles [5] Details: expected \
                 \"Overview\", got \"Details\""
            ]
        );
    }

    #[test]
    fn an_ungated_rust_failure_is_one_warning() {
        let s = summary(false, RunState::Timeout, ScenarioResult::RustNotReady);
        assert_eq!(
            lines(&annotations(FILE, &s, &[finding(App::Rust)])),
            [
                "::warning file=parity/scenarios/startup.toml::startup: rust-not-ready, gate = false (160x48 rust: \
                 timeout at step 0 (15.0s); 1 structural mismatch(es))"
            ]
        );
    }

    #[test]
    fn python_failures_always_gate() {
        let mut s = summary(false, RunState::Ok, ScenarioResult::Fail);
        let python = s.sizes[0].apps.get_mut(&App::Python).unwrap();
        python.fixture_problems = true;
        let out = annotations(FILE, &s, &[finding(App::Python)]);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out.iter().all(|a| a.level == Level::Error), "{out:?}");
        assert!(
            out[0].message.starts_with("startup/160x48/python: fixture problems"),
            "{out:?}"
        );
    }

    #[test]
    fn flaky_runs_and_harness_errors_are_annotated() {
        let mut s = summary(true, RunState::Ok, ScenarioResult::Pass);
        s.sizes[0].apps.get_mut(&App::Rust).unwrap().flaky = true;
        assert_eq!(
            lines(&annotations(FILE, &s, &[])),
            ["::warning file=parity/scenarios/startup.toml::startup/160x48/rust: flaky, passed on retry"]
        );
        s.error = Some("wiping out: denied".into());
        assert_eq!(
            annotations(FILE, &s, &[])[0].message,
            "startup: harness error: wiping out: denied"
        );
    }

    #[test]
    fn commands_are_escaped() {
        let a = Annotation {
            level: Level::Error,
            file: "a:b,c.toml".into(),
            message: "100%\nnext".into(),
        };
        assert_eq!(a.to_string(), "::error file=a%3Ab%2Cc.toml::100%25%0Anext");
    }

    #[test]
    fn scenario_files_are_repo_relative() {
        let repo = Path::new("/repo");
        assert_eq!(
            scenario_file(repo, &repo.join("parity").join("scenarios").join("startup.toml")),
            FILE
        );
        assert_eq!(scenario_file(repo, Path::new("other.toml")), "other.toml");
    }
}
