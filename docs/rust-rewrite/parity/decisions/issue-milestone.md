# The Issue Overview shows the issue's milestone, and `m` searches its issues

- Decision: When an issue belongs to a milestone, the Issue Overview adds a line under its header:
  `Milestone: <title> • <open|closed> • due <YYYY-MM-DD> • <closed> of <total> closed`, the title a link to the
  milestone's page, the due date left out when there's none. Nothing changes for an issue without one. With the
  Overview's content focused, a new Rust-only binding `issue.milestone` (`m`, "Milestone") opens the Search modal
  with `is:issue milestone:"<title>" repo:<owner>/<name>` and runs it at once, so the milestone's issues list in the
  modal's results table, open in Details with `enter`, and the query can be refined there. The binding is hidden and
  does nothing while the issue has no milestone. The Python app has no milestone support.
- Decided by: proposed by the design agent, 2026-11-09, on #410; pending Chris's answers to D1-D3 there.
- Link: #410, #417 (the same for pull requests), #386 and #408 (the Search modal it reuses)
- Rows: `issues.overview-milestone`, `issues.overview-milestone-search`, `keybindings.issue-milestone`

New behavior, not tied to the Python app (an M5 improvement). It adds rows instead of changing existing ones:
`issues.overview-tab` stays `parity`, since every issue in the shared fixtures has no milestone and draws as Python
draws it. The footer and help panel of a milestoned issue's Overview gain `m Milestone` after `E Edit Issue`; nothing
shared with Python shows it.

Reusing the Search modal instead of a dedicated milestone screen (D1 on #410) keeps one results table, one way to
open a hit (#405's `OpenInRepo`) and one session: the jump is a saved-query run with a computed query. GitHub's
`milestone:` qualifier matches the title exactly (case-insensitively) and has no escape for `"`, so a title with a
double quote can't be searched; the quote is dropped and the search finds nothing, which the modal's status line
says.
