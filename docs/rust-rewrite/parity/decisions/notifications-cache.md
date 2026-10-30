# The Notifications modal opens on the saved first page, then refreshes it

- Decision: The Rust app saves the Notifications modal's last live first page (`GET /notifications?all=true&page=1`,
  up to 50 threads) to `notifications.json` in `cache.cache_directory`, in Python's file shape, with the threads
  marked read in the modal since then flipped to `unread: false`. When the modal opens and that file holds at least
  one thread, both tabs show the saved threads at once, with the active tab's table focused and the line "Showing
  saved notifications - refreshing..." under the heading, instead of the loading overlay. The live first page then
  replaces them: the cursor stays on its thread, the tab follows the live rows unless the user picked one, and marks
  made meanwhile carry over. If the live page fails, the saved threads stay, the line becomes "Showing saved
  notifications - refresh failed" and nothing pages. With no file, an empty one or a bad one, the modal loads as
  Python's does. The 60 s unread poll is unchanged.
- Decided by: Chris, 2026-10-30, asked for a cache the modal reads (#332). The storage, stale-row and poll choices
  are pending his answers on #332; this file and the rows follow the recommendations there until he answers.
- Link: https://github.com/gizmo385/gh-lazy/issues/332
- Rows: `modals.notifications-cached`, `modals.notifications-cached-refresh`, `caches-drafts.notifications-json`

Python fetches the first page every time the modal opens and shows the loading overlay until it answers, so the
modal is slow to show anything. It also writes `notifications.json` from both tabs and never reads it (Py bug B29),
which [no-notifications-json](./no-notifications-json.md) answered by not writing it at all. This reverses that: the
file is written again, and read.

Keeping Python's name and shape (one JSON array of `Notification.model_dump(mode="json")`, so the subject's kind is
`subject_type`) means `clear-cache` removes it and switching apps is harmless: Python overwrites the file with one
tab's rows and never reads it, and Rust shows whatever readable threads it finds as the saved rows until the live
page lands. Python's own model couldn't read the file back (its `type` alias has no `populate_by_name`); Rust
accepts both `subject_type` and GitHub's `type`.

The saved rows are a preview, not a source of truth, so nothing the user does on them can mislead: opening a saved
row opens the same repo or pull request the live row would, marking a saved row read is harmless if GitHub already
dropped it, and a mark made before the live page lands is applied to the live rows, so the live page can't bring
back a thread the user just marked. The status bar's count stays the poll's.
