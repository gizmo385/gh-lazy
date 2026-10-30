# A rate limit shows one toast and pauses the notifications poll

- Decision: When a GitHub response is a rate limit (HTTP 403 or 429 with `Retry-After`, or with
  `X-RateLimit-Remaining: 0` and `X-RateLimit-Reset`), the app shows one error toast, titled "Rate limited", "GitHub
  API rate limit reached. Try again at <HH:MM>" (the reset in local time, or now + `Retry-After`), once per limit
  window rather than once per failed call. The 60 s notifications poll skips its ticks until the reset, then resumes.
  Requests the user asks for still run and fail with their own errors, as before. The command log gets a warning with
  the rate-limit headers for each limited response.
  A response with no headers to read (a `gh api --paginate` call) is a limit when it's a 403 or 429 whose message
  says "API rate limit exceeded" or "secondary rate limit", lifting a minute after it arrived; wherever headers
  exist, only they count. The PR diff keeps its headers like any `gh api -i` call (#333).
- Decided by: Chris, 2026-10-30 (#324), and Chris, 2026-10-30, option 2 on #333 for paginated calls and the diff
- Link: https://github.com/gizmo385/gh-lazy/issues/324#issuecomment-6516698391 (#324, parent #201); https://github.com/gizmo385/gh-lazy/issues/333#issuecomment-6531304636 (#333)
- Rows: `toasts.rate-limited`

Python's `_parse_cli_api_response` treats a 403 or 429 as any other HTTP error: nothing reads `X-RateLimit-*` or
`Retry-After`, each failing call shows (or logs) its own generic error, and the notifications poll in `ui/app.py`
keeps asking every 60 s, which only spends more of the limit.

Only responses from `gh api -i` carry headers. Requests made with `gh api --paginate` (the repositories list, for
one) see the 403 or 429 without them, so for those GitHub's message decides, and as nothing says when the limit
lifts, the window is the minute GitHub's docs advise waiting on a secondary limit without `Retry-After`. Reading
each page's headers with `--paginate -i` instead (option 1 on #333) would have changed how pages are read, which
fake-gh and the parity harness depend on. A 403 with headers that quotes the text but has requests left (a permission
error) stays its own error.
Proactive throttling from `X-RateLimit-Remaining` before it reaches zero is out of scope.
