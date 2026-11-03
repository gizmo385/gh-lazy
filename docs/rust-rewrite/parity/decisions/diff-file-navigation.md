# The diff jumps between files and folds them

- Decision: In the Diff tab, `]` and `[` (`diff.next_file`, `diff.previous_file`) move between file headers,
  `enter` (`diff.toggle`) folds or unfolds the cursor's file, `z` (`diff.fold_all`) folds or unfolds every shown
  file, `g` and `G` (`diff.first_line`, `diff.last_line`) go to the first and last row, and the cursor's file header
  sticks to the top while you're inside it. These are binding ids Python doesn't have.
- Decided by: Chris, 2026-11-01 (#344, D2 a: folding every file with `z` is the file list; D3 a: rebindable
  Rust-only `diff.*` ids after Python's in the registry, listed in `RUST_ONLY_IDS` in `tests/registry.rs`, with the
  rule in PLAN §5: navigation keys get rebindable Rust-only ids, editor-internal keys stay fixed internal ids)
- Link: #344, #389
- Rows: `diff-viewer.file-navigation`, `diff-viewer.file-folding`, `diff-viewer.sticky-file-header`,
  `keybindings.diff-next-file`, `keybindings.diff-previous-file`, `keybindings.diff-toggle`,
  `keybindings.diff-fold-all`, `keybindings.diff-first-line`, `keybindings.diff-last-line`

Status: built in #389. Fold state lasts while the pull request is shown; a new selection or a reload starts over,
every file unfolded but deleted ones. On the body, `up` and `down` move the line cursor as `j` and `k` do, the view
following (Chris, 2026-11-01), and a wheel notch scrolls two rows, as the app's other scrolls do.

Why: the continuous diff (decision [diff-continuous-view](diff-continuous-view.md)) puts every file in one list, so a
long pull request needs a way to skip a file and to see what's in it at a glance. Folding everything gives the file
list without a picker modal or a sidebar (D2 options b and c), and one key per file moves through it. Python's
registry has no ids for these keys, and PLAN §5 asks for Python's ids; Rust-only ids keep them rebindable through
`bindings.overrides` like every other navigation key (Python ignores an override it doesn't know), listed in Settings'
Key Bindings tab and the help panel. Python has no file jumps and folds only through each file's collapsible title.
