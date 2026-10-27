//! Comments on issues and pull requests.

use jiff::Timestamp;
use serde::{Deserialize, Deserializer};

use super::User;

/// A comment in an issue's or pull request's conversation, as `GET .../issues/{number}/comments` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssueComment {
    /// GitHub's numeric id, which its reactions are asked for by.
    pub id: u64,
    /// The markdown body: `""` when GitHub sends `null` or leaves it out.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub body: String,
    /// Who wrote it: `None` when the account is gone (GitHub's ghost user aside, the API sends `null`).
    #[serde(default)]
    pub user: Option<User>,
    /// When it was written.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// The author's relationship to the repository, as the API spells it: `OWNER`, `CONTRIBUTOR`, `NONE`, ... Kept
    /// as a string, so a value GitHub adds later doesn't fail the whole list (Python's enum would).
    #[serde(default)]
    pub author_association: String,
}

/// Which comment [`crate::github::Github::comment_reactions`] asks about. Issue comments and pull request review
/// comments have separate id spaces and endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentRef {
    /// An issue comment (`/issues/comments/{id}`), including the top-level comments of a pull request.
    Issue(u64),
    /// A pull request review comment (`/pulls/comments/{id}`), the inline kind in a review thread.
    Review(u64),
}

impl CommentRef {
    /// The comment's id.
    pub fn id(self) -> u64 {
        match self {
            CommentRef::Issue(id) | CommentRef::Review(id) => id,
        }
    }
}

fn null_as_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn comment(body: &serde_json::Value, user: &serde_json::Value) -> serde_json::Value {
        json!({
            "id": 7,
            "body": body,
            "user": user,
            "created_at": "2026-02-03T12:00:00Z",
            "updated_at": "2026-02-03T12:30:00Z",
            "author_association": "CONTRIBUTOR",
            "node_id": "IC_x",
        })
    }

    #[test]
    fn a_null_body_is_empty_and_a_null_user_is_none() {
        let parsed: IssueComment = serde_json::from_value(comment(&json!(null), &json!(null))).unwrap();
        assert_eq!(parsed.body, "");
        assert_eq!(parsed.user, None);
        assert_eq!(parsed.author_association, "CONTRIBUTOR");
        assert_eq!(parsed.created_at, "2026-02-03T12:00:00Z".parse::<Timestamp>().unwrap());

        let user = json!({"login": "a", "id": 1, "html_url": "https://github.com/a"});
        let parsed: IssueComment = serde_json::from_value(comment(&json!("hi"), &user)).unwrap();
        assert_eq!(parsed.body, "hi");
        assert_eq!(parsed.user.map(|u| u.login), Some("a".to_owned()));

        let mut missing = comment(&json!("x"), &json!(null));
        missing.as_object_mut().unwrap().remove("body");
        missing.as_object_mut().unwrap().remove("author_association");
        let parsed: IssueComment = serde_json::from_value(missing).unwrap();
        assert_eq!((parsed.body.as_str(), parsed.author_association.as_str()), ("", ""));
    }

    #[test]
    fn bad_comments_fail() {
        let mut no_id = comment(&json!("x"), &json!(null));
        no_id.as_object_mut().unwrap().remove("id");
        assert!(serde_json::from_value::<IssueComment>(no_id).is_err());
        assert!(serde_json::from_value::<IssueComment>(comment(&json!(3), &json!(null))).is_err());
    }

    #[test]
    fn comment_ref_ids() {
        assert_eq!(CommentRef::Issue(3).id(), 3);
        assert_eq!(CommentRef::Review(9).id(), 9);
    }
}
