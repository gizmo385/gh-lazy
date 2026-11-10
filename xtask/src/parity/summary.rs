//! `summary.json`: the machine-readable results of a parity run, and the rule that turns a run's status and its
//! captures' checks into a scenario result.

use serde::{Deserialize, Serialize};

use super::runner::AppRunStatus;

/// The only summary format version written. Version 1 had a per-app split and the Python-only results.
pub const SUMMARY_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub version: u32,
    pub scenarios: Vec<ScenarioSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioSummary {
    pub name: String,
    pub description: String,
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
    /// How the run at this size went (its `status.json`).
    pub status: AppRunStatus,
    pub captures: Vec<CaptureSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureSummary {
    pub name: String,
    /// The capture's check: its `[expect]` and its snapshot (`<capture>.check.json` has the details).
    pub check: CheckOutcome,
    pub mismatches: usize,
}

impl CaptureSummary {
    /// A capture nothing checked.
    pub fn unchecked(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            check: CheckOutcome::NotApplicable,
            mismatches: 0,
        }
    }
}

/// A capture's check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckOutcome {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    /// Nothing checked it: the run didn't reach it, or it has no `[expect]` and the run's snapshot wasn't compared
    /// (a size the scenario doesn't list).
    #[serde(rename = "n/a")]
    NotApplicable,
}

impl CheckOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckOutcome::Pass => "pass",
            CheckOutcome::Fail => "fail",
            CheckOutcome::NotApplicable => "n/a",
        }
    }
}

/// A scenario's result (`parity/README.md`, "Results").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioResult {
    /// Every size ran, and every check passed.
    Pass,
    /// A run failed, or a capture's check did.
    Fail,
}

impl ScenarioResult {
    pub fn as_str(self) -> &'static str {
        match self {
            ScenarioResult::Pass => "pass",
            ScenarioResult::Fail => "fail",
        }
    }

    /// The result of the runs at `sizes`: a fail when any run failed or any capture has mismatches.
    pub fn from_sizes(sizes: &[SizeSummary]) -> Self {
        let failed = sizes
            .iter()
            .any(|s| !s.status.passed() || s.captures.iter().any(|c| c.mismatches > 0));
        if failed {
            ScenarioResult::Fail
        } else {
            ScenarioResult::Pass
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

    fn size(status: AppRunStatus, mismatches: usize) -> SizeSummary {
        SizeSummary {
            size: "160x48".into(),
            status,
            captures: vec![CaptureSummary {
                name: "main".into(),
                check: if mismatches == 0 {
                    CheckOutcome::Pass
                } else {
                    CheckOutcome::Fail
                },
                mismatches,
            }],
        }
    }

    #[test]
    fn a_failed_run_or_a_mismatch_fails_the_scenario() {
        let ok = || status(RunState::Ok);
        let r = ScenarioResult::from_sizes;
        assert_eq!(r(&[size(ok(), 0)]), ScenarioResult::Pass);
        assert_eq!(
            r(&[size(ok(), 0), size(status(RunState::Timeout), 0)]),
            ScenarioResult::Fail
        );
        assert_eq!(r(&[size(ok(), 0), size(ok(), 2)]), ScenarioResult::Fail);
        let mut miss = ok();
        miss.fixture_problems = true;
        assert_eq!(r(&[size(miss, 0)]), ScenarioResult::Fail);
        assert_eq!(r(&[]), ScenarioResult::Pass);
    }

    #[test]
    fn serializes_one_status_per_size() {
        let json = serde_json::to_value(size(status(RunState::Ok), 0)).unwrap();
        assert_eq!(json["status"]["status"], "ok");
        assert_eq!(
            json["captures"][0],
            serde_json::json!({"name": "main", "check": "pass", "mismatches": 0})
        );
        assert_eq!(serde_json::to_value(CheckOutcome::NotApplicable).unwrap(), "n/a");
        assert_eq!(serde_json::to_value(ScenarioResult::Fail).unwrap(), "fail");
    }
}
