# Parity Matrix

Every user-visible behavior of the Python app, and the state of its Rust equivalent. The design agent owns adding rows;
the parity reviewer owns moving a row to `parity`. Status is one of `not started`, `in progress`, `parity`,
`intentional diff` (link the decision).

Generated from `lazy_github/` at v0.6.6 (completed in #71). Paths in "Python reference" are relative to `lazy_github/`.

Columns:

- **Milestone**: where the row lands, per [PLAN.md §8](./PLAN.md#8-milestones). M0 is only used for infrastructure rows.
- **Issue**: the issue that implements the row, where one is filed. Rows with no issue are the backlog the milestone
  issues get filed from.
- **Notes**: details a spec needs, `TERM:` for terminal caveats (see [Terminal caveats](#terminal-caveats)), and
  `Py bug Bn` where the Python behavior looks like a bug (see [Suspected Python bugs](#suspected-python-bugs)). Rows
  with a `Py bug` note must not be copied silently: the spec for that row follows Chris's answer.

Wording in quotes is the exact text the Python app shows. Toast rows give severity and title as
`severity / "title"`; no title means Textual's default (none).

## Infrastructure (M0 and shell plumbing)

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Cargo workspace and crate skeletons | n/a | M0 | not started | #62 | |
| CI for `rust-main` (fmt, clippy, tests on Linux and macOS) | `.github/workflows/code-checks.yaml` (Python equivalent) | M0 | not started | #63 | |
| Config load/save compatible with `config.json` (see [Config](#config)) | `lib/config.py` `Config.load_config`, `Config.save` | M0 | not started | #64 | Must also accept values the Python Settings modal wrote as strings (Py bug B2). |
| `Github` trait, `gh` CLI client, auth check | `lib/github/client.py`, `lib/github/backends/cli.py`, `lib/github/auth.py` `is_logged_in_to_cli` | M0 | not started | #65 | |
| Rolling log file at `core.logfile_path` (`LEVEL YYYY-mm-dd HH:MM:SS - message`) | `lib/logging.py` `LazyGithubLogFormatter`, `lib/context.py` `_setup_logging_handler` | M0 | not started | #66 | Rotates at `logfile_max_bytes`, keeps `logfile_count` files. |
| `fake-gh` and fixture format | n/a | M0 | not started | #67 | |
| Fixture record/scrub xtask | n/a | M0 | not started | #68 | |
| Parity harness (PTY runner, scenarios, structural check, report) | n/a | M0 | not started | #69 | |
| Parity check on every PR into `rust-main` | n/a | M0 | not started | #70 | |
| This matrix | `lazy_github/ui/**`, `lib/*` | M0 | not started | #71 | |
| TUI event loop and Action/Effect runtime | `ui/app.py` `LazyGithub` (Textual App) | M1 | not started | #72 | |
| `gh` HTTP caching per request (`default_ttl`, `list_repos_ttl`, `list_issues_ttl`) via `gh api --cache` | `lib/github/backends/cli.py` `github_headers` | M0 | not started | #65 | hishel cache dropped (PLAN §1). |
| Current-directory git detection (remote `origin` → `owner/repo`, branch, HEAD sha) honoring `LAZY_GITHUB_ORIGINAL_PWD` | `lib/git_cli.py`, `lib/context.py`, `cli.py` `cli`, `gh-lazy` | M2 | not started | | Py bug B5 (`.git` suffix and dotted names). |

## Startup and authentication

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| First start / gh auth check | `ui/app.py` `FirstStartScreen`, `ui/screens/auth.py` | M0 | intentional diff (gh only) | #65 | PLAN §10 decision 1. |
| First-start backend picker ("Welcome to LazyGithub! How would you like to authenticate with the Github API?", radio "Via the Github CLI" / "Directly via HTTPS", Continue / Cancel) | `ui/app.py` `FirstStartScreen` | M0 | intentional diff (gh only) | #65 | Not shown in Rust. `core.first_start` is still read and written as `false`. |
| Skip the auth check while `cache.auth_last_checked` is within `cache.auth_cache_duration`; otherwise check and write `auth_last_checked` | `ui/app.py` `LazyGithub.authenticate_with_github` | M0 | not started | #65 | |
| Not-logged-in screen: border "GitHub Authentication", "To proceed, please log into the Github CLI via gh auth login", intro text below, polls `gh auth status` every 5 s and switches to the main screen once logged in; `q` quits | `ui/screens/auth.py` `AuthenticationModal`, `GithubCliAuthInstructions` | M1 | not started | | Device-flow branch (`UserTokenDisplay`) is dropped. |
| Theme and keymap applied on mount from config | `ui/app.py` `LazyGithub.on_mount` | M1 | not started | #73, #77 | |

## Main screen layout and shell

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Main layout (status bar, selections pane 40%, details, command log, footer) | `ui/screens/primary.py` `LazyGithubMainScreen`, `MainViewPane`, `SelectionsPane`, `SelectionDetailsPane` | M1 | not started | #74 | Selections pane docked left at 40% width; details fill the rest. |
| Section order in selections pane: Repositories, Pull Requests, Issues, Workflows | `ui/screens/primary.py` `SelectionsPane.compose` | M1 | not started | #74 | |
| Sections hidden at startup per `appearance.show_pull_requests`, `show_issues`, `show_workflows`, `show_command_log` | `ui/screens/primary.py` `SelectionsPane.compose`, `SelectionDetailsPane.compose` | M1 | not started | #74 | |
| Section border titles show their focus key: "[1] Repositories", "[2] Pull Requests", "[3] Issues", "[4] Workflows", "[5] Details", "[6] Command Log" | `ui/widgets/repositories.py`, `ui/widgets/pull_requests.py`, `ui/widgets/issues.py`, `ui/widgets/workflows.py`, `ui/widgets/command_log.py`, `ui/screens/primary.py` `SelectionDetailsContainer` | M1 | not started | #74, #73 | Key comes from `bindings.overrides`. Titles are not refreshed after rebinding in Settings until restart. |
| Focused section gets a highlighted border and grows to at least 40% height | `ui/widgets/common.py` `LazyGithubContainer` (`:focus-within`) | M1 | not started | #74 | |
| Empty section collapses to its title bar with " (empty)" suffix | `ui/widgets/common.py` `LazyGithubContainer.set_compact` | M2 | not started | | Applied to PRs, issues and workflows after their fetch. |
| Details border title per selection: "[5] Details", "[5] owner/repo Details", "[5] PR #N Details", "[5] Issue #N Details", "[5] Workflow Run #N Details" | `ui/screens/primary.py` `MainViewPane.load_*` | M2 | not started | | |
| Status summary: current repo, unread notifications | `ui/screens/primary.py` `LazyGithubStatusSummary` | M1 | not started | #81 | |
| Status left: "Current repo: owner/repo" (repo name highlighted) or "No repository selected" | `ui/screens/primary.py` `CurrentlySelectedRepo` | M1 | not started | #81 | |
| Status right: blank when notifications disabled, "No unread notifications" (green), "• Unread Notifications: N" (red), "N+" when N ≥ 30 | `ui/screens/primary.py` `UnreadNotifications` | M1 | not started | #81 | Py bug B6 (cap is the 50-per-page fetch, not 30). |
| Unread count fetched on mount and every 60 s when `notifications.enabled`; toast error / "Failed to Load Notifiations" "Cannot load notifications - please login to the gh CLI: gh auth login" when not logged in | `ui/screens/primary.py` `refresh_notification_count`, `lib/constants.py` `NOTIFICATION_REFRESH_INTERVAL`, `lib/github/notifications.py` `unread_notification_count` | M1 | not started | #81 | Title typo is Python's (Py bug B14). Timer starts after Settings save even if disabled. |
| Command log pane: INFO and above, one line per record, `LEVEL YYYY-mm-dd HH:MM:SS - message`, no tracebacks | `ui/widgets/command_log.py` `CommandLogSection`, `CommandLogLoggingHandler` | M1 | not started | #80 | Hidden by default (`show_command_log: false`). |
| Footer: bindings with `show=True` for the focused widget chain; command palette key hidden | `ui/widgets/common.py` `LazyGithubFooter` | M1 | not started | #76 | TERM: Textual renders `ctrl+x` as `^x`; decide the Rust footer key format in #76. |
| Help panel | `ui/app.py` `action_open_help` (Textual `HelpPanel`) | M1 | not started | #76 | `?` toggles a side panel with the focused widget's bindings, descriptions and tooltips. |
| Maximize focused widget (`M`); `escape` restores; toast "View Maximized" "Current view maximized. Press escape to return." only the first time per session | `ui/app.py` `action_maximize` | M1 | not started | #75 | Textual `escape_to_minimize`. |
| Focus switching `1`–`6`; target ignored if hidden | `ui/screens/primary.py` `MainViewPane.action_focus_section`, `action_focus_workflow_tabs`, `action_focus_tabs` | M1 | not started | #75 | `4` focuses the workflow runs table even when Workflows is hidden; `5` focuses the first details tab. |
| Textual implicit keys: `tab`/`shift+tab` focus next/previous, `ctrl+q` quit, `ctrl+c` hint toast, arrows/`pageup`/`pagedown`/`home`/`end` in tables and lists | Textual `App.BINDINGS`, `Screen.BINDINGS`, `DataTable.BINDINGS` | M1 | not started | #73, #78 | Not in `lib/bindings.py`, so not rebindable. |
| Mouse: click to focus, click table rows and tabs, click links, scroll wheel | Textual defaults | M1 | not started | #72 | |
| Toast notifications (information / warning / error, optional title, 5 s timeout, stacked bottom-right) | Textual `App.notify` | M1 | not started | #79 | Every toast text is listed in [Toasts](#toasts). |
| Themes: all Textual built-in names (`textual-dark` default, `textual-light`, `nord`, `gruvbox`, `catppuccin-mocha`, `dracula`, `tokyo-night`, `monokai`, `flexoki`, `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `solarized-light`, `solarized-dark`, `rose-pine`, `rose-pine-moon`, `rose-pine-dawn`, `atom-one-dark`, `atom-one-light`, `ansi-dark`, `ansi-light`); unknown name falls back to `textual-dark` | `lib/config.py` `AppearanceSettings.validate_theme`, `ui/app.py` | M1 | not started | #77 | |
| Fixed accent colors independent of theme (greenyellow, red, orchid, yellow, cyan) for states | throughout `ui/` | M1 | not started | #77 | Decide whether these map to theme slots. |
| Loading spinner overlay on a table or pane while its data loads | `ui/widgets/common.py` (`loading = True`), detail tab panes | M1 | not started | #78 | Several spinners never clear on API errors (Py bug B16). |

## Searchable tables (shared component)

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Row cursor table with vim keys (see `common.table.*` bindings) | `ui/widgets/common.py` `_VimLikeDataTable` | M1 | not started | #78 | |
| `/` reveals a "Search..." input above the table; Enter filters rows by case-insensitive substring over the row's cell text; empty query restores all rows; focus returns to the table | `ui/widgets/common.py` `SearchableDataTable.action_focus_search`, `handle_submitted_search` | M1 | not started | #78 | Filter is applied on submit, not per keystroke. Matches the raw cell text, including markup such as `[green]★[/]`. |
| Search input hides again when it loses focus while empty | `ui/widgets/common.py` `ToggleableSearchInput._on_blur` | M1 | not started | #78 | |
| Rows sorted by a column after each insert (PRs/issues by number desc, runs by time desc, repos by favorite) | `ui/widgets/common.py` `SearchableDataTable.add_item`, `sort_table` | M1 | not started | #78 | |
| Re-adding an existing key replaces the row | `ui/widgets/common.py` `SearchableDataTable.add_item` | M1 | not started | #78 | |
| Lazy loading: when the cursor is within 5 rows of the end, fetch the next page (30 per page) and append; stop after an empty page | `ui/widgets/common.py` `LazilyLoadedDataTable` | M2 | not started | | Hook belongs in #78's component; fetchers land in M2. |
| Table rows written to the per-repo cache after each fetch and restored before the live fetch | `ui/widgets/common.py` `initialize_from_cache`, `save_to_cache` | M2 | not started | | See [Local caches and drafts](#local-caches-and-drafts). |

## Repositories

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Repositories table | `ui/widgets/repositories.py` `ReposContainer` | M2 | not started | | |
| Columns: "★" (favorite: green "★" or "☆"), "Owner", "Name", "Private" ("✓" / "✘") | `ui/widgets/repositories.py` `repo_to_row`, `on_mount`; `lib/constants.py` | M2 | not started | | |
| Rows: all repos for the user (`/user/repos?type=all&sort=full_name`, 50 per page, up to 29 pages) plus `repositories.additional_repos_to_track`, favorites first | `ui/widgets/repositories.py` `load_repos`; `lib/github/repositories.py` `list_all` | M2 | not started | | Tracked repos are appended after the sorted list. Uses `cache.list_repos_ttl`. |
| Loading: spinner; cached repos shown immediately from `repos.json`; on API error the cached rows stay and nothing is shown | `ui/widgets/repositories.py` `load_repos` | M2 | not started | | Error is only logged. |
| Auto-select the current directory's repo (if present in the table and nothing selected yet), after cache load and after fetch | `ui/widgets/repositories.py` `check_current_directory_repo` | M2 | not started | | Py bug B5. |
| Enter/space selects a repo: status bar updates, PR/issue/workflow tables reload, details show "Repo Overview" | `ui/widgets/repositories.py` `repo_selected`; `ui/screens/primary.py` `handle_repo_selection`, `SelectionsPane.load_repository` | M2 | not started | | Details are not focused. |
| `ctrl+f` toggles favorite: writes `repositories.favorites`, updates the star, re-sorts | `ui/widgets/repositories.py` `action_toggle_favorite_repo` | M2 | not started | | Py bug B13 (empty table). |
| `O` lookup repository modal; result added to the table and selected | `ui/widgets/repositories.py` `action_lookup_repository` | M2 | not started | | |
| `u` opens the owner's profile; choosing a repo there adds it to the table and selects it; toast error "No repository currently selected" on empty table | `ui/widgets/repositories.py` `action_view_user_profile` | M2 | not started | | |
| Repo overview tab "Repo Overview": "owner/repo • Public/Private • Archived", "Owned by <owner>", "Default branch: <branch>", description, rule, README markdown or "*No README found.*" | `ui/widgets/repository_details.py` `RepoOverviewTabPane` | M2 | not started | | Repo and owner are hint/clickable links (open in app or browser). README via `Accept: application/vnd.github.raw`. |
| Introduction tab before any selection ("Introduction", `LAZY_GITHUB_INFO` markdown with version) | `ui/widgets/info.py` `LazyGithubInfoTabPane`; `ui/screens/primary.py` `SelectionDetailsContainer.on_mount` | M2 | not started | | Same text is shown on the auth screen. |

## Pull requests

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Pull requests table | `ui/widgets/pull_requests.py` `PullRequestsContainer` | M2 | not started | | |
| Columns: "#", "Status" (raw `open`/`closed`), "Author", "Title"; sorted by number desc | `ui/widgets/pull_requests.py` `pull_request_to_cell`, `on_mount` | M2 | not started | | Merged PRs show `closed`. |
| Rows come from the issues endpoint (items with `draft` are PRs); first page fetched together with issues | `ui/screens/primary.py` `SelectionsPane.fetch_issues_and_pull_requests`; `lib/github/issues.py` `list_issues`; `lib/messages.py` `IssuesAndPullRequestsFetched` | M2 | not started | | Py bug B3 (first page uses `issues.*` filters, later pages `pull_requests.*`). |
| Filters: `pull_requests.state_filter` (`all`/`open`/`closed`), `owner_filter` (`mine` adds `creator=<login>`) | `ui/widgets/pull_requests.py` `fetch_more_pull_requests` | M2 | not started | | |
| Loading / error: spinner; cached rows from `<owner>_<repo>_pull_requests.json`; on API error the table is left with cached rows and nothing is shown | `ui/widgets/pull_requests.py` `load_cached_pull_requests_for_repo`, `on_issues_and_pull_requests_fetched` | M2 | not started | | |
| Enter/space selects a PR: fetch full PR, details get Overview / Diff / Conversation tabs, focus moves to the details; toast error / "API Error" "Error fetching pull request!" on failure | `ui/widgets/pull_requests.py` `pr_selected`; `ui/screens/primary.py` `MainViewPane.load_pull_request` | M2 | not started | | |
| `O` lookup PR modal; result added to the table if missing and selected | `ui/widgets/pull_requests.py` `action_lookup_pull_request` | M2 | not started | | Py bug B12. |
| `E` edit PR from the table (fetches full PR; toast warning "This PR has already been merged!" if merged) | `ui/widgets/pull_requests.py` `action_edit_pull_request` | M3 | not started | | Py bug B13 (empty table raises). |
| `u` opens the author's profile; choosing a repo selects it; toast error "No pull request currently selected" on empty table | `ui/widgets/pull_requests.py` `action_view_user_profile` | M2 | not started | | |
| PR Overview tab: header "<Open/closed/Merged> <title> (#N) by <user>" | `ui/widgets/pull_requests.py` `PrOverviewTabPane.compose` | M2 | not started | | "closed" is lowercase, "Open"/"Merged" capitalized. PR and user are links. |
| PR Overview: change summary "N commits • N files changed • +A • -D • head_owner:head_ref → base_owner:base_ref" (singular "1 commit", "1 file changed") | `ui/widgets/pull_requests.py` `PrOverviewTabPane.compose` | M2 | not started | | |
| PR Overview: "Opened on <%c>" plus " • Merged on <%c>" | `ui/widgets/pull_requests.py` `PrOverviewTabPane.compose` | M2 | not started | | Times are UTC (Py bug B26). |
| PR Overview: reactions summary collapsible ("👍 2 \| 🎉 1"), expands to "emoji: a, b, c, and N more"; hidden when none | `ui/widgets/conversations.py` `ReactionsDisplay`; `ui/widgets/pull_requests.py` `load_reactions` | M2 | not started | | |
| PR Overview: status checks collapsible, title "Status checks: ✓ Passed / ... Pending / ✘ Failed" (failure or error wins, then pending), one line per commit status ("<state> context - description") and check run ("<state> name"), each a link; hidden with title "No status checks on PR" when none; spinner while loading | `ui/widgets/pull_requests.py` `load_checks`, `_overall_pr_status`; `lib/github/checks.py` | M2 | not started | | Py bug B23 (ordering), B16 (spinner on error). |
| PR Overview: reviews collapsible, latest submitted review per author ("user: <state>"), title "PR Reviews: ✓ Approved" if all approved, "PR Reviews: ✘ Changes Requested" if any, else "PR Reviews Summary"; hidden when none | `ui/widgets/pull_requests.py` `add_reviews`; `ui/screens/primary.py` `handle_reviews_loaded` | M2 | not started | | Reviews arrive from the Conversation tab's fetch. |
| PR Overview: rule then body markdown with references expanded | `ui/widgets/pull_requests.py` `PrOverviewTabPane.compose` | M2 | not started | | See [Markdown and links](#markdown-and-links). |
| PR Overview `ctrl+m` merge with `pull_requests.preferred_merge_method` and the head sha; toasts (see [Toasts](#toasts)); no confirmation | `ui/widgets/pull_requests.py` `action_merge_pull_request`; `lib/github/pull_requests.py` `merge_pull_request` | M3 | not started | | TERM: `ctrl+m` is Enter in legacy terminals (Py bug B30). Py bug B15 (stale view after merge). |
| PR Overview `E` edit PR (toast warning "This PR has already been merged!" if merged) | `ui/widgets/pull_requests.py` `PrOverviewTabPane.action_edit_pull_request` | M3 | not started | | |
| PR Overview `R` add reaction (opens Add reaction modal, posts each added reaction, refreshes the summary) | `ui/widgets/pull_requests.py` `trigger_reactions_modal` | M3 | not started | | Py bug B20 (add only). |
| Preload PR for the current commit on repo load when `pull_requests.preload_pull_request_for_current_commit` and the local branch isn't the default branch; only if exactly one PR matches; details not focused | `ui/screens/primary.py` `SelectionsPane.load_repository`; `ui/widgets/pull_requests.py` `load_pull_request_for_current_commit` | M4 | not started | | |
| Diff tab "Diff": spinner, then the split diff viewer; "No diff contents found" (404), "Diff too large" (406), "Error fetching diff (<status>)" | `ui/widgets/pull_requests.py` `PrDiffTabPane`; `ui/widgets/diff_viewer.py` `DiffViewerContainer` | M2 | not started | | Diff via `Accept: application/vnd.github.diff`. |
| Conversation tab "Conversation": reviews (collapsible "Review from <user> (<state>)", collapsed when dismissed, body plus threaded review comments, or "No additional review content") then issue comments; "No reviews or comments available" when empty; spinner while loading | `ui/widgets/pull_requests.py` `PrConversationTabPane`; `ui/widgets/conversations.py` `ReviewContainer`, `ReviewConversation`, `IssueCommentContainer`; `lib/github/pull_requests.py` `reconstruct_review_conversation_hierarchy` | M2 | not started | | Py bug B10 (body-less comment reviews hidden), B16. |
| Comment block: markdown body (references expanded), then "<author> • <%c>" ("Unknown" if no user); focusable | `ui/widgets/conversations.py` `IssueCommentContainer` | M2 | not started | | |
| Per-comment reactions shown under each comment once loaded | `ui/widgets/pull_requests.py` `fetch_reactions`, `handle_comment_reactions_loaded`; `lib/messages.py` `CommentReactionsLoaded` | M2 | not started | | |
| Conversation `n` new comment, then refetch the conversation | `ui/widgets/pull_requests.py` `PrConversationTabPane.action_new_comment` | M3 | not started | | |
| Conversation `r` on a comment replies (review comment → review reply API, else new issue comment); `r` on a review opens a reply modal that posts an issue comment | `ui/widgets/conversations.py` `action_reply_to_individual_comment`, `ReviewContainer.action_reply_to_review` | M3 | not started | | Py bug B18 (reply not shown until reload). |

## Split diff viewer and review

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Split diff viewer, add comment, submit review | `ui/widgets/split_diff_viewer.py` | M2 | not started | | Read-only view M2; comments and review M3 (rows below). |
| File filter input at top, placeholder "Filter files (press '/' to focus, ESC to clear)"; filters file sections by case-insensitive path substring as you type; Enter blurs and focuses the first visible hunk; Esc clears and blurs | `ui/widgets/split_diff_viewer.py` `FileFilterInput`, `filter_files`, `on_filter_submitted` | M2 | not started | | Placeholder uses the default key, not the override. |
| One collapsible per file: "📄 <basename> (+A -D)", or "🗑️  <basename> (deleted)" collapsed with "file was removed" | `ui/widgets/split_diff_viewer.py` `_format_file_header` | M2 | not started | | Only the basename is shown. |
| One collapsed collapsible per hunk: "Line N" / "Lines N-M (+a -d) • <context>"; raw header if unparsable | `ui/widgets/split_diff_viewer.py` `_format_hunk_header` | M2 | not started | | |
| Hunk pane: up to 25 rows tall, gutter "► " on current line, line number, " │ ", syntax-highlighted code (monokai), red/green backgrounds, brighter on the current line, footer "Line: n/m" | `ui/widgets/split_diff_viewer.py` `UnifiedDiffPane` | M2 | not started | | Py bug B22 (line numbers count deleted lines). |
| Parse errors: "error parsing diff - please view on github: <err>", "unexpected error: <err>", "error rendering hunk: <err>" | `ui/widgets/split_diff_viewer.py` `SplitDiffViewer.compose`; `lib/diff_parser.py` | M2 | not started | | |
| `j`/`k` move the current line inside a hunk (hard-coded, not the `diff.cursor_*` ids) | `ui/widgets/split_diff_viewer.py` `UnifiedDiffPane.BINDINGS` | M2 | not started | | Py bug B9. |
| `J`/`K` next/previous hunk | `ui/widgets/split_diff_viewer.py` `action_next_hunk`, `action_previous_hunk` | M2 | not started | | Py bug B8 (does nothing in Python). |
| `c` add comment on the current line: modal "Add Comment", "Commenting on:" (read-only line), "Comment:" textarea, "Add Comment" / "Cancel"; `shift+enter` submits, `q`/`escape` closes; toast warning "Comment cannot be empty!" | `ui/widgets/split_diff_viewer.py` `AddCommentModal` | M3 | not started | | TERM: `shift+enter`. |
| Pending comments: "Pending Comments:" header and a preview per comment ("<file>:<line> - <line text>", body, "Remove" button); hidden when none; toast "New comment added" | `ui/widgets/split_diff_viewer.py` `CommentPreview`, `on_trigger_add_comment`, `on_comment_deleted` | M3 | not started | | Py bug B22 (line is hunk-relative). |
| Submit review: "Review Status:" select (Approved / Changes Requested / Commented, default Commented), "Review summary" input, "Submit Review"; if the viewer is the PR author, only the summary and "Add Comments"; button disabled while submitting; toast "New review created!" then reload the PR; toast error "Please select a status for the new review!" | `ui/widgets/split_diff_viewer.py` `SubmitReview`, `on_trigger_review_submission`; `lib/github/pull_requests.py` `create_new_review` | M3 | not started | | Comments use the diff `position` field. Pending list is not cleared after submit. Py bug B15. |

## Issues

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Issues table | `ui/widgets/issues.py` `IssuesContainer` | M2 | not started | | |
| Columns: "#", "Status" (raw `open`/`closed`), "Author", "Title"; sorted by number desc | `ui/widgets/issues.py` `issue_to_cell`, `on_mount` | M2 | not started | | |
| Filters `issues.state_filter`, `issues.owner_filter`; first page shared with PRs, then lazy pages of 30 | `ui/widgets/issues.py` `fetch_more_issues` | M2 | not started | | Issue lists use `cache.default_ttl`; `list_issues_ttl` is never read. |
| Loading / error: spinner; cached rows from `<owner>_<repo>_issues.json`; API error leaves cached rows | `ui/widgets/issues.py` `load_cached_issues_for_repo` | M2 | not started | | |
| Enter/space selects an issue: details get Overview / Comments, focus moves to details | `ui/widgets/issues.py` `issue_selected`; `ui/screens/primary.py` `MainViewPane.load_issue` | M2 | not started | | |
| `O` lookup issue modal; result added if missing and selected | `ui/widgets/issues.py` `action_lookup_issue` | M2 | not started | | Py bug B12. |
| `E` edit issue from table (updated row replaces the old one); toast error "No issue currently selected" | `ui/widgets/issues.py` `trigger_edit_issue_flow` | M3 | not started | | |
| `u` opens the author's profile; toast error "No issue currently selected" | `ui/widgets/issues.py` `action_view_user_profile` | M2 | not started | | |
| Details: issue overview / conversation | `ui/widgets/issues.py`, `ui/widgets/conversations.py` | M2 | not started | | |
| Issue Overview tab "Overview": "<Open/Closed> <title> (#N) by <user>", rule, body markdown | `ui/widgets/issues.py` `IssueOverviewTabPane` | M2 | not started | | Py bug B11 (title markup). |
| Issue Overview `E` edit issue | `ui/widgets/issues.py` `IssueOverviewTabPane.action_edit_issue` | M3 | not started | | Overview and table are not refreshed afterward. |
| Issue Comments tab "Comments": comment blocks, "No comments available", spinner | `ui/widgets/issues.py` `IssueConversationTabPane` | M2 | not started | | Py bug B16. |
| Comments `n` new comment (appended on success), `r` reply to a comment (appended) | `ui/widgets/issues.py` `new_comment_flow`, `handle_new_comment_added` | M3 | not started | | |

## Workflows

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Workflows tabs and runs table | `ui/widgets/workflows.py` `WorkflowsContainer`, `WorkflowRunsContainer` | M2 | not started | | There are no tabs any more; one runs table. |
| Columns: "Time" (`%Y-%m-%d %H:%M`), "Result" (conclusion or status), "Job Name" (workflow name), "Run Name" (display title), "Run #"; sorted by time desc | `ui/widgets/workflows.py` `workflow_run_to_cell`, `WorkflowRunsContainer.on_mount` | M2 | not started | | Py bug B4 (keyed by run number). |
| Loading: spinner, cached rows from `<owner>_<repo>_workflow_runs.json`, workflows and runs fetched together, lazy pages of 30 | `ui/widgets/workflows.py` `load_repo`, `initialize_tables_from_cache` | M2 | not started | | No error handling (Py bug B16). |
| Enter/space selects a run: details get Overview / Jobs / Logs, focus moves to details | `ui/widgets/workflows.py` `workflow_run_selected`; `ui/screens/primary.py` `load_workflow_run` | M2 | not started | | |
| `T` trigger workflow; toasts warning "No repository selected", "No workflows found in this repository", "Successfully triggered workflow" | `ui/widgets/workflows.py` `action_trigger_workflow` | M3 | not started | | |
| Details: workflow run overview / jobs / logs | `ui/widgets/workflow_run_details.py` | M2 | not started | | |
| Run Overview tab "Overview": "<✓ Success/✗ Failure/○ Cancelled/− Skipped/⋯ In Progress/⋯ Queued/status> <title> (#N)"; markdown list Run Number, Event, Branch, Status, Conclusion, Attempt, Created, Updated | `ui/widgets/workflow_run_details.py` `WorkflowRunOverviewTabPane` | M2 | not started | | `(#N)` is a terminal hyperlink, not a hint target. |
| Run Overview "Commit Information": SHA (short and full), Triggered by, Actor | `ui/widgets/workflow_run_details.py` `WorkflowRunOverviewTabPane.compose` | M2 | not started | | |
| Run Overview "Jobs Summary": "Loading jobs...", then "<icon> <job> (<duration>)" ("Not started", "In progress", `Ns`, `Nm Ns`, `Nh Nm`), or "No jobs found" | `ui/widgets/workflow_run_details.py` `refresh_job_summary`, `_calculate_duration` | M2 | not started | | |
| Run Overview "Actions": "Re-run Workflow"; "Re-run Failed Jobs" only when a completed job failed; toasts "Re-run Started" / error "Error" | `ui/widgets/workflow_run_details.py` `on_button_pressed` | M3 | not started | | Py bug B15 (stale refresh). |
| Jobs tab "Jobs": "Jobs for Run #N", tree of jobs "<icon> <name> (<status>)" with steps; "No jobs found" | `ui/widgets/workflow_run_details.py` `WorkflowRunJobsTabPane` | M2 | not started | | |
| Jobs `l` shows the selected job's logs (a step shows its job's logs) under "Logs for Job: <name>": "Loading logs...", fenced log text truncated at 50,000 chars with "... (logs truncated, too long to display)", or "No logs available or failed to fetch logs." | `ui/widgets/workflow_run_details.py` `action_view_logs`, `view_job_logs` | M2 | not started | | Py bug B21 (hint text). |
| Logs tab "Logs": "Full Logs for Run #N", every job's logs with `=` separators and "Job: <name> (Status: s, Conclusion: c)", truncated at 100,000 chars; "No jobs found for this workflow run." | `ui/widgets/workflow_run_details.py` `WorkflowRunLogsTabPane` | M2 | not started | | Python fetches all logs on mount even if the tab is never opened; Rust may fetch lazily. |

## Markdown and links

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Markdown rendering of bodies, comments, README, debug info | Textual `Markdown` | M2 | not started | | PLAN §4: own pulldown-cmark renderer. |
| Reference expansion: `#N`, `owner/repo#N`, `@user` become links, not inside code fences or inline code | `lib/github/references.py` `expand_github_references` | M2 | not started | | Port the regexes' lookbehind rules. |
| Clicking a markdown link or `@click` link: PR/issue URLs open in-app (repo switches), user URLs open the profile modal, everything else opens the browser | `ui/screens/primary.py` `action_open_gh_link`, `handle_markdown_link_clicked`, `_open_link_subject`; `ui/screens/link_paste.py` `resolve_github_url` (`accept_repo=False`) | M2 | not started | | Bare repo URLs go to the browser here, unlike the paste modal. |
| Link hints overlay | `ui/widgets/link_hints.py` | M4 | not started | | |
| `f` shows hint labels (from `asdfghjklqwertyuiopzxcvbnm`, two letters when more than 26) on every visible link; typing a label follows it in-app; `F` opens it in the browser with toast "Opening <url> externally"; `escape` or a non-matching prefix cancels; toast "No links visible" | `ui/widgets/link_hints.py` `follow_link`, `LinkHintScreen`, `collect_link_hits` | M4 | not started | | |
| Paste link modal (see [Modals](#modals)) result: PR → select repo and PR, toast "Opening pull request for link"; issue → "Opening issue for link"; repo → "Opening repository for link"; user → profile modal | `ui/screens/primary.py` `action_open_link_paste_modal`, `_open_link_subject` | M4 | not started | | TERM: `ctrl+v`. |

## Command palette

`ctrl+p` opens Textual's command palette. Fuzzy search over command names; each hit shows its help text.

| Command | Help text | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| Command palette and its commands | | `ui/screens/primary.py` `MainScreenCommandProvider` | M4 | not started | | |
| "Toggle Command Log" | "Toggle showing or hiding the command log" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | | Session only, not written to config. |
| "Toggle Workflows" | "Toggle showing or hiding repo actions" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | | Showing reloads cached and live runs for the current repo. |
| "Toggle Issues" | "Toggle showing or hiding repo issues" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | | Showing refetches issues and PRs. |
| "Toggle Pull Requests" | "Toggle showing or hiding repo pull requests" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | | Showing refetches issues and PRs. |
| "Change Settings" | "Adjust LazyGithub settings" | `ui/screens/primary.py` `action_show_settings_modal` | M4 | not started | | |
| "Debug Info" | "View Debug Information" | `ui/screens/primary.py` `action_show_debug_info` | M4 | not started | | |
| "Open Bug Report" | "Open Bug Report" | `ui/screens/primary.py` `action_open_bug_report`; `lib/debug.py` `get_bug_report_template` | M4 | not started | | Selects `gizmo385/gh-lazy` and opens New issue prefilled with the template; toast error / "Error opening bug report" "Could not load LazyGithub repo! Please open the bug report on Github". |
| "Refresh notifications" (only when `notifications.enabled`) | "Refresh the unread notifications count" | `ui/screens/primary.py` `action_refresh_notifications` | M4 | not started | | |
| Textual "Theme" | "Change the current theme" | Textual `App.get_system_commands` | M4 | not started | | Live theme picker; not saved to config (Py bug B27). |
| Textual "Quit" | "Quit the application as soon as possible" | Textual | M4 | not started | | |
| Textual "Keys" | "Show help for the focused widget and a summary of available keys" / "Hide the keys and widget help panel" | Textual | M4 | not started | | |
| Textual "Maximize" / "Minimize" | "Maximize the focused widget" / "Minimize the widget and restore to normal size" | Textual | M4 | not started | | |
| Textual "Screenshot" | "Save an SVG 'screenshot' of the current screen" | Textual | M4 | not started | | Candidate `intentional diff` (no SVG export in ratatui). |

## Modals

All modals: `q`/`escape` close (`modal.close`) unless a text field has focus; `shift+enter` submits (`modal.submit`) where
listed. Every modal except Create/Edit PR shows the footer.

| Modal | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| Confirm | `ui/screens/confirm.py` | M3 | not started | | Question text, "Yes" / "No" buttons; `y` yes, `n` no, `q`/`escape` no. Only used by Create PR's push-branch prompt. |
| New issue | `ui/screens/new_issue.py` | M3 | not started | | "Title" input, "Description" code-editor textarea (soft wrap, tab moves focus, optional prefilled body), "Save" / "Cancel"; `shift+enter` saves. Toasts error "Must have a non-empty issue title!", "Must have a non-empty issue body!" (Py bug B25), "Creating new issue...", "Successfully created issue #N". New issue is added to the table and selected. `I` without a repo: toast error / "Cannot open new pull request" "Please select a repository first!" (Py bug B14). |
| Edit issue | `ui/screens/edit_issue.py` | M3 | not started | | Prefilled "Title", "Status" select (Open / Closed), "Description"; "Save" (becomes "Saving..." and disabled) / "Cancel"; `shift+enter` saves. Toasts "Updating issue #N..." (1 s) then "Successfully updated issue #N". No error handling. |
| New comment / reply | `ui/screens/new_comment.py` | M3 | not started | | When replying: "Replying to comment from <author> at <%c>", the quoted body, rule. "New comment" textarea, live "Preview" markdown, "Post Comment" / "Cancel". No `shift+enter` (Py bug B17). |
| Add reaction | `ui/screens/add_reactions_modal.py` | M3 | not started | | "Add a new reaction", selection list of 👍 👎 😄 😕 ❤️ 🎉 🚀 👀 minus ones the user already gave, "Submit" / "Cancel". Py bug B20. |
| Create / edit PR | `ui/screens/create_or_edit_pull_request.py` | M3 | not started | | Rows below. |
| Lookup repository / issue / PR | `ui/screens/lookup_repository.py`, `ui/screens/lookup_issue.py`, `ui/screens/lookup_pull_request.py` | M2 | not started | | Rows below. |
| Trigger workflow | `ui/screens/trigger_workflow.py` | M3 | not started | | "Trigger Workflow", "Workflow" select (prompt "Select a workflow"), "Branch" input prefilled with the default branch (or "main") with inline completion from the repo's branches, "Trigger" / "Cancel"; `shift+enter` triggers. Toasts error / "Validation Error" "You must select a workflow!", "You must enter a branch!"; error / "Error Triggering Build" "Could not trigger build - are you sure this workflow supports dispatch events?". No `workflow_dispatch` inputs. |
| Notifications | `ui/screens/notifications.py` | M4 | not started | | Rows below. |
| Paste link | `ui/screens/link_paste.py` | M4 | not started | | "Paste a web GitHub link:", "Link:" input (placeholder "https://github.com/...", validated against repo/issue/PR/user URL patterns), "Open" / "Cancel"; `shift+enter` opens; buttons disabled while resolving. Toasts "Unknown link"; error / "GitHub lookup failed" "Could not resolve link!". |
| User profile | `ui/screens/user_profile.py` | M2 | not started | | Rows below. |
| Settings (incl. key rebinding) | `ui/screens/settings.py` | M4 | not started | | Rows below. |
| Debug info / bug report | `ui/screens/debug.py`, `lib/debug.py` | M4 | not started | | Rows below. |
| Lookup repository: "Search for a repository by name:", "Repository (owner/name):" input validated by `^[^/]+/[^/]+$`, "Continue tracking this repo?" switch, "Lookup" / "Cancel"; tracking adds to `repositories.additional_repos_to_track`; toasts error / "Validation Error" "Could not find repo!", "You must enter a repo name!" | `ui/screens/lookup_repository.py` | M2 | not started | | Py bug B19. |
| Lookup issue: "Search for an issue by number:", "Issue Number:" numeric input, "Lookup" / "Cancel"; toasts error / "Invalid Issue Number" "Must enter a valid issue number!", error / "Unknown Issue" "Could not find issue!" | `ui/screens/lookup_issue.py` | M2 | not started | | Py bug B12. |
| Lookup PR: "Search for a pull request by number:", "Pull Request Number:" numeric input; toasts error / "Invalid PR Number" "Must enter a valid pull request number!", error / "Unknown PR" "Could not find pull request!" | `ui/screens/lookup_pull_request.py` | M2 | not started | | Py bug B12. |
| User profile: "<name> (@login) — <user/organization>", bio, Company / Location / Email / Blog / Twitter lines when set, "Public repos: N  Gists: N  Followers: N  Following: N", "Joined: YYYY-MM-DD" | `ui/screens/user_profile.py` `UserProfileContainer` | M2 | not started | | `@login` is a terminal hyperlink. |
| User profile repos: "Recent repositories (showing N of M, press enter to open)" and a table (Owner, Name, Private, Description) of up to 30 public repos by last update, or "No public repositories."; Enter returns the repo to the caller | `ui/screens/user_profile.py` `UserProfileModal`, `open_user_profile`; `lib/github/repositories.py` `list_repos_for_user` | M2 | not started | | Toasts error "Failed to load profile for @<user>", "Could not find user @<user>". |
| Create PR: heading "New Pull Request"; "Base" (default branch) ← "Compare" (prefilled with the local branch when the cwd repo is the selected repo) and "Draft" switch; branch inputs complete from the repo's branches | `ui/screens/create_or_edit_pull_request.py` `BranchSelection` | M3 | not started | | `P` without a repo: toast error / "Cannot open new pull request" "Please select a repository first!". |
| Create PR: warning "Warning: Current local branch has no configured upstream" when the cwd branch has no upstream | `ui/screens/create_or_edit_pull_request.py` `ensure_directory_branch_has_configured_upstream` | M3 | not started | | Py bug B28. |
| Create PR: "Title" (required), "Description" prefilled from `pull_requests.pull_request_template` if the file exists | `ui/screens/create_or_edit_pull_request.py` `CreateOrEditPullRequestContainer.compose` | M3 | not started | | |
| Create/Edit PR: "Reviewers" input with completion from collaborators plus `pull_requests.additional_suggested_pr_reviewers`; Enter adds (lower-cased, user must exist); "Current Reviewers - Click/Select them to remove" list; toasts error "Already a reviewer!", "Could not find user <u> - are you sure they exist?", "Removing <u>" | `ui/screens/create_or_edit_pull_request.py` `ReviewerSelectionContainer` | M3 | not started | | Py bug B24. |
| Create PR submit ("Create" or `shift+enter`): toasts error / "Invalid PR!" "Missing required fields!", "Creating PR...", "Successfully created PR!", error / "Error creating pull request" "Check that your branches are valid and that a PR does not already exist"; on failure offers Confirm "Warning: Branch <b> may not exist on remote. Do you want to create it?" then pushes (toasts "Pushed branch <b> to remote" / error "Error while pushing branch <b> to remote"); new PR added to the table and selected | `ui/screens/create_or_edit_pull_request.py` `_create_pr`, `check_for_branch_creation`; `lib/git_cli.py` `push_branch_to_remote` | M3 | not started | | Py bug B28. |
| Edit PR: heading "Editing PR N", base/head/draft disabled, prefilled title and body, existing requested reviewers listed, "Update"; toasts "Updating PR...", "Successfully updated PR!", error / "Error updating pull request" "Unexpected error while attempting to update PR"; requests only new reviewers | `ui/screens/create_or_edit_pull_request.py` `_edit_pr` | M3 | not started | | Py bug B24. |
| PR drafts: autosave 2 s after title/base/head/description edits, on Cancel and on close; restore on open with toast "Restored saved PR draft"; cleared when empty or after a successful create; never for edits | `ui/screens/create_or_edit_pull_request.py` `_save_draft_state`, `_restore_draft_state`, `_schedule_draft_save`; `lib/pr_drafts.py` | M3 | not started | | Toggling Draft alone doesn't schedule a save. |
| Notifications modal: "Notifications", tabs "•Unread" (red) and "✓Read" (green); opens on Unread if any, else Read | `ui/screens/notifications.py` `NotificationsContainer` | M4 | not started | | |
| Notifications table columns: "Updated At", "Subject" (type), "Title", "Reason" (title-cased, `_` → space), "Thread ID"; sorted by updated desc; lazy pages of 50 | `ui/screens/notifications.py` `notification_to_row`, `_NotificationsTableTabPane`; `lib/github/notifications.py` | M4 | not started | | |
| Notifications: `r`/`u` switch to Read/Unread; on Unread `R` marks one read (moves it to Read), `ctrl+r` marks all read (toast "Marked all notifications as read", switches to Read) | `ui/screens/notifications.py` `UnreadNotificationTabPane`, `NotificationsContainer` | M4 | not started | | |
| Notifications: Enter selects; marks read when `notifications.mark_notification_as_read_when_selected`; closes; selects the repo; if the subject is a PR, loads it (toast "Opening pull request for notification"), else toast "Opening repository for notification"; unread count refreshes | `ui/screens/notifications.py` `handle_notification_selected`; `ui/screens/primary.py` `action_view_notifications`; `lib/github/notifications.py` `extract_notification_subject` | M4 | not started | | Issue notifications only open the repo. Py bug B7 (`ctrl+n` works when disabled). |
| Settings modal: "LazyGithub Settings", tabs in config order "Appearance", "Notifications", "Key Bindings", "Repositories", "Pull Requests", "Issues", "Cache", "Core", "Api"; each tab shows the section docstring dimmed and one "Field Name:" row per field | `ui/screens/settings.py` `SettingsContainer`, `SettingsSection`, `FieldSetting` | M4 | not started | | Py bug B1, B2. |
| Settings widgets by type: bool → switch, enum → select (title-cased), theme → select of theme names, `list[str]` → "Add new item..." input plus checked list (uncheck to remove), optional path → "Enter file path..." input, other → text input | `ui/screens/settings.py` `FieldSetting._field_to_widget`, `ListSettingWidget`, `PathInput` | M4 | not started | | `api.client_type` select is gh-only in Rust (intentional diff). |
| Settings key rebinding tab: one row per binding id sorted by id, "<description or id>: <current key>"; focus a row and press a key to set it, `escape` resets to default, `tab`/`shift+tab` move on; overrides equal to the default are removed | `ui/screens/settings.py` `BindingsSettingsSection`, `KeySelectionInput` | M4 | not started | | Saved values are Textual key names (`question_mark`, `slash`, `dollar_sign`, `circumflex_accent`). Keymap must parse them (#73). |
| Settings `/` search: fuzzy match on field names (and binding description/id) on Enter | `ui/screens/settings.py` `action_search`, `change_displayed_settings` | M4 | not started | | |
| Settings Save (button or `shift+enter`) / Cancel (button, `q`, `escape`); on save: toast "Settings updated", apply theme, keymap, shown sections, restart notification refresh | `ui/screens/settings.py` `_update_settings`; `ui/app.py` `on_settings_modal_dismissed`; `ui/screens/primary.py` `handle_settings_update`; `lib/messages.py` `SettingsModalDismissed` | M4 | not started | | |
| Debug modal: markdown "Library Versions" (LazyGithub, Textual, Click, Httpx, Hishel, Pydantic) and "LazyGithub Config" JSON; "Copy to Clipboard" (toast information / "Copied" "Debug info copied to clipboard") / "Close" | `ui/screens/debug.py` `DebugModal`; `lib/debug.py` `collect_debug_info` | M4 | not started | | Library list becomes Rust/gh versions (intentional diff). `shift+enter` is bound but does nothing. |

## Toasts

Exact texts, so parity scenarios can assert them. Rows above say when each fires. Toasts raised inside a modal are
listed with that modal under [Modals](#modals).

| Text | Severity / title | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| "Settings updated" | information | `ui/app.py` | M4 | not started | | |
| "Current view maximized. Press escape to return." | information / "View Maximized" | `ui/app.py` | M1 | not started | #75 | Once per session. |
| "Please select a repository first!" | error / "Cannot open new pull request" | `ui/screens/primary.py` | M3 | not started | | Same title for issues (Py bug B14). |
| "Error fetching pull request!" | error / "API Error" | `ui/screens/primary.py` | M2 | not started | | |
| "Opening pull request for link", "Opening issue for link", "Opening repository for link" | information | `ui/screens/primary.py` | M4 | not started | | |
| "Opening repository for notification", "Opening pull request for notification" | information | `ui/screens/primary.py` | M4 | not started | | |
| "Cannot load notifications - please login to the gh CLI: gh auth login" | error / "Failed to Load Notifiations" | `ui/screens/primary.py` | M1 | not started | #81 | Py bug B14. |
| "Could not load LazyGithub repo! Please open the bug report on Github" | error / "Error opening bug report" | `ui/screens/primary.py` | M4 | not started | | |
| "No repository currently selected" / "No pull request currently selected" / "No issue currently selected" | error | `ui/widgets/repositories.py`, `ui/widgets/pull_requests.py`, `ui/widgets/issues.py` | M2 | not started | | |
| "This PR has already been merged!" | warning | `ui/widgets/pull_requests.py` | M3 | not started | | |
| "PR has already been merged!" | warning / "Already Merged" | `ui/widgets/pull_requests.py` | M3 | not started | | |
| "Pull request N merged. Note some cached information on the UI may be out of date." | information / "PR Merged" | `ui/widgets/pull_requests.py` | M3 | not started | | |
| "Pull request N could not be merged" | error / "Error Merging PR" | `ui/widgets/pull_requests.py` | M3 | not started | | |
| "Comment cannot be empty!" | warning | `ui/widgets/split_diff_viewer.py` | M3 | not started | | |
| "New comment added", "New review created!" | information | `ui/widgets/split_diff_viewer.py` | M3 | not started | | |
| "Please select a status for the new review!" | error | `ui/widgets/split_diff_viewer.py` | M3 | not started | | |
| "Workflow run queued for re-run", "Failed jobs queued for re-run" | information / "Re-run Started" | `ui/widgets/workflow_run_details.py` | M3 | not started | | |
| "Failed to re-run workflow", "Failed to re-run failed jobs" | error / "Error" | `ui/widgets/workflow_run_details.py` | M3 | not started | | |
| "No repository selected", "No workflows found in this repository" | warning | `ui/widgets/workflows.py` | M3 | not started | | |
| "Successfully triggered workflow" | information | `ui/widgets/workflows.py` | M3 | not started | | |
| "No links visible" | information | `ui/widgets/link_hints.py` | M4 | not started | | |
| "Opening <url> externally" | information | `ui/widgets/link_hints.py` | M4 | not started | | |

## Keybindings

Ids and defaults from `lib/bindings.py`. The Rust keymap must use the same ids so `bindings.overrides` keeps working.
Override values may be comma-separated lists (`"q, escape"`, with or without spaces) and use Textual key names.

| Binding id | Default key | Description | Context | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| (all ids) | | Registered with defaults and `bindings.overrides` | keymap | M1 | not started | #73 | Ids below whose feature lands later are still registered in M1 so overrides round-trip. |
| `app.quit` | `q` | Quit | app | M1 | not started | #73 | Also on the auth screen. |
| `app.command_palette` | `ctrl+p` | Command Pallete | app | M4 | not started | | Description typo is Python's (Py bug B14). Hidden from footer. |
| `app.maximize_widget` | `M` | Maximize | app | M1 | not started | #75 | |
| `app.open_help` | `?` | Keybindings | app | M1 | not started | #76 | Toggles. TERM: shifted character; override may be saved as `question_mark`. |
| `conversation.comment.new` | `n` | New Comment | PR Conversation, Issue Comments | M3 | not started | | |
| `conversation.review.reply` | `r` | Reply to Review | review block | M3 | not started | | Same key as `conversation.comment.reply`; the innermost focused block wins. |
| `conversation.comment.reply` | `r` | Reply to Comment | comment block | M3 | not started | | |
| `issue.new` | `I` | New Issue | selections pane | M3 | not started | | |
| `issue.edit` | `E` | Edit Issue | issues table, Issue Overview | M3 | not started | | |
| `issue.lookup` | `O` | Lookup Issue | issues table | M2 | not started | | |
| `pull_request.new` | `P` | New PR | selections pane | M3 | not started | | |
| `pull_request.merge` | `ctrl+m` | Merge PR | PR Overview | M3 | not started | | TERM: `ctrl+m` sends CR, same as Enter, without the kitty protocol. Must never fire on plain Enter (Py bug B30). |
| `pull_request.edit` | `E` | Edit PR | PR table, PR Overview | M3 | not started | | |
| `pull_request.lookup` | `O` | Lookup Pull Request | PR table | M2 | not started | | |
| `repositories.favorite` | `ctrl+f` | Toggle Favorite | repos table | M2 | not started | | |
| `repositories.lookup` | `O` | Lookup Repository | repos table | M2 | not started | | |
| `user.view_profile` | `u` | View User | repos, PR and issues tables | M2 | not started | | |
| `common.table.select` | `enter,space` | Select table entry | tables | M1 | not started | #78 | |
| `common.table.search` | `/` | Search | tables | M1 | not started | #78 | Override may be saved as `slash`. |
| `common.table.cursor_down` | `j` | Table cursor down | tables | M1 | not started | #78 | |
| `common.table.page_down` | `J` | Table page down | tables | M1 | not started | #78 | |
| `common.table.cursor_up` | `k` | Table cursor up | tables | M1 | not started | #78 | |
| `common.table.page_up` | `K` | Table page up | tables | M1 | not started | #78 | |
| `common.table.scroll_right` | `l` | Table scroll right | tables | M1 | not started | #78 | |
| `common.table.page_right` | `$` | Table page right | tables | M1 | not started | #78 | An earlier `L` definition is shadowed (Py bug B9). Override may be saved as `dollar_sign`. |
| `common.table.scroll_left` | `h` | Table scroll left | tables | M1 | not started | #78 | |
| `common.table.page_left` | `^` | Table page left | tables | M1 | not started | #78 | An earlier `H` definition is shadowed (Py bug B9). Override may be saved as `circumflex_accent`. |
| `common.table.scroll_top` | `g` | Table scroll to top | tables | M1 | not started | #78 | |
| `common.table.scroll_bottom` | `G` | Table scroll to bottom | tables | M1 | not started | #78 | |
| `workflows.trigger` | `T` | Trigger Workflow | workflows section | M3 | not started | | |
| `workflows.view_run_details` | `enter` | View Run Details | (unused) | M2 | not started | | Defined, rebindable in Settings, never bound (Py bug B9). |
| `workflows.view_job_logs` | `l` | View Job Logs | Jobs tab | M2 | not started | | |
| `diff.add_comment` | `c` | Add Comment | hunk pane | M3 | not started | | |
| `diff.cursor_down` | `j` | cursor_down | (unused) | M2 | not started | | Hunk pane hard-codes `j` (Py bug B9). |
| `diff.cursor_up` | `k` | cursor_up | (unused) | M2 | not started | | Hunk pane hard-codes `k` (Py bug B9). |
| `diff.clear_selection` | `escape` | Clear Selection | (unused) | M2 | not started | | Never bound (Py bug B9). |
| `diff.next_hunk` | `J` | Next Hunk | diff viewer | M2 | not started | | Py bug B8. |
| `diff.previous_hunk` | `K` | Previous Hunk | diff viewer | M2 | not started | | Py bug B8. |
| `diff.filter_files` | `/` | Filter Files | diff viewer | M2 | not started | | |
| `notifications.open` | `ctrl+n` | Notifications | main screen | M4 | not started | | Shown in footer only when enabled at startup (Py bug B7). |
| `notifications.mark_all_as_read` | `ctrl+r` | Mark all as read | Unread tab | M4 | not started | | |
| `notifications.mark_read` | `R` | Mark as Read | Unread tab | M4 | not started | | |
| `notifications.view_read` | `r` | View Read Notifications | notifications modal | M4 | not started | | |
| `notifications.view_unread` | `u` | View Unread Notifications | notifications modal | M4 | not started | | |
| `modal.submit` | `shift+enter` | Submit | most modals | M3 | not started | | TERM: needs the kitty keyboard protocol (crossterm `PushKeyboardEnhancementFlags`); legacy terminals send plain Enter. Buttons are the fallback. |
| `modal.close` | `q, escape` | Close | all modals | M3 | not started | | Inactive while a text input has focus (Textual inputs consume `q`). |
| `modal.search` | `/` | Search | Settings | M4 | not started | | |
| `confirm.yes` | `y` | Yes | Confirm | M3 | not started | | |
| `confirm.no` | `n` | No | Confirm | M3 | not started | | |
| `react.add` | `R` | Add reaction | PR Overview | M3 | not started | | |
| `link.paste` | `ctrl+v` | Paste link | main screen | M4 | not started | | TERM: many terminals turn `ctrl+v` into a paste; consider also opening the modal on a bracketed-paste event on the main screen. |
| `link.follow` | `f` | Follow link | main screen | M4 | not started | | |
| `link.follow_external` | `F` | Follow link (browser) | main screen | M4 | not started | | |
| `main.repos.focus` | `1` | Focus repos table | main view | M1 | not started | #75 | |
| `main.pull_requests.focus` | `2` | Focus PRs table | main view | M1 | not started | #75 | |
| `main.issues.focus` | `3` | Focus issues table | main view | M1 | not started | #75 | |
| `main.workflows.focus` | `4` | Focus workflows table | main view | M1 | not started | #75 | |
| `main.details.focus` | `5` | Focus details tabs | main view | M1 | not started | #75 | |
| `main.command_log.focus` | `6` | Focus command log | main view | M1 | not started | #75 | |

## Config

Every field in `lib/config.py` must load from an existing `config.json` and round-trip on save. File:
`~/.config/lazy-github/config.json`, written with 4-space indent. Unknown fields are ignored. Values written by the Python
Settings modal can be strings where a number is expected (Py bug B2), so the loader accepts both.

| Section | Field | Default | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `appearance` | `theme` | `"textual-dark"` | `lib/config.py` `AppearanceSettings` | M0 | not started | #64 | Unknown name → default. Applied in #77. |
| `appearance` | `show_command_log` | `false` | `lib/config.py` | M0 | not started | #64 | Used by #74/#80. |
| `appearance` | `show_workflows` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `appearance` | `show_issues` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `appearance` | `show_pull_requests` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `notifications` | `enabled` | `false` | `lib/config.py` `NotificationSettings` | M0 | not started | #64 | |
| `notifications` | `mark_notification_as_read_when_selected` | `true` | `lib/config.py` | M0 | not started | #64 | |
| `bindings` | `overrides` | `{}` | `lib/config.py` `BindingsSettings` | M0 | not started | #64 | Map of binding id → key string; applied in #73. |
| `repositories` | `additional_repos_to_track` | `[]` | `lib/config.py` `RepositorySettings` | M0 | not started | #64 | |
| `repositories` | `favorites` | `[]` | `lib/config.py` | M0 | not started | #64 | |
| `pull_requests` | `state_filter` | `"all"` (`all`/`open`/`closed`) | `lib/config.py` `PullRequestSettings`, `lib/constants.py` `IssueStateFilter` | M0 | not started | #64 | |
| `pull_requests` | `owner_filter` | `"all"` (`all`/`mine`) | `lib/config.py`, `lib/constants.py` `IssueOwnerFilter` | M0 | not started | #64 | |
| `pull_requests` | `preferred_merge_method` | `"squash"` (`merge`/`squash`/`rebase`) | `lib/config.py` `MergeMethod` | M0 | not started | #64 | |
| `pull_requests` | `additional_suggested_pr_reviewers` | `[]` | `lib/config.py` | M0 | not started | #64 | |
| `pull_requests` | `preload_pull_request_for_current_commit` | `false` | `lib/config.py` | M0 | not started | #64 | |
| `pull_requests` | `pull_request_template` | `null` (path) | `lib/config.py` | M0 | not started | #64 | |
| `issues` | `state_filter` | `"all"` | `lib/config.py` `IssueSettings` | M0 | not started | #64 | |
| `issues` | `owner_filter` | `"all"` | `lib/config.py` | M0 | not started | #64 | |
| `cache` | `auth_cache_duration` | `86400` | `lib/config.py` `CacheSettings` | M0 | not started | #64 | Seconds. |
| `cache` | `auth_last_checked` | `null` (ISO datetime) | `lib/config.py` | M0 | not started | #64 | Python writes naive local time. A `"None"` string can appear (Py bug B2). |
| `cache` | `cache_directory` | `~/.config/lazy-github/.cache` | `lib/config.py` | M0 | not started | #64 | Absolute path is written. |
| `cache` | `default_ttl` | `600` | `lib/config.py` | M0 | not started | #64 | |
| `cache` | `list_repos_ttl` | `86400` | `lib/config.py` | M0 | not started | #64 | |
| `cache` | `list_issues_ttl` | `3600` | `lib/config.py` | M0 | not started | #64 | Never read by the Python app. |
| `core` | `first_start` | `true` | `lib/config.py` `CoreConfig` | M0 | not started | #64 | Rust writes `false` after the gh check (no picker). |
| `core` | `logfile_path` | `~/.config/lazy-github/lazy_github.log` | `lib/config.py` | M0 | not started | #64, #66 | |
| `core` | `logfile_max_bytes` | `5000000` | `lib/config.py` | M0 | not started | #64, #66 | |
| `core` | `logfile_count` | `5` | `lib/config.py` | M0 | not started | #64, #66 | |
| `api` | `base_url` | `"https://api.github.com"` | `lib/config.py` `ApiConfig` | M0 | not started | #64 | Preserved; decide whether it maps to `gh --hostname`. |
| `api` | `client_type` | `"RAW_HTTP"` (ignored, gh only) | `lib/config.py`, `lib/github/backends/protocol.py` `BackendType` | M0 | intentional diff (gh only) | #64 | Preserved on save, not acted on. |

## Local caches and drafts

All under `cache.cache_directory`. Repo-scoped names use `full_name` with `/` replaced by `_`. A missing, corrupt or
schema-mismatched file is ignored (logged as a warning) and fresh data is fetched.

| Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| `repos.json`: list of repository models for the repos table | `lib/cache.py`, `ui/widgets/repositories.py` | M2 | not started | | |
| `<owner>_<repo>_pull_requests.json` | `lib/cache.py`, `ui/widgets/pull_requests.py` | M2 | not started | | |
| `<owner>_<repo>_issues.json` | `lib/cache.py`, `ui/widgets/issues.py` | M2 | not started | | |
| `<owner>_<repo>_workflow_runs.json` | `lib/cache.py`, `ui/widgets/workflows.py` | M2 | not started | | |
| `notifications.json`: written by both notification tabs, never read | `ui/screens/notifications.py` | M4 | not started | | Candidate `intentional diff`: don't write it (Py bug B29). |
| All-or-nothing load: one bad entry discards the whole file | `lib/cache.py` `load_models_from_cache` | M2 | not started | | Rust models must accept the Python `model_dump(mode="json")` shape. |
| PR drafts `pr_drafts/<owner>_<repo>.json`: `repo_full_name`, `title`, `description`, `base_ref`, `head_ref`, `is_draft`, `reviewers` (2-space indent) | `lib/pr_drafts.py` | M3 | not started | | Corrupt draft ignored. Behavior row under [Modals](#modals). |

## CLI

`lazy-github` (and `gh lazy`) with no subcommand runs `run`. `-h`/`--help` on every command.

| Command | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- |
| `run [--auth-backend]` | `cli.py` | M1 | not started | | Bare launch in M1; `--auth-backend` (env `AUTH_BACKEND`) writes `api.client_type` to config, M4. |
| `dump-config` | `cli.py` | M4 | not started | | Prints "Config file location: <path> (exists => True/False)" then the config as pretty JSON. |
| `clear-auth` | `cli.py` | M4 | intentional diff (defer to `gh auth logout`) | | |
| `clear-config [--no-confirm]` | `cli.py` | M4 | not started | | Prompt "Confirm deletion your LazyGithub settings"; prints "Your settings have been cleared" or "Canceling". |
| `clear-cache [--no-confirm]` | `cli.py` | M4 | not started | | Prompt "Confirm deletion of everything in <dir>"; prints "Cache cleared" or "Canceling cache deletion". |
| `debug` | `cli.py`, `lib/debug.py` | M4 | not started | | Prints the debug markdown. |
| `version` | `cli.py` | M4 | not started | | Prints "LazyGithub version <x>". |
| `LAZY_GITHUB_ORIGINAL_PWD` set to the cwd if unset (the `gh-lazy` extension script sets it) | `cli.py` `cli`, `gh-lazy` | M1 | not started | | |

## Terminal caveats

| Caveat | Affects | Recommendation |
| --- | --- | --- |
| `ctrl+m` and Enter are the same byte (CR) without the kitty keyboard protocol. | `pull_request.merge` | Keep the id and default. Only treat a disambiguated `ctrl+m` as merge; never Enter. See Py bug B30. |
| `shift+enter` arrives as plain Enter in legacy terminals. | `modal.submit` | Push kitty enhancement flags when supported; keep the Submit buttons working; don't map plain Enter to submit inside multi-line editors. |
| `ctrl+v` is often captured by the terminal as paste. | `link.paste` | Also handle bracketed paste on the main screen as "open paste modal with this text" (needs a decision if it changes UX). |
| Uppercase bindings (`M`, `I`, `P`, `E`, `O`, `J`, `K`, `G`, `R`, `T`, `F`) arrive as shifted characters; some terminals report `shift+m`. | all uppercase ids | Normalize `shift+<letter>` and `<LETTER>` to the same key. |
| Punctuation keys are saved by the Python Settings modal as Textual names (`question_mark`, `slash`, `dollar_sign`, `circumflex_accent`, …). | `bindings.overrides` | Keymap parser accepts Textual key names and literal characters (#73). |
| `escape` is ambiguous with Alt-prefixed sequences; crossterm handles the timeout. | `modal.close`, maximize restore, link hints | No action beyond crossterm defaults. |
| `q` closes modals and quits the app, but must type a `q` inside text inputs. | `app.quit`, `modal.close` | Text inputs get first refusal on printable keys. |
| Footer key labels: Textual shows `^p` for `ctrl+p`. | footer, help panel | Decide in #76. |
| Terminal hyperlinks (`[link=…]`) are used for a few links (run number, profile login); others are in-app links. | Run Overview, User profile | Render as OSC 8 where supported; treat as hint targets only if the spec says so. |

## Suspected Python bugs

Listed for Chris in [this comment on #71](https://github.com/gizmo385/gh-lazy/issues/71#issuecomment-6392375110). Each row in the matrix that says `Py bug Bn` follows the answer. Until
answered, the recommendation stands. None of them blocks the matrix.

| Id | Behavior in Python | Python reference | Recommendation |
| --- | --- | --- | --- |
| B1 | Settings field widget ids aren't section-qualified; `state_filter`/`owner_filter` exist in both Pull Requests and Issues, so saving reads the PR widgets for Issues and the Issues filters can't be changed. | `ui/screens/settings.py` `_id_for_field_input`, `_update_settings` | Fix in Rust (`intentional diff`). |
| B2 | Settings stores text-input values as strings: numeric fields are saved as `"600"`, and a null `auth_last_checked` becomes `"None"` (which fails to load). | `ui/screens/settings.py` `_update_settings` | Rust saves typed values; the loader accepts numeric strings and treats `"None"` as null. |
| B3 | The first PR page is fetched with `issues.state_filter`/`owner_filter`; later pages use `pull_requests.*`. | `ui/screens/primary.py` `fetch_issues_and_pull_requests` | Use `pull_requests.*` for the PR table throughout. |
| B4 | Workflow runs are keyed by `run_number`, which is per workflow, so runs of different workflows overwrite each other. | `ui/widgets/workflows.py` | Key by run id; keep the "Run #" column. |
| B5 | Current-directory repo detection keeps `.git` on HTTPS remotes and truncates SSH repo names at the first `.`. | `lib/git_cli.py` | Parse remotes correctly. |
| B6 | Unread badge shows "N+" at N ≥ 30, but the count is capped by a 50-item page. | `ui/screens/primary.py` `UnreadNotifications` | Show "50+" only when the page is full, exact count otherwise. |
| B7 | `ctrl+n` opens Notifications even when disabled; the footer hint is decided once at import time. | `lib/bindings.py`, `ui/screens/primary.py` | Binding inert and hidden while disabled, re-evaluated after Settings. |
| B8 | `J`/`K` in the diff viewer do nothing (they target non-focusable widgets inside collapsed sections). | `ui/widgets/split_diff_viewer.py` | Implement: move to next/previous hunk, expanding it. |
| B9 | Dead binding ids `diff.cursor_down`, `diff.cursor_up`, `diff.clear_selection`, `workflows.view_run_details` (rebindable, no effect); hunk `j`/`k` hard-coded; `H`/`L` table defs shadowed by `^`/`$`. | `lib/bindings.py`, `ui/widgets/split_diff_viewer.py` | Wire `diff.cursor_*` to hunk movement; keep the other two ids accepted but hidden; drop `H`/`L`. |
| B10 | PR Conversation skips reviews that are `COMMENTED` with an empty body, which hides single inline comments. | `ui/widgets/pull_requests.py` `handle_reviews_loaded` | Show their comment threads. |
| B11 | Issue overview title uses unclosed `[b]` and unescaped title markup. | `ui/widgets/issues.py` `IssueOverviewTabPane.compose` | Render the title literally in bold. |
| B12 | Lookup issue/PR with no repository selected crashes (assert). | `ui/screens/lookup_issue.py`, `ui/screens/lookup_pull_request.py` | Toast error "Please select a repository first!". |
| B13 | `ctrl+f`, Enter and `E` on an empty table raise instead of no-op. | `ui/widgets/repositories.py`, `ui/widgets/pull_requests.py` | No-op (or the "No … currently selected" toast). |
| B14 | Text typos: issue flow toast title "Cannot open new pull request", "Failed to Load Notifiations", binding description "Command Pallete". | `ui/screens/primary.py`, `lib/bindings.py` | Fix the text (`intentional diff`). |
| B15 | After merge, review submit and re-run, the app re-posts the stale object instead of refetching, so the view doesn't update. | `ui/widgets/pull_requests.py`, `ui/widgets/split_diff_viewer.py`, `ui/widgets/workflow_run_details.py` | Refetch. |
| B16 | Loading spinners never clear on API errors (PR conversation, PR checks, issue comments, workflows, jobs). | several | Show an inline error message (`intentional diff`). |
| B17 | New comment: errors from the gh backend aren't caught; handled errors close the modal and lose the text; no `shift+enter`. | `ui/screens/new_comment.py` | Keep the modal open with an error toast; bind `modal.submit`. |
| B18 | Replies in PR Conversation don't appear until reload; replying to a review posts a top-level comment. | `ui/widgets/conversations.py`, `ui/widgets/pull_requests.py` | Refetch after any reply; keep top-level reply semantics. |
| B19 | Lookup repository saves "continue tracking" before checking the repo exists. | `ui/screens/lookup_repository.py` | Save only on success. |
| B20 | Add reaction can't remove reactions (already-given ones aren't listed). | `ui/screens/add_reactions_modal.py` | Keep parity (add only); file removal as an enhancement. |
| B21 | Jobs tab hint says "press 'L' or 'Enter'"; the key is `l` and Enter only expands. | `ui/widgets/workflow_run_details.py` | Hint shows the actual key from the keymap. |
| B22 | Diff gutter numbers count deleted lines as new-file lines; pending comment shows a hunk-relative index as `file:line`. | `ui/widgets/split_diff_viewer.py` | Show real old/new line numbers. |
| B23 | Status checks are sorted by their markup string, giving an arbitrary order. | `ui/widgets/pull_requests.py` `load_checks` | Sort failing, pending, passing, then by name. |
| B24 | Edit PR: removing an already-requested reviewer does nothing on GitHub; an empty title isn't rejected. | `ui/screens/create_or_edit_pull_request.py` `_edit_pr` | Keep reviewer behavior for parity; reject empty title. |
| B25 | New issue requires a non-empty body (GitHub doesn't). | `ui/screens/new_issue.py` | Allow empty body. |
| B26 | Times are shown in UTC (aware UTC datetimes formatted with `%c` or fixed formats). | throughout | Keep UTC for parity now; local time is a separate decision. |
| B27 | Theme picked from the command palette isn't saved. | Textual system command | Keep parity (session only). |
| B28 | Branch-exists, upstream and push checks run in the process cwd, not `LAZY_GITHUB_ORIGINAL_PWD`; push sends `HEAD` to the named branch. | `lib/git_cli.py` | Run in the original cwd; push the named branch. |
| B29 | `notifications.json` is written by both tabs and never read. | `ui/screens/notifications.py` | Don't write it. |
| B30 | Merge has no confirmation, and its default `ctrl+m` is indistinguishable from Enter in most terminals. | `ui/widgets/pull_requests.py`, `lib/bindings.py` | Keep id and default; never fire on Enter; keep no-confirm for parity unless Chris wants a Confirm. |
