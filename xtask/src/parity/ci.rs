//! `--ci` output: GitHub Actions workflow commands (`::error` and `::warning`) for the results of a run.
//!
//! Errors mark what fails the job: every mismatch, every run that fails, and any harness error (an invalid scenario
//! file included). Warnings mark what doesn't: a run that only passed on retry.

use std::fmt;
use std::path::Path;

use super::check::Mismatch;
use super::scenario::ScenarioError;
use super::summary::ScenarioSummary;

/// Which check found a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The capture's `[expect]` table.
    Expect,
    /// The committed snapshot.
    Snapshot,
}

/// A mismatch from one capture at one size.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub size: String,
    pub capture: String,
    pub source: Source,
    pub mismatch: Mismatch,
}

impl Finding {
    /// A mismatch between the screen and the capture's `[expect]` table.
    pub fn expect(size: &str, capture: &str, mismatch: &Mismatch) -> Self {
        Self::new(size, capture, Source::Expect, mismatch)
    }

    /// A run that differs from its committed snapshot.
    pub fn snapshot(size: &str, capture: &str, mismatch: &Mismatch) -> Self {
        Self::new(size, capture, Source::Snapshot, mismatch)
    }

    fn new(size: &str, capture: &str, source: Source, mismatch: &Mismatch) -> Self {
        Self {
            size: size.to_owned(),
            capture: capture.to_owned(),
            source,
            mismatch: mismatch.clone(),
        }
    }

    /// The check that failed, worded like `check_size`'s console lines: `expect titles [5] Details` or `snapshot`.
    fn target(&self) -> String {
        let m = &self.mismatch;
        let check = match self.source {
            Source::Expect => format!("expect {}", m.check.as_str()),
            Source::Snapshot => m.check.as_str().to_owned(),
        };
        match &m.in_box {
            Some(b) => format!("{check} {b}"),
            None => check,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

/// One workflow command, attached to a scenario file when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub level: Level,
    /// Repo-relative, with `/` separators, so GitHub can attach it to the file in the PR.
    pub file: Option<String>,
    /// 1-based, within `file`.
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Annotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
        };
        write!(f, "::{level}")?;
        if let Some(file) = &self.file {
            write!(f, " file={}", escape_property(file))?;
            if let Some(line) = self.line {
                write!(f, ",line={line}")?;
            }
        }
        write!(f, "::{}", escape_data(&self.message))
    }
}

/// The error for a harness failure that stops the whole run, which no scenario file is to blame for.
pub fn harness_error(message: String) -> Annotation {
    Annotation {
        level: Level::Error,
        file: None,
        line: None,
        message,
    }
}

