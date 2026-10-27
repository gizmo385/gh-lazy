//! Pull request reviews, their inline comments, the threads those comments form, and the per-author summary the PR
//! Overview shows.

use std::collections::HashMap;

use jiff::Timestamp;
use serde::Deserialize;

use super::{IssueComment, User};

/// Where a review stands, as the API spells it (`APPROVED`, `CHANGES_REQUESTED`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewState {
    /// The reviewer approved the changes.
    Approved,
    /// The reviewer asked for changes.
    ChangesRequested,
    /// The reviewer only commented.
    Commented,
    /// The review was dismissed.
    Dismissed,
    /// The review is a draft its author hasn't submitted.
    Pending,
}

impl ReviewState {
    /// Python's `str.title()` of the state: `Approved`, `Changes_Requested`, `Commented`, `Dismissed`, `Pending`.
    /// Views word approved and changes requested their own way; the others show as this.
    pub fn title(self) -> &'static str {
        match self {
            ReviewState::Approved => "Approved",
            ReviewState::ChangesRequested => "Changes_Requested",
            ReviewState::Commented => "Commented",
            ReviewState::Dismissed => "Dismissed",
            ReviewState::Pending => "Pending",
        }
    }
}

/// An inline comment of a review, as `GET .../pulls/{n}/reviews/{id}/comments` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReviewComment {
    /// The fields an issue comment has too, read inline.
    #[serde(flatten)]
    pub comment: IssueComment,
    /// The review it belongs to.
    pub pull_request_review_id: u64,
    /// The file it's on.
    pub path: String,
    /// Its line in the diff, `None` when the diff has moved on.
    #[serde(default)]
    pub position: Option<u64>,
    /// Its line in the diff it was written on.
    #[serde(default)]
    pub original_position: Option<u64>,
    /// The comment it answers, `None` for the first comment of a thread.
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
}

/// A submitted review of a pull request, with its inline comments.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Review {
    /// GitHub's numeric id, which its comments are asked for by.
    pub id: u64,
    /// Who reviewed.
    pub user: User,
    /// The review's own text, as markdown: `""` when it has none.
    #[serde(default)]
    pub body: String,
    /// Where it stands.
    pub state: ReviewState,
    /// When it was submitted: `None` while pending.
    #[serde(default)]
    pub submitted_at: Option<Timestamp>,
    /// Its inline comments, in API order. The API's review object has none; the client fills them in.
    #[serde(default)]
    pub comments: Vec<ReviewComment>,
}

/// One conversation on a review: a first comment, then every reply to it from any review, depth first.
pub type Thread<'a> = Vec<&'a ReviewComment>;

/// The threads each review starts, by review, in `reviews`' order. A thread starts at each of a review's comments that
/// isn't a reply, and holds every comment that answers it, or answers one of those, from any review: depth first,
/// each comment's replies in the order the comments come in `reviews`. A reply whose parent isn't among the comments
/// is in no thread. Python's `reconstruct_review_conversation_hierarchy` and `ReviewConversation._flatten_comments`.
pub fn threads(reviews: &[Review]) -> Vec<Vec<Thread<'_>>> {
    let all: Vec<&ReviewComment> = reviews.iter().flat_map(|review| &review.comments).collect();
    // As in Python, a comment id seen twice keeps its last copy.
    let by_id: HashMap<u64, usize> = all
        .iter()
        .enumerate()
        .map(|(index, comment)| (comment.comment.id, index))
        .collect();
    let mut replies: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, comment) in all.iter().enumerate() {
        if by_id.get(&comment.comment.id) != Some(&index) {
            continue;
        }
        if let Some(parent) = comment.in_reply_to_id
            && by_id.contains_key(&parent)
        {
            replies.entry(parent).or_default().push(index);
        }
    }

    reviews
        .iter()
        .map(|review| {
            review
                .comments
                .iter()
                .filter(|comment| comment.in_reply_to_id.is_none())
                .map(|root| {
                    let mut thread = Vec::new();
                    // Depth first: a stack of comments still to visit, the next one on top.
                    let mut stack = vec![root];
                    while let Some(comment) = stack.pop() {
                        // Each comment answers one other, so the walk is a tree; only a comment id listed twice can
                        // loop it, which this stops.
                        if thread.len() > all.len() {
                            break;
                        }
                        thread.push(comment);
                        if let Some(children) = replies.get(&comment.comment.id) {
                            stack.extend(children.iter().rev().map(|&index| all[index]));
                        }
                    }
                    thread
                })
                .collect()
        })
        .collect()
}

