# Repositories

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `repositories.table` | Repositories table | `ui/widgets/repositories.py` `ReposContainer` | M2 | not started | | |
| `repositories.columns` | Columns: "★" (favorite: green "★" or "☆"), "Owner", "Name", "Private" ("✓" / "✘") | `ui/widgets/repositories.py` `repo_to_row`, `on_mount`; `lib/constants.py` | M2 | not started | | |
| `repositories.rows` | Rows: all repos for the user (`/user/repos?type=all&sort=full_name`, 50 per page, up to 29 pages) plus `repositories.additional_repos_to_track`, favorites first | `ui/widgets/repositories.py` `load_repos`; `lib/github/repositories.py` `list_all` | M2 | not started | | Tracked repos are appended after the sorted list. Uses `cache.list_repos_ttl`. |
| `repositories.loading` | Loading: spinner; cached repos shown immediately from `repos.json`; on API error the cached rows stay and nothing is shown | `ui/widgets/repositories.py` `load_repos` | M2 | not started | | Error is only logged. |
| `repositories.auto-select-cwd-repo` | Auto-select the current directory's repo (if present in the table and nothing selected yet), after cache load and after fetch | `ui/widgets/repositories.py` `check_current_directory_repo` | M2 | not started | | Py bug B5. |
| `repositories.select` | Enter/space selects a repo: status bar updates, PR/issue/workflow tables reload, details show "Repo Overview" | `ui/widgets/repositories.py` `repo_selected`; `ui/screens/primary.py` `handle_repo_selection`, `SelectionsPane.load_repository` | M2 | not started | | Details are not focused. |
| `repositories.toggle-favorite` | `ctrl+f` toggles favorite: writes `repositories.favorites`, updates the star, re-sorts | `ui/widgets/repositories.py` `action_toggle_favorite_repo` | M2 | not started | | Py bug B13 (empty table). |
| `repositories.lookup` | `O` lookup repository modal; result added to the table and selected | `ui/widgets/repositories.py` `action_lookup_repository` | M2 | not started | | |
| `repositories.user-profile` | `u` opens the owner's profile; choosing a repo there adds it to the table and selects it; toast error "No repository currently selected" on empty table | `ui/widgets/repositories.py` `action_view_user_profile` | M2 | not started | | |
| `repositories.overview-tab` | Repo overview tab "Repo Overview": "owner/repo • Public/Private • Archived", "Owned by <owner>", "Default branch: <branch>", description, rule, README markdown or "*No README found.*" | `ui/widgets/repository_details.py` `RepoOverviewTabPane` | M2 | not started | | Repo and owner are hint/clickable links (open in app or browser). README via `Accept: application/vnd.github.raw`. |
| `repositories.introduction-tab` | Introduction tab before any selection ("Introduction", `LAZY_GITHUB_INFO` markdown with version) | `ui/widgets/info.py` `LazyGithubInfoTabPane`; `ui/screens/primary.py` `SelectionDetailsContainer.on_mount` | M2 | not started | | Same text is shown on the auth screen. |
