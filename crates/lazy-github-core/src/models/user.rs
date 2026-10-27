//! GitHub users.

use std::hash::{Hash, Hasher};

use jiff::Timestamp;
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

/// A user's or organization's whole profile, as `GET /users/{login}` returns it.
///
/// Every field past [`User`]'s is optional in practice: an organization has no `twitter_username`, a user may leave
/// any of them blank, and GitHub sends `null` or `""` for what isn't set. Counts default to 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullUser {
    /// The fields every user object has.
    #[serde(flatten)]
    pub user: User,
    /// The profile's bio, as markdown.
    #[serde(default)]
    pub bio: Option<String>,
    /// The company, such as `@github`.
    #[serde(default)]
    pub company: Option<String>,
    /// Where the user is.
    #[serde(default)]
    pub location: Option<String>,
    /// The blog or website URL.
    #[serde(default)]
    pub blog: Option<String>,
    /// The public email address.
    #[serde(default)]
    pub email: Option<String>,
    /// The Twitter handle, without the `@`.
    #[serde(default)]
    pub twitter_username: Option<String>,
    /// How many public repositories the user owns.
    #[serde(default)]
    pub public_repos: u64,
    /// How many public gists.
    #[serde(default)]
    pub public_gists: u64,
    /// How many followers.
    #[serde(default)]
    pub followers: u64,
    /// How many users this one follows.
    #[serde(default)]
    pub following: u64,
    /// When the account was created.
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    /// GitHub's account type, such as `User` or `Organization`.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
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

    /// The `body` of a parity fixture.
    fn parity_body(set: &str, file: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures")
            .join(set)
            .join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
        fixture["response"]["body"].clone()
    }

    #[test]
    fn a_full_profile_decodes_every_field() {
        let user: FullUser = serde_json::from_value(parity_body("user-profile", "users/parity-bot/GET.json")).unwrap();
        assert_eq!(user.user.login, "parity-bot");
        assert_eq!(user.user.id, 1_000_001);
        assert_eq!(user.user.name.as_deref(), Some("Parity Bot"));
        assert_eq!(user.user.html_url, "https://github.com/parity-bot");
        assert_eq!(
            user.bio.as_deref(),
            Some("Fictional account for **LazyGithub** parity fixtures.\n\nKeeps `dotfiles` and a few demos.")
        );
        assert_eq!(user.company.as_deref(), Some("@parity-org"));
        assert_eq!(user.location.as_deref(), Some("Testville"));
        assert_eq!(user.blog.as_deref(), Some("https://example.com/parity-bot"));
        assert_eq!(user.email.as_deref(), Some("user@example.com"));
        assert_eq!(user.twitter_username.as_deref(), Some("paritybot"));
        assert_eq!(
            (user.public_repos, user.public_gists, user.followers, user.following),
            (4, 2, 12, 3)
        );
        assert_eq!(
            user.created_at.map(|at| at.to_string()).as_deref(),
            Some("2024-01-02T03:04:05Z")
        );
        assert_eq!(user.kind.as_deref(), Some("User"));
    }

    #[test]
    fn an_organization_with_most_fields_null_decodes() {
        let org: FullUser = serde_json::from_value(parity_body("user-profile", "users/parity-org/GET.json")).unwrap();
        assert_eq!(org.user.login, "parity-org");
        assert_eq!(org.user.name, None);
        assert_eq!(org.bio, None);
        assert_eq!(org.company, None);
        assert_eq!(org.blog.as_deref(), Some(""));
        assert_eq!(org.location.as_deref(), Some("Remote"));
        assert_eq!((org.email, org.twitter_username), (None, None));
        assert_eq!((org.public_repos, org.followers), (0, 1));
        assert_eq!(org.kind.as_deref(), Some("Organization"));
    }

    #[test]
    fn the_base_user_fixture_decodes_with_defaults() {
        let user: FullUser = serde_json::from_value(parity_body("base", "user/GET.json")).unwrap();
        assert_eq!(user.user.login, "parity-bot");
        // Missing counts are 0, and a missing date or type is `None`.
        let minimal: FullUser =
            serde_json::from_str(r#"{"login":"a","id":7,"html_url":"https://github.com/a"}"#).unwrap();
        assert_eq!(
            (
                minimal.public_repos,
                minimal.public_gists,
                minimal.followers,
                minimal.following
            ),
            (0, 0, 0, 0)
        );
        assert_eq!((minimal.created_at, minimal.kind), (None, None));
    }
}
