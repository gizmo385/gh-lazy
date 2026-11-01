# The PR table lists pull requests from their own endpoint

- Decision: The PR table's pages come from `GET /repos/{owner}/{repo}/pulls?state=<s>&sort=created&direction=desc`
  for `pull_requests.owner_filter = all`, and from `GET /search/issues` with
  `q=is:pr repo:<owner>/<repo> author:@me[ is:open| is:closed]&advanced_search=true&sort=created&order=desc` for
  `mine` (`advanced_search` since #393, which shares the search request), 30 per page. Every page is all PRs, so a
  repo whose newest issues-endpoint items are issues still shows its PRs, and paging never fetches pages of issues. The Issues table keeps `GET /repos/{owner}/{repo}/issues`, now as its own request:
  the two tables no longer share a first page. A repo with pull requests turned off (`/pulls` answers 404) shows an
  empty PR table with no error line, as Python's empty issues page does.
- Decided by: Chris, 2026-10-31 (#384: "owner preference: the proper one", with the endpoint for `mine` left to the
  designer); search for `mine` chosen in #384's spec
- Link: https://github.com/gizmo385/gh-lazy/issues/384
- Rows: `pull-requests.rows-from-issues`, `pull-requests.filters`

Python fetches one mixed page from the issues endpoint and gives its PRs to the PR table and the rest to the Issues
table (B180). A run of new issues pushes every PR off page 1, and an empty table never pages, so the PR table stays
empty.

`/pulls` can't filter by author. Filtering its pages on the client would bring the same bug back for `mine` (30 PRs
by others, none yours, an empty table), and the old `creator=<login>` on `/issues` brings it back whenever your newest
30 items are issues. Search filters on the server and needs no `GET /user` first. Its costs: a separate limit of 30
requests a minute (shared with #386's search), results capped at 1,000 (paging stops at page 34), and an index that can
trail a just-opened PR by a few seconds; a PR made in the app is added to the table directly, so that only shows on a
reload within seconds. All three are acceptable for one user's PRs in one repo.

Requests change, the screens don't: in the parity scenarios the PR table shows the same rows in both apps, because
the `/pulls` and `/search/issues` fixtures are generated from each set's issues pages (#384). The rows that differ
on screen are only those of a repo like `pr-list-issues-first`, where Python's PR table is empty.

Row ids stay as they are; `pull-requests.rows-from-issues` keeps its id though its rows no longer come from the
issues endpoint.
