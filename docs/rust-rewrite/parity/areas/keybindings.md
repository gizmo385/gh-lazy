# Keybindings

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

Ids and defaults from `lib/bindings.py`. The Rust keymap must use the same ids so `bindings.overrides` keeps working.
Override values may be comma-separated lists (`"q, escape"`, with or without spaces) and use Textual key names.

| Id | Binding id | Default key | Description | Context | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `keybindings.all-ids` | (all ids) | | Registered with defaults and `bindings.overrides` | keymap | M1 | parity | #73, #157 | Ids below whose feature lands later are still registered in M1 so overrides round-trip. |
| `keybindings.override-keeps-shared-keys` | (all ids) | | Overriding a binding removes only that binding's default keys, not other bindings' in the same component that share them | keymap | M1 | intentional diff | #119 | Textual's `apply_keymap` drops the whole key entry, so they lose it too. No two bindings in one component share a default key today, so nothing visible differs. Decision: [override-strips-only-its-own-keys](../decisions/override-strips-only-its-own-keys.md). |
| `keybindings.app-quit` | `app.quit` | `q` | Quit | app | M1 | parity | #73, #106, #158 | Also on the auth screen. |
| `keybindings.app-command-palette` | `app.command_palette` | `ctrl+p` | Command Pallete | app | M4 | not started | #191 | Description typo is Python's (Py bug B14). Shown in the footer as `^p Command Pallete`, after `q Quit` (#107). |
| `keybindings.app-maximize-widget` | `app.maximize_widget` | `M` | Maximize | app | M1 | parity | #75, #158 | |
| `keybindings.app-open-help` | `app.open_help` | `?` | Keybindings | app | M1 | parity | #76, #108, #158 | Toggles the help panel (`main-layout.help-panel`). TERM: shifted character; override may be saved as `question_mark`. |
| `keybindings.conversation-comment-new` | `conversation.comment.new` | `n` | New Comment | PR Conversation, Issue Comments | M3 | not started | #182 | |
| `keybindings.conversation-review-reply` | `conversation.review.reply` | `r` | Reply to Review | review block | M3 | not started | #182 | Same key as `conversation.comment.reply`; the innermost focused block wins. |
| `keybindings.conversation-comment-reply` | `conversation.comment.reply` | `r` | Reply to Comment | comment block | M3 | not started | #182 | |
| `keybindings.issue-new` | `issue.new` | `I` | New Issue | selections pane | M3 | not started | #181 | |
| `keybindings.issue-edit` | `issue.edit` | `E` | Edit Issue | issues table, Issue Overview | M3 | not started | #181 | |
| `keybindings.issue-lookup` | `issue.lookup` | `O` | Lookup Issue | issues table | M2 | not started | | |
| `keybindings.pull-request-new` | `pull_request.new` | `P` | New PR | selections pane | M3 | not started | #184 | |
| `keybindings.pull-request-merge` | `pull_request.merge` | `ctrl+m` | Merge PR | PR Overview | M3 | not started | #183 | TERM: `ctrl+m` sends CR, same as Enter, without the kitty protocol. Must never fire on plain Enter (Py bug B30). |
| `keybindings.pull-request-edit` | `pull_request.edit` | `E` | Edit PR | PR table, PR Overview | M3 | not started | #185 | |
| `keybindings.pull-request-lookup` | `pull_request.lookup` | `O` | Lookup Pull Request | PR table | M2 | not started | | |
| `keybindings.repositories-favorite` | `repositories.favorite` | `ctrl+f` | Toggle Favorite | repos table | M2 | not started | | |
| `keybindings.repositories-lookup` | `repositories.lookup` | `O` | Lookup Repository | repos table | M2 | not started | | |
| `keybindings.user-view-profile` | `user.view_profile` | `u` | View User | repos, PR and issues tables | M2 | not started | | |
| `keybindings.common-table-select` | `common.table.select` | `enter,space` | Select table entry | tables | M1 | in progress | #78, #100, #158, #170 | |
| `keybindings.common-table-search` | `common.table.search` | `/` | Search | tables | M1 | in progress | #78, #101, #158, #169 | Override may be saved as `slash`. Rust (#101): `SEARCHABLE_BINDINGS` in `components/table/`, checked after the table's own bindings. |
| `keybindings.common-table-cursor-down` | `common.table.cursor_down` | `j` | Table cursor down | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-page-down` | `common.table.page_down` | `J` | Table page down | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-cursor-up` | `common.table.cursor_up` | `k` | Table cursor up | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-page-up` | `common.table.page_up` | `K` | Table page up | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-scroll-right` | `common.table.scroll_right` | `l` | Table scroll right | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-page-right` | `common.table.page_right` | `$` | Table page right | tables | M1 | not started | #78, #100, #158 | An earlier `L` definition is shadowed (Py bug B9). Override may be saved as `dollar_sign`. |
| `keybindings.common-table-scroll-left` | `common.table.scroll_left` | `h` | Table scroll left | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-page-left` | `common.table.page_left` | `^` | Table page left | tables | M1 | not started | #78, #100, #158 | An earlier `H` definition is shadowed (Py bug B9). Override may be saved as `circumflex_accent`. |
| `keybindings.common-table-scroll-top` | `common.table.scroll_top` | `g` | Table scroll to top | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.common-table-scroll-bottom` | `common.table.scroll_bottom` | `G` | Table scroll to bottom | tables | M1 | not started | #78, #100, #158 | |
| `keybindings.workflows-trigger` | `workflows.trigger` | `T` | Trigger Workflow | workflows section | M3 | not started | #188 | |
| `keybindings.workflows-view-run-details` | `workflows.view_run_details` | `enter` | View Run Details | (unused) | M2 | not started | | Defined, rebindable in Settings, never bound (Py bug B9). |
| `keybindings.workflows-view-job-logs` | `workflows.view_job_logs` | `l` | View Job Logs | Jobs tab | M2 | not started | | |
| `keybindings.diff-add-comment` | `diff.add_comment` | `c` | Add Comment | hunk pane | M3 | not started | #187 | |
| `keybindings.diff-cursor-down` | `diff.cursor_down` | `j` | cursor_down | (unused) | M2 | not started | | Hunk pane hard-codes `j` (Py bug B9). |
| `keybindings.diff-cursor-up` | `diff.cursor_up` | `k` | cursor_up | (unused) | M2 | not started | | Hunk pane hard-codes `k` (Py bug B9). |
| `keybindings.diff-clear-selection` | `diff.clear_selection` | `escape` | Clear Selection | (unused) | M2 | not started | | Never bound (Py bug B9). |
| `keybindings.diff-next-hunk` | `diff.next_hunk` | `J` | Next Hunk | diff viewer | M2 | not started | | Py bug B8. |
| `keybindings.diff-previous-hunk` | `diff.previous_hunk` | `K` | Previous Hunk | diff viewer | M2 | not started | | Py bug B8. |
| `keybindings.diff-filter-files` | `diff.filter_files` | `/` | Filter Files | diff viewer | M2 | not started | | |
| `keybindings.notifications-open` | `notifications.open` | `ctrl+n` | Notifications | main screen | M4 | not started | #75, #190 | Shown in footer only when enabled at startup (Py bug B7). Rust (#75): hidden and inactive while `notifications.enabled` is false, re-read on config change; the key falls through. |
| `keybindings.notifications-mark-all-as-read` | `notifications.mark_all_as_read` | `ctrl+r` | Mark all as read | Unread tab | M4 | not started | #189 | |
| `keybindings.notifications-mark-read` | `notifications.mark_read` | `R` | Mark as Read | Unread tab | M4 | not started | #189 | |
| `keybindings.notifications-view-read` | `notifications.view_read` | `r` | View Read Notifications | notifications modal | M4 | not started | #189 | |
| `keybindings.notifications-view-unread` | `notifications.view_unread` | `u` | View Unread Notifications | notifications modal | M4 | not started | #189 | |
| `keybindings.modal-submit` | `modal.submit` | `shift+enter` | Submit | most modals | M2 | not started | #213 | TERM: needs the kitty keyboard protocol (crossterm `PushKeyboardEnhancementFlags`); legacy terminals send plain Enter. Buttons are the fallback. |
| `keybindings.modal-close` | `modal.close` | `q, escape` | Close | all modals | M2 | in progress | #212 | Inactive while a text input has focus (Textual inputs consume `q`). |
| `keybindings.modal-search` | `modal.search` | `/` | Search | Settings | M4 | not started | #193 | |
| `keybindings.confirm-yes` | `confirm.yes` | `y` | Yes | Confirm | M3 | not started | #214, #184 | |
| `keybindings.confirm-no` | `confirm.no` | `n` | No | Confirm | M3 | not started | #214, #184 | |
| `keybindings.react-add` | `react.add` | `R` | Add reaction | PR Overview | M3 | not started | #183 | |
| `keybindings.link-paste` | `link.paste` | `ctrl+v` | Paste link | main screen | M4 | not started | #197 | TERM: many terminals turn `ctrl+v` into a paste; consider also opening the modal on a bracketed-paste event on the main screen. |
| `keybindings.link-follow` | `link.follow` | `f` | Follow link | main screen | M4 | not started | #196 | |
| `keybindings.link-follow-external` | `link.follow_external` | `F` | Follow link (browser) | main screen | M4 | not started | #196 | |
| `keybindings.main-repos-focus` | `main.repos.focus` | `1` | Focus repos table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-pull-requests-focus` | `main.pull_requests.focus` | `2` | Focus PRs table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-issues-focus` | `main.issues.focus` | `3` | Focus issues table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-workflows-focus` | `main.workflows.focus` | `4` | Focus workflows table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-details-focus` | `main.details.focus` | `5` | Focus details tabs | main view | M1 | parity | #75, #158 | |
| `keybindings.main-command-log-focus` | `main.command_log.focus` | `6` | Focus command log | main view | M1 | parity | #75, #158 | |
