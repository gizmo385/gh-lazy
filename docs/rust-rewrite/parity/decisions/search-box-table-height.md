# The table below the search box gets only the rows the box leaves

- Decision: While the `/` search box is shown, the table is laid out in the rows left below the box and its margin,
  not at the whole section's height. Every row the cursor reaches is drawn, the view scrolls as the cursor moves, and
  the horizontal scrollbar stays visible.
- Decided by: Chris, 2026-10-25
- Link: Py bug [B40](../py-bugs/B40.md), answered "fix it" in a Claude Code session
- Rows: `searchable-tables.search-box-table-height`

`SearchableDataTable` is a `Vertical` with the search box (3 rows and `margin-bottom: 1`) above the `DataTable`.
Textual 8.2.5's `DataTable` has `max-height: 100%`, a percentage of the container rather than of the rows the box
leaves, so a table with more rows than fit stays as tall as the whole container and the container clips its bottom 4
rows. The table believes those rows are visible: its horizontal scrollbar is clipped away, and `j` moves the cursor
onto hidden rows without scrolling. In the repos section at 160x48, the box leaves the header and 10 rows on screen
while the table thinks 13 body rows show; after 12 presses of `j` no row on screen is highlighted.

Rust's `SearchBox::split` (`crates/lazy-github-tui/src/components/table/search.rs`) gives the table only the
rows below the box, so its body height, and with it the cursor's scrolling, the vertical scrollbar and page sizes,
match what's drawn. The test `the_table_below_the_box_keeps_the_cursor_row_in_view` in
`components/table/search/tests.rs` moves the cursor past the rows Python would clip and checks each is drawn.

What differs on screen, and only with the box shown over a table with more rows than fit:

- Rust draws the horizontal scrollbar when the columns overflow, so one fewer row shows than in Python (9 against 10
  in the repos section at 160x48). The `table-search` scenario's `open` capture therefore checks `cursor` and `focus`
  but not `rows`.
- The vertical scrollbar's thumb reflects the shorter view.
- Moving the cursor past the bottom row on screen scrolls the view, where Python leaves the cursor out of view.
- Opening the box scrolls the table, with the least movement, when the cursor is on one of the bottom rows the box
  now covers, so the cursor's row stays drawn. The same goes for the first frame that draws the box after the table
  was maximized with it open. In Python the cursor stays on its row, out of view. The test
  `opening_the_box_scrolls_a_covered_cursor_row_into_view` pins it.

A table whose rows all fit below the box (a filtered one, usually) is laid out the same in both apps.
