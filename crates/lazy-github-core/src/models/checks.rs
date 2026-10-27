//! A commit's status checks: the commit statuses of `GET .../commits/{ref}/status` and the check runs of
//! `GET .../commits/{ref}/check-runs`, and the summary the PR Overview draws from them.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Where a status check stands. Commit statuses carry it as is; a check run's is derived ([`CheckRun::state`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    /// It passed.
    Success,
    /// It hasn't finished.
    Pending,
    /// It couldn't run.
    Error,
    /// It failed.
    Failure,
}

impl CheckState {
    /// Whether it counts as failing: a failure or an error.
    pub fn is_failing(self) -> bool {
        matches!(self, CheckState::Failure | CheckState::Error)
    }

    /// Where a row in this state sorts: failing first, then pending, then passing.
    fn rank(self) -> u8 {
        match self {
            CheckState::Failure | CheckState::Error => 0,
            CheckState::Pending => 1,
            CheckState::Success => 2,
        }
    }
}

/// One commit status, as an external service reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStatus {
    /// What reported it, such as `ci/lint`.
    pub context: String,
    /// The service's short description. The API allows `null`.
    #[serde(default)]
    pub description: Option<String>,
    /// Where it stands.
    pub state: CheckState,
    /// The service's page for it, if it gave one.
    #[serde(default)]
    pub target_url: Option<String>,
}

/// A commit's combined status: every commit status, and GitHub's overall state for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombinedStatus {
    /// GitHub's overall state. It's `pending` when there are no statuses at all.
    pub state: CheckState,
    /// The latest status of each context.
    pub statuses: Vec<CommitStatus>,
}

/// Where a check run is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckRunStatus {
    /// Waiting for a runner.
    Queued,
    /// Running.
    InProgress,
    /// Done: its [`CheckRun::conclusion`] says how it went.
    Completed,
    /// Waiting for a deployment protection rule.
    Waiting,
    /// Requested but not queued yet.
    Requested,
    /// Pending.
    Pending,
}

/// How a completed check run went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckConclusion {
    /// It passed.
    Success,
    /// It failed.
    Failure,
    /// It finished without passing or failing.
    Neutral,
    /// It was cancelled.
    Cancelled,
    /// It was skipped.
    Skipped,
    /// It ran out of time.
    TimedOut,
    /// It needs someone to act.
    ActionRequired,
    /// GitHub marked it stale.
    Stale,
    /// It failed to start.
    StartupFailure,
}

/// One check run, such as a GitHub Actions job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    /// GitHub's numeric id.
    pub id: u64,
    /// Its name, such as `build`.
    pub name: String,
    /// Where it is in its life.
    pub status: CheckRunStatus,
    /// How it went, once completed.
    #[serde(default)]
    pub conclusion: Option<CheckConclusion>,
    /// Its page on GitHub.
    #[serde(default)]
    pub html_url: Option<String>,
}

impl CheckRun {
    /// Where it stands: pending until completed, then passed for `success`, `neutral` and `skipped` and failed for any
    /// other conclusion. A completed run without a conclusion is still pending.
    pub fn state(&self) -> CheckState {
        match (self.status, self.conclusion) {
            (CheckRunStatus::Completed, Some(conclusion)) => match conclusion {
                CheckConclusion::Success | CheckConclusion::Neutral | CheckConclusion::Skipped => CheckState::Success,
                CheckConclusion::Failure
                | CheckConclusion::Cancelled
                | CheckConclusion::TimedOut
                | CheckConclusion::ActionRequired
                | CheckConclusion::Stale
                | CheckConclusion::StartupFailure => CheckState::Failure,
            },
            _ => CheckState::Pending,
        }
    }
}

/// The overall state of a commit's checks: failed if any is failing (a failure or an error), else pending if any is
/// pending, else passed. The combined status's own state counts only when there are commit statuses, as GitHub
/// reports `pending` for a commit with none.
pub fn overall_state(status: &CombinedStatus, runs: &[CheckRun]) -> CheckState {
    let combined = (!status.statuses.is_empty()).then_some(status.state);
    let all: Vec<CheckState> = combined.into_iter().chain(runs.iter().map(CheckRun::state)).collect();
    if all.iter().any(|state| state.is_failing()) {
        CheckState::Failure
    } else if all.contains(&CheckState::Pending) {
        CheckState::Pending
    } else {
        CheckState::Success
    }
}

/// One row of the checks list: a commit status or a check run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRow {
    /// Where it stands.
    pub state: CheckState,
    /// The status's context or the run's name.
    pub name: String,
    /// The status's description, if it has one. Check runs have none.
    pub description: Option<String>,
    /// Its page: the status's `target_url` or the run's `html_url`.
    pub url: Option<String>,
}

