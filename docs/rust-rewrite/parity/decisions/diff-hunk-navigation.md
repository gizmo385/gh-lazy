# `J`/`K` move between hunks in the diff viewer

- Decision: In the Diff tab, `J` (`diff.next_hunk`) and `K` (`diff.previous_hunk`) move to the next or previous hunk and
  expand it. In Python they do nothing (Py bug B8), so no scenario can match the two apps.
- Decided by: Chris, 2026-10-27, in a Claude Code session
- Link: [Py bug B8](../py-bugs/B08.md)
- Rows: `diff-viewer.hunk-navigation`, `keybindings.diff-next-hunk`, `keybindings.diff-previous-hunk`

Python binds `J`/`K` on the diff viewer, but the actions target widgets inside collapsed sections that can't take focus,
so the keys have no effect. Rust implements what the bindings describe (#236). The unit tests in
`crates/lazy-github-tui/src/screens/main/pr_details/diff/tests.rs` cover the movement; the parity harness can't, since
Python never moves.
