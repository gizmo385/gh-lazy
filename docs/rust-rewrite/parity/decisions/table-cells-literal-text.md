# Table cells draw GitHub's text literally

- Decision: Cell text that comes from GitHub (PR and issue titles, logins, workflow names and run titles) is drawn
  as typed, not parsed as Rich markup. Sorting and the `/` search use the raw strings, as in Python.
- Decided by: Chris, 2026-10-26, in a Claude Code session
- Link: Py bug [B46](../py-bugs/B46.md), answered "follow the recommendation"
- Rows: `searchable-tables.cell-markup`

Textual's `DataTable` runs every `str` cell through `Text.from_markup`, so a lowercase `[tag]` in a title (`[docs]`,
`[ci]`, `fix [x] box`) vanishes or styles the text, and an unmatched `[/tag]` raises `MarkupError` while the row is
drawn. Rust shows the text as GitHub returns it. The repos table's `[green]★[/]` is the app's own markup and isn't
affected. Text without a lowercase bracket tag looks the same in both apps.
