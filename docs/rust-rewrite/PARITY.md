# Parity Matrix

Every user-visible behavior of the Python app, and the state of its Rust equivalent. The design agent owns adding rows;
the parity reviewer owns moving a row to `parity`. Status is one of `not started`, `in progress`, `parity`,
`intentional diff` (link the decision).

This is a seed generated from `lazy_github/` at v0.6.6. The M0 design issue completes it.

## Screens and panes

| Item | Python reference | Status | Issue |
| --- | --- | --- | --- |
| First start / gh auth check | `ui/app.py` `FirstStartScreen`, `ui/screens/auth.py` | intentional diff (gh only) | |
| Main layout (status bar, selections pane 40%, details, command log, footer) | `ui/screens/primary.py` | not started | |
| Status summary: current repo, unread notifications | `ui/screens/primary.py` `LazyGithubStatusSummary` | not started | |
| Repositories table | `ui/widgets/repositories.py` | not started | |
| Pull requests table | `ui/widgets/pull_requests.py` `PullRequestsContainer` | not started | |
| Issues table | `ui/widgets/issues.py` `IssuesContainer` | not started | |
| Workflows tabs and runs table | `ui/widgets/workflows.py` | not started | |
| Command log | `ui/widgets/command_log.py` | not started | |
| Details: info tab | `ui/widgets/info.py` | not started | |
| Details: repo overview (README) | `ui/widgets/repository_details.py` | not started | |
| Details: PR overview / diff / conversation | `ui/widgets/pull_requests.py` | not started | |
| Split diff viewer, add comment, submit review | `ui/widgets/split_diff_viewer.py` | not started | |
| Details: issue overview / conversation | `ui/widgets/issues.py`, `ui/widgets/conversations.py` | not started | |
| Details: workflow run overview / jobs / logs | `ui/widgets/workflow_run_details.py` | not started | |
| Link hints overlay | `ui/widgets/link_hints.py` | not started | |
| Command palette and its commands | `ui/screens/primary.py` `MainScreenCommandProvider` | not started | |
| Help panel | `ui/app.py` | not started | |

## Modals

| Modal | Python reference | Status | Issue |
| --- | --- | --- | --- |
| Confirm | `ui/screens/confirm.py` | not started | |
| New issue | `ui/screens/new_issue.py` | not started | |
| Edit issue | `ui/screens/edit_issue.py` | not started | |
| New comment / reply | `ui/screens/new_comment.py` | not started | |
| Add reaction | `ui/screens/add_reactions_modal.py` | not started | |
| Create / edit PR | `ui/screens/create_or_edit_pull_request.py` | not started | |
| Lookup repository / issue / PR | `ui/screens/lookup_*.py` | not started | |
| Trigger workflow | `ui/screens/trigger_workflow.py` | not started | |
| Notifications | `ui/screens/notifications.py` | not started | |
| Paste link | `ui/screens/link_paste.py` | not started | |
| User profile | `ui/screens/user_profile.py` | not started | |
| Settings (incl. key rebinding) | `ui/screens/settings.py` | not started | |
| Debug info / bug report | `ui/screens/debug.py`, `lib/debug.py` | not started | |

## Keybindings

Ids and defaults from `lib/bindings.py`. The Rust keymap must use the same ids so `bindings.overrides` keeps working.

Known terminal caveats to settle in design: `ctrl+m` is indistinguishable from Enter in most terminals, and
`shift+enter` needs the kitty keyboard protocol (crossterm `PushKeyboardEnhancementFlags`) with a fallback.

