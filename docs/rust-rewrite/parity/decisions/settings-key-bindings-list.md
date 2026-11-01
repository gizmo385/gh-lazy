# Settings Key Bindings is a grouped list with an explicit capture mode

- Decision: The Settings Key Bindings tab is one focus stop (D1 A): a list of every rebindable binding, one line
  each, grouped by area (General, Focus, Tables, Repositories, Pull requests, Issues, Conversations, Diff viewer,
  Workflows, Notifications, Dialogs), with the action, its keys and its id, and a detail line for the highlighted
  binding. The arrow keys move a highlight; `enter` waits for the next key and replaces the binding's keys with it,
  `a` adds the next key alongside them (D6 A), `escape` cancels the wait, and `r` resets the binding to its default.
  These editor keys are fixed, under internal binding ids that `bindings.overrides` can't name and the tab doesn't
  list (D3 A). A binding whose keys parse to its default keys, in order, is the default (D5 A): no marker, and Save
  removes its override. A changed binding is marked, and a binding whose key clashes with another binding active at
  the same time is flagged (#382, D4 A). `bindings.overrides` in `config.json` keeps its format: id to Textual key
  names, comma-separated. Python's one focusable row per binding, sorted by id, which records any key the moment it
  has focus, isn't copied.
- Decided by: Chris, 2026-10-30, that the tab is reworked as an improvement after parity sign-off (#177, #200);
  Chris, 2026-11-01, on #342, the recommended option of each question: D1 A (one focus stop, a grouped one-line
  list with explicit capture), D3 A (the editor's keys fixed under internal ids; across the app, navigation keys
  users may want to change get rebindable Rust-only ids, and editor-internal keys stay fixed), D4 A (conflicts
  flagged in context, only when one side was changed, inline plus a non-blocking toast on Save), D5 A (compare
  parsed keys, which revisits Py bug B93) and D6 A (`a` adds a key, `enter` replaces).
- Link: https://github.com/gizmo385/gh-lazy/issues/342, #381, #382
- Rows: `modals.settings-key-rebinding`

Python's `KeySelectionInput` rows each take the next key as soon as they have focus. So the arrow keys, `j`, `k`,
`enter` and `q` rebind the row instead of moving or closing, and only `tab` and `shift+tab` move on, through about
60 rows. The focused row grows from two rows to three, so the rows below it jump at every step. At 100x30 four
bindings are visible. A binding with two default keys (`q, escape`) loses one as soon as a key is recorded, there's
no sign of which bindings are changed, nothing warns when a key is taken twice, and `escape`, which resets, can't
cancel.

The rework keeps what the tab is for (see and change every `bindings.overrides` entry, search them with `/`, Save
and Cancel as before) and reads and writes the same overrides, so a `config.json` written by either app loads in
the other. Non-ASCII keys are still saved as the character itself
([key-names-non-ascii-literal](./key-names-non-ascii-literal.md)). It replaces the layout question that
[key-binding-values-plain](./key-binding-values-plain.md) left to #342.

One rule changes with it: a binding whose keys parse to its default keys counts as the default, so capturing `?` on
"Keybindings" removes the override instead of saving `question_mark`. That departs from Py bug B93's "keep parity"
answer, which was about Python's tab (D5 on #342, answered A).
