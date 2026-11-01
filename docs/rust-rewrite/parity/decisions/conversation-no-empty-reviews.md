# PR Conversation: no open, empty review sections

- Decision: A body-less review that starts no thread of its own is closed with "No additional review content"
  (approved, changes requested, dismissed) or hidden (commented); its replies stay in their parent's thread.
  Python opens such a section with nothing in it.
- Decided by: Chris, 2026-10-30 (#346, an M5 improvement: "each submission shows once with its inline comments
  under it and no empty entries, while still following B10's answer"); Chris, 2026-10-31 (#346): D1 a, a body-less
  review that starts threads stays as it is (`▼ Review from <login> (<state>)` with its threads); D3 a, reviews stay
  first, then the comments, with no interleaving by time
- Link: https://github.com/gizmo385/gh-lazy/issues/346
- Rows: `pull-requests.conversation-empty-review`

GitHub records every inline-comment submission as a review: a github.com "Add single comment" is a body-less
`COMMENTED` review with one root comment, and each reply in a thread is a body-less `COMMENTED` review whose only
comment answers a comment of another review. It never copies a review's body or inline comments into the issue
comments, so the same text never shows twice. The empty entry comes from deciding a review's section by its raw
comments rather than by the threads it starts. A body-less approved, changes-requested or dismissed review whose
comments are all replies drawn in another review's thread (approving on github.com after replying while a review was
pending), or replies to a missing comment, has comments but no thread, so both apps open its section with nothing in
it (Python's `ReviewContainer.compose` tests `self.review.body or self.review.comments`).

Rust decides by the threads instead (`review_shape` in `pr_details/conversation.rs`): such a review is closed with
the one line, as a review with no comments already is, and opening it shows that line. A body-less comment review
with no thread stays hidden (B10's recommendation); one that starts threads shows, as before. Its replies show in the
parent's thread, and a reply to a missing comment shows nowhere, as before. Nothing else in the tab changes. Python
differs on purpose, so the `pr-conversation-bodyless` scenario runs Rust only.
