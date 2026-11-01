# Review comments show inline on their diff lines

- Decision: Proposed, waiting on Chris (#345, D4 to D6). The Diff tab shows the pull request's review threads under
  the lines they're on, collapsed to one summary row each, with a mark in the gutter; outdated threads are listed at
  the end of their file; pending comments (added with `c`, not yet submitted) show under their line too and are
  removed there with a key, in place of Python's "Pending Comments:" section after the files. Python shows no
  existing review comment in the Diff tab, and lists pending comments in a separate section with a Remove button
  each.
- Decided by: pending: Chris, on #345 (proposed by the design agent, 2026-10-31)
- Link: #345, #177 (M5 improvements, Chris 2026-10-30)
- Rows: `diff-viewer.pending-comments`, `diff-viewer.add-comment`

Status: proposed. When Chris answers on #345, the implementers set `Decided by`, add the new row ids
(`diff-viewer.review-threads`, `diff-viewer.outdated-threads`, `diff-viewer.comment-navigation`,
`keybindings.diff-next-comment`, `keybindings.diff-previous-comment`, `keybindings.diff-remove-comment`) to `Rows`,
and move `diff-viewer.pending-comments` to `intentional diff`. `diff-viewer.add-comment` stays `parity` (the dialog
doesn't change) and is listed for its Notes.

Why: in Python it's hard to tell which line a pending comment belongs to ("<file>:<line> - <line text>" in a list
after every file), and the comments already on the pull request appear only in the Conversation tab, with no file or
line at all. Putting both under their lines makes the diff the place to read and write a review. The data is what
the Conversation tab already fetches (`GET .../pulls/{n}/reviews` and each review's comments): no new request.
