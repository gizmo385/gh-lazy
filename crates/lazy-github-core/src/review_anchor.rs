//! Where a pull request's review threads go in its diff (#390, decision `diff-inline-comments`).
//!
//! A thread is anchored by its first comment's `path` and `position`: the diff position GitHub gives each comment in
//! the pull request's current diff, the numbering [`crate::diff::DiffLine::position`] uses. A comment whose line is no
//! longer in that diff has no `position`, only the `original_position` it was written at: it's outdated, and the last
//! line of its `diff_hunk` is the line it was written on. A comment with neither is on the whole file.

use crate::diff::{LineKind, classify, parse_hunk_header};
use crate::models::{Review, ReviewComment, review_threads};

/// One thread of review comments, for the diff: its file, where it goes, and its comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffThread {
    /// The file it's on, as the diff names it.
    pub path: String,
    /// Where in the file it goes.
    pub anchor: Anchor,
    /// The first comment, then its replies from any review, depth first, as the Conversation shows them.
    pub comments: Vec<ReviewComment>,
}

impl DiffThread {
    /// The thread's first comment.
    pub fn root(&self) -> &ReviewComment {
        // `diff_threads` builds each thread from its root, so it's never empty.
        &self.comments[0]
    }

    /// How many comments answer the first one.
    pub fn replies(&self) -> usize {
        self.comments.len().saturating_sub(1)
    }
}

/// Where a thread goes in its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// Under the line at this diff position, when the file has one there.
    Position(u32),
    /// The line is no longer in the diff: listed after the file's hunks, with the line it was written on when its
    /// hunk says.
    Outdated {
        /// The line it was written on.
        original: Option<OriginalLine>,
    },
    /// On the whole file: right after its header.
    File,
}

/// The line an outdated comment was written on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalLine {
    /// Added, removed or context.
    pub kind: LineKind,
    /// Its number as the diff's gutter shows it: the old side's for a removed line, the new side's otherwise.
    pub number: u32,
    /// The line, without its marker.
    pub text: String,
}

/// Every thread of `reviews`, built as [`review_threads`] builds them, ordered by their first comment's `created_at`,
/// then its id.
pub fn diff_threads(reviews: &[Review]) -> Vec<DiffThread> {
    let mut threads: Vec<DiffThread> = review_threads(reviews)
        .into_iter()
        .flatten()
        .filter_map(|thread| {
            let root = *thread.first()?;
            let anchor = match (root.position, root.original_position) {
                // A position too large for a diff matches no line, which shows the thread as outdated.
                (Some(position), _) => u32::try_from(position).map_or_else(
                    |_| Anchor::Outdated {
                        original: original_line(&root.diff_hunk),
                    },
                    Anchor::Position,
                ),
                (None, Some(_)) => Anchor::Outdated {
                    original: original_line(&root.diff_hunk),
                },
                (None, None) => Anchor::File,
            };
            Some(DiffThread {
                path: root.path.clone(),
                anchor,
                comments: thread.into_iter().cloned().collect(),
            })
        })
        .collect();
    threads.sort_by_key(|thread| (thread.root().comment.created_at, thread.root().comment.id));
    threads
}

