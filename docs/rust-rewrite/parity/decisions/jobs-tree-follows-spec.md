# The jobs tree follows its spec where Textual's `Tree` quirks differ

- Decision: Where Textual 8.2.5's `Tree` behaves oddly because of its hidden root and its clamping of `cursor_line`,
  the Rust jobs tree follows its spec (#242): `shift+left` at the top level and `shift+right`/`shift+down` with no
  next node do nothing; a collapse that hides the cursor's node clears the cursor; `space` with no cursor does
  nothing.
- Decided by: Chris, 2026-10-26, in a Claude Code session
- Link: this decision; no Py bug
- Rows: `workflows.jobs-tree-keys`

In Textual, `shift+left` at the top level and `shift+right`/`shift+down` with no next node send the cursor to the
first line; a collapse that hides the cursor's node leaves the cursor at that node's old line index, clamped to the
last line, so it lands on an unrelated node; and `space` with no cursor toggles the last node. None of these is
something a user would expect, and the tree's other keys and clicks are meant to match.
