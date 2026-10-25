# Searchable tables (shared component)

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `searchable-tables.vim-cursor` | Row cursor table with vim keys (see `common.table.*` bindings) | `ui/widgets/common.py` `_VimLikeDataTable` | M1 | not started | #78 | |
| `searchable-tables.search` | `/` reveals a "Search..." input above the table; Enter filters rows by case-insensitive substring over the row's cell text; empty query restores all rows; focus returns to the table | `ui/widgets/common.py` `SearchableDataTable.action_focus_search`, `handle_submitted_search` | M1 | not started | #78 | Filter is applied on submit, not per keystroke. Matches the raw cell text, including markup such as `[green]★[/]`. |
| `searchable-tables.search-hide-on-blur` | Search input hides again when it loses focus while empty | `ui/widgets/common.py` `ToggleableSearchInput._on_blur` | M1 | not started | #78 | |
| `searchable-tables.sort-on-insert` | Rows sorted by a column after each insert (PRs/issues by number desc, runs by time desc, repos by favorite) | `ui/widgets/common.py` `SearchableDataTable.add_item`, `sort_table` | M1 | not started | #78 | |
| `searchable-tables.replace-on-readd` | Re-adding an existing key replaces the row | `ui/widgets/common.py` `SearchableDataTable.add_item` | M1 | not started | #78 | |
| `searchable-tables.lazy-loading` | Lazy loading: when the cursor is within 5 rows of the end, fetch the next page (30 per page) and append; stop after an empty page | `ui/widgets/common.py` `LazilyLoadedDataTable` | M2 | not started | | Hook belongs in #78's component; fetchers land in M2. |
| `searchable-tables.row-cache` | Table rows written to the per-repo cache after each fetch and restored before the live fetch | `ui/widgets/common.py` `initialize_from_cache`, `save_to_cache` | M2 | not started | | See [Local caches and drafts](./caches-drafts.md). |
