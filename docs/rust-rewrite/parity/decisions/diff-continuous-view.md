# The Diff tab is one continuous diff with a line cursor

- Decision: The Diff tab shows every unfolded file's hunks as one continuous list of rows, under the file filter: a
  header row per file (its full path and counts), a separator row per hunk, then the hunk's lines. One line cursor
  moves through it, crossing hunk and file boundaries (`j`/`k`), and `J`/`K` jump hunks. There are no hunk sections,
  no hunk pane boxes with their own scroll, and no per-pane `Line: n/m`; one position row under the diff says where
  the cursor is. The inner focus ring is the filter, the diff, then the review form. `M` on the diff maximizes the
  details tab area. Python nests a collapsible per file and per hunk, with a 25-row pane per open hunk.
- Decided by: Chris, 2026-11-01, on #344 (D1 a; proposed by the design agent, 2026-10-31)
- Link: #344, #177 (M5 improvements, Chris 2026-10-30)
- Rows: `diff-viewer.viewer`, `diff-viewer.file-header`, `diff-viewer.hunk-header`, `diff-viewer.hunk-pane`,
  `diff-viewer.line-cursor`, `diff-viewer.hunk-navigation`, `diff-viewer.focus-ring`, `diff-viewer.file-filter`,
  `keybindings.diff-cursor-down`, `keybindings.diff-cursor-up`, `keybindings.diff-next-hunk`,
  `keybindings.diff-previous-hunk`, `diff-viewer.maximize`, `diff-viewer.position-line`, `diff-viewer.body-mouse`

Status: decided; being built. #387 built the continuous body as a component on its own. #388 wired it into the Diff
tab (the focus ring, the filter, `c` on the cursor's line, the mouse, `M` maximizing the tab area), added the rows
`diff-viewer.maximize`, `diff-viewer.position-line` and `diff-viewer.body-mouse`, and moved the rows above to
`intentional diff`; the Diff tab's scenarios are Rust-only (`diff-navigation`, `diff-review`, `diff-unicode`), and
`pr-diff`, `pr-review` and `pr-maximize-diff-form` keep what both apps share (the tab, the review form). #389 added
folding, file jumps and the sticky header ([diff-file-navigation](diff-file-navigation.md)).

Why: reading a pull request in the Python layout takes a focus change per hunk, every hunk opens in its own box with
its own scroll (at most 22 lines), `j` stops dead at the end of a hunk, and nothing moves between files. Two scroll
levels and roughly ten rows of chrome per hunk make small terminals worse. A single list with one cursor, as in
lazygit, tig or `git diff | less`, takes one key per line and keeps the review flow (`c` on the cursor's line, the
review form after the files) as it is. The parity scenarios that walk the Diff tab's focus ring can't share
keystrokes with Python after this, so their Diff steps move to Rust-only scenarios; the shared scenarios keep the
review form, which both apps still reach the same way. That cost ends at cutover (M6), when every scenario becomes
Rust-only.
