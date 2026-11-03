# Issues and pull requests can be searched through GitHub's search endpoint

- Decision: A "Search issues and pull requests" modal sends a query in GitHub's search syntax to `GET /search/issues`
  (with `advanced_search=true`, so `AND`, `OR` and parentheses work as on github.com), scoped by a select (the
  selected repo by default) unless the query names a scope itself, and lists the hits from every page GitHub returns,
  up to its 1,000-result cap. `enter` on a hit opens it in Details after selecting its repo, even when it's the
  selected one (#405 D2 a), as a chosen notification does. It opens with `s` on the Pull Requests and Issues tables
  (`search.open`, a Rust-only id) and from the command palette, and keeps its query, scope and results for the
  session. Recent queries are kept, and saved queries live in a new `search.saved_queries` config field. The Python app has no search beyond the `/` box, which only
  filters the rows already loaded. A rate limit on GitHub's separate search bucket (30 requests a minute) shows its
  own toast, "GitHub search rate limit reached. Try again at <HH:MM>", once per search window, and doesn't pause
  the notifications poll, which uses the core bucket.
- Decided by: Chris, 2026-11-01, on #386: D1 a (a Search modal), D2 a (a scope select of This repo, This owner,
  Favorites and tracked repos (up to 20) and All of GitHub, overridden by a scope typed in the query), D3 a (the last
  20 queries persisted, plus saved queries in config as palette commands), D4 a (`s` on the PR and Issues tables,
  `ctrl+s` saves, both rebindable Rust-only ids), D5 b (opening a hit from another repo switches the selected repo,
  sharing the code path with notifications, #405). Proposed by the design agent, 2026-11-01
- Link: #386 (parent; #393, #394, #395), #44 and #59 (the Python-era requests), #177 (M5 improvements, Chris 2026-10-30),
  #408 (the modal's keys and layout)
- Rows: `toasts.rate-limited`, `toasts.search-rate-limited`, `modals.search`, `modals.search-scope`,
  `modals.search-results`, `modals.search-states`, `modals.search-open-hit`, `modals.search-session`,
  `keybindings.search-open`, `command-palette.search`

New behavior, not tied to the Python app (an M5 improvement). It adds new matrix rows instead of changing
existing ones, apart from the rate-limit split on `toasts.rate-limited`. The sub-issues add the rows and list them
on the `Rows:` line above as they land:

- #393 (core): `toasts.search-rate-limited`. Also a Notes update on `toasts.rate-limited`: a limit whose
  `X-RateLimit-Resource` is `search` has its own window and toast and leaves the notifications poll running.
- #394 (the modal): `modals.search`, `modals.search-scope`, `modals.search-results`, `modals.search-states`,
  `modals.search-open-hit`, `modals.search-session`, `keybindings.search-open`, `command-palette.search`.
- #395 (recent and saved queries): `modals.search-history`, `modals.search-saved-queries`,
  `keybindings.search-save-query`, `config.search-saved-queries`, `caches-drafts.search-history`,
  `command-palette.search-saved-query`.

The PR and Issues tables, their `/` box, their paging and their caches don't change. Results stay in the modal:
they can come from many repos, while the tables are keyed by number and belong to the selected repo.

What the real API does (checked 2026-11-01 with `gh api search/issues`):

- Without `advanced_search=true`, a `(` or `OR` in the query is read as text or a login and fails with a 422. With
  it, several `repo:` qualifiers are ANDed (no hits), so a set of repos has to be `(repo:a OR repo:b ...)`, and the
  user's own query has to be wrapped in parentheses when a scope is added. Otherwise a top-level `OR` escapes the
  scope: `repo:x a OR b` matched 11 million issues.
- Up to 24 OR'ed `repo:` terms work. At 25, GitHub answers 200 with no hits and no error, so the
  "Favorites and tracked repos" scope sends at most 20.
- Paging stops at 1,000 results. `page=21&per_page=50` answers 422 "Only the first 1000 search results are
  available". `total_count` still gives the full count (885,203 for one query).
- A bad query answers 422 "Validation Failed" with readable `errors[].message` lines (an unknown user or repo, a bad
  date). An empty `q` answers `errors[].code = "missing"` with no message. An unknown qualifier (`foo:bar`) isn't an
  error: it's searched as text.
- The search bucket allows 30 requests a minute (`X-RateLimit-Resource: search`) and is separate from the core
  bucket of 5,000 an hour. Past it, GitHub answers 403 "API rate limit exceeded for user ID ..." with
  `X-RateLimit-Remaining: 0` and `X-RateLimit-Reset`.
- `user:<login>` works for orgs as well as users. `sort:updated-desc` and `sort:created-asc` in `q` are honored. A
  response also carries `search_type: "lexical"`.

## The modal's keys and layout (#408)

Chris, 2026-11-03, after trying the modal: the results table showed its own `/` filter box as a second search bar,
`/` opened that box instead of going back to the query, and `escape` closed the modal straight from the query
input. So:

- The results table isn't searchable: there's no filter box and nothing opens one. The query input is the modal's
  only text box. The focus ring is the input, the scope select (unless the query disables it) and the table.
- `/` (`modal.search`, footer "Query") focuses the query input with the cursor at the end of the text, from the
  table or the closed scope select. In the input it types `/`; with the scope list open it searches the list, as any
  printable key does.
- `escape` (`modal.close`) in the query input leaves it, for the results table, or for the scope select while there
  are no results (the table, when the query disables the select). With the scope list open it closes the list. On the
  table or the closed select it closes the modal, so two presses from the input close it. `q` (`modal.close` too)
  closes whenever focus isn't in the input, where it types `q`.
- The footer is `⏎ Search  esc Results` in the input and `⏎ Open  / Query  q Close` on the table (`/ Query  q Close`
  on the select). The labels are the modal's own for existing ids (`textual.input.submit`, `common.table.select`,
  `modal.search` and `modal.close`). No id is new, so the Key Bindings tab and the conflict check list them where
  they already were (`modal.search` and `modal.close` under Dialogs). Rebinding either moves its key here too.
- The heading sits one blank row below the top border, with one blank row under it. (Notifications keeps Python's
  `MarkdownH1` margins, two rows above.) The hint line, the status line and the table start one column in, under the
  input's and the select's left edges, and the table ends one column inside the right edge. The "Title" column gives
  up width, cut with an ellipsis, when the columns are wider than the table (the shared table's `Column::shrinking`),
  so "Updated" always shows in full and there's no horizontal scrollbar under the rows.
