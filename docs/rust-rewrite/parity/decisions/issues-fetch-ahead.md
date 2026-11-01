# An empty Issues table fetches ahead

- Decision: While the Issues table has no rows after an issues-endpoint page that wasn't empty, the next page is
  asked for at once, with no cursor move. Loading stops at the first page that adds a row (lazy loading then goes on
  as usual), at an empty page, at a failed page, or after 5 pages in all; the loading overlay stays up until then. A
  repo with issues turned off (`has_issues: false`) fetches nothing ahead. Python's Issues table stays empty and never
  asks for page 2.
- Decided by: Chris, 2026-11-01 (#399, option D1 a)
- Link: https://github.com/gizmo385/gh-lazy/issues/399
- Rows: `issues.filters`

There's no REST endpoint that lists only issues, so the Issues table keeps `GET /repos/{owner}/{repo}/issues` and
drops the PRs (the PR table moved to `/pulls` and search in #384, decision `pr-list-from-pulls-endpoint`). When the
newest 30 items are all PRs, common where bots open PRs or for `owner_filter = mine` by someone who mostly opens PRs,
Python's table gets no rows and has no cursor to page with (B181).

Rust asks for up to 4 more core-API pages per repo selection, and only in that case; a repo whose first page has an
issue sends the same requests as Python. The cap keeps a repo with hundreds of recent PRs from paging through all of
them. A repo with issues off lists only PRs on the issues endpoint (`git/git`), so fetching ahead there would never
find a row. `has_issues` is read from the API's repository objects; a repo read from the repos cache, which doesn't
store the field (it stays byte-equal to Python's), counts as having issues.

`GET /search/issues` with `is:issue` (option c) lists only issues but spends the 30-a-minute search limit on every
repo selection, stops at 1,000 results and lags behind new issues.

Scenario `issues-fetch-ahead` (Rust only); `table-lazy-load`'s all-PR Issues table now asks for pages 2 to 4 and
stays ` (empty)`.