/// The error for a scenario file that doesn't validate, on the line at fault when there is one.
pub fn invalid_scenario(repo: &Path, error: &ScenarioError) -> Annotation {
    Annotation {
        level: Level::Error,
        file: Some(scenario_file(repo, &error.file)),
        line: (error.line > 0).then_some(error.line),
        message: error.message.clone(),
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

/// The annotations for one scenario's results.
pub fn annotations(file: &str, summary: &ScenarioSummary, findings: &[Finding]) -> Vec<Annotation> {
    let name = &summary.name;
    let at = |level, message| Annotation {
        level,
        file: Some(file.to_owned()),
        line: None,
        message,
    };
    let mut out = Vec::new();
    if let Some(error) = &summary.error {
        out.push(at(Level::Error, format!("{name}: harness error: {error}")));
    }
    for size in &summary.sizes {
        let where_ = format!("{name}/{}", size.size);
        if !size.status.passed() {
            out.push(at(Level::Error, format!("{where_}: {}", size.status.describe())));
        } else if size.status.flaky {
            out.push(at(Level::Warning, format!("{where_}: flaky, passed on retry")));
        }
    }
    for f in findings {
        out.push(at(
            Level::Error,
            format!(
                "{name}/{}/{}: {}: {}",
                f.size,
                f.capture,
                f.target(),
                f.mismatch.message
            ),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::parity::check::Kind;
    use crate::parity::runner::{AppRunStatus, RunState};
    use crate::parity::summary::{ScenarioResult, SizeSummary};

    fn status(state: RunState) -> AppRunStatus {
        serde_json::from_value(serde_json::json!({
            "status": state, "step": 0, "exit_code": null, "duration_ms": 15000, "flaky": false,
            "fixture_problems": false, "misses": [], "errors": []
        }))
        .unwrap()
    }

    fn summary(state: RunState) -> ScenarioSummary {
        let sizes = vec![SizeSummary {
            size: "160x48".into(),
            status: status(state),
            captures: Vec::new(),
        }];
        ScenarioSummary {
            name: "startup".into(),
            description: String::new(),
            parity_rows: Vec::new(),
            result: ScenarioResult::from_sizes(&sizes),
            sizes,
            error: None,
        }
    }

    fn mismatch() -> Mismatch {
        Mismatch {
            check: Kind::Titles,
            in_box: Some("[5] Details".into()),
            actual: Value::Null,
            expected: None,
            message: "expected \"Overview\", got \"Details\"".into(),
        }
    }

    const FILE: &str = "parity/scenarios/startup.toml";

    fn lines(annotations: &[Annotation]) -> Vec<String> {
        annotations.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn an_expect_mismatch_is_an_error_naming_the_check() {
        let s = summary(RunState::Ok);
        let mut whole_screen = Finding::expect("160x48", "main", &mismatch());
        whole_screen.mismatch.in_box = None;
        assert_eq!(
            lines(&annotations(
                FILE,
                &s,
                &[Finding::expect("160x48", "main", &mismatch()), whole_screen]
            )),
            [
                "::error file=parity/scenarios/startup.toml::startup/160x48/main: expect titles [5] Details: \
                 expected \"Overview\", got \"Details\"",
                "::error file=parity/scenarios/startup.toml::startup/160x48/main: expect titles: expected \
                 \"Overview\", got \"Details\"",
            ]
        );
    }

    #[test]
    fn a_snapshot_mismatch_is_an_error() {
        let s = summary(RunState::Ok);
        let m = Mismatch::snapshot(
            Kind::Snapshot,
            "rows 12-14 differ; run cargo xtask parity --accept startup and review the diff".into(),
        );
        let f = Finding::snapshot("160x48", "main", &m);
        assert_eq!(f.source, Source::Snapshot);
        assert_eq!(
            lines(&annotations(FILE, &s, &[f])),
            [
                "::error file=parity/scenarios/startup.toml::startup/160x48/main: snapshot: rows 12-14 differ; \
                 run cargo xtask parity --accept startup and review the diff"
            ]
        );
    }

    #[test]
    fn harness_errors_are_errors() {
        assert_eq!(
            harness_error("parity: no scenario \"nope\"".into()).to_string(),
            "::error::parity: no scenario \"nope\""
        );
        let repo = Path::new("/repo");
        let bad = |line| ScenarioError {
            file: repo.join("parity").join("scenarios").join("startup.toml"),
            line,
            message: "unknown field `stpes`".into(),
        };
        assert_eq!(
            invalid_scenario(repo, &bad(3)).to_string(),
            "::error file=parity/scenarios/startup.toml,line=3::unknown field `stpes`"
        );
        assert_eq!(
            invalid_scenario(repo, &bad(0)).to_string(),
            "::error file=parity/scenarios/startup.toml::unknown field `stpes`"
        );
    }

    #[test]
    fn a_failed_run_is_an_error() {
        let s = summary(RunState::Timeout);
        assert_eq!(
            lines(&annotations(FILE, &s, &[])),
            ["::error file=parity/scenarios/startup.toml::startup/160x48: timeout at step 0 (15.0s)"]
        );
        let mut s = summary(RunState::Ok);
        s.sizes[0].status.fixture_problems = true;
        let out = annotations(FILE, &s, &[]);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(
            out[0].message.starts_with("startup/160x48: fixture problems"),
            "{out:?}"
        );
    }

    #[test]
    fn flaky_runs_and_harness_errors_are_annotated() {
        let mut s = summary(RunState::Ok);
        s.sizes[0].status.flaky = true;
        assert_eq!(
            lines(&annotations(FILE, &s, &[])),
            ["::warning file=parity/scenarios/startup.toml::startup/160x48: flaky, passed on retry"]
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
            file: Some("a:b,c.toml".into()),
            line: None,
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
