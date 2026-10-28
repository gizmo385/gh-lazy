//! Notifications, from `GET /notifications`.

use jiff::Timestamp;
use serde::{Deserialize, Deserializer};

use super::Repository;

/// One notification thread: Python's `Notification`.
///
/// Not `Serialize`: notifications are never cached (Python's are, Py bug B29).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Notification {
    /// The thread id, which marking it read goes by. GitHub sends it as a JSON string (`"9100001"`); a number is
    /// accepted too.
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NotificationSubject {
    /// The subject's title, verbatim (untrimmed).
    pub title: String,
    /// The subject's API URL: `None` for subjects without one, such as a check suite.
    pub url: Option<String>,
    /// The API URL of the latest comment on it, if any.
    pub latest_comment_url: Option<String>,
    /// The subject kind, as the API spells it: `PullRequest`, `Issue`, `Release`, `CheckSuite`, `Discussion`, ...
    #[serde(rename = "type")]
    pub kind: String,
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

    #[test]
    fn a_string_or_a_numeric_id_decodes() {
        for id in [json!("42"), json!(42)] {
            let parsed: Notification = serde_json::from_value(notification(&id)).unwrap();
            assert_eq!(parsed.id, 42);
            assert_eq!(parsed.subject.kind, "Discussion");
        }
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
