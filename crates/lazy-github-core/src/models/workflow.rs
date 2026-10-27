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

/// Declares a string-valued API enum: the listed variants by their API spelling, and `Other` for any other value, so
/// a value GitHub adds later still decodes (Python rejects it, and its jobs worker crashes).
macro_rules! api_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $api:literal,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
            /// A value not listed here, as the API spells it.
            Other(String),
        }

        impl $name {
            /// The API's spelling.
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $api,)*
                    $name::Other(other) => other,
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $($api => $name::$variant,)*
                    _ => $name::Other(value),
                }
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                match value {
                    $name::Other(other) => other,
                    known => known.as_str().to_owned(),
                }
            }
        }
    };
}

api_enum! {
    /// Where a job or a step is: Python's `WorkflowJobStatus` and `WorkflowStepStatus`.
    JobStatus {
        /// Waiting for a runner.
        Queued = "queued",
        /// Running.
        InProgress = "in_progress",
        /// Not started yet.
        Pending = "pending",
        /// Finished: the conclusion says how.
        Completed = "completed",
        /// Waiting on a protection rule, such as a required review.
        Waiting = "waiting",
        /// Requested, not yet queued.
        Requested = "requested",
    }
}

api_enum! {
    /// How a completed job or step ended: Python's `WorkflowJobConclusion` and `WorkflowStepConclusion`.
    JobConclusion {
        /// It passed.
        Success = "success",
        /// It failed.
        Failure = "failure",
        /// It was cancelled.
        Cancelled = "cancelled",
        /// It was skipped.
        Skipped = "skipped",
        /// It finished without passing or failing.
        Neutral = "neutral",
        /// It needs someone to act.
        ActionRequired = "action_required",
        /// It ran out of time.
        TimedOut = "timed_out",
    }
}

/// One step of a [`WorkflowJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// The step's name, such as `Run cargo build`.
    pub name: String,
    /// Where it is.
    pub status: JobStatus,
    /// How it ended, once it completed.
    #[serde(default)]
    pub conclusion: Option<JobConclusion>,
    /// Its position in the job, from 1.
    pub number: u64,
    /// When it started.
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    /// When it finished.
    #[serde(default)]
    pub completed_at: Option<Timestamp>,
}

/// One job of a [`WorkflowRun`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowJob {
    /// GitHub's numeric id.
    pub id: u64,
    /// The run it belongs to.
    pub run_id: u64,
    /// The run attempt it belongs to, from 1.
    #[serde(default)]
    pub run_attempt: Option<u64>,
    /// The job's name, such as `build`.
    pub name: String,
    /// Where it is.
    pub status: JobStatus,
    /// How it ended, once it completed.
    #[serde(default)]
    pub conclusion: Option<JobConclusion>,
    /// When it started.
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    /// When it finished.
    #[serde(default)]
    pub completed_at: Option<Timestamp>,
    /// Its steps, in order.
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
    /// Its page on GitHub.
    pub html_url: String,
    /// The runner labels it asked for, such as `ubuntu-latest`.
    #[serde(default)]
    pub labels: Vec<String>,
}

impl WorkflowJob {
    /// Whether it completed with a failure: what makes "Re-run Failed Jobs" worth offering.
    pub fn failed(&self) -> bool {
        self.status == JobStatus::Completed && self.conclusion == Some(JobConclusion::Failure)
    }
}
