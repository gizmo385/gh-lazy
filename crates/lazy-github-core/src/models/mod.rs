//! Typed GitHub REST API models.
//!
//! Each model reads only the fields LazyGithub uses: unknown fields are ignored, and optional fields default to
//! `None` when missing. Serializing writes every field (`None` as `null`), the shape the Python app's pydantic models
//! produce, so local table caches stay readable by both apps.

mod issue;
mod repo_name;
mod repository;
mod user;

pub use issue::{Issue, IssueListItem, IssueState, PartialPullRequest};
pub use repo_name::RepoName;
pub use repository::{Repository, RepositoryPermission};
pub use user::User;
