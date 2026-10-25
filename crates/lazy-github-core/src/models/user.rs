//! GitHub users.

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

/// A GitHub user or organization, as embedded in most API responses.
///
/// Two users are equal when their `id`s are, whatever their other fields say, so a user fetched at different times
/// (with a changed `name`, say) still compares equal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// The login, such as `octocat`.
    pub login: String,
    /// GitHub's numeric user id.
    pub id: u64,
    /// The display name, when the user has set one.
    #[serde(default)]
    pub name: Option<String>,
    /// The user's profile page.
    pub html_url: String,
}

impl PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for User {}

impl Hash for User {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn user(login: &str, id: u64) -> User {
        User {
            login: login.into(),
            id,
            name: None,
            html_url: format!("https://github.com/{login}"),
        }
    }

    #[test]
    fn equality_and_hash_go_by_id() {
        let renamed = User {
            name: Some("New Name".into()),
            ..user("old-login", 1)
        };
        assert_eq!(user("octocat", 1), renamed);
        assert_ne!(user("octocat", 1), user("octocat", 2));
        let set: HashSet<User> = [user("a", 1), renamed, user("b", 2)].into_iter().collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn missing_name_is_none_and_serializes_as_null() {
        let parsed: User =
            serde_json::from_str(r#"{"login":"a","id":7,"html_url":"https://github.com/a","site_admin":false}"#)
                .unwrap();
        assert_eq!(parsed.name, None);
        assert_eq!(
            serde_json::to_string(&parsed).unwrap(),
            r#"{"login":"a","id":7,"name":null,"html_url":"https://github.com/a"}"#
        );
    }
}
