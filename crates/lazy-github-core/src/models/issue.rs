//! GitHub issues, and pull requests as the issues endpoint lists them.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Repository, User};

/// Whether an issue or pull request is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    /// Open.
    Open,
    /// Closed (for a pull request, also merged).
    Closed,
}

impl IssueState {
    /// `open` or `closed`, as the API spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        }
    }
}

/// A GitHub issue. Pull requests listed through the issues endpoint have these fields too; see
/// [`PartialPullRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// GitHub's numeric id, unique across repositories.
    pub id: u64,
    /// The number within the repository, as in `#12`.
    pub number: u64,
    /// Whether the conversation is locked.
    pub locked: bool,
    /// Open or closed.
    pub state: IssueState,
    /// The title.
    pub title: String,
    /// The Markdown body, `None` when it's empty.
    #[serde(default)]
    pub body: Option<String>,
    /// Who opened it.
    pub user: User,
    /// When it was opened.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// When it was closed, if it is.
    #[serde(default)]
    pub closed_at: Option<Timestamp>,
    /// Who closed it. The list endpoint doesn't send this; the single-issue endpoint does.
    #[serde(default)]
    pub closed_by: Option<User>,
    /// The first assignee.
    #[serde(default)]
    pub assignee: Option<User>,
    /// Every assignee.
    #[serde(default)]
    pub assignees: Option<Vec<User>>,
    /// The API URL of its comments.
    pub comments_url: String,
    /// Its page on GitHub.
    pub html_url: String,
    /// The repository it was fetched from. The API doesn't send this: the client fills it in, so follow-up requests
    /// know where the issue lives.
    pub repo: Repository,
}

/// A pull request as the issues endpoint lists it: the issue fields plus `draft`, without the branch and diff details
/// of the pull request endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialPullRequest {
    /// The fields it shares with an issue, serialized inline.
    #[serde(flatten)]
    pub issue: Issue,
    /// Whether it's a draft.
    pub draft: bool,
}

/// One item of an issues-endpoint page: an issue or a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueListItem {
    /// An issue.
    Issue(Issue),
    /// A pull request.
    PullRequest(PartialPullRequest),
}

impl IssueListItem {
    /// Decodes one item of a `GET /repos/{owner}/{repo}/issues` page, listed for `repo`.
    ///
    /// An object with a `draft` key, whatever its value, is a pull request; anything else is an issue. That is the
    /// Python app's rule: an item with `pull_request` but no `draft` is an issue.
    ///
    /// # Errors
    ///
    /// When the item isn't an object with the fields of an [`Issue`] (and, for a pull request, a boolean `draft`).
    pub fn from_api_item(item: Value, repo: &Repository) -> Result<Self, serde_json::Error> {
        let is_pull_request = item.get("draft").is_some();
        let mut item = item;
        if let Value::Object(fields) = &mut item {
            fields.insert("repo".to_owned(), serde_json::to_value(repo)?);
        }
        if is_pull_request {
            serde_json::from_value(item).map(Self::PullRequest)
        } else {
            serde_json::from_value(item).map(Self::Issue)
        }
    }

    /// Whether this is a pull request.
    pub fn is_pull_request(&self) -> bool {
        matches!(self, Self::PullRequest(_))
    }

    /// The fields every item has.
    pub fn issue(&self) -> &Issue {
        match self {
            Self::Issue(issue) => issue,
            Self::PullRequest(pull_request) => &pull_request.issue,
        }
    }

    /// The issue, or `None` for a pull request.
    pub fn into_issue(self) -> Option<Issue> {
        match self {
            Self::Issue(issue) => Some(issue),
            Self::PullRequest(_) => None,
        }
    }

