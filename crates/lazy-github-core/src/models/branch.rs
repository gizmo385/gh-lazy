//! Branches, from `GET /repos/{owner}/{repo}/branches`.

use serde::{Deserialize, Serialize};

/// A branch of a repository: Python's `Branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// The branch name, such as `main`.
    pub name: String,
    /// The commit it points at.
    pub commit: CommitRef,
    /// Whether branch protection is on.
    #[serde(default)]
    pub protected: bool,
}

/// A commit as a branch points at it: Python's `Commit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRef {
    /// The commit's SHA.
    pub sha: String,
    /// Its API URL.
    #[serde(default)]
    pub url: String,
}
