# The PR Overview shows the pull request's milestone, and `m` searches its pull requests

- Decision: The same as [issue-milestone](./issue-milestone.md), for pull requests. When a pull request belongs to a
  milestone, the PR Overview adds a line under its header: `Milestone: <title> • <open|closed> • due <YYYY-MM-DD> •
  <closed> of <total> closed`, the title a link to the milestone's page, the due date left out when there's none.
  Nothing changes for a pull request without one. With the Overview's content focused, a new Rust-only binding
  `pull_request.milestone` (`m`, "Milestone") opens the Search modal with
  `is:pr milestone:"<title>" repo:<owner>/<name>` and runs it at once. The binding is hidden and does nothing while
  the pull request has no milestone. The Python app has no milestone support.
- Decided by: pending. It follows Chris's answers on #410 (D1 a: the Search modal; D2 a: `m` on the Overview only,
  hidden without a milestone; D3 a: the query names the item type, here `is:pr`), and Chris asked for #417 to go
  ahead on 2026-11-10.
- Link: #417, #410 (the same for issues), #386 and #408 (the Search modal it reuses)
- Rows: `pull-requests.overview-milestone`, `pull-requests.overview-milestone-search`,
  `keybindings.pull-request-milestone`

New behavior, not tied to the Python app (an M5 improvement). It adds rows instead of changing existing ones:
`pull-requests.overview-header` and `pull-requests.overview-focus` stay `parity`, since every pull request in the
shared fixtures has no milestone and draws as Python draws it. The footer and help panel of a milestoned pull
request's Overview gain `m Milestone` after `R Add reaction`; nothing shared with Python shows it.

The line sits right under the header line, before the change summary, as it does on the Issue Overview, and both
Overviews draw it with the same function. `m` is free in the PR Overview's context: merge is `ctrl+m`.

`is:pr` keeps the jump to the pull requests, as `is:issue` does for issues (#410 D3). A milestone's other items are a
query edit away in the modal.
