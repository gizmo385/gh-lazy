# Compose dialogs size themselves to what's typed

- Decision: New comment and its replies, New issue, Edit issue, Create PR, Edit PR and the diff's Add comment share
  one compose sheet instead of Python's fixed layouts. The sheet is as wide as `min(100, screen - 4)` (Add comment 80)
  and as tall as its content, up to the screen less the footer and a row of margin: a one-row heading, a quote capped
  with "⋯ N more lines", one-row fields with their labels on the left, a `Write  Preview` tab row, and the body editor,
  which grows from 3 (comments) or 5 (descriptions) rows with its wrapped text and then scrolls. Create/Edit PR's
  Reviewers collapse to a summary row until focused. There are no button rows: every compose dialog shows its footer
  (Create/Edit PR included) with `ctrl+s` (Rust-only `compose.submit`) and `shift+enter` (`modal.submit`) to submit,
  `ctrl+t` (`compose.toggle_preview`) to switch to a markdown preview in place, and `ctrl+o` (`compose.expand`) to
  make the sheet full-screen. `escape` in a field leaves it; `escape` or `q` on the sheet closes the dialog. What
  each dialog sends doesn't change.
- Decided by: proposed by the design agent, 2026-11-09, on #419; pending Chris's answers to D1-D7 there.
- Link: #419 (tracking), #420, #421, #422, #423
- Rows: `modals.compose-sheet`, `modals.compose-preview`, `modals.compose-expand`, `keybindings.compose-submit`, `keybindings.compose-toggle-preview`, `keybindings.compose-expand`

Python's compose dialogs have fixed sizes: New comment is 100x50 (taller than a 48-row screen) with a 40% editor and a
40% preview, New issue's editor is 75% of an 80%-tall dialog, Edit issue is 100x30 with a 15-row editor of which 6
rows show, Create/Edit PR is 80% tall over 59 rows of content with a 30-row editor, and Add comment is 80x31 to write
one line. An empty comment uses one row of 45; at 100x30, New comment and Create PR run past the screen's edges and
their buttons are below the fold. The sheet follows the text instead (7 rows for an empty comment, 15 for an empty
Create PR, nearly the whole screen for a long body) and fits every screen.

When each sub-issue lands, these existing rows become `intentional diff` linking this file, keeping their notes on
wording, requests and toasts: `modals.new-comment` (#420); `modals.new-issue`, `modals.edit-issue` (#421);
`modals.create-pr-branches`, `modals.create-pr-title-description`, `modals.pr-reviewers`, `modals.edit-pr`,
`modals.dialog-footer` (#422); `diff-viewer.add-comment` (#423). The requests each dialog sends stay Python's, so the
parity scenarios keep comparing writes; only the captures of these dialogs stop comparing their footer and focus.

The alternatives weighed on #419 were keeping Python's anatomy with elastic heights (less change, but an empty Create
PR still scrolls at 100x30) and a docked, full-width composer (a new placement for the modal stack). `ctrl+s` is
there because `shift+enter` needs the kitty keyboard protocol (`terminal-caveats.shift-enter`) and the button rows it
replaces were the fallback; crossterm's raw mode turns flow control off, so `ctrl+s` arrives.
