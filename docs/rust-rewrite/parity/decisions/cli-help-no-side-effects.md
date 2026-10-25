# `--help` and `--version` have no side effects

- Decision: `-h`/`--help` and `--version` print and exit without creating the config directory, the log file or the
  cache.
- Decided by: Chris, 2026-10-25
- Link: [project thread](https://claude.ai/code/project/chan_012pc8Wphbdkgs8rn26pJBv9?thread=cmsg_012pc8Wphbdkgs8rn26pJBv91YRDXqF1kGmJaE3iA28mKh), implemented in #72
- Rows: `cli.help-version-no-side-effects`

Python's Click `cli` group runs its setup (config directory and logfile) before Click prints help, so asking for help
writes to `~/.config/lazy-github`. The Rust CLI parses arguments first.
