# A rate limit shows one toast and pauses the notifications poll

- Decision: When a GitHub response is a rate limit (HTTP 403 or 429 with `Retry-After`, or with
  `X-RateLimit-Remaining: 0` and `X-RateLimit-Reset`), the app shows one error toast, titled "Rate limited", "GitHub
  API rate limit reached. Try again at <HH:MM>" (the reset in local time, or now + `Retry-After`), once per limit
  window rather than once per failed call. The 60 s notifications poll skips its ticks until the reset, then resumes.
  Requests the user asks for still run and fail with their own errors, as before. The command log gets a warning with
  the rate-limit headers for each limited response.
- Decided by: Chris, 2026-10-30
- Link: https://github.com/gizmo385/gh-lazy/issues/324#issuecomment-6516698391 (#324, parent #201)
- Rows: `toasts.rate-limited`

Python's `_parse_cli_api_response` treats a 403 or 429 as any other HTTP error: nothing reads `X-RateLimit-*` or
`Retry-After`, each failing call shows (or logs) its own generic error, and the notifications poll in `ui/app.py`
keeps asking every 60 s, which only spends more of the limit.

Only responses from `gh api -i` carry headers. Requests made with `gh api --paginate` (the repositories list, for
one) see the 403 or 429 without them, and the PR diff's error keeps only the status, so those aren't recognised as a
rate limit and only show their own error.
Proactive throttling from `X-RateLimit-Remaining` before it reaches zero is out of scope.
