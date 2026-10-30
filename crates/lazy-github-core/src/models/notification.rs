//! Notifications, from `GET /notifications`, and as the Notifications modal saves its first page to
//! `notifications.json` (see [`crate::cache::NOTIFICATIONS`]).

use std::sync::LazyLock;

use jiff::Timestamp;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};

use super::Repository;

/// One notification thread: Python's `Notification`.
///
/// It serializes as Python's `model_dump(mode="json")` writes it, for `notifications.json`: every field, `null` for
/// none, the id as a number and the subject's kind as `subject_type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// The thread id, which marking it read goes by. GitHub sends it as a JSON string (`"9100001"`); a number, as the
    /// cache file holds it, is accepted too.
    #[serde(deserialize_with = "string_or_number")]
    pub id: u64,
    /// The repository the thread is in, in GitHub's minimal shape (no default branch, archived flag or permissions).
    pub repository: Repository,
    /// What the thread is about.
    pub subject: NotificationSubject,
    /// Why the user was notified, as the API spells it: `review_requested`, `mention`, `ci_activity`, ...
    pub reason: String,
    /// Whether the thread is unread.
    pub unread: bool,
    /// When the thread last changed.
    pub updated_at: Timestamp,
    /// When the user last read it: `None` for one never read.
    pub last_read_at: Option<Timestamp>,
}

/// What a notification is about: Python's `NotificationSubject`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSubject {
    /// The subject's title, verbatim (untrimmed).
    pub title: String,
    /// The subject's API URL: `None` for subjects without one, such as a check suite.
    pub url: Option<String>,
    /// The API URL of the latest comment on it, if any.
    pub latest_comment_url: Option<String>,
    /// The subject kind, as the API spells it: `PullRequest`, `Issue`, `Release`, `CheckSuite`, `Discussion`, ...
    ///
    /// GitHub sends it as `type`; Python's `model_dump` writes it as `subject_type`, which is what the cache file
    /// holds. Both read.
    #[serde(rename(serialize = "subject_type", deserialize = "type"), alias = "subject_type")]
    pub kind: String,
}

/// A pull request API URL: Python's `_PULL_REQUEST_URL_REGEX`, a prefix match of a scheme, slashes and one host
/// segment, then `/repos/<owner>/<repo>/pulls/<digits>`. Any path may come between the host and `/repos/`, so GitHub
/// Enterprise's `/api/v3` matches too (Py bug B83's recommendation). `(?s)` and `[^/]` let a newline through as
/// Python's pattern does; the digits are ASCII only, which is all GitHub sends.
static PULL_REQUEST_URL: LazyLock<Regex> = LazyLock::new(|| {
    // A literal pattern, checked by `pull_request_numbers` in the tests.
    Regex::new(r"(?s)^[^:]+:/+[^/]+(?:/.*)?/repos/[^/]+/[^/]+/pulls/([0-9]+)")
        .expect("a valid pull request URL pattern")
});

impl NotificationSubject {
    /// The number of the pull request this subject is, when its URL is a pull request API URL
    /// (`https://api.github.com/repos/<owner>/<repo>/pulls/5`, or anything after the digits). The URL's owner and repo
    /// aren't returned: Python fetches the number from the notification's repository.
    pub fn pull_request_number(&self) -> Option<u64> {
        pull_request_number(self.url.as_deref()?)
    }
}

/// The pull request number in `url`, if it's a pull request API URL (see [`NotificationSubject::pull_request_number`]).
fn pull_request_number(url: &str) -> Option<u64> {
    PULL_REQUEST_URL.captures(url)?.get(1)?.as_str().parse().ok()
}

