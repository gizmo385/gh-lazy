//! `summary.json`: the machine-readable results of a parity run, and the rules that turn app statuses into a
//! scenario result. #93 adds `check` per capture; #94 renders it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::runner::AppRunStatus;
use super::scenario::App;

/// The only summary format version written.
pub const SUMMARY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub version: u32,
    pub scenarios: Vec<ScenarioSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioSummary {
    pub name: String,
    pub description: String,
    pub gate: bool,
    pub parity_rows: Vec<String>,
    pub result: ScenarioResult,
    pub sizes: Vec<SizeSummary>,
    /// A harness failure that stopped this scenario (exit code 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeSummary {
    /// `COLSxROWS`, which is also the directory name.
    pub size: String,
    /// The apps that ran at this size. An app left out didn't run.
    pub apps: BTreeMap<App, AppRunStatus>,
    pub captures: Vec<CaptureSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureSummary {
    pub name: String,
}

/// A scenario's result (`parity/README.md`, "Results").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioResult {
    /// Every app that ran passed, and both ran.
    Pass,
    /// The Python side failed, or a gated scenario's Rust side did.
    Fail,
    /// Not gated, and the Rust side failed.
    RustNotReady,
    /// Only the Python app ran (a flag or the scenario's `apps`), and it passed.
    PythonOnly,
    /// Only the Rust app ran, and it passed.
    RustOnly,
}

impl ScenarioResult {
    pub fn as_str(self) -> &'static str {
        match self {
            ScenarioResult::Pass => "pass",
            ScenarioResult::Fail => "fail",
            ScenarioResult::RustNotReady => "rust-not-ready",
            ScenarioResult::PythonOnly => "python-only",
            ScenarioResult::RustOnly => "rust-only",
        }
    }

    /// Computes a result from app statuses alone. #93 adds structural checks to this.
    pub fn from_statuses(gate: bool, sizes: &[SizeSummary]) -> Self {
        let ran = |app| sizes.iter().any(|s| s.apps.contains_key(&app));
        let failed = |app| {
            sizes
                .iter()
                .filter_map(|s| s.apps.get(&app))
                .any(|status| !status.passed())
        };
        if failed(App::Python) {
            return ScenarioResult::Fail;
        }
        if failed(App::Rust) {
            return if gate {
                ScenarioResult::Fail
            } else {
                ScenarioResult::RustNotReady
            };
        }
        match (ran(App::Python), ran(App::Rust)) {
            (true, false) => ScenarioResult::PythonOnly,
            (false, true) => ScenarioResult::RustOnly,
            _ => ScenarioResult::Pass,
        }
    }
}

impl std::fmt::Display for ScenarioResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::runner::RunState;

    fn status(state: RunState) -> AppRunStatus {
        serde_json::from_value(serde_json::json!({
            "status": state, "step": null, "exit_code": null, "duration_ms": 1, "flaky": false,
            "fixture_problems": false, "misses": [], "errors": []
        }))
        .unwrap()
    }

    fn size(apps: &[(App, AppRunStatus)]) -> SizeSummary {
        SizeSummary {
            size: "160x48".into(),
            apps: apps.iter().cloned().collect(),
            captures: Vec::new(),
        }
    }

    #[test]
    fn results_follow_the_rules() {
        use App::{Python, Rust};
        let ok = || status(RunState::Ok);
        let bad = || status(RunState::Timeout);
        let mut miss = ok();
        miss.fixture_problems = true;

        let r = ScenarioResult::from_statuses;
        assert_eq!(r(true, &[size(&[(Python, ok()), (Rust, ok())])]), ScenarioResult::Pass);
        assert_eq!(
            r(false, &[size(&[(Python, ok()), (Rust, bad())])]),
            ScenarioResult::RustNotReady
        );
        assert_eq!(r(true, &[size(&[(Python, ok()), (Rust, bad())])]), ScenarioResult::Fail);
        assert_eq!(
            r(false, &[size(&[(Python, bad()), (Rust, ok())])]),
            ScenarioResult::Fail
        );
        assert_eq!(
            r(false, &[size(&[(Python, ok())]), size(&[(Python, miss)])]),
            ScenarioResult::Fail
        );
        assert_eq!(r(false, &[size(&[(Python, ok())])]), ScenarioResult::PythonOnly);
        assert_eq!(r(false, &[size(&[(Rust, ok())])]), ScenarioResult::RustOnly);
        assert_eq!(r(false, &[size(&[(Rust, bad())])]), ScenarioResult::RustNotReady);
    }

    #[test]
    fn app_keys_serialize_as_names() {
        let s = size(&[(App::Python, status(RunState::Ok))]);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["apps"]["python"]["status"], "ok");
        assert_eq!(
            serde_json::to_value(ScenarioResult::RustNotReady).unwrap(),
            "rust-not-ready"
        );
    }
}
