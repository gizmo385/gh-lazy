# Toasts

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

Exact texts, so parity scenarios can assert them. Rows above say when each fires. Toasts raised inside a modal are
listed with that modal under [Modals](./modals.md).

| Id | Text | Severity / title | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `toasts.settings-updated` | "Settings updated" | information | `ui/app.py` | M4 | not started | #193 | |
| `toasts.view-maximized` | "Current view maximized. Press escape to return." | information / "View Maximized" | `ui/app.py` | M1 | parity | #75, #79 | Once per session. Emitted by #75; #79 draws it; scenarios `maximize` (`max`) and `toast-maximize`. Py bug B32. |
| `toasts.select-repository-first` | "Please select a repository first!" | error / "Cannot open new pull request" | `ui/screens/primary.py` | M3 | in progress | #258, #278 | Same title for issues (Py bug B14). |
| `toasts.error-fetching-pr` | "Error fetching pull request!" | error / "API Error" | `ui/screens/primary.py` | M2 | parity | #231 | Rust (#231): `screens/main/mod.rs` `pull_request_failed`. |
| `toasts.opening-for-link` | "Opening pull request for link", "Opening issue for link", "Opening repository for link" | information | `ui/screens/primary.py` | M4 | not started | #197 | |
| `toasts.opening-for-notification` | "Opening repository for notification", "Opening pull request for notification" | information | `ui/screens/primary.py` | M4 | not started | #190 | |
| `toasts.notifications-login` | "Cannot load notifications - please login to the gh CLI: gh auth login" | error / "Failed to Load Notifiations" | `ui/screens/primary.py` | M1 | parity | #81, #79 | Py bug B14: Rust's title is "Failed to Load Notifications". #79 draws it; scenario `command-log` (`shown`). |
| `toasts.bug-report-error` | "Could not load LazyGithub repo! Please open the bug report on Github" | error / "Error opening bug report" | `ui/screens/primary.py` | M4 | not started | #195 | |
| `toasts.nothing-selected` | "No repository currently selected" / "No pull request currently selected" / "No issue currently selected" | error | `ui/widgets/repositories.py`, `ui/widgets/pull_requests.py`, `ui/widgets/issues.py` | M2 | parity | #225, #226 | Rust (#225): the repos message (scenario `user-profile-no-repo`). Rust (#226): the PR and issue messages (scenario `user-profile-authors`). |
| `toasts.edit-pr-already-merged` | "This PR has already been merged!" | warning | `ui/widgets/pull_requests.py` | M3 | not started | #185, #283 | From the PR table (after fetching the PR) and the PR Overview. Scenario `pr-edit`. |
| `toasts.merge-already-merged` | "PR has already been merged!" | warning / "Already Merged" | `ui/widgets/pull_requests.py` | M3 | not started | #183, #262 | |
| `toasts.pr-merged` | "Pull request N merged. Note some cached information on the UI may be out of date." | information / "PR Merged" | `ui/widgets/pull_requests.py` | M3 | not started | #183, #262 | |
| `toasts.pr-merge-failed` | "Pull request N could not be merged" | error / "Error Merging PR" | `ui/widgets/pull_requests.py` | M3 | not started | #183, #262 | |
| `toasts.comment-empty` | "Comment cannot be empty!" | warning | `ui/widgets/split_diff_viewer.py` | M3 | in progress | #187, #272 | |
| `toasts.comment-review-added` | "New comment added", "New review created!" | information | `ui/widgets/split_diff_viewer.py` | M3 | not started | #187, #272, #273 | |
| `toasts.review-status-missing` | "Please select a status for the new review!" | error | `ui/widgets/split_diff_viewer.py` | M3 | not started | #187, #273 | |
| `toasts.rerun-started` | "Workflow run queued for re-run", "Failed jobs queued for re-run" | information / "Re-run Started" | `ui/widgets/workflow_run_details.py` | M3 | not started | #188, #265 | |
| `toasts.rerun-failed` | "Failed to re-run workflow", "Failed to re-run failed jobs" | error / "Error" | `ui/widgets/workflow_run_details.py` | M3 | not started | #188, #265 | |
| `toasts.workflow-nothing-to-trigger` | "No repository selected", "No workflows found in this repository" | warning | `ui/widgets/workflows.py` | M3 | not started | #188, #264 | |
| `toasts.workflow-triggered` | "Successfully triggered workflow" | information | `ui/widgets/workflows.py` | M3 | not started | #188, #264 | Py bug B68. |
| `toasts.no-links-visible` | "No links visible" | information | `ui/widgets/link_hints.py` | M4 | not started | #196 | |
| `toasts.opening-externally` | "Opening <url> externally" | information | `ui/widgets/link_hints.py` | M4 | not started | #196 | |
