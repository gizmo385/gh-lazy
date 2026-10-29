# The palette lists LazyGithub's commands when it opens, in a fixed order

- Decision: With an empty query, the Rust command palette lists Textual's system commands and then LazyGithub's own
  commands, where Python lists only the system ones (Py bug B87, the alternative chosen). Hits with equal scores
  come in one fixed order, LazyGithub's commands first and then the system ones, each in its declared order, where
  Python's order changes from run to run (Py bug B86, the recommendation).
- Decided by: Chris, 2026-10-28
- Link: [comment on #191](https://github.com/gizmo385/gh-lazy/issues/191#issuecomment-6483308596)
- Rows: `command-palette.palette`

Python's `MainScreenCommandProvider` has no `discover`, so someone who opens the palette to see what it can do never
sees the app's own commands ("Toggle Command Log", "Toggle Workflows", "Toggle Issues", "Toggle Pull Requests",
"Change Settings", "Debug Info", "Open Bug Report", "Refresh notifications") until they type something that matches
one. The Rust palette shows them after the system commands. Textual keeps its providers in a `set`, so ties between
LazyGithub's and Textual's hits come in an order that depends on memory addresses; Rust can't reproduce that and
picks one of the orders Python sometimes shows.

The scenario `command-palette` checks the system commands in the opening list with `[expect]` and leaves the rest out
of the cross-app check. The opening list is also shorter than Python's by "Screenshot"
([palette-screenshot-deferred](./palette-screenshot-deferred.md)) and, until #192, by "Theme"
(`command-palette.theme`, its own row).
