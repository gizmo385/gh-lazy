//! Pull requests as the pull request endpoint returns them.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{PartialPullRequest, Repository, User};

/// A pull request from `GET /repos/{owner}/{repo}/pulls/{number}`: the fields the issues endpoint lists, plus the
/// branch and diff details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullPullRequest {
    /// The fields the issues endpoint also lists, serialized inline.
    #[serde(flatten)]
    pub pr: PartialPullRequest,
    /// Lines added.
    pub additions: u64,
    /// Lines deleted.
    pub deletions: u64,
    /// Files changed.
    pub changed_files: u64,
    /// Commits on the branch.
    pub commits: u64,
    /// The branch the changes come from.
    pub head: GitRef,
    /// The branch they merge into.
    pub base: GitRef,
    /// When it was merged, if it was.
    #[serde(default)]
    pub merged_at: Option<Timestamp>,
    /// The `.diff` page of the pull request on GitHub.
    pub diff_url: String,
}

/// One end of a pull request: a branch in someone's repository, at a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRef {
    /// The owner of the repository the branch is in.
    pub user: User,
    /// The branch name, the API's `ref`.
    #[serde(rename = "ref")]
    pub name: String,
    /// The commit the branch points at.
    pub sha: String,
}

impl FullPullRequest {
    /// Decodes a pull request endpoint body fetched from `repo`, which is recorded in it as the issues endpoint's
    /// items record theirs.
    ///
    /// # Errors
    ///
    /// When the body isn't an object with the fields of a [`FullPullRequest`].
    pub fn from_api(body: Value, repo: &Repository) -> Result<Self, serde_json::Error> {
        let mut body = body;
        if let Value::Object(fields) = &mut body {
            fields.insert("repo".to_owned(), serde_json::to_value(repo)?);
        }
        serde_json::from_value(body)
    }

    /// The number within the repository, as in `#5`.
    pub fn number(&self) -> u64 {
        self.pr.issue.number
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `pr-details` fixture of pull request `number`, as the API returned it.
    fn fixture_body(number: u64) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls")
            .join(number.to_string())
            .join("GET.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fixture: Value = serde_json::from_str(&text).unwrap();
        fixture["response"]["body"].clone()
    }

    fn repo() -> Repository {
        serde_json::from_value(serde_json::json!({
            "name": "pr-details",
            "full_name": "parity-bot/pr-details",
            "private": false,
            "owner": {"login": "parity-bot", "id": 1_000_001, "html_url": "https://github.com/parity-bot"}
        }))
        .unwrap()
    }

    #[test]
    fn an_open_pull_request_decodes() {
        let pr = FullPullRequest::from_api(fixture_body(5), &repo()).unwrap();
        assert_eq!(pr.number(), 5);
        assert_eq!(pr.pr.issue.title, "Add a cursor to the viewer");
        assert_eq!(pr.pr.issue.repo, repo());
        assert_eq!(pr.pr.issue.user.login, "parity-friend");
        assert!(!pr.pr.draft);
        assert_eq!(pr.pr.issue.closed_at, None);
        assert_eq!(pr.merged_at, None);
        assert_eq!(
            (pr.commits, pr.changed_files, pr.additions, pr.deletions),
            (3, 5, 12, 6)
        );
        assert_eq!(pr.head.user.login, "parity-friend");
        assert_eq!(pr.head.name, "cursor");
        assert_eq!(pr.head.sha, "c0ffee5555555555555555555555555555555555");
        assert_eq!(
            (pr.base.user.login.as_str(), pr.base.name.as_str()),
            ("parity-bot", "main")
        );
        assert_eq!(pr.diff_url, "https://github.com/parity-bot/pr-details/pull/5.diff");
        assert!(pr.pr.issue.body.as_deref().unwrap().starts_with("Adds a cursor"));
    }

    #[test]
    fn closed_and_merged_pull_requests_decode() {
        let closed = FullPullRequest::from_api(fixture_body(4), &repo()).unwrap();
        assert!(closed.pr.issue.closed_at.is_some());
        assert_eq!(closed.merged_at, None);
        let merged = FullPullRequest::from_api(fixture_body(3), &repo()).unwrap();
        assert_eq!(merged.merged_at.unwrap().to_string(), "2026-01-12T16:20:00Z");
        let other = FullPullRequest::from_api(fixture_body(9), &repo()).unwrap();
        assert_eq!(other.number(), 9);
    }

    #[test]
    fn a_missing_field_fails_and_the_shape_round_trips() {
        let mut body = fixture_body(5);
        body.as_object_mut().unwrap().remove("head");
        assert!(FullPullRequest::from_api(body, &repo()).is_err());
        assert!(FullPullRequest::from_api(Value::from(5), &repo()).is_err());

        let pr = FullPullRequest::from_api(fixture_body(3), &repo()).unwrap();
        let json = serde_json::to_value(&pr).unwrap();
        assert_eq!(json["head"]["ref"], "release-0.2.0");
        assert_eq!(json["number"], 3);
        assert_eq!(json["draft"], false);
        let back: FullPullRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, pr);
    }
}
