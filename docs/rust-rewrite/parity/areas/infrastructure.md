# Infrastructure (M0 and shell plumbing)

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `infrastructure.workspace` | Cargo workspace and crate skeletons | n/a | M0 | not started | #62 | |
| `infrastructure.ci` | CI for `rust-main` (fmt, clippy, tests on Linux; macOS dropped for cost, PLAN §7) | `.github/workflows/code-checks.yaml` (Python equivalent) | M0 | not started | #63 | |
| `infrastructure.config-load-save` | Config load/save compatible with `config.json` (see [Config](./config.md)) | `lib/config.py` `Config.load_config`, `Config.save` | M0 | not started | #64, #159 | Must also accept values the Python Settings modal wrote as strings (Py bug B2). |
| `infrastructure.gh-client` | `Github` trait, `gh` CLI client, auth check | `lib/github/client.py`, `lib/github/backends/cli.py`, `lib/github/auth.py` `is_logged_in_to_cli` | M0 | not started | #65 | |
| `infrastructure.gh-timeout` | Every `gh` call has a time limit (auth 20 s, request 30 s, paginate 180 s, per-request override); on expiry the child is killed and the caller gets `GhError::Timeout` | `lib/github/backends/cli.py` `run_gh_cli_command` (no timeout) | M1 | not started | #147 | Robustness, not parity: Python waits forever, so the only visible difference is where Python hangs. |
| `infrastructure.log-file` | Rolling log file at `core.logfile_path` (`LEVEL YYYY-mm-dd HH:MM:SS - message`) | `lib/logging.py` `LazyGithubLogFormatter`, `lib/context.py` `_setup_logging_handler` | M0 | not started | #66 | Rotates at `logfile_max_bytes`, keeps `logfile_count` files. |
| `infrastructure.fake-gh` | `fake-gh` and fixture format | n/a | M0 | not started | #67, #109 | |
| `infrastructure.fixture-recorder` | Fixture record/scrub xtask | n/a | M0 | not started | #68, #151 | Numeric-id `Link` URLs are mapped to the request path (#151). |
| `infrastructure.parity-harness` | Parity harness (PTY runner, scenarios, structural check, report) | n/a | M0 | not started | #69 | |
| `infrastructure.parity-ci` | Parity check on every PR into `rust-main` | n/a | M0 | not started | #70 | |
| `infrastructure.matrix` | This matrix | `lazy_github/ui/**`, `lib/*` | M0 | not started | #71 | |
| `infrastructure.python-goldens` | Python-dumped goldens compared in `cargo test`, with manifests in `parity/goldens/` naming the rows they cover, a coverage report and stale-golden checks | n/a | M0 | not started | #152, #157 | |
| `infrastructure.tui-runtime` | TUI event loop and Action/Effect runtime | `ui/app.py` `LazyGithub` (Textual App) | M1 | not started | #72 | |
| `infrastructure.gh-http-cache` | `gh` HTTP caching per request (`default_ttl`, `list_repos_ttl`, `list_issues_ttl`) via `gh api --cache` | `lib/github/backends/cli.py` `github_headers` | M0 | not started | #65 | hishel cache dropped (PLAN §1). |
| `infrastructure.git-detection` | Current-directory git detection (remote `origin` → `owner/repo`, branch, HEAD sha) honoring `LAZY_GITHUB_ORIGINAL_PWD` | `lib/git_cli.py`, `lib/context.py`, `cli.py` `cli`, `gh-lazy` | M2 | not started | | Py bug B5 (`.git` suffix and dotted names). |
