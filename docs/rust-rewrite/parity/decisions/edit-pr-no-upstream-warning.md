# Edit PR doesn't warn about the current directory's upstream

- Decision: The Rust Edit PR dialog never runs the upstream check, so it never shows "Warning: Current local branch
  has no configured upstream". Create PR keeps the warning (`modals.create-pr-upstream-warning`).
- Decided by: Chris, 2026-10-28 (Py bug B85)
- Link: [comment on #283](https://github.com/gizmo385/gh-lazy/issues/283#issuecomment-6481326964)
- Rows: `modals.edit-pr`

Python's `CreateOrEditPullRequestContainer.on_mount` runs `ensure_directory_branch_has_configured_upstream` in both
modes. An existing PR's base and head are fixed and shown disabled, and the branch checked out in the current
directory has nothing to do with them, so in Edit PR the warning is misleading. Everything else in Edit PR follows
Python, with the Py bug answers its row lists; the scenario `pr-edit` runs outside a git repo, where Python shows no
warning either.
