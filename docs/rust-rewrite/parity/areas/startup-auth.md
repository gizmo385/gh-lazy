# Startup and authentication

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `startup-auth.gh-auth-check` | First start / gh auth check | `ui/app.py` `FirstStartScreen`, `ui/screens/auth.py` | M0 | intentional diff (gh only) | #65 | PLAN §10 decision 1. |
| `startup-auth.backend-picker` | First-start backend picker ("Welcome to LazyGithub! How would you like to authenticate with the Github API?", radio "Via the Github CLI" / "Directly via HTTPS", Continue / Cancel) | `ui/app.py` `FirstStartScreen` | M0 | intentional diff (gh only) | #65 | Not shown in Rust. `core.first_start` is still read and written as `false`. |
| `startup-auth.auth-cache` | Skip the auth check while `cache.auth_last_checked` is within `cache.auth_cache_duration`; otherwise check and write `auth_last_checked` | `ui/app.py` `LazyGithub.authenticate_with_github` | M0 | not started | #65 | |
| `startup-auth.not-logged-in-screen` | Not-logged-in screen: border "GitHub Authentication", "To proceed, please log into the Github CLI via gh auth login", intro text below, polls `gh auth status` every 5 s and switches to the main screen once logged in; `q` quits | `ui/screens/auth.py` `AuthenticationModal`, `GithubCliAuthInstructions` | M1 | not started | | Device-flow branch (`UserTokenDisplay`) is dropped. |
| `startup-auth.theme-keymap-on-mount` | Theme and keymap applied on mount from config | `ui/app.py` `LazyGithub.on_mount` | M1 | not started | #73, #77 | |