    /// The pull request, or `None` for an issue.
    pub fn into_pull_request(self) -> Option<PartialPullRequest> {
        match self {
            Self::Issue(_) => None,
            Self::PullRequest(pull_request) => Some(pull_request),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn repo() -> Repository {
        serde_json::from_value(json!({
            "name": "lazy-fixture",
            "full_name": "parity-bot/lazy-fixture",
            "private": false,
            "owner": {"login": "parity-bot", "id": 1_000_001, "html_url": "https://github.com/parity-bot"}
        }))
        .unwrap()
    }

    fn item(number: u64) -> Value {
        json!({
            "id": 7_000_000 + number,
            "number": number,
            "locked": false,
            "state": "open",
            "title": format!("Item {number}"),
            "body": null,
            "user": {"login": "parity-bot", "id": 1_000_001, "html_url": "https://github.com/parity-bot"},
            "created_at": "2024-01-02T03:04:05Z",
            "updated_at": "2024-01-03T03:04:05Z",
            "closed_at": null,
            "assignee": null,
            "assignees": [],
            "comments_url": format!("https://api.github.com/repos/parity-bot/lazy-fixture/issues/{number}/comments"),
            "html_url": format!("https://github.com/parity-bot/lazy-fixture/issues/{number}"),
            "labels": [],
        })
    }

    fn with(mut value: Value, key: &str, field: Value) -> Value {
        value.as_object_mut().unwrap().insert(key.to_owned(), field);
        value
    }

    #[test]
    fn the_draft_key_decides() {
        let issue = IssueListItem::from_api_item(item(1), &repo()).unwrap();
        assert!(!issue.is_pull_request());
        assert_eq!(issue.issue().repo, repo());
        assert_eq!(issue.issue().number, 1);

        for draft in [true, false] {
            let pr = IssueListItem::from_api_item(with(item(2), "draft", json!(draft)), &repo()).unwrap();
            assert!(pr.is_pull_request());
            let pr = pr.into_pull_request().unwrap();
            assert_eq!(pr.draft, draft);
            assert_eq!(pr.issue.repo, repo());
        }

        // `pull_request` alone doesn't make a pull request, as in Python.
        let link = json!({"url": "https://api.github.com/repos/parity-bot/lazy-fixture/pulls/3"});
        let issue = IssueListItem::from_api_item(with(item(3), "pull_request", link), &repo()).unwrap();
        assert!(issue.clone().into_pull_request().is_none());
        assert_eq!(issue.into_issue().unwrap().number, 3);
    }

    #[test]
    fn bad_items_fail() {
        let mut missing_number = item(4);
        missing_number.as_object_mut().unwrap().remove("number");
        assert!(IssueListItem::from_api_item(missing_number, &repo()).is_err());
        // A `draft` key that isn't a bool fails, as pydantic's `draft: bool` does.
        assert!(IssueListItem::from_api_item(with(item(5), "draft", Value::Null), &repo()).is_err());
        assert!(IssueListItem::from_api_item(with(item(6), "state", json!("merged")), &repo()).is_err());
        assert!(IssueListItem::from_api_item(json!(7), &repo()).is_err());
    }

    #[test]
    fn optional_fields_default_and_timestamps_parse() {
        let mut minimal = item(8);
        let fields = minimal.as_object_mut().unwrap();
        for key in ["body", "closed_at", "assignee", "assignees"] {
            fields.remove(key);
        }
        let issue = IssueListItem::from_api_item(minimal, &repo())
            .unwrap()
            .into_issue()
            .unwrap();
        assert_eq!(issue.body, None);
        assert_eq!(issue.closed_at, None);
        assert_eq!(issue.closed_by, None);
        assert_eq!(issue.assignee, None);
        assert_eq!(issue.assignees, None);
        assert_eq!(issue.created_at, "2024-01-02T03:04:05Z".parse::<Timestamp>().unwrap());
        assert_eq!(issue.state.as_str(), "open");
        assert_eq!(IssueState::Closed.as_str(), "closed");
    }

    /// Every field is written, `None` as `null`, and a pull request's issue fields sit beside `draft`: the shape of
    /// Python's pydantic dump.
    #[test]
    fn serializes_in_python_shape() {
        let pr = IssueListItem::from_api_item(with(item(9), "draft", json!(true)), &repo())
            .unwrap()
            .into_pull_request()
            .unwrap();
        let json = serde_json::to_value(&pr).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "assignee",
                "assignees",
                "body",
                "closed_at",
                "closed_by",
                "comments_url",
                "created_at",
                "draft",
                "html_url",
                "id",
                "locked",
                "number",
                "repo",
                "state",
                "title",
                "updated_at",
                "user"
            ]
        );
        assert_eq!(json["created_at"], "2024-01-02T03:04:05Z");
        assert_eq!(json["closed_by"], Value::Null);
        assert_eq!(json["state"], "open");
        let back: PartialPullRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, pr);
    }
}