| Binding id | Default key | Description | Status | Issue |
| --- | --- | --- | --- | --- |
| `app.quit` | `q` | Quit | not started | |
| `app.command_palette` | `ctrl+p` | Command Pallete | not started | |
| `app.maximize_widget` | `M` | Maximize | not started | |
| `app.open_help` | `?` | Keybindings | not started | |
| `conversation.comment.new` | `n` | New Comment | not started | |
| `conversation.review.reply` | `r` | Reply to Review | not started | |
| `conversation.comment.reply` | `r` | Reply to Comment | not started | |
| `issue.new` | `I` | New Issue | not started | |
| `issue.edit` | `E` | Edit Issue | not started | |
| `issue.lookup` | `O` | Lookup Issue | not started | |
| `pull_request.new` | `P` | New PR | not started | |
| `pull_request.merge` | `ctrl+m` | Merge PR | not started | |
| `pull_request.edit` | `E` | Edit PR | not started | |
| `pull_request.lookup` | `O` | Lookup Pull Request | not started | |
| `repositories.favorite` | `ctrl+f` | Toggle Favorite | not started | |
| `repositories.lookup` | `O` | Lookup Repository | not started | |
| `user.view_profile` | `u` | View User | not started | |
| `common.table.select` | `enter,space` | Select table entry | not started | |
| `common.table.search` | `/` | Search | not started | |
| `common.table.cursor_down` | `j` | Table cursor down | not started | |
| `common.table.page_down` | `J` | Table page down | not started | |
| `common.table.cursor_up` | `k` | Table cursor up | not started | |
| `common.table.page_up` | `K` | Table page up | not started | |
| `common.table.scroll_right` | `l` | Table scroll right | not started | |
| `common.table.page_right` | `$` | Table page right | not started | |
| `common.table.scroll_left` | `h` | Table scroll left | not started | |
| `common.table.page_left` | `^` | Table page left | not started | |
| `common.table.scroll_top` | `g` | Table scroll to top | not started | |
| `common.table.scroll_bottom` | `G` | Table scroll to bottom | not started | |
| `workflows.trigger` | `T` | Trigger Workflow | not started | |
| `workflows.view_run_details` | `enter` | View Run Details | not started | |
| `workflows.view_job_logs` | `l` | View Job Logs | not started | |
| `diff.add_comment` | `c` | Add Comment | not started | |
| `diff.cursor_down` | `j` | cursor_down | not started | |
| `diff.cursor_up` | `k` | cursor_up | not started | |
| `diff.clear_selection` | `escape` | Clear Selection | not started | |
| `diff.next_hunk` | `J` | Next Hunk | not started | |
| `diff.previous_hunk` | `K` | Previous Hunk | not started | |
| `diff.filter_files` | `/` | Filter Files | not started | |
| `notifications.open` | `ctrl+n` | Notifications | not started | |
| `notifications.mark_all_as_read` | `ctrl+r` | Mark all as read | not started | |
| `notifications.mark_read` | `R` | Mark as Read | not started | |
| `notifications.view_read` | `r` | View Read Notifications | not started | |
| `notifications.view_unread` | `u` | View Unread Notifications | not started | |
| `modal.submit` | `shift+enter` | Submit | not started | |
| `modal.close` | `q, escape` | Close | not started | |
| `modal.search` | `/` | Search | not started | |
| `confirm.yes` | `y` | Yes | not started | |
| `confirm.no` | `n` | No | not started | |
| `react.add` | `R` | Add reaction | not started | |
| `link.paste` | `ctrl+v` | Paste link | not started | |
| `link.follow` | `f` | Follow link | not started | |
| `link.follow_external` | `F` | Follow link (browser) | not started | |
| `main.repos.focus` | `1` | Focus repos table | not started | |
| `main.pull_requests.focus` | `2` | Focus PRs table | not started | |
| `main.issues.focus` | `3` | Focus issues table | not started | |
| `main.workflows.focus` | `4` | Focus workflows table | not started | |
| `main.details.focus` | `5` | Focus details tabs | not started | |
| `main.command_log.focus` | `6` | Focus command log | not started | |

## Config

Every field in `lib/config.py` must load from an existing `config.json` and round-trip on save.

| Section | Fields | Status | Issue |
| --- | --- | --- | --- |
| `appearance` | `theme`, `show_command_log`, `show_workflows`, `show_issues`, `show_pull_requests` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `notifications` | `enabled`, `mark_notification_as_read_when_selected` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `bindings` | `overrides` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `repositories` | `additional_repos_to_track`, `favorites` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `pull_requests` | `state_filter`, `owner_filter`, `preferred_merge_method`, `additional_suggested_pr_reviewers`, `preload_pull_request_for_current_commit`, `pull_request_template` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `issues` | `state_filter`, `owner_filter` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `cache` | `auth_cache_duration`, `auth_last_checked`, `cache_directory`, `default_ttl`, `list_repos_ttl`, `list_issues_ttl` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `core` | `first_start`, `logfile_path`, `logfile_max_bytes`, `logfile_count` | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |
| `api` | `base_url`, `client_type` (ignored, gh only) | not started | [#64](https://github.com/gizmo385/gh-lazy/issues/64) |

## CLI

| Command | Python reference | Status | Issue |
| --- | --- | --- | --- |
| `run [--auth-backend]` | `cli.py` | not started | |
| `dump-config` | `cli.py` | not started | |
| `clear-auth` | `cli.py` | intentional diff (defer to `gh auth logout`) | |
| `clear-config [--no-confirm]` | `cli.py` | not started | |
| `clear-cache [--no-confirm]` | `cli.py` | not started | |
| `debug` | `cli.py` | not started | |
| `version` | `cli.py` | not started | |
