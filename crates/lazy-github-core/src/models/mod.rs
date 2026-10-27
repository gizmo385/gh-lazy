//! Typed GitHub REST API models.
//!
//! Each model reads only the fields LazyGithub uses: unknown fields are ignored, and optional fields default to
//! `None` when missing. Serializing writes every field (`None` as `null`), the shape the Python app's pydantic models
//! produce, so local table caches stay readable by both apps.

mod checks;
mod comment;
mod issue;
mod pull_request;
mod reaction;
mod repo_name;
mod repository;
mod review;
mod user;
mod workflow;

pub use checks::{
    CheckConclusion, CheckRow, CheckRun, CheckRunStatus, CheckState, CombinedStatus, CommitStatus, check_rows,
    overall_state,
};
pub use comment::{CommentRef, IssueComment};
pub use issue::{Issue, IssueListItem, IssueState, PartialPullRequest};
pub use pull_request::{FullPullRequest, GitRef};
pub use reaction::{Reaction, ReactionKind, Reactions};
pub use repo_name::RepoName;
pub use repository::{Repository, RepositoryPermission};
pub use review::{
    Review, ReviewComment, ReviewState, ReviewSummary, ReviewVerdict, Thread, summary as review_summary,
    threads as review_threads,
};
pub use user::{FullUser, User};
pub use workflow::{JobConclusion, JobStatus, Workflow, WorkflowJob, WorkflowRun, WorkflowState, WorkflowStep};
