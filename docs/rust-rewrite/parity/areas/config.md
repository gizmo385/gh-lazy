# Config

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

Every field in `lib/config.py` must load from an existing `config.json` and round-trip on save. File:
`~/.config/lazy-github/config.json`, written with 4-space indent. Unknown fields are ignored. Values written by the Python
Settings modal can be strings where a number is expected (Py bug B2), so the loader accepts both.

| Id | Section | Field | Default | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `config.appearance-theme` | `appearance` | `theme` | `"textual-dark"` | `lib/config.py` `AppearanceSettings` | M0 | not started | #64 | Unknown name → default. Applied in #77. |
| `config.appearance-show-command-log` | `appearance` | `show_command_log` | `false` | `lib/config.py` | M0 | not started | #64 | Used by #74/#80. |
| `config.appearance-show-workflows` | `appearance` | `show_workflows` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `config.appearance-show-issues` | `appearance` | `show_issues` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `config.appearance-show-pull-requests` | `appearance` | `show_pull_requests` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `config.notifications-enabled` | `notifications` | `enabled` | `false` | `lib/config.py` `NotificationSettings` | M0 | not started | #64 | |
| `config.notifications-mark-notification-as-read-when-selected` | `notifications` | `mark_notification_as_read_when_selected` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `config.bindings-overrides` | `bindings` | `overrides` | `{}` | `lib/config.py` `BindingsSettings` | M0 | not started | #64 | Map of binding id → key string; applied in #73. |
| `config.repositories-additional-repos-to-track` | `repositories` | `additional_repos_to_track` | `[]` | `lib/config.py` `RepositorySettings` | M0 | not started | #64 | |
| `config.repositories-favorites` | `repositories` | `favorites` | `[]` | `lib/config.py` | M0 | not started | #64 | |
| `config.pull-requests-state-filter` | `pull_requests` | `state_filter` | `"all"` (`all`/`open`/`closed`) | `lib/config.py` `PullRequestSettings`, `lib/constants.py` `IssueStateFilter` | M0 | not started | #64 | |
| `config.pull-requests-owner-filter` | `pull_requests` | `owner_filter` | `"all"` (`all`/`mine`) | `lib/config.py`, `lib/constants.py` `IssueOwnerFilter` | M0 | not started | #64 | |
| `config.pull-requests-preferred-merge-method` | `pull_requests` | `preferred_merge_method` | `"squash"` (`merge`/`squash`/`rebase`) | `lib/config.py` `MergeMethod` | M0 | not started | #64 | |
| `config.pull-requests-additional-suggested-pr-reviewers` | `pull_requests` | `additional_suggested_pr_reviewers` | `[]` | `lib/config.py` | M0 | not started | #64 | |
| `config.pull-requests-preload-pull-request-for-current-commit` | `pull_requests` | `preload_pull_request_for_current_commit` | `false` | `lib/config.py` | M0 | not started | #64 | |
| `config.pull-requests-pull-request-template` | `pull_requests` | `pull_request_template` | `null` (path) | `lib/config.py` | M0 | not started | #64 | |
| `config.issues-state-filter` | `issues` | `state_filter` | `"all"` | `lib/config.py` `IssueSettings` | M0 | not started | #64 | |
| `config.issues-owner-filter` | `issues` | `owner_filter` | `"all"` | `lib/config.py` | M0 | not started | #64 | |
| `config.cache-auth-cache-duration` | `cache` | `auth_cache_duration` | `86400` | `lib/config.py` `CacheSettings` | M0 | not started | #64 | Seconds. |
| `config.cache-auth-last-checked` | `cache` | `auth_last_checked` | `null` (ISO datetime) | `lib/config.py` | M0 | not started | #64 | Python writes naive local time. A `"None"` string can appear (Py bug B2). |
| `config.cache-cache-directory` | `cache` | `cache_directory` | `~/.config/lazy-github/.cache` | `lib/config.py` | M0 | not started | #64 | Absolute path is written. |
| `config.cache-default-ttl` | `cache` | `default_ttl` | `600` | `lib/config.py` | M0 | not started | #64 | |
| `config.cache-list-repos-ttl` | `cache` | `list_repos_ttl` | `86400` | `lib/config.py` | M0 | not started | #64 | |
| `config.cache-list-issues-ttl` | `cache` | `list_issues_ttl` | `3600` | `lib/config.py` | M0 | not started | #64 | Never read by the Python app. |
| `config.core-first-start` | `core` | `first_start` | `true` | `lib/config.py` `CoreConfig` | M0 | not started | #64 | Rust writes `false` after the gh check (no picker). |
| `config.core-logfile-path` | `core` | `logfile_path` | `~/.config/lazy-github/lazy_github.log` | `lib/config.py` | M0 | not started | #64, #66 | |
| `config.core-logfile-max-bytes` | `core` | `logfile_max_bytes` | `5000000` | `lib/config.py` | M0 | not started | #64, #66 | |
| `config.core-logfile-count` | `core` | `logfile_count` | `5` | `lib/config.py` | M0 | not started | #64, #66 | |
| `config.api-base-url` | `api` | `base_url` | `"https://api.github.com"` | `lib/config.py` `ApiConfig` | M0 | not started | #64 | Preserved; decide whether it maps to `gh --hostname`. |
| `config.api-client-type` | `api` | `client_type` | `"RAW_HTTP"` (ignored, gh only) | `lib/config.py`, `lib/github/backends/protocol.py` `BackendType` | M0 | intentional diff (gh only) | #64 | Preserved on save, not acted on. |
