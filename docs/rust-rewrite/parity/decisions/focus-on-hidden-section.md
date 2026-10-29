# Hiding the focused section moves focus to the first stop

- Decision: When a palette toggle hides the section (its table or search box) or the command log that has focus,
  the Rust app moves focus to the first stop of the focus ring, the Repositories table (or its search box while
  that's shown), after restoring the layout if the hidden section was maximized. Python leaves nothing focused, and
  a hidden maximized section stays drawn on the hatch until `escape`.
- Decided by: Chris, 2026-10-28 (Py bug B88, the recommendation)
- Link: [comment on #191](https://github.com/gizmo385/gh-lazy/issues/191#issuecomment-6483308596)
- Rows: `main-layout.focus-lost`, `command-palette.toggle-command-log`, `command-palette.toggle-workflows`,
  `command-palette.toggle-issues`, `command-palette.toggle-pull-requests`

In Python, hiding the focused widget drops focus: no pane shows the focus color, the footer shrinks to the screen's
and the app's bindings, table keys and `M` do nothing, and the screen looks as if it stopped responding until a
focus key or `tab`. The Rust main screen has no "nothing focused" state, and the first stop is where Python's next
`tab` lands anyway. Everything else about the toggles (what they show and hide, the requests on showing a section
again, session only) matches Python; the scenario `command-palette-commands` compares only the titles in its
`log-hidden` capture, where the two apps' focus differs.
