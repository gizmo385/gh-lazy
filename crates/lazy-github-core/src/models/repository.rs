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
    /// Whether the repository has issues turned on. Read from the API but never written, so the repos cache stays
    /// byte-equal to Python's (which has no such field); a repo read without it, such as a cached one, has issues.
    #[serde(default = "issues_on", skip_serializing)]
    pub has_issues: bool,
}

/// [`Repository::has_issues`] when the JSON doesn't say.
fn issues_on() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn repo_json() -> serde_json::Value {
        json!({
            "name": "lists",
            "full_name": "octo/lists",
            "private": false,
            "owner": {"login": "octo", "id": 1, "html_url": "https://github.com/octo"},
        })
    }

    #[test]
    fn has_issues_is_read_and_defaults_to_true() {
        let repo: Repository = serde_json::from_value(repo_json()).unwrap();
        assert!(repo.has_issues);
        let mut off = repo_json();
        off["has_issues"] = json!(false);
        let repo: Repository = serde_json::from_value(off).unwrap();
        assert!(!repo.has_issues);
    }

    #[test]
    fn has_issues_is_never_written() {
        let mut off = repo_json();
        off["has_issues"] = json!(false);
        let repo: Repository = serde_json::from_value(off).unwrap();
        let written = serde_json::to_value(&repo).unwrap();
        assert!(written.get("has_issues").is_none(), "{written}");
    }
}