/// The last line of a comment's `diff_hunk`, numbered from its `@@` header as the diff gutter numbers lines (the old
/// number for a removed line, the new one for the rest). A later `@@` header numbers the lines after it. `None` for an
/// empty or malformed hunk: one that doesn't start with a header that parses, or has no line after it.
pub fn original_line(diff_hunk: &str) -> Option<OriginalLine> {
    let mut lines = diff_hunk.lines();
    let (range, _) = parse_hunk_header(lines.next()?)?;
    let (mut old, mut new) = (range.old_start, range.new_start);
    let mut last = None;
    for line in lines {
        if line.starts_with("@@") {
            let (range, _) = parse_hunk_header(line)?;
            (old, new) = (range.old_start, range.new_start);
            last = None;
            continue;
        }
        let (kind, text) = classify(line);
        let number = match kind {
            LineKind::Context => {
                old += 1;
                new += 1;
                new - 1
            }
            LineKind::Removed => {
                old += 1;
                old - 1
            }
            LineKind::Added => {
                new += 1;
                new - 1
            }
            // The marker belongs to the line before it.
            LineKind::NoNewline => continue,
        };
        last = Some(OriginalLine {
            kind,
            number,
            text: text.to_owned(),
        });
    }
    last
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;
    use crate::models::{IssueComment, ReviewState, User};

    fn comment(id: u64, review: u64, created: i64, reply_to: Option<u64>) -> ReviewComment {
        ReviewComment {
            comment: IssueComment {
                id,
                body: format!("comment {id}"),
                user: None,
                created_at: Timestamp::from_second(created).unwrap(),
                updated_at: Timestamp::from_second(created).unwrap(),
                author_association: String::new(),
            },
            pull_request_review_id: review,
            path: "src/a.rs".to_owned(),
            position: None,
            original_position: None,
            in_reply_to_id: reply_to,
            diff_hunk: String::new(),
        }
    }

    fn at(mut comment: ReviewComment, position: Option<u64>, original: Option<u64>, hunk: &str) -> ReviewComment {
        comment.position = position;
        comment.original_position = original;
        comment.diff_hunk = hunk.to_owned();
        comment
    }

    fn review(id: u64, comments: Vec<ReviewComment>) -> Review {
        Review {
            id,
            user: User {
                login: "a".to_owned(),
                id: 1,
                name: None,
                html_url: "https://github.com/a".to_owned(),
            },
            body: String::new(),
            state: ReviewState::Commented,
            submitted_at: Some(Timestamp::from_second(id.cast_signed()).unwrap()),
            comments,
        }
    }

    fn ids(threads: &[DiffThread]) -> Vec<Vec<u64>> {
        threads
            .iter()
            .map(|thread| thread.comments.iter().map(|c| c.comment.id).collect())
            .collect()
    }

    #[test]
    fn threads_are_anchored_outdated_or_on_the_file() {
        let hunk = "@@ -1,2 +1,2 @@\n a\n-b";
        let reviews = [review(
            1,
            vec![
                at(comment(10, 1, 10, None), Some(6), Some(6), "@@ -1,6 +1,8 @@\n x"),
                at(comment(11, 1, 11, None), None, Some(4), hunk),
                at(comment(12, 1, 12, None), None, None, ""),
            ],
        )];
        let threads = diff_threads(&reviews);
        assert_eq!(ids(&threads), [vec![10], vec![11], vec![12]]);
        assert_eq!(threads[0].anchor, Anchor::Position(6));
        assert_eq!(
            threads[1].anchor,
            Anchor::Outdated {
                original: Some(OriginalLine {
                    kind: LineKind::Removed,
                    number: 2,
                    text: "b".to_owned()
                })
            }
        );
        assert_eq!(threads[2].anchor, Anchor::File);
        assert_eq!(threads[0].path, "src/a.rs");
        // An outdated comment whose hunk says nothing has no original line.
        let bare = [review(1, vec![at(comment(10, 1, 10, None), None, Some(3), "")])];
        assert_eq!(diff_threads(&bare)[0].anchor, Anchor::Outdated { original: None });
    }

    #[test]
    fn a_reply_from_another_review_joins_its_root_s_thread() {
        let reviews = [
            review(1, vec![at(comment(10, 1, 10, None), Some(6), Some(6), "")]),
            // The reply has no position of its own: the root's anchors the thread.
            review(2, vec![comment(20, 2, 20, Some(10)), comment(21, 2, 21, Some(20))]),
        ];
        let threads = diff_threads(&reviews);
        assert_eq!(ids(&threads), [vec![10, 20, 21]]);
        assert_eq!(threads[0].anchor, Anchor::Position(6));
        assert_eq!(threads[0].replies(), 2);
        assert_eq!(threads[0].root().comment.id, 10);
    }

    #[test]
    fn threads_are_ordered_by_creation_then_id() {
        let reviews = [
            review(1, vec![comment(30, 1, 300, None), comment(12, 1, 100, None)]),
            review(2, vec![comment(11, 2, 100, None), comment(40, 2, 50, None)]),
        ];
        assert_eq!(ids(&diff_threads(&reviews)), [vec![40], vec![11], vec![12], vec![30]]);
        assert!(diff_threads(&[]).is_empty());
    }

    #[test]
    fn a_position_past_any_diff_is_outdated() {
        let reviews = [review(
            1,
            vec![at(comment(10, 1, 10, None), Some(u64::MAX), Some(3), "")],
        )];
        assert_eq!(diff_threads(&reviews)[0].anchor, Anchor::Outdated { original: None });
    }

    #[allow(clippy::unnecessary_wraps)] // Compared with what `original_line` returns.
    fn line(kind: LineKind, number: u32, text: &str) -> Option<OriginalLine> {
        Some(OriginalLine {
            kind,
            number,
            text: text.to_owned(),
        })
    }

    #[test]
    fn original_line_numbers_the_last_line_from_the_header() {
        let added = "@@ -1,6 +1,8 @@\n use std::io;\n+use std::fmt;";
        assert_eq!(original_line(added), line(LineKind::Added, 2, "use std::fmt;"));
        let removed = "@@ -20,6 +22,7 @@ impl Viewer {\n     pub fn render(&self) {\n-        for line in &self.lines {\n-            println!(\"{line}\");";
        assert_eq!(
            original_line(removed),
            line(LineKind::Removed, 22, "            println!(\"{line}\");")
        );
        let context = "@@ -20,3 +30,3 @@\n-a\n+b\n c";
        assert_eq!(original_line(context), line(LineKind::Context, 31, "c"));
        // A header alone, and a trailing no-newline marker, which belongs to the line before it.
        assert_eq!(original_line("@@ -1 +1 @@"), None);
        assert_eq!(
            original_line("@@ -1 +1 @@\n-a\n\\ No newline at end of file"),
            line(LineKind::Removed, 1, "a")
        );
    }

    #[test]
    fn original_line_follows_several_lines_and_a_later_header() {
        let long = "@@ -10,8 +10,9 @@\n a\n b\n+c\n d\n-e\n-f\n g\n h";
        // Context 10, 11; added 12 (new); context d is old 12 / new 13; e, f removed (old 13, 14); g old 15 / new 14;
        // h old 16 / new 15.
        assert_eq!(original_line(long), line(LineKind::Context, 15, "h"));
        let two = "@@ -1,2 +1,2 @@\n a\n-b\n@@ -40,2 +41,2 @@\n x\n+y";
        assert_eq!(original_line(two), line(LineKind::Added, 42, "y"));
    }

    #[test]
    fn original_line_of_garbage_is_none() {
        assert_eq!(original_line(""), None);
        assert_eq!(original_line("not a hunk\n+a"), None);
        assert_eq!(original_line("@@ -x +y @@\n+a"), None);
        assert_eq!(original_line(" a\n b\n+c"), None, "lines without a header");
        assert_eq!(original_line("@@ -1 +1 @@\n a\n@@ broken\n b"), None);
    }
}
