# Review comments show inline on their diff lines

- Decision: The Diff tab shows the pull request's review threads under
  the lines they're on, collapsed to one summary row each, with a mark in the gutter; outdated threads are listed at
  the end of their file; pending comments (added with `c`, not yet submitted) show under their line too and are
  removed there with a key, in place of Python's "Pending Comments:" section after the files. Python shows no
  existing review comment in the Diff tab, and lists pending comments in a separate section with a Remove button
  each.
- Decided by: Chris, 2026-11-01, on #344 (D4 a, D5 a, D6 a, for #345; proposed by the design agent, 2026-10-31).
  D6 (a): pending comments inline only, `x` removes one, the "Pending Comments:" section goes, and a one-row count
  sits above the review form.
- Link: #345, #177 (M5 improvements, Chris 2026-10-30)
- Rows: `diff-viewer.pending-comments`, `diff-viewer.add-comment`, `diff-viewer.review-threads`,
  `diff-viewer.outdated-threads`, `diff-viewer.comment-navigation`, `keybindings.diff-next-comment`,
  `keybindings.diff-previous-comment`, `keybindings.diff-remove-comment`

Status: decided; built. #390 shows the review threads (anchored, file-level and outdated) and added the rows
`diff-viewer.review-threads`, `diff-viewer.outdated-threads`, `diff-viewer.comment-navigation`,
`keybindings.diff-next-comment` and `keybindings.diff-previous-comment` (scenario `diff-threads`). #391 shows the
pending comments under their lines (removed with `x`, `diff.remove_comment`), drops the "Pending Comments:" section,
counts the comments in a row above the review form, adds `keybindings.diff-remove-comment` and moves
`diff-viewer.pending-comments` to `intentional diff` (scenario `diff-review`). `diff-viewer.add-comment` stays
`parity` (the dialog doesn't change) and is listed for its Notes.

Why: in Python it's hard to tell which line a pending comment belongs to ("<file>:<line> - <line text>" in a list
after every file), and the comments already on the pull request appear only in the Conversation tab, with no file or
line at all. Putting both under their lines makes the diff the place to read and write a review. The data is what
the Conversation tab already fetches (`GET .../pulls/{n}/reviews` and each review's comments): no new request.
