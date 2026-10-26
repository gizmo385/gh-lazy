//! GitHub Actions workflows and their runs.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{Repository, User};

/// Whether a workflow can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    /// Enabled.
    Active,
    /// Its file was deleted.
    Deleted,
    /// Disabled because the repository is a fork.
    DisabledFork,
    /// Disabled after a period without activity.
    DisabledInactivity,
    /// Disabled by a user.
    DisabledManually,
}

/// A GitHub Actions workflow: one workflow file in a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    /// GitHub's numeric id.
    pub id: u64,
    /// The workflow's `name`, such as `CI`.
    pub name: String,
    /// Whether it can run.
    pub state: WorkflowState,
    /// The workflow file's path, such as `.github/workflows/ci.yml`.
    pub path: String,
    /// When it was added.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// Its API URL.
    pub url: String,
}

/// One run of a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// GitHub's numeric id, unique across workflows and repositories.
    pub id: u64,
    /// The workflow's name.
    pub name: String,
    /// The run's title, such as the head commit's message.
    pub display_title: String,
    /// The workflow file's path.
    pub path: String,
    /// The run's number within its workflow. Not unique across a repository's workflows.
    pub run_number: u64,
    /// Which attempt this is, from 1.
    #[serde(default)]
    pub run_attempt: Option<u64>,
    /// The branch it ran on.
    pub head_branch: String,
    /// The commit it ran on.
    pub head_sha: String,
    /// Such as `queued`, `in_progress` or `completed`.
    pub status: String,
    /// Such as `success` or `failure`, once the run completed.
    #[serde(default)]
    pub conclusion: Option<String>,
    /// The event that triggered it, such as `push`.
    pub event: String,
    /// Who started the first attempt.
    pub actor: User,
    /// Who started this attempt.
    pub triggering_actor: User,
    /// The repository it ran in.
    pub repository: Repository,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// Its page on GitHub.
    pub html_url: String,
    /// The API URL of its jobs.
    pub jobs_url: String,
    /// The API URL of its logs archive.
    pub logs_url: String,
    /// When this attempt started.
    #[serde(default)]
    pub run_started_at: Option<Timestamp>,
}