/// A `u64` sent as a JSON number or as a string of digits.
fn string_or_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Number(u64),
        String(String),
    }
    match Id::deserialize(deserializer)? {
        Id::Number(id) => Ok(id),
        Id::String(id) => id.parse().map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// The `response.body` of a parity fixture.
    fn fixture_body(set: &str, file: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures")
            .join(set)
            .join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut fixture: Value = serde_json::from_str(&text).unwrap();
        fixture["response"]["body"].take()
    }

    fn timestamp(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn every_modal_notification_decodes() {
        let body = fixture_body(
            "notifications-modal",
            "notifications/GET.all-true_page-1_per_page-50.json",
        );
        let notifications: Vec<Notification> = serde_json::from_value(body).unwrap();
        let summary: Vec<(u64, &str, bool)> = notifications
            .iter()
            .map(|n| (n.id, n.subject.kind.as_str(), n.unread))
            .collect();
        assert_eq!(
            summary,
            [
                (9_100_001, "PullRequest", true),
                (9_100_002, "Issue", true),
                (9_100_003, "Release", false),
                (9_100_004, "CheckSuite", true),
                (9_100_005, "PullRequest", false),
                (9_100_006, "PullRequest", true),
            ]
        );

        let first = &notifications[0];
        assert_eq!(first.reason, "review_requested");
        assert_eq!(first.updated_at, timestamp("2026-10-24T23:00:00Z"));
        assert_eq!(first.last_read_at, None);
        assert_eq!(first.subject.title, "Add the notifications modal");
        assert_eq!(
            first.subject.url.as_deref(),
            Some("https://api.github.com/repos/parity-bot/pr-details/pulls/5")
        );
        // GitHub's minimal repository: no default branch, archived flag or permissions.
        assert_eq!(first.repository.full_name, "parity-bot/pr-details");
        assert_eq!(first.repository.owner.login, "parity-bot");
        assert_eq!(first.repository.default_branch, None);
        assert_eq!(first.repository.archived, None);
        assert_eq!(first.repository.permissions, None);

        // Titles are kept verbatim.
        assert_eq!(notifications[1].subject.title, "  Typo in the install docs  ");
        assert_eq!(notifications[2].last_read_at, Some(timestamp("2026-10-24T22:40:00Z")));
        let check_suite = &notifications[3].subject;
        assert_eq!(
            (check_suite.url.as_deref(), check_suite.latest_comment_url.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn every_unread_notification_of_notifications_3_decodes() {
        let body = fixture_body("notifications-3", "notifications/GET.all-false_page-1_per_page-50.json");
        let notifications: Vec<Notification> = serde_json::from_value(body).unwrap();
        let ids: Vec<u64> = notifications.iter().map(|n| n.id).collect();
        assert_eq!(ids, [9_000_001, 9_000_002, 9_000_003]);
        assert!(notifications.iter().all(|n| n.unread && n.last_read_at.is_none()));
        let repos: Vec<&str> = notifications.iter().map(|n| n.repository.full_name.as_str()).collect();
        assert_eq!(
            repos,
            ["parity-bot/hello-world", "parity-bot/dotfiles", "parity-bot/lazy-demo"]
        );
    }

    fn notification(id: &Value) -> Value {
        json!({
            "id": id,
            "unread": true,
            "reason": "mention",
            "updated_at": "2026-10-24T22:00:00Z",
            "last_read_at": null,
            "subject": {"title": "t", "url": null, "latest_comment_url": null, "type": "Discussion"},
            "repository": {
                "name": "r",
                "full_name": "o/r",
                "private": true,
                "owner": {"login": "o", "id": 1, "html_url": "https://github.com/o"},
            },
        })
    }

    #[rstest::rstest]
    #[case::github_com("https://api.github.com/repos/parity-bot/pr-details/pulls/5", Some(5))]
    #[case::trailing_path("https://api.github.com/repos/o/r/pulls/5/files", Some(5))]
    #[case::trailing_non_digits("https://api.github.com/repos/o/r/pulls/5abc", Some(5))]
    #[case::many_slashes("https:///api.github.com/repos/o/r/pulls/12", Some(12))]
    #[case::ghe("https://ghe.example.com/api/v3/repos/o/r/pulls/7", Some(7))]
    #[case::issue("https://api.github.com/repos/o/r/issues/5", None)]
    #[case::no_digits("https://api.github.com/repos/o/r/pulls/", None)]
    #[case::not_a_number("https://api.github.com/repos/o/r/pulls/x", None)]
    #[case::empty("", None)]
    #[case::no_scheme("api.github.com/repos/o/r/pulls/5", None)]
    #[case::no_host("https:///repos/o/r/pulls/5", None)]
    #[case::empty_owner("https://api.github.com/repos//r/pulls/5", None)]
    // Python's scheme is anything up to the first colon.
    #[case::loose_scheme("see https://api.github.com/repos/o/r/pulls/5", Some(5))]
    #[case::port("https://ghe.example.com:8443/repos/o/r/pulls/3", Some(3))]
    #[case::too_big("https://api.github.com/repos/o/r/pulls/99999999999999999999999", None)]
    fn pull_request_numbers(#[case] url: &str, #[case] number: Option<u64>) {
        assert_eq!(pull_request_number(url), number);
    }

    #[test]
    fn a_subject_without_a_url_is_no_pull_request() {
        let mut subject: Notification = serde_json::from_value(notification(&json!(1))).unwrap();
        assert_eq!(subject.subject.pull_request_number(), None);
        // The owner and repo in the URL aren't the caller's: only the number comes back.
        subject.subject.url = Some("https://api.github.com/repos/someone/else/pulls/9".to_owned());
        assert_eq!(subject.subject.pull_request_number(), Some(9));
    }

    #[test]
    fn a_string_or_a_numeric_id_decodes() {
        for id in [json!("42"), json!(42)] {
            let parsed: Notification = serde_json::from_value(notification(&id)).unwrap();
            assert_eq!(parsed.id, 42);
            assert_eq!(parsed.subject.kind, "Discussion");
        }
    }

    #[test]
    fn the_subject_kind_reads_as_type_or_subject_type() {
        let mut cached = notification(&json!(7));
        let subject = cached["subject"].as_object_mut().unwrap();
        let kind = subject.remove("type").unwrap();
        subject.insert("subject_type".into(), kind);
        let parsed: Notification = serde_json::from_value(cached).unwrap();
        assert_eq!(parsed.subject.kind, "Discussion");
        let parsed: Notification = serde_json::from_value(notification(&json!(7))).unwrap();
        assert_eq!(parsed.subject.kind, "Discussion");
    }

    #[test]
    fn it_serializes_in_pythons_dump_shape() {
        let parsed: Notification = serde_json::from_value(notification(&json!("42"))).unwrap();
        let dumped = serde_json::to_value(&parsed).unwrap();
        assert_eq!(dumped["id"], json!(42));
        assert_eq!(dumped["last_read_at"], Value::Null);
        assert_eq!(dumped["updated_at"], json!("2026-10-24T22:00:00Z"));
        assert_eq!(
            dumped["subject"],
            json!({"title": "t", "url": null, "latest_comment_url": null, "subject_type": "Discussion"})
        );
        // Every repository field, `null` for the ones GitHub's minimal shape leaves out.
        assert_eq!(dumped["repository"]["default_branch"], Value::Null);
        assert_eq!(dumped["repository"]["permissions"], Value::Null);
        let back: Notification = serde_json::from_value(dumped).unwrap();
        assert_eq!(back, parsed);
    }

    #[test]
    fn one_bad_thread_makes_the_whole_file_none_with_the_schema_warning() {
        let path = std::path::Path::new("notifications.json");
        let good: Notification = serde_json::from_value(notification(&json!(1))).unwrap();
        let good = serde_json::to_value(good).unwrap();
        let mut bad = good.clone();
        bad["unread"] = json!("yes");
        let text = Value::Array(vec![good, bad]).to_string();
        let error = crate::cache::try_parse::<Notification>(path, &text, "Notification").unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("doesn't match the expected schema for Notification")
                && message.contains("item 1: unread"),
            "{message}"
        );
        assert_eq!(crate::cache::parse::<Notification>(path, &text, "Notification"), []);
    }

    #[test]
    fn a_bad_id_fails() {
        for id in [json!("x"), json!("-1"), json!(-1), json!(null), json!(1.5)] {
            assert!(
                serde_json::from_value::<Notification>(notification(&id)).is_err(),
                "{id}"
            );
        }
    }
}
