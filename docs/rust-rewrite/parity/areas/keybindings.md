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
| `keybindings.app-command-palette` | `app.command_palette` | `ctrl+p` | Command Pallete | app | M4 | parity | #191, #300 | Description typo is Python's (Py bug B14, answered: Rust says "Command Palette"). Shown in the footer as `^p Command Pallete`, after `q Quit` (#107). Opens the palette on the main screen; does nothing under a modal, the palette included. Scenario `command-palette`. |
| `keybindings.app-maximize-widget` | `app.maximize_widget` | `M` | Maximize | app | M1 | parity | #75, #158 | |
| `keybindings.app-open-help` | `app.open_help` | `?` | Keybindings | app | M1 | parity | #76, #108, #158 | Toggles the help panel (`main-layout.help-panel`). TERM: shifted character; override may be saved as `question_mark`. |
| `keybindings.conversation-comment-new` | `conversation.comment.new` | `n` | New Comment | PR Conversation, Issue Comments | M3 | parity | #182, #260, #261 | |
| `keybindings.conversation-review-reply` | `conversation.review.reply` | `r` | Reply to Review | review block | M3 | parity | #182, #261 | Same key as `conversation.comment.reply`; the innermost focused block wins. Unit-tested (`r_on_a_review_replies_with_an_issue_comment`, the comment's `r` winning in `r_on_a_review_comment_replies_in_its_thread`); scenario `pr-comment` sends the comment's `r`, and `pr-review-reply` the review's. |
| `keybindings.conversation-comment-reply` | `conversation.comment.reply` | `r` | Reply to Comment | comment block | M3 | parity | #182, #260, #261 | |
| `keybindings.issue-new` | `issue.new` | `I` | New Issue | selections pane | M3 | parity | #181, #258 | |
| `keybindings.issue-edit` | `issue.edit` | `E` | Edit Issue | issues table, Issue Overview | M3 | parity | #181, #259 | |
| `keybindings.issue-lookup` | `issue.lookup` | `O` | Lookup Issue | issues table | M2 | parity | #247 | |
| `keybindings.pull-request-new` | `pull_request.new` | `P` | New PR | selections pane | M3 | parity | #278 | |
| `keybindings.pull-request-merge` | `pull_request.merge` | `ctrl+m` | Merge PR | PR Overview | M3 | parity | #183, #262 | TERM: `ctrl+m` sends CR, same as Enter, without the kitty protocol. Must never fire on plain Enter (Py bug B30). Scenario `pr-merge` sends it as CSI 109;5 u. |
| `keybindings.pull-request-edit` | `pull_request.edit` | `E` | Edit PR | PR table, PR Overview | M3 | parity | #185, #283 | Scenario `pr-edit`. |
| `keybindings.pull-request-lookup` | `pull_request.lookup` | `O` | Lookup Pull Request | PR table | M2 | parity | #238 | |
| `keybindings.repositories-favorite` | `repositories.favorite` | `ctrl+f` | Toggle Favorite | repos table | M2 | parity | #222 | |
| `keybindings.repositories-lookup` | `repositories.lookup` | `O` | Lookup Repository | repos table | M2 | parity | #224 | |
| `keybindings.user-view-profile` | `user.view_profile` | `u` | View User | repos, PR and issues tables | M2 | parity | #225, #226 | Rust (#225): the repos table; the PR and issues tables are #226. |
| `keybindings.common-table-select` | `common.table.select` | `enter,space` | Select table entry | tables | M1 | parity | #78, #100, #158, #170 | |
| `keybindings.common-table-search` | `common.table.search` | `/` | Search | tables | M1 | parity | #78, #101, #158, #169 | Override may be saved as `slash`. Rust (#101): `SEARCHABLE_BINDINGS` in `components/table/`, checked after the table's own bindings. |
| `keybindings.common-table-cursor-down` | `common.table.cursor_down` | `j` | Table cursor down | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-page-down` | `common.table.page_down` | `J` | Table page down | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-cursor-up` | `common.table.cursor_up` | `k` | Table cursor up | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-page-up` | `common.table.page_up` | `K` | Table page up | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-scroll-right` | `common.table.scroll_right` | `l` | Table scroll right | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-page-right` | `common.table.page_right` | `$` | Table page right | tables | M1 | parity | #78, #100, #158 | An earlier `L` definition is shadowed (Py bug B9). Override may be saved as `dollar_sign`. |
| `keybindings.common-table-scroll-left` | `common.table.scroll_left` | `h` | Table scroll left | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-page-left` | `common.table.page_left` | `^` | Table page left | tables | M1 | parity | #78, #100, #158 | An earlier `H` definition is shadowed (Py bug B9). Override may be saved as `circumflex_accent`. |
| `keybindings.common-table-scroll-top` | `common.table.scroll_top` | `g` | Table scroll to top | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.common-table-scroll-bottom` | `common.table.scroll_bottom` | `G` | Table scroll to bottom | tables | M1 | parity | #78, #100, #158 | |
| `keybindings.workflows-trigger` | `workflows.trigger` | `T` | Trigger Workflow | workflows section | M3 | parity | #188, #264 | |
| `keybindings.workflows-view-run-details` | `workflows.view_run_details` | `enter` | View Run Details | (unused) | M2 | parity | | Defined, rebindable in Settings, never bound (Py bug B9). Keybindings golden (`parity/goldens/keybindings.toml`). |
| `keybindings.workflows-view-job-logs` | `workflows.view_job_logs` | `l` | View Job Logs | Jobs tab | M2 | parity | #243 | |
| `keybindings.diff-add-comment` | `diff.add_comment` | `c` | Add Comment | hunk pane | M3 | parity | #187, #272 | |
| `keybindings.diff-cursor-down` | `diff.cursor_down` | `j` | cursor_down | (unused) | M2 | parity | #236 | Hunk pane hard-codes `j` (Py bug B9). Rust (#236): bound in the hunk pane, labeled "Line down". |
| `keybindings.diff-cursor-up` | `diff.cursor_up` | `k` | cursor_up | (unused) | M2 | parity | #236 | Hunk pane hard-codes `k` (Py bug B9). Rust (#236): bound in the hunk pane, labeled "Line up". |
| `keybindings.diff-clear-selection` | `diff.clear_selection` | `escape` | Clear Selection | (unused) | M2 | parity | #236 | Never bound (Py bug B9). Keybindings golden (`parity/goldens/keybindings.toml`). |
| `keybindings.diff-next-hunk` | `diff.next_hunk` | `J` | Next Hunk | diff viewer | M2 | intentional diff | #236 | Py bug B8. Rust (#236): see `diff-viewer.hunk-navigation`. Audit (M2, 2026-10-26): unit tests only: no scenario presses `J`/`K` (Python does nothing, B8). Intentional diff: [diff-hunk-navigation](../decisions/diff-hunk-navigation.md) (B8). |
| `keybindings.diff-previous-hunk` | `diff.previous_hunk` | `K` | Previous Hunk | diff viewer | M2 | intentional diff | #236 | Py bug B8. Rust (#236): see `diff-viewer.hunk-navigation`. Audit (M2, 2026-10-26): unit tests only: no scenario presses `J`/`K` (Python does nothing, B8). Intentional diff: [diff-hunk-navigation](../decisions/diff-hunk-navigation.md) (B8). |
| `keybindings.diff-filter-files` | `diff.filter_files` | `/` | Filter Files | diff viewer | M2 | parity | #236 | Rust (#236): focuses the filter from anywhere in the viewer. |
| `keybindings.notifications-open` | `notifications.open` | `ctrl+n` | Notifications | main screen | M4 | parity | #75, #293, #190 | Shown in footer only when enabled at startup (Py bug B7). Rust (#75): hidden and inactive while `notifications.enabled` is false, re-read on config change; the key falls through. Rust (#293): opens the Notifications modal. Rust (#190): Enter there opens the chosen notification; scenario `notifications-open` gates. |
| `keybindings.notifications-mark-all-as-read` | `notifications.mark_all_as_read` | `ctrl+r` | Mark all as read | Unread tab | M4 | parity | #293 | Py bug B82. Rust (#293): scenario `notifications-mark-all`. |
| `keybindings.notifications-mark-read` | `notifications.mark_read` | `R` | Mark as Read | Unread tab | M4 | parity | #293 | Py bug B80, B81. Rust (#293): scenario `notifications` (`marked-one`). |
| `keybindings.notifications-view-read` | `notifications.view_read` | `r` | View Read Notifications | notifications modal | M4 | parity | #293 | Rust (#293): scenario `notifications` (`read-tab`). |
| `keybindings.notifications-view-unread` | `notifications.view_unread` | `u` | View Unread Notifications | notifications modal | M4 | parity | #293 | Rust (#293): scenarios `notifications` (`unread-tab`) and `notifications-mark-all`. |
| `keybindings.modal-submit` | `modal.submit` | `shift+enter` | Submit | most modals | M2 | parity | #213 | TERM: needs the kitty keyboard protocol (crossterm `PushKeyboardEnhancementFlags`); legacy terminals send plain Enter. Buttons are the fallback. Scenario `repo-lookup` (gated) submits an unknown name from the input with `shift+enter`, sent as its kitty encoding `CSI 13;2 u`, and both apps show the "Could not find repo!" toast (2026-10-27). |
| `keybindings.modal-close` | `modal.close` | `q, escape` | Close | all modals | M2 | parity | #212 | Inactive while a text input has focus (Textual inputs consume `q`). |
| `keybindings.textual-input-group` | (Textual `Input`) | | Help panel "Input" group with a text input focused: the inherited scroll keys (`↑` Scroll Up, `↓` Scroll Down), `←`/`→`, `home ^a`, `end ^e`, `pgup`/`pgdn`, `^pgup`/`^pgdn`, then the rest of `Input.BINDINGS` in order, 29 rows | table search box, dialog inputs | M3 | parity | #145, #252 | The six scroll keys are consumed by the input. Scenario `help-panel-input`. |
| `keybindings.modal-search` | `modal.search` | `/` | Search | Settings | M4 | parity | #306 | Not while a text input (it types `/`) or a Key Bindings row (it captures `slash`) has focus. Scenario `settings-search`. Audit (2026-10-30, 94ddedef): scenario `settings-search` passes with Python live at 160x48; signed off. |
| `keybindings.confirm-yes` | `confirm.yes` | `y` | Yes | Confirm | M3 | parity | #214, #279 | |
| `keybindings.confirm-no` | `confirm.no` | `n` | No | Confirm | M3 | parity | #214, #279 | Unit-tested (`modal/confirm/tests.rs`, `everything_else_answers_false`); scenario `pr-new-push-no` answers No. |
| `keybindings.react-add` | `react.add` | `R` | Add reaction | PR Overview | M3 | parity | #183, #263 | Scenario `pr-reactions`. |
| `keybindings.link-paste` | `link.paste` | `ctrl+v` | Paste link | main screen | M4 | parity | #197 | TERM: many terminals turn `ctrl+v` into a paste; a bracketed paste on the main screen opens the modal prefilled (`markdown-links.paste-opens-link-modal`). A focused text input (search box, diff file filter) takes `ctrl+v` first (`textual.input.paste`). Scenario `paste-link`. Rust (#197): `MainScreen::on_binding(LinkPaste)`. Sign-off (#321, 2026-10-30, cdcb98a4): scenario `paste-link` passes with Python live; footer `^v Paste link` in both. |
| `keybindings.link-follow` | `link.follow` | `f` | Follow link | main screen | M4 | parity | #340 | Rust (#340): `MainScreen::on_binding` gives `Action::ShowLinkHints { external: false }`. Sign-off (#321, 2026-10-30, cdcb98a4): scenario `link-hints` passes with Python live; footer `f Follow link` in both. |
| `keybindings.link-follow-external` | `link.follow_external` | `F` | Follow link (browser) | main screen | M4 | parity | #340 | Rust (#340): `Action::ShowLinkHints { external: true }`; the pick is `Action::FollowLink { external: true }`. Sign-off (#321, 2026-10-30, cdcb98a4): scenario `link-hints` (`external`) passes with Python live; footer `F Follow link (browser)` in both. |
| `keybindings.main-repos-focus` | `main.repos.focus` | `1` | Focus repos table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-pull-requests-focus` | `main.pull_requests.focus` | `2` | Focus PRs table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-issues-focus` | `main.issues.focus` | `3` | Focus issues table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-workflows-focus` | `main.workflows.focus` | `4` | Focus workflows table | main view | M1 | parity | #75, #158 | |
| `keybindings.main-details-focus` | `main.details.focus` | `5` | Focus details tabs | main view | M1 | parity | #75, #158 | |
| `keybindings.main-command-log-focus` | `main.command_log.focus` | `6` | Focus command log | main view | M1 | parity | #75, #158 | |
