# `--help` and `--version` have no side effects

- Decision: `-h`/`--help` and `--version` print and exit without creating the config directory, the log file or the
  cache. Neither do the subcommands other than `run` (`version`, `dump-config`, `debug`, `clear-auth`,
  `clear-config`, `clear-cache`, `check-config`): they start no logging, and create the config folder only to write
  a file they're asked to write (`check-config --fix`).
- Decided by: Chris, 2026-10-25; extended to the subcommands on 2026-10-28
- Link: [project thread](https://claude.ai/code/project/chan_012pc8Wphbdkgs8rn26pJBv9?thread=cmsg_012pc8Wphbdkgs8rn26pJBv91YRDXqF1kGmJaE3iA28mKh), implemented in #72; subcommands: [#308](https://github.com/gizmo385/gh-lazy/issues/308#issuecomment-6485042816)
- Rows: `cli.help-version-no-side-effects`, `cli.subcommand-side-effects`

Python's Click `cli` group runs its setup (config directory and logfile) before Click prints help, so asking for help
writes to `~/.config/lazy-github`. The Rust CLI parses arguments first.

Every Python subcommand loads the config through `LazyGithubContext`, which sets up the file logger, so each one
creates `~/.config/lazy-github/lazy_github.log`. The Rust subcommands log nothing, so they create nothing.
