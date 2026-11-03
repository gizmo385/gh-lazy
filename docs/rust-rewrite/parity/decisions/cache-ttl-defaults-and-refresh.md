# Shorter cache TTLs, a refresh key, and an app-owned gh cache

- Decision: The cache defaults are `default_ttl` 60 s, `list_repos_ttl` 3600 s and `list_issues_ttl` 60 s (Python:
  600, 86400 and 3600). A `config.json` holding exactly Python's old trio for all three fields (as numbers or numeric
  strings) loads as the new trio, on every load, logs one info line, and the next save writes the new values; any
  other combination loads as written. "Refresh from GitHub" (palette command, and `ctrl+r` on the main screen as the
  rebindable Rust-only id `main.refresh`, hidden from the footer and the help panel) refetches everything the main
  screen shows, in place, past `gh`'s cache, and every cached read made after it while it's younger than the read's
  TTL skips the cache once. Every `gh` the app runs gets `XDG_CACHE_HOME=<cache.cache_directory>/http`, so
  `clear-cache` clears the app's HTTP cache.
- Decided by: Chris, 2026-11-02 (#404, options D1 a, D2 a and D3 a)
- Link: https://github.com/gizmo385/gh-lazy/issues/404#issuecomment-6596998231 (#404; core #406, TUI #407)
- Rows: `config.cache-default-ttl`, `config.cache-list-repos-ttl`, `config.cache-list-issues-ttl`, `config.cache-legacy-ttls`, `main-layout.refresh`, `keybindings.main-refresh`, `command-palette.refresh-from-github`

Python's default raw-HTTP backend (hishel) kept its cache in `cache_directory`, took freshness from GitHub's
`Cache-Control: private, max-age=60` and revalidated with ETags; its gh backend never cached at all, because `gh`
ignores a `Cache-Control` request header. So Python users saw changes made outside the app within about a minute, or
at once. Rust passes the TTLs to `gh api --cache`, which honours them, so Python's numbers made a new comment, PR or
CI status take up to 10 minutes to show (an hour for the repos list). 60 s matches GitHub's own `max-age`; an hour
keeps `/user/repos`, up to 29 pages, cheap. `list_issues_ttl` is still unread by both apps; 60 keeps the file
consistent. The worst case, every read uncached, is what Python's gh backend did on every read, and #324's
rate-limit handling stays as it is.

Python's `Config.save` writes every field, so most existing files hold the old defaults explicitly, and changing the
defaults alone would do nothing for them. Migrating only the exact trio fixes every untouched file while keeping any
file where even one TTL was tuned. The cost, accepted: typing exactly 600 / 86400 / 3600 in Settings reverts to the
new defaults at the next launch. Per-field migration would make 600 unchoosable for `default_ttl`; a marker key
would add a config field that Python drops on save; leaving files alone would keep most users at 600 s. `check-config`
doesn't flag the trio.

`gh api --cache` can't revalidate with ETags, so a refresh marks every cached read stale through #383's cache marks
(`CacheScope::All`) and the next read of each sends `--cache 1ns`. `ctrl+r` is free on the main screen and a plain
control byte in every terminal; inside the notifications modal it stays "Mark all as read". It's hidden, as the
`1`-`6` focus keys are, so every footer and help panel shared with Python stays the same; the palette and Settings'
Key Bindings make it discoverable. `f5` often needs `fn` on laptops.

go-gh reads `XDG_CACHE_HOME` before anything else for its cache directory on every OS (gh 2.101.0), so setting it per
spawned `gh` moves the app's `--cache` entries into `<cache_directory>/http/gh/`, which `clear-cache` already
deletes, as Python's did with hishel's cache. `~/.cache/gh`, shared with other `gh` tools, isn't touched; entries the
app left there before are never served to it again and `gh config clear-cache` removes them. Running
`gh config clear-cache` from `clear-cache` would wipe every tool's cache, and go-gh's `X-GH-CACHE-DIR` request header
would also be sent to GitHub. `infrastructure.gh-cache-location` aims at `parity`, since it reproduces where
Python's default backend kept its cache.
