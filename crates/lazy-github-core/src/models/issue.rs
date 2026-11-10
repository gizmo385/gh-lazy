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
    /// The milestone it belongs to, if any. Left out of the serialized form when `None`, so a table cache of issues
    /// without milestones is byte-identical to the Python app's, which has no such field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<Milestone>,
}

/// A milestone, as an issue's `milestone` field carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    /// The number within the repository, as in `/milestone/2`.
    pub number: u64,
    /// The title, as typed.
    pub title: String,
    /// Open or closed (the API spells it as it does an issue's state).
    pub state: IssueState,
    /// When it's due. GitHub stores the chosen day at 07:00 or 08:00 UTC, so its UTC date is the day picked.
    #[serde(default)]
    pub due_on: Option<Timestamp>,
    /// How many open issues and pull requests it holds.
    #[serde(default)]
    pub open_issues: u64,
    /// How many closed issues and pull requests it holds.
    #[serde(default)]
    pub closed_issues: u64,
    /// Its page on GitHub.
    pub html_url: String,
}

impl Milestone {
    /// The search query that lists the issues in this milestone of `repo` (`owner/name`):
    /// `is:issue milestone:"<title>" repo:<repo>`.
    ///
    /// GitHub's search has no escape for `"` inside a quoted value, so double quotes are dropped from the title.
    pub fn search_query(&self, repo: &str) -> String {
        let title: String = self.title.chars().filter(|&c| c != '"').collect();
        format!("is:issue milestone:\"{title}\" repo:{repo}")
    }
}

impl Issue {
    /// Decodes an issue the API sent for `repo`, such as the `201` body of `POST /repos/{owner}/{repo}/issues`,
    /// tagged with `repo`.
    ///
    /// # Errors
    ///
    /// When the body isn't an object with the fields of an [`Issue`].
    pub fn from_api(body: Value, repo: &Repository) -> Result<Self, serde_json::Error> {
        let mut body = body;
        if let Value::Object(fields) = &mut body {
            fields.insert("repo".to_owned(), serde_json::to_value(repo)?);
        }
        serde_json::from_value(body)
    }
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

impl PartialPullRequest {
    /// Decodes a pull request the API sent for `repo`, such as one item of `GET
    /// /repos/{owner}/{repo}/commits/{sha}/pulls`, tagged with `repo`.
    ///
    /// # Errors
    ///
    /// When the body isn't an object with the fields of an [`Issue`] and a boolean `draft`.
    pub fn from_api(body: Value, repo: &Repository) -> Result<Self, serde_json::Error> {
        let mut body = body;
        if let Value::Object(fields) = &mut body {
            fields.insert("repo".to_owned(), serde_json::to_value(repo)?);
        }
        serde_json::from_value(body)
    }
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

    fn milestone(due_on: Option<&str>) -> Value {
        json!({
            "url": "https://api.github.com/repos/parity-bot/lazy-fixture/milestones/2",
            "html_url": "https://github.com/parity-bot/lazy-fixture/milestone/2",
            "id": 9_000_002,
            "number": 2,
            "title": "v0.7",
            "description": "The next release",
            "creator": {"login": "parity-bot", "id": 1_000_001, "html_url": "https://github.com/parity-bot"},
            "state": "open",
            "due_on": due_on,
            "open_issues": 2,
            "closed_issues": 1,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z",
            "closed_at": null,
        })
    }

    #[test]
    fn milestones_decode_with_and_without_a_due_date() {
        let none = IssueListItem::from_api_item(with(item(10), "milestone", Value::Null), &repo()).unwrap();
        assert_eq!(none.issue().milestone, None);
        let absent = IssueListItem::from_api_item(item(10), &repo()).unwrap();
        assert_eq!(absent.issue().milestone, None);

        let due = Some("2026-03-01T08:00:00Z");
        let issue = IssueListItem::from_api_item(with(item(11), "milestone", milestone(due)), &repo()).unwrap();
        let ms = issue.issue().milestone.clone().unwrap();
        assert_eq!(ms.number, 2);
        assert_eq!(ms.title, "v0.7");
        assert_eq!(ms.state, IssueState::Open);
        assert_eq!(ms.due_on, Some("2026-03-01T08:00:00Z".parse::<Timestamp>().unwrap()));
        assert_eq!((ms.open_issues, ms.closed_issues), (2, 1));
        assert_eq!(ms.html_url, "https://github.com/parity-bot/lazy-fixture/milestone/2");

        let mut closed = milestone(None);
        closed["state"] = json!("closed");
        let pr = IssueListItem::from_api_item(
            with(with(item(12), "milestone", closed), "draft", json!(false)),
            &repo(),
        )
        .unwrap();
        let ms = pr.issue().milestone.clone().unwrap();
        assert_eq!(ms.state, IssueState::Closed);
        assert_eq!(ms.due_on, None);
    }

    /// No `milestone` key when there's none (the Python cache's shape); a milestone survives a round trip.
    #[test]
    fn milestones_serialize_only_when_present() {
        let plain = IssueListItem::from_api_item(item(13), &repo())
            .unwrap()
            .into_issue()
            .unwrap();
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("milestone").is_none());

        let due = Some("2026-03-01T08:00:00Z");
        let issue = IssueListItem::from_api_item(with(item(14), "milestone", milestone(due)), &repo())
            .unwrap()
            .into_issue()
            .unwrap();
        let json = serde_json::to_string(&issue).unwrap();
        let back: Issue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, issue);
        assert!(back.milestone.is_some());
    }

    #[test]
    fn milestone_search_query() {
        let mut ms: Milestone = serde_json::from_value(milestone(None)).unwrap();
        assert_eq!(
            ms.search_query("parity-bot/issue-fixture"),
            r#"is:issue milestone:"v0.7" repo:parity-bot/issue-fixture"#
        );
        ms.title = "Rust M5: Parity sign-off".to_owned();
        assert_eq!(
            ms.search_query("gizmo385/gh-lazy"),
            r#"is:issue milestone:"Rust M5: Parity sign-off" repo:gizmo385/gh-lazy"#
        );
        ms.title = r#"The "big" one"#.to_owned();
        assert_eq!(ms.search_query("o/r"), r#"is:issue milestone:"The big one" repo:o/r"#);
    }
}
