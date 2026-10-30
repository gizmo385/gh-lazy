# An unwritable config folder is reported, not fatal

- Decision: When `config.json` can't be written (read-only or permissionless folder, full disk), the app shows an
  error toast "Failed to save settings: failed to write <path>: <I/O error>", logs it, and keeps the change in memory
  for the session; the existing file is never truncated. When the log file can't be opened, the app runs without
  one and, once the main screen is up, logs one warning to the command log: "Could not open log file <path>:
  <I/O error>. Continuing without a log file." A failed PR draft write logs Python's warning, which the command log
  shows. A failed table-cache write logs a warning to the log file only, since with an unwritable cache folder there
  would be one per table load and the user can't act on it (Python crashes on it: `save_models_to_cache` doesn't
  catch the `OSError`).
- Decided by: Chris, 2026-10-30. He approved both texts and asked to keep cache-write failures out of the command log.
- Link: #328, parent #201
- Rows: `toasts.settings-save-failed`, `infrastructure.log-file-unavailable`

Python's `Config.save` doesn't catch the `OSError`, so a Settings Save, a favorite toggle or a tracked lookup with
an unwritable config folder crashes the app with a traceback, and its `write_text` truncates `config.json` before
writing, so a full disk can leave it half written. Python's failed log-file setup (`LazyGithubContext._setup_logging_handler`)
goes to `logging`'s last-resort handler on stderr, and the command log never mentions it.

Rust writes every config, cache and draft file through a temp file and a rename, so a failed write leaves the
previous file byte for byte. The stderr line for a log file that can't be opened stays as well.