/// The PR Overview's reviews section: each author's latest submitted review, and the title they add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSummary {
    /// One row per author, in the order of their first submitted review: the login and their latest review's state.
    pub rows: Vec<(String, ReviewState)>,
    /// What the rows add up to.
    pub verdict: ReviewVerdict,
}

/// The title of the reviews summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewVerdict {
    /// Every latest review approves.
    Approved,
    /// Not every one approves, and one asks for changes.
    ChangesRequested,
    /// Anything else.
    Mixed,
}

/// The summary of `reviews`, or `None` when there are none: Python's `PrOverviewTabPane.add_reviews`. Reviews never
/// submitted are skipped; an author's latest review is the one submitted last (the first of equals).
pub fn summary(reviews: &[Review]) -> Option<ReviewSummary> {
    if reviews.is_empty() {
        return None;
    }
    let mut latest: Vec<&Review> = Vec::new();
    for review in reviews {
        let Some(submitted) = review.submitted_at else {
            continue;
        };
        match latest.iter_mut().find(|kept| kept.user.login == review.user.login) {
            Some(kept) => {
                if kept.submitted_at.is_none_or(|at| submitted > at) {
                    *kept = review;
                }
            }
            None => latest.push(review),
        }
    }
    let verdict = if latest.iter().all(|review| review.state == ReviewState::Approved) {
        ReviewVerdict::Approved
    } else if latest
        .iter()
        .any(|review| review.state == ReviewState::ChangesRequested)
    {
        ReviewVerdict::ChangesRequested
    } else {
        ReviewVerdict::Mixed
    };
    Some(ReviewSummary {
        rows: latest
            .iter()
            .map(|review| (review.user.login.clone(), review.state))
            .collect(),
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn user(login: &str) -> User {
        User {
            login: login.to_owned(),
            id: login.len() as u64,
            name: None,
            html_url: format!("https://github.com/{login}"),
        }
    }

    fn comment(id: u64, review: u64, reply_to: Option<u64>) -> ReviewComment {
        ReviewComment {
            comment: IssueComment {
                id,
                body: format!("comment {id}"),
                user: None,
                created_at: Timestamp::UNIX_EPOCH,
                updated_at: Timestamp::UNIX_EPOCH,
                author_association: String::new(),
            },
            pull_request_review_id: review,
            path: "f".to_owned(),
            position: None,
            original_position: None,
            in_reply_to_id: reply_to,
        }
    }

    fn review(
        id: u64,
        login: &str,
        state: ReviewState,
        submitted: Option<i64>,
        comments: Vec<ReviewComment>,
    ) -> Review {
        Review {
            id,
            user: user(login),
            body: String::new(),
            state,
            submitted_at: submitted.map(|s| Timestamp::from_second(s).unwrap()),
            comments,
        }
    }

    fn ids(threads: &[Vec<Thread<'_>>]) -> Vec<Vec<Vec<u64>>> {
        threads
            .iter()
            .map(|review| {
                review
                    .iter()
                    .map(|thread| thread.iter().map(|c| c.comment.id).collect())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn threads_gather_replies_from_every_review_depth_first() {
        let reviews = [
            review(
                1,
                "a",
                ReviewState::Commented,
                Some(1),
                vec![comment(10, 1, None), comment(11, 1, None)],
            ),
            // A reply to 10 from another review, a reply to that reply, and a reply whose parent is missing.
            review(
                2,
                "b",
                ReviewState::Commented,
                Some(2),
                vec![comment(20, 2, Some(10)), comment(21, 2, Some(99))],
            ),
            review(
                3,
                "c",
                ReviewState::Approved,
                Some(3),
                vec![comment(30, 3, Some(20)), comment(31, 3, Some(10)), comment(32, 3, None)],
            ),
        ];
        assert_eq!(
            ids(&threads(&reviews)),
            [vec![vec![10, 20, 30, 31], vec![11]], vec![], vec![vec![32]]]
        );
    }

    #[test]
    fn a_body_less_review_has_threads_only_for_its_roots() {
        // B10: a comment review without a body shows when it starts a thread, and not when it only answers.
        let with_root = [review(
            1,
            "a",
            ReviewState::Commented,
            Some(1),
            vec![comment(10, 1, None)],
        )];
        assert_eq!(ids(&threads(&with_root)), [vec![vec![10]]]);
        let only_replies = [
            review(1, "a", ReviewState::Commented, Some(1), vec![comment(10, 1, None)]),
            review(2, "b", ReviewState::Commented, Some(2), vec![comment(20, 2, Some(10))]),
        ];
        assert_eq!(ids(&threads(&only_replies)), [vec![vec![10, 20]], vec![]]);
        assert!(threads(&[]).is_empty());
    }

    #[test]
    fn a_reply_loop_ends() {
        let reviews = [review(
            1,
            "a",
            ReviewState::Commented,
            Some(1),
            vec![comment(10, 1, None), comment(11, 1, Some(12)), comment(12, 1, Some(11))],
        )];
        assert_eq!(ids(&threads(&reviews)), [vec![vec![10]]]);
    }

    #[test]
    fn summary_keeps_each_authors_latest_review_in_first_review_order() {
        let reviews = [
            review(1, "a", ReviewState::Commented, Some(10), vec![]),
            review(2, "b", ReviewState::Approved, Some(20), vec![]),
            // Later: replaces a's first.
            review(3, "a", ReviewState::ChangesRequested, Some(30), vec![]),
            // Earlier than b's: kept out.
            review(4, "b", ReviewState::Dismissed, Some(5), vec![]),
            // Never submitted: skipped.
            review(5, "c", ReviewState::Pending, None, vec![]),
        ];
        let summary = summary(&reviews).unwrap();
        assert_eq!(
            summary.rows,
            [
                ("a".to_owned(), ReviewState::ChangesRequested),
                ("b".to_owned(), ReviewState::Approved)
            ]
        );
        assert_eq!(summary.verdict, ReviewVerdict::ChangesRequested);
    }

    #[test]
    fn summary_titles() {
        let approved = [
            review(1, "a", ReviewState::Approved, Some(1), vec![]),
            review(2, "b", ReviewState::Approved, Some(2), vec![]),
        ];
        assert_eq!(summary(&approved).unwrap().verdict, ReviewVerdict::Approved);
        let mixed = [
            review(1, "a", ReviewState::Approved, Some(1), vec![]),
            review(2, "b", ReviewState::Commented, Some(2), vec![]),
        ];
        assert_eq!(summary(&mixed).unwrap().verdict, ReviewVerdict::Mixed);
        assert_eq!(summary(&[]), None);
        // Reviews, none submitted: Python's `all` over nothing approves.
        let unsubmitted = [review(1, "a", ReviewState::Pending, None, vec![])];
        let summary = summary(&unsubmitted).unwrap();
        assert!(summary.rows.is_empty());
        assert_eq!(summary.verdict, ReviewVerdict::Approved);
    }

    #[test]
    fn decode_a_review_and_a_review_comment() {
        let review: Review = serde_json::from_value(json!({
            "id": 5,
            "user": {"login": "a", "id": 1, "html_url": "https://github.com/a"},
            "body": "hi",
            "state": "CHANGES_REQUESTED",
            "submitted_at": "2026-02-02T10:00:00Z",
        }))
        .unwrap();
        assert_eq!(review.state, ReviewState::ChangesRequested);
        assert!(review.comments.is_empty());
        let pending: Review = serde_json::from_value(json!({
            "id": 6,
            "user": {"login": "a", "id": 1, "html_url": "https://github.com/a"},
            "body": "",
            "state": "PENDING",
        }))
        .unwrap();
        assert_eq!((pending.state, pending.submitted_at), (ReviewState::Pending, None));

        let comment: ReviewComment = serde_json::from_value(json!({
            "id": 9,
            "body": null,
            "user": null,
            "created_at": "2026-02-02T10:00:00Z",
            "updated_at": "2026-02-02T10:00:00Z",
            "pull_request_review_id": 5,
            "path": "src/a.rs",
            "position": null,
            "original_position": 3,
            "in_reply_to_id": 8,
        }))
        .unwrap();
        assert_eq!(comment.comment.body, "");
        assert_eq!(
            (comment.position, comment.original_position, comment.in_reply_to_id),
            (None, Some(3), Some(8))
        );
        assert!(
            serde_json::from_value::<ReviewState>(json!("LGTM")).is_err(),
            "an unknown state fails"
        );
        assert_eq!(ReviewState::ChangesRequested.title(), "Changes_Requested");
    }
}
