# Command palette

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

`ctrl+p` opens Textual's command palette. Fuzzy search over command names; each hit shows its help text.

| Id | Command | Help text | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `command-palette.palette` | Command palette and its commands | | `ui/screens/primary.py` `MainScreenCommandProvider` | M4 | not started | #191 | |
| `command-palette.toggle-command-log` | "Toggle Command Log" | "Toggle showing or hiding the command log" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | #191 | Session only, not written to config. |
| `command-palette.toggle-workflows` | "Toggle Workflows" | "Toggle showing or hiding repo actions" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | #191 | Showing reloads cached and live runs for the current repo. |
| `command-palette.toggle-issues` | "Toggle Issues" | "Toggle showing or hiding repo issues" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | #191 | Showing refetches issues and PRs. |
| `command-palette.toggle-pull-requests` | "Toggle Pull Requests" | "Toggle showing or hiding repo pull requests" | `ui/screens/primary.py` `action_toggle_ui` | M4 | not started | #191 | Showing refetches issues and PRs. |
| `command-palette.change-settings` | "Change Settings" | "Adjust LazyGithub settings" | `ui/screens/primary.py` `action_show_settings_modal` | M4 | not started | #193 | |
| `command-palette.debug-info` | "Debug Info" | "View Debug Information" | `ui/screens/primary.py` `action_show_debug_info` | M4 | not started | #195 | |
| `command-palette.open-bug-report` | "Open Bug Report" | "Open Bug Report" | `ui/screens/primary.py` `action_open_bug_report`; `lib/debug.py` `get_bug_report_template` | M4 | not started | #195 | Selects `gizmo385/gh-lazy` and opens New issue prefilled with the template; toast error / "Error opening bug report" "Could not load LazyGithub repo! Please open the bug report on Github". |
| `command-palette.refresh-notifications` | "Refresh notifications" (only when `notifications.enabled`) | "Refresh the unread notifications count" | `ui/screens/primary.py` `action_refresh_notifications` | M4 | not started | #191 | |
| `command-palette.theme` | Textual "Theme" | "Change the current theme" | Textual `App.get_system_commands` | M4 | not started | #192 | Live theme picker; not saved to config (Py bug B27). |
| `command-palette.quit` | Textual "Quit" | "Quit the application as soon as possible" | Textual | M4 | not started | #191 | |
| `command-palette.keys` | Textual "Keys" | "Show help for the focused widget and a summary of available keys" / "Hide the keys and widget help panel" | Textual | M4 | not started | #191 | |
| `command-palette.maximize-minimize` | Textual "Maximize" / "Minimize" | "Maximize the focused widget" / "Minimize the widget and restore to normal size" | Textual | M4 | not started | #191 | |
| `command-palette.screenshot` | Textual "Screenshot" | "Save an SVG 'screenshot' of the current screen" | Textual | M4 | not started | #191 | Candidate `intentional diff` (no SVG export in ratatui). |
