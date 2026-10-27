//! Emoji reactions on an issue, a pull request or a comment.

use serde::Deserialize;

use super::User;

/// The kinds of reaction GitHub has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReactionKind {
    /// 👍, `+1` in the API.
    ThumbsUp,
    /// 👎, `-1`.
    ThumbsDown,
    /// 😄, `laugh`.
    Laugh,
    /// 😕, `confused`.
    Confused,
    /// ❤️, `heart`.
    Heart,
    /// 🎉, `hooray`.
    Hooray,
    /// 🚀, `rocket`.
    Rocket,
    /// 👀, `eyes`.
    Eyes,
}

impl ReactionKind {
    /// Every kind, in Python's `ReactionType` order.
    pub const ALL: [ReactionKind; 8] = [
        ReactionKind::ThumbsUp,
        ReactionKind::ThumbsDown,
        ReactionKind::Laugh,
        ReactionKind::Confused,
        ReactionKind::Heart,
        ReactionKind::Hooray,
        ReactionKind::Rocket,
        ReactionKind::Eyes,
    ];

    /// The kind an API `content` string names, or `None` for one GitHub added since.
    pub fn from_api(content: &str) -> Option<ReactionKind> {
        ReactionKind::ALL.into_iter().find(|kind| kind.api_name() == content)
    }

    /// The API's `content` string: `+1`, `-1`, `laugh`, ...
    pub fn api_name(self) -> &'static str {
        match self {
            ReactionKind::ThumbsUp => "+1",
            ReactionKind::ThumbsDown => "-1",
            ReactionKind::Laugh => "laugh",
            ReactionKind::Confused => "confused",
            ReactionKind::Heart => "heart",
            ReactionKind::Hooray => "hooray",
            ReactionKind::Rocket => "rocket",
            ReactionKind::Eyes => "eyes",
        }
    }

    /// The emoji it shows as.
    pub fn emoji(self) -> &'static str {
        match self {
            ReactionKind::ThumbsUp => "👍",
            ReactionKind::ThumbsDown => "👎",
            ReactionKind::Laugh => "😄",
            ReactionKind::Confused => "😕",
            ReactionKind::Heart => "❤️",
            ReactionKind::Hooray => "🎉",
            ReactionKind::Rocket => "🚀",
            ReactionKind::Eyes => "👀",
        }
    }
}

/// One user's reaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    /// Which emoji.
    pub kind: ReactionKind,
    /// Who reacted.
    pub user: User,
}

/// The reactions on one item, grouped by kind: the kinds in the order they first appear, each with its users in the
/// order they reacted, each user once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reactions {
    by_kind: Vec<(ReactionKind, Vec<User>)>,
}

impl Reactions {
    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }

    /// Each kind with its users, in first-seen order. No kind has an empty list.
    pub fn by_kind(&self) -> impl Iterator<Item = (ReactionKind, &[User])> {
        self.by_kind.iter().map(|(kind, users)| (*kind, users.as_slice()))
    }

    /// Adds `user`'s reaction of `kind`, unless they already have one.
    pub fn add(&mut self, kind: ReactionKind, user: User) {
        match self.by_kind.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, users)) => {
                if !users.contains(&user) {
                    users.push(user);
                }
            }
            None => self.by_kind.push((kind, vec![user])),
        }
    }

    /// Decodes a reactions endpoint body: a list of reactions. One with a `content` this build doesn't know is skipped
    /// with a debug line (Python raises, failing the whole list).
    ///
    /// # Errors
    ///
    /// When the body isn't a list of objects with a `content` string and a `user`.
    pub fn from_api(body: serde_json::Value) -> Result<Reactions, serde_json::Error> {
        let raw: Vec<RawReaction> = serde_json::from_value(body)?;
        Ok(raw
            .into_iter()
            .filter_map(|raw| {
                let kind = ReactionKind::from_api(&raw.content);
                if kind.is_none() {
                    tracing::debug!("Skipping a reaction of unknown kind {:?}", raw.content);
                }
                Some(Reaction {
                    kind: kind?,
                    user: raw.user,
                })
            })
            .collect())
    }
}

impl FromIterator<Reaction> for Reactions {
    fn from_iter<I: IntoIterator<Item = Reaction>>(iter: I) -> Self {
        let mut reactions = Reactions::default();
        for reaction in iter {
            reactions.add(reaction.kind, reaction.user);
        }
        reactions
    }
}

/// One reaction as the API lists it.
#[derive(Debug, Deserialize)]
struct RawReaction {
    content: String,
    user: User,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn user(login: &str, id: u64) -> User {
        User {
            login: login.into(),
            id,
            name: None,
            html_url: format!("https://github.com/{login}"),
        }
    }

    fn raw(content: &str, login: &str, id: u64) -> serde_json::Value {
        json!({"content": content, "user": {"login": login, "id": id, "html_url": "https://github.com/x"}})
    }

    #[test]
    fn api_names_round_trip() {
        for kind in ReactionKind::ALL {
            assert_eq!(ReactionKind::from_api(kind.api_name()), Some(kind));
        }
        assert_eq!(ReactionKind::from_api("thumbs_up"), None);
        assert_eq!(ReactionKind::ThumbsUp.emoji(), "👍");
        assert_eq!(ReactionKind::Heart.emoji(), "❤️");
    }

    #[test]
    fn kinds_keep_first_seen_order_and_users_are_deduped() {
        let reactions: Reactions = [
            (ReactionKind::Eyes, user("a", 1)),
            (ReactionKind::ThumbsUp, user("b", 2)),
            (ReactionKind::Eyes, user("c", 3)),
            (ReactionKind::Eyes, user("a-renamed", 1)),
            (ReactionKind::ThumbsUp, user("b", 2)),
        ]
        .into_iter()
        .map(|(kind, user)| Reaction { kind, user })
        .collect();
        let got: Vec<(ReactionKind, Vec<&str>)> = reactions
            .by_kind()
            .map(|(kind, users)| (kind, users.iter().map(|u| u.login.as_str()).collect()))
            .collect();
        assert_eq!(
            got,
            [
                (ReactionKind::Eyes, vec!["a", "c"]),
                (ReactionKind::ThumbsUp, vec!["b"]),
            ]
        );
        assert!(!reactions.is_empty());
        assert!(Reactions::default().is_empty());
    }

    #[test]
    fn an_unknown_kind_is_skipped() {
        let body = json!([raw("+1", "a", 1), raw("party", "b", 2), raw("-1", "c", 3)]);
        let reactions = Reactions::from_api(body).unwrap();
        let kinds: Vec<ReactionKind> = reactions.by_kind().map(|(kind, _)| kind).collect();
        assert_eq!(kinds, [ReactionKind::ThumbsUp, ReactionKind::ThumbsDown]);
        assert!(Reactions::from_api(json!({"message": "x"})).is_err());
        assert!(Reactions::from_api(json!([{"content": "+1"}])).is_err());
    }
}
