# gh going missing, logging out or hanging mid-session shows one toast and keeps the app usable

- Decision: When a request on the main screen finds `gh` missing (`GhError::NotInstalled`), logged out (`gh api`
  exit 4, or an HTTP 401) or past its time limit (killed after 20 s for `auth status`, 30 s for a request, 180 s for
  `--paginate`), the app shows one error toast titled "API Error" with the error's text ("The GitHub CLI (gh) is not
  installed or not on your PATH. Install it from https://cli.github.com, then run `gh auth login`.", "To proceed,
  please log into the Github CLI via `gh auth login`", "gh did not respond within <n> seconds"), once until a request
  succeeds again. Every request still fails with its own toast, inline message or log line, as before, and nothing is
  left loading. While `gh` is missing or logged out the 60 s notifications poll skips its ticks, and the next request
  that succeeds ends that; the poll's own failures keep Python's "please login" toast, except that a missing `gh`
  says so. A `gh` that fails without an HTTP response (offline) puts its error output in the command log. Requests
  Python crashes on with no message of their own (the branch suggestions of Trigger workflow and the PR dialog, the
  current user for Add reaction) show the same "API Error" toast with the error's text. No retries or backoff: the
  next action simply runs `gh` again.
- Decided by: Chris, 2026-10-30 (proposed in the #323 spec under #201's hardening rule)
- Link: https://github.com/gizmo385/gh-lazy/issues/323#issuecomment-6532454300
- Rows: `toasts.gh-unavailable`

Python's `run_gh_cli_command` creates the `gh` process outside its `try`, so a missing `gh` raises out of every
request and ends the worker that made it (the app with it, for most workers). It has no time limit, so a hung `gh`
leaves whatever waits on it loading for good. A logged-out `gh` makes each request fail with its own generic error,
and the notifications poll checks `gh auth status` every 60 s and toasts "Cannot load notifications - please login to
the gh CLI: gh auth login" each time.

The toast is the error's own text rather than one per request because selecting a repo alone makes four requests,
and four toasts saying `gh` is missing would bury the screen. The poll's requests don't raise it, as the poll toasts
its own failures; the startup and authentication screens don't either, as they say what's wrong with `gh`
themselves. A timeout doesn't pause the poll, since the next tick may get through.