/// Every commit status and check run as one list: failing first, then pending, then passing, each by name (Py bug
/// B23's recommendation; Python sorts by the rows' markup).
pub fn check_rows(status: &CombinedStatus, runs: &[CheckRun]) -> Vec<CheckRow> {
    let statuses = status.statuses.iter().map(|status| CheckRow {
        state: status.state,
        name: status.context.clone(),
        description: status.description.clone(),
        url: status.target_url.clone(),
    });
    let runs = runs.iter().map(|run| CheckRow {
        state: run.state(),
        name: run.name.clone(),
        description: None,
        url: run.html_url.clone(),
    });
    let mut rows: Vec<CheckRow> = statuses.chain(runs).collect();
    rows.sort_by(|a, b| match a.state.rank().cmp(&b.state.rank()) {
        Ordering::Equal => a.name.cmp(&b.name),
        other => other,
    });
    rows
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn run(status: CheckRunStatus, conclusion: Option<CheckConclusion>) -> CheckRun {
        CheckRun {
            id: 1,
            name: "build".into(),
            status,
            conclusion,
            html_url: None,
        }
    }

    fn status(context: &str, state: CheckState) -> CommitStatus {
        CommitStatus {
            context: context.into(),
            description: Some("desc".into()),
            state,
            target_url: None,
        }
    }

    #[test]
    fn a_run_is_pending_until_it_completes() {
        for status in [
            CheckRunStatus::Queued,
            CheckRunStatus::InProgress,
            CheckRunStatus::Waiting,
            CheckRunStatus::Requested,
            CheckRunStatus::Pending,
        ] {
            assert_eq!(run(status, None).state(), CheckState::Pending, "{status:?}");
            // A conclusion before completion doesn't count.
            assert_eq!(
                run(status, Some(CheckConclusion::Failure)).state(),
                CheckState::Pending,
                "{status:?}"
            );
        }
        assert_eq!(run(CheckRunStatus::Completed, None).state(), CheckState::Pending);
    }

    #[test]
    fn a_completed_run_passes_or_fails_by_its_conclusion() {
        for (conclusion, state) in [
            (CheckConclusion::Success, CheckState::Success),
            (CheckConclusion::Neutral, CheckState::Success),
            (CheckConclusion::Skipped, CheckState::Success),
            (CheckConclusion::Failure, CheckState::Failure),
            (CheckConclusion::Cancelled, CheckState::Failure),
            (CheckConclusion::TimedOut, CheckState::Failure),
            (CheckConclusion::ActionRequired, CheckState::Failure),
            (CheckConclusion::Stale, CheckState::Failure),
            (CheckConclusion::StartupFailure, CheckState::Failure),
        ] {
            assert_eq!(
                run(CheckRunStatus::Completed, Some(conclusion)).state(),
                state,
                "{conclusion:?}"
            );
        }
    }

    #[test]
    fn the_api_spellings_decode() {
        let run: CheckRun = serde_json::from_value(json!({
            "id": 7, "name": "x", "status": "in_progress", "conclusion": null, "html_url": null
        }))
        .unwrap();
        assert_eq!(run.status, CheckRunStatus::InProgress);
        let run: CheckRun = serde_json::from_value(json!({
            "id": 7, "name": "x", "status": "completed", "conclusion": "startup_failure"
        }))
        .unwrap();
        assert_eq!(run.conclusion, Some(CheckConclusion::StartupFailure));
        assert_eq!(run.html_url, None);
        let status: CommitStatus = serde_json::from_value(json!({
            "context": "ci", "description": null, "state": "error", "target_url": null
        }))
        .unwrap();
        assert_eq!(status.state, CheckState::Error);
        assert_eq!(status.description, None);
    }

    #[test]
    fn failing_wins_then_pending_else_passed() {
        let combined = |state, statuses| CombinedStatus { state, statuses };
        let passed = run(CheckRunStatus::Completed, Some(CheckConclusion::Success));
        let failed = run(CheckRunStatus::Completed, Some(CheckConclusion::Failure));
        let pending = run(CheckRunStatus::Queued, None);
        let none = combined(CheckState::Pending, Vec::new());

        assert_eq!(overall_state(&none, &[]), CheckState::Success);
        assert_eq!(overall_state(&none, std::slice::from_ref(&passed)), CheckState::Success);
        assert_eq!(
            overall_state(&none, &[passed.clone(), pending.clone()]),
            CheckState::Pending
        );
        assert_eq!(
            overall_state(&none, &[pending.clone(), failed.clone(), passed.clone()]),
            CheckState::Failure
        );
        // The combined state counts only with commit statuses: GitHub says `pending` when there are none.
        let ok = combined(CheckState::Success, vec![status("a", CheckState::Success)]);
        assert_eq!(overall_state(&ok, std::slice::from_ref(&passed)), CheckState::Success);
        let errored = combined(CheckState::Error, vec![status("a", CheckState::Error)]);
        assert_eq!(
            overall_state(&errored, std::slice::from_ref(&pending)),
            CheckState::Failure
        );
        let waiting = combined(CheckState::Pending, vec![status("a", CheckState::Pending)]);
        assert_eq!(overall_state(&waiting, &[passed]), CheckState::Pending);
        let failure = combined(CheckState::Failure, vec![status("a", CheckState::Failure)]);
        assert_eq!(overall_state(&failure, &[]), CheckState::Failure);
    }

    #[test]
    fn rows_sort_failing_pending_passing_then_by_name() {
        let combined = CombinedStatus {
            state: CheckState::Pending,
            statuses: vec![
                status("ci/lint", CheckState::Success),
                status("ci/docs", CheckState::Pending),
                status("ci/err", CheckState::Error),
            ],
        };
        let named = |name: &str, status, conclusion| CheckRun {
            name: name.into(),
            ..run(status, conclusion)
        };
        let runs = [
            named("build", CheckRunStatus::Completed, Some(CheckConclusion::Success)),
            named("test", CheckRunStatus::Completed, Some(CheckConclusion::Failure)),
            named("deploy", CheckRunStatus::InProgress, None),
        ];
        let rows = check_rows(&combined, &runs);
        let order: Vec<(&str, CheckState)> = rows.iter().map(|row| (row.name.as_str(), row.state)).collect();
        assert_eq!(
            order,
            [
                ("ci/err", CheckState::Error),
                ("test", CheckState::Failure),
                ("ci/docs", CheckState::Pending),
                ("deploy", CheckState::Pending),
                ("build", CheckState::Success),
                ("ci/lint", CheckState::Success),
            ]
        );
        assert_eq!(rows[0].description.as_deref(), Some("desc"));
        assert_eq!(rows[1].description, None);
    }
}
