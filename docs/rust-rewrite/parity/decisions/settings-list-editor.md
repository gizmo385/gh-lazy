# Settings list fields use one list editor

- Decision: Each list field in Settings (`repositories.additional_repos_to_track`, `repositories.favorites`,
  `pull_requests.additional_suggested_pr_reviewers`) is one focus stop: a boxed list of its items with an
  "+ Add item..." row at the end. The arrow keys move a highlight; `enter` edits the highlighted item in place (or
  starts a new one on the add row), `a` adds one, `d` or `delete` removes the highlighted item, and `shift+up` /
  `shift+down` move it. Save writes the items in the order shown. Python's "Add new item..." input above a
  checklist, where unchecking marks an item for removal on Save and Save writes the items in the order they were
  checked, isn't copied.
- Decided by: Chris, 2026-10-30, that the list editor is reworked as an improvement after parity sign-off (#177,
  #200). The interaction details follow his answers to the questions on #343.
- Link: https://github.com/gizmo385/gh-lazy/issues/343
- Rows: `modals.settings-list-field`

Python's `ListSettingWidget` is an `Input` and a `SelectionList`, two focus stops per field. Its input box is always
shown (three rows and a margin) even when the list is empty, and an empty list still draws an empty bordered box,
so the Repositories tab shows only the first field at 100x30. An item can't be edited, only unchecked and added
again, an unchecked item stays on screen until the modal is reopened, re-checking an item moves it to the end of
what's saved, and the order can't otherwise be changed. A duplicate entry is silently ignored.

The values written are the same type and shape (`list[str]` in `config.json`), so either app reads what the other
writes.
