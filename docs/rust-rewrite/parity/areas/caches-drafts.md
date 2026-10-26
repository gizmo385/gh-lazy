# Local caches and drafts

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

All under `cache.cache_directory`. Repo-scoped names use `full_name` with `/` replaced by `_`. A missing, corrupt or
schema-mismatched file is ignored (logged as a warning) and fresh data is fetched.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `caches-drafts.repos-json` | `repos.json`: list of repository models for the repos table | `lib/cache.py`, `ui/widgets/repositories.py` | M2 | not started | | |
| `caches-drafts.pull-requests-json` | `<owner>_<repo>_pull_requests.json` | `lib/cache.py`, `ui/widgets/pull_requests.py` | M2 | not started | | |
| `caches-drafts.issues-json` | `<owner>_<repo>_issues.json` | `lib/cache.py`, `ui/widgets/issues.py` | M2 | not started | | |
| `caches-drafts.workflow-runs-json` | `<owner>_<repo>_workflow_runs.json` | `lib/cache.py`, `ui/widgets/workflows.py` | M2 | not started | | |
| `caches-drafts.notifications-json` | `notifications.json`: written by both notification tabs, never read | `ui/screens/notifications.py` | M4 | not started | #189 | Candidate `intentional diff`: don't write it (Py bug B29). |
| `caches-drafts.all-or-nothing-load` | All-or-nothing load: one bad entry discards the whole file | `lib/cache.py` `load_models_from_cache` | M2 | not started | | Rust models must accept the Python `model_dump(mode="json")` shape. |
| `caches-drafts.pr-drafts` | PR drafts `pr_drafts/<owner>_<repo>.json`: `repo_full_name`, `title`, `description`, `base_ref`, `head_ref`, `is_draft`, `reviewers` (2-space indent) | `lib/pr_drafts.py` | M3 | not started | #186 | Corrupt draft ignored. Behavior row under [Modals](./modals.md). |
