# Closing a lookup dialog cancels the lookup in flight

- Decision: While a repository, issue or pull request lookup is in flight, the dialog stays responsive. `modal.close`
  (and Cancel) closes it at once, cancels the request and drops a late answer, so nothing is selected. A second
  submit while one is in flight is ignored. Typing and focus moves work as usual.
- Decided by: Chris, 2026-10-26, in a Claude Code session
- Link: Py bug [B51](../py-bugs/B51.md), answered "follow the recommendation"
- Rows: `modals.lookup-in-flight`

Python awaits the lookup inside the screen's message handler, so every key that reaches the dialog waits in its
queue until GitHub answers, then runs against whatever is on top, normally the main screen after a dismiss. A user
who presses escape because the lookup is slow can't cancel it. Since the apps differ on purpose, no parity scenario
compares them; unit tests in `modal/lookup/tests.rs` pin the Rust side.
