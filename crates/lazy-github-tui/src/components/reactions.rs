//! The reactions on an issue, a pull request or a comment as a section shows them: Python's `ReactionsDisplay`. The
//! title counts each kind (`👍 2 | 🎉 1`) and the rows name who reacted (`👍: a, b`).

use lazy_github_core::models::{Reactions, User};

/// How many users a row names before `and N more`.
const NAMED_USERS: usize = 3;

/// A reactions section's title and rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionsSummary {
    /// `<emoji> <count>` per kind, in first-seen order, joined by ` | `.
    pub title: String,
    /// `<emoji>: <users>` per kind, in the same order.
    pub rows: Vec<String>,
}

impl ReactionsSummary {
    /// The summary of `reactions`, or `None` when there are none: the section isn't shown then.
    pub fn new(reactions: &Reactions) -> Option<Self> {
        if reactions.is_empty() {
            return None;
        }
        let title = reactions
            .by_kind()
            .map(|(kind, users)| format!("{} {}", kind.emoji(), users.len()))
            .collect::<Vec<_>>()
            .join(" | ");
        let rows = reactions
            .by_kind()
            .map(|(kind, users)| format!("{}: {}", kind.emoji(), users_text(users)))
            .collect();
        Some(Self { title, rows })
    }
}

/// The logins joined by `, `, or with more than three, the first three then `, and N more`.
fn users_text(users: &[User]) -> String {
    let logins: Vec<&str> = users.iter().take(NAMED_USERS).map(|u| u.login.as_str()).collect();
    let text = logins.join(", ");
    match users.len().checked_sub(NAMED_USERS) {
        Some(more) if more > 0 => format!("{text}, and {more} more"),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use lazy_github_core::models::{Reaction, ReactionKind};

    use super::*;

    fn user(login: &str, id: u64) -> User {
        User {
            login: login.into(),
            id,
            name: None,
            html_url: String::new(),
        }
    }

    fn reactions(list: &[(ReactionKind, &str, u64)]) -> Reactions {
        list.iter()
            .map(|&(kind, login, id)| Reaction {
                kind,
                user: user(login, id),
            })
            .collect()
    }

    #[test]
    fn none_shows_no_section() {
        assert_eq!(ReactionsSummary::new(&Reactions::default()), None);
    }

    #[test]
    fn counts_and_users_per_kind_in_first_seen_order() {
        use ReactionKind::{Eyes, Hooray, ThumbsUp};
        let summary = ReactionsSummary::new(&reactions(&[
            (ThumbsUp, "a", 1),
            (Hooray, "a", 1),
            (ThumbsUp, "b", 2),
            (Eyes, "a", 1),
            (Eyes, "b", 2),
            (Eyes, "c", 3),
        ]))
        .unwrap();
        assert_eq!(summary.title, "👍 2 | 🎉 1 | 👀 3");
        assert_eq!(summary.rows, ["👍: a, b", "🎉: a", "👀: a, b, c"]);
    }

    #[test]
    fn more_than_three_users_end_with_and_n_more() {
        let users: Vec<User> = (1..=5).map(|i| user(&format!("u{i}"), i)).collect();
        assert_eq!(users_text(&users[..3]), "u1, u2, u3");
        assert_eq!(users_text(&users[..4]), "u1, u2, u3, and 1 more");
        assert_eq!(users_text(&users), "u1, u2, u3, and 2 more");
        assert_eq!(users_text(&users[..1]), "u1");
    }
}
