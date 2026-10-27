# After a short page, lazy loading waits for the next cursor move

- Decision: While a page is in flight, the Rust table records only the highest cursor and re-checks it once when the
  page lands. If that page was short and the cursor is still near the end, the next page is requested on the next
  cursor move. Python keeps fetching.
- Decided by: Chris, 2026-10-26, in a Claude Code session
- Link: this decision; no Py bug
- Rows: `searchable-tables.lazy-loading`

Python's `LazilyLoadedDataTable` serializes loads with `@work` and `fetch_lock` and re-checks every queued highlight
after each page lands, so a run of short pages (an issues-endpoint page filtered down to PRs or issues, say) can chain
several fetches without the user moving. Rust's hook in `components/table/lazy.rs` waits for the user instead.
Everything else about lazy loading (the 5-row threshold, 30 per page, stopping after an empty API page) is meant to
match.
