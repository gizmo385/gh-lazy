# A failed issue-comments request shows an empty Comments tab

- Decision: When the issue comments request fails, the Rust app logs ERROR `Error retrieving issue comments` (the
  error itself at debug), clears the loading state and shows "No comments available". No toast.
- Decided by: Chris, 2026-10-26, in a Claude Code session
- Link: Py bug [B60](../py-bugs/B60.md), answered "follow the recommendation"
- Rows: `issues.comments-error`

In Python the Comments tab's `fetch_issue_comments` worker lets `GithubApiRequestFailed` escape, and Textual's
`exit_on_error` exits the app with a traceback. Rust treats the failure the way the Workflows section treats a failed
fetch: an empty list and a log line. Since Python crashes, no parity scenario can compare the two; unit tests pin the
Rust side (the `issue-details` fixture's issue 9 answers 500).
