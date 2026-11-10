# Review comments show inline on their diff lines

- Decision: The Diff tab shows the pull request's review threads under
  the lines they're on, collapsed to one summary row each, with a mark in the gutter; outdated threads are listed at
  the end of their file; pending comments (added with `c`, not yet submitted) show under their line too and are
  removed there with a key, in place of Python's "Pending Comments:" section after the files; a key on a thread's rows
  replies to the thread, in the New comment dialog. Python shows no existing review comment in the Diff tab, and lists
  pending comments in a separate section with a Remove button each.
- Decided by: Chris, 2026-11-01, on #344 (D4 a, D5 a, D6 a, for #345; proposed by the design agent, 2026-10-31).
  D6 (a): pending comments inline only, `x` removes one, the "Pending Comments:" section goes, and a one-row count
  sits above the review form. Chris, 2026-11-09, on #385: D1 (a) replying is a new Rust-only `diff.reply_to_thread`
  (`r`, "Reply"); D2 (a) outdated threads take replies too; D3 (a) the reply is typed in the existing New comment
  dialog, posted to the thread's root.
- Link: #345, #177 (M5 improvements, Chris 2026-10-30)
- Rows: `diff-viewer.pending-comments`, `diff-viewer.add-comment`, `diff-viewer.review-threads`,
  `diff-viewer.outdated-threads`, `diff-viewer.comment-navigation`, `keybindings.diff-next-comment`,
  `keybindings.diff-previous-comment`, `keybindings.diff-remove-comment`, `diff-viewer.thread-reply`,
  `keybindings.diff-reply-to-thread`

Status: decided; built. #390 shows the review threads (anchored, file-level and outdated) and added the rows
`diff-viewer.review-threads`, `diff-viewer.outdated-threads`, `diff-viewer.comment-navigation`,
`keybindings.diff-next-comment` and `keybindings.diff-previous-comment` (scenario `diff-threads`). #391 shows the
pending comments under their lines (removed with `x`, `diff.remove_comment`), drops the "Pending Comments:" section,
counts the comments in a row above the review form, adds `keybindings.diff-remove-comment` and moves
`diff-viewer.pending-comments` to `intentional diff` (scenario `diff-review`). `diff-viewer.add-comment` stays
`parity` (the dialog doesn't change) and is listed for its Notes. #385 adds replying to a thread with `r`
(`diff.reply_to_thread`) from any of its rows, outdated and file-level threads included, through the New comment
dialog quoting the comment under the cursor and posting to the thread's root, and adds `diff-viewer.thread-reply` and
`keybindings.diff-reply-to-thread` (scenario `diff-reply`).

Why: in Python it's hard to tell which line a pending comment belongs to ("<file>:<line> - <line text>" in a list
after every file), and the comments already on the pull request appear only in the Conversation tab, with no file or
line at all. Putting both under their lines makes the diff the place to read and write a review. The data is what
the Conversation tab already fetches (`GET .../pulls/{n}/reviews` and each review's comments): no new request.
