# The New and Edit issue dialogs fit inside a short or narrow terminal

- Decision: In a terminal too small for them, the New issue and Edit issue dialogs shrink to fit the screen
  (`frame_rect`) with their right border drawn on every row, and their forms scroll inside the dialog. Python lets
  them run past the screen's right edge, gives Edit issue a 6-line editor that makes its frame taller than the screen
  at 48x16 (the modal screen itself then scrolls), and at 100x30 draws Edit issue taller than Rust does.
- Decided by: Chris, 2026-10-30, on #341: keep fitting them.
- Link: https://github.com/gizmo385/gh-lazy/issues/341#issuecomment-6534467711
- Rows: `modals.issue-dialogs-small-terminal`

A dialog cut off at the screen's edge is a Textual layout artifact rather than designed behavior (PLAN §10.8). The
form's fields, their order, the opening scroll position and the buttons match Python's. The Create PR, Edit PR and
New comment dialogs, whose frames run past the edge in Python, are cut the same way in Rust (#334).
