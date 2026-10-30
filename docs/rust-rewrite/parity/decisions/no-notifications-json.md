# The Rust app doesn't write `notifications.json`

- Decision: The Rust Notifications modal writes no `notifications.json` cache. Python writes it from both
  notification tabs and never reads it back.
- Decided by: Chris, 2026-10-28 (Py bug B29, the recommendation)
- Link: [comment on #293](https://github.com/gizmo385/gh-lazy/issues/293#issuecomment-6484757518)
- Rows: `caches-drafts.notifications-json`

Nothing in the Python app reads the file: the modal fetches its pages from GitHub each time it opens, and the unread
count comes from its own request. Writing it only leaves a stale copy of the user's notifications on disk. Nothing the
user sees changes; a unit test checks that the Rust app writes nothing.

Superseded by [notifications-cache](./notifications-cache.md) (#332, 2026-10-30): Chris asked for a cache the modal
reads, so the Rust app writes `notifications.json` again, in Python's shape, and reads it when the modal opens. He
chose the on-disk file (D1), so this decision no longer applies.
