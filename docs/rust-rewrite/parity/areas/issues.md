# Issues

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `issues.table` | Issues table | `ui/widgets/issues.py` `IssuesContainer` | M2 | not started | #210, #172 | |
| `issues.columns` | Columns: "#", "Status" (raw `open`/`closed`), "Author", "Title"; sorted by number desc | `ui/widgets/issues.py` `issue_to_cell`, `on_mount` | M2 | not started | #172 | Keyed by number. Python draws titles as markup (Py bug B46). |
| `issues.filters` | Filters `issues.state_filter`, `issues.owner_filter`; first page shared with PRs, then lazy pages of 30 | `ui/widgets/issues.py` `fetch_more_issues` | M2 | in progress | #211, #172 | Issue lists use `cache.default_ttl`; `list_issues_ttl` is never read. Paging ends at a page with no issues (Py bug B44). A failed later page crashes Python (Py bug B43). |
| `issues.loading` | Loading / error: spinner; cached rows from `<owner>_<repo>_issues.json`; API error leaves cached rows | `ui/widgets/issues.py` `load_cached_issues_for_repo` | M2 | not started | #172 | Error: the PR table's ERROR line, logged once for the shared first page; without a cache the section collapses to ` (empty)`. Py bug B45. |
| `issues.select` | Enter/space selects an issue: details get Overview / Comments, focus moves to details | `ui/widgets/issues.py` `issue_selected`; `ui/screens/primary.py` `MainViewPane.load_issue` | M2 | not started | | |
| `issues.lookup` | `O` lookup issue modal; result added if missing and selected | `ui/widgets/issues.py` `action_lookup_issue` | M2 | not started | | Py bug B12. |
| `issues.edit-from-table` | `E` edit issue from table (updated row replaces the old one); toast error "No issue currently selected" | `ui/widgets/issues.py` `trigger_edit_issue_flow` | M3 | not started | #181 | |
| `issues.user-profile` | `u` opens the author's profile; toast error "No issue currently selected" | `ui/widgets/issues.py` `action_view_user_profile` | M2 | not started | | |
| `issues.details` | Details: issue overview / conversation | `ui/widgets/issues.py`, `ui/widgets/conversations.py` | M2 | not started | | |
| `issues.overview-tab` | Issue Overview tab "Overview": "<Open/Closed> <title> (#N) by <user>", rule, body markdown | `ui/widgets/issues.py` `IssueOverviewTabPane` | M2 | not started | | Py bug B11 (title markup). |
| `issues.overview-edit` | Issue Overview `E` edit issue | `ui/widgets/issues.py` `IssueOverviewTabPane.action_edit_issue` | M3 | not started | #181 | Overview and table are not refreshed afterward. |
| `issues.comments-tab` | Issue Comments tab "Comments": comment blocks, "No comments available", spinner | `ui/widgets/issues.py` `IssueConversationTabPane` | M2 | not started | | Py bug B16. |
| `issues.comments-new-reply` | Comments `n` new comment (appended on success), `r` reply to a comment (appended) | `ui/widgets/issues.py` `new_comment_flow`, `handle_new_comment_added` | M3 | not started | #182 | |
