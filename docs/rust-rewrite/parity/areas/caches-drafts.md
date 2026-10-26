# Local caches and drafts

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

All under `cache.cache_directory`. Repo-scoped names use `full_name` with `/` replaced by `_`. A missing, corrupt or
schema-mismatched file is ignored (logged as a warning) and fresh data is fetched.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `caches-drafts.repos-json` | `repos.json`: list of repository models for the repos table | `lib/cache.py`, `ui/widgets/repositories.py` | M2 | not started | #227 | Read at startup, under the repos table's loading overlay, and the current-directory check runs on the cached rows; rewritten after the list (and tracked repos) arrive, which replace the rows, and after a lookup or a profile pick adds a repo. A failed list leaves the cached rows and the file as they were. |
| `caches-drafts.pull-requests-json` | `<owner>_<repo>_pull_requests.json` | `lib/cache.py`, `ui/widgets/pull_requests.py` | M2 | not started | #228 | Read when a repo is selected with the section shown, under the loading overlay; rewritten from the whole table after the first page, each later page and each added row, under the selected repo. Py bug B53. Scenario `table-caches`. |
| `caches-drafts.issues-json` | `<owner>_<repo>_issues.json` | `lib/cache.py`, `ui/widgets/issues.py` | M2 | not started | #228 | As `caches-drafts.pull-requests-json`. Needs #172. Py bug B53. Scenario `table-caches`. |
| `caches-drafts.workflow-runs-json` | `<owner>_<repo>_workflow_runs.json` | `lib/cache.py`, `ui/widgets/workflows.py` | M2 | not started | #228 | As `caches-drafts.pull-requests-json`; also read when the Workflows section is shown again (`action_toggle_ui`). Needs #173. Py bug B53. Scenario `table-caches`. |
| `caches-drafts.notifications-json` | `notifications.json`: written by both notification tabs, never read | `ui/screens/notifications.py` | M4 | not started | #189 | Candidate `intentional diff`: don't write it (Py bug B29). |
| `caches-drafts.all-or-nothing-load` | All-or-nothing load: one bad entry discards the whole file | `lib/cache.py` `load_models_from_cache` | M2 | not started | #227 | Rust models must accept the Python `model_dump(mode="json")` shape. |
| `caches-drafts.file-format` | Cache file format: one JSON array of the models as Pydantic's `model_dump(mode="json")` writes them (every field, `null` for none, timestamps as `YYYY-MM-DDTHH:MM:SSZ`), written compactly by `json.dumps`; the directory is created on save | `lib/cache.py` `save_models_to_cache` | M2 | not started | #227 | Byte-equal output isn't required, only the same JSON values, so either app reads the other's files. A golden manifest (`parity/goldens/table-caches.toml`, #227, #228) checks both directions. |
| `caches-drafts.pr-drafts` | PR drafts `pr_drafts/<owner>_<repo>.json`: `repo_full_name`, `title`, `description`, `base_ref`, `head_ref`, `is_draft`, `reviewers` (2-space indent) | `lib/pr_drafts.py` | M3 | not started | #186 | Corrupt draft ignored. Behavior row under [Modals](./modals.md). |
