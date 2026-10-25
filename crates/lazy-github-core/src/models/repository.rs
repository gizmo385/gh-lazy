//! GitHub repositories.

use serde::{Deserialize, Serialize};

use super::User;

/// What the authenticated user may do in a repository.
// GitHub's wire shape: five independent flags, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryPermission {
    /// Full administrative access.
    pub admin: bool,
    /// Manage the repository without access to sensitive or destructive actions.
    pub maintain: bool,
    /// Push to the repository.
    pub push: bool,
    /// Manage issues and pull requests without write access.
    pub triage: bool,
    /// Read and clone the repository.
    pub pull: bool,
}

/// A GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    /// The repository name, such as `gh-lazy`.
    pub name: String,
    /// `owner/name`, such as `gizmo385/gh-lazy`.
    pub full_name: String,
    /// The default branch, such as `main`.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// Whether the repository is private.
    pub private: bool,
    /// Whether the repository is archived.
    #[serde(default)]
    pub archived: Option<bool>,
    /// The owning user or organization.
    pub owner: User,
    /// The repository description.
    #[serde(default)]
    pub description: Option<String>,
    /// The authenticated user's permissions. GitHub only includes them for authenticated requests.
    #[serde(default)]
    pub permissions: Option<RepositoryPermission>,
}
