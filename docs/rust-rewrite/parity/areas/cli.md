# CLI

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

`lazy-github` (and `gh lazy`) with no subcommand runs `run`. `-h`/`--help` on every command.

| Id | Command | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `cli.run` | `run [--auth-backend]` | `cli.py` | M1 | not started | #199 | Bare launch in M1; `--auth-backend` (env `AUTH_BACKEND`) writes `api.client_type` to config, M4. Audit (M2, 2026-10-26): the bare launch is in every scenario; `--auth-backend` waits for #199. |
| `cli.dump-config` | `dump-config` | `cli.py` | M4 | not started | #199 | Prints "Config file location: <path> (exists => True/False)" then the config as pretty JSON. |
| `cli.clear-auth` | `clear-auth` | `cli.py` | M4 | intentional diff (defer to `gh auth logout`) | #199 | Decision: [clear-auth-defers-to-gh](../decisions/clear-auth-defers-to-gh.md). |
| `cli.clear-config` | `clear-config [--no-confirm]` | `cli.py` | M4 | not started | #199 | Prompt "Confirm deletion your LazyGithub settings"; prints "Your settings have been cleared" or "Canceling". |
| `cli.clear-cache` | `clear-cache [--no-confirm]` | `cli.py` | M4 | not started | #199 | Prompt "Confirm deletion of everything in <dir>"; prints "Cache cleared" or "Canceling cache deletion". |
| `cli.debug` | `debug` | `cli.py`, `lib/debug.py` | M4 | not started | #195 | Prints the debug markdown. |
| `cli.version` | `version` | `cli.py` | M4 | not started | #199 | Prints "LazyGithub version <x>". |
| `cli.help-version-no-side-effects` | `-h`/`--help` and `--version` have no side effects: no config dir, log file or cache is created | `cli.py` (Click runs `cli` setup first) | M1 | intentional diff | #72 | Python's `cli` group creates the config dir and logfile before printing help. Decision: [cli-help-no-side-effects](../decisions/cli-help-no-side-effects.md). |
| `cli.broken-config-error` | A config file that fails to load prints a one-line error naming the file and suggesting `lazy-github clear-config`, then exits 1 | `lib/config.py` (pydantic `ValidationError` traceback) | M1 | intentional diff | #72 | Python crashes with a traceback. Decision: [broken-config-error](../decisions/broken-config-error.md). |
| `cli.original-pwd` | `LAZY_GITHUB_ORIGINAL_PWD` set to the cwd if unset (the `gh-lazy` extension script sets it) | `cli.py` `cli`, `gh-lazy` | M2 | parity | #105, #115 | Rust (#105, #115): `main.rs` resolves it once (unset or empty: the current directory) and hands it to the git runner. It isn't exported to child processes: `set_var` is `unsafe` and the workspace forbids `unsafe`, and nothing else reads it. |
| `cli.gh-extension-install` | `gh extension install gizmo385/gh-lazy` installs a precompiled extension (release assets `gh-lazy-<os>-<arch>[.exe]`) and `gh lazy` runs it | `gh-lazy`, `start.sh` (script extension that needs `uv`) | M5 | not started | #203 | The binary reads its own cwd, so `cli.original-pwd` works without the script. Owner decision: what existing script-extension installs see after `gh extension upgrade` (a shim `gh-lazy` that prints the reinstall command, or a documented remove and reinstall). |
| `cli.cargo-install` | `cargo install lazy-github` | n/a (Python ships on PyPI) | M5 | not started | #204 | Owner decision: publish `lazy-github`, `lazy-github-core` and `lazy-github-tui` to crates.io in dependency order on each release tag (recommended), or `cargo install --git` only. Crate metadata, `exclude` for fixtures and snapshots, `cargo publish --dry-run` in CI; `fake-gh` and `xtask` stay `publish = false`. |
| `cli.nix-flake` | `flake.nix` builds the Rust app (`nix run github:gizmo385/gh-lazy`) | `flake.nix` (`buildPythonApplication`, version from `lazy_github/version.py`) | M5 | not started | #205 | `rustPlatform.buildRustPackage` (or crane) with `cargoLock.lockFile = ./Cargo.lock`, the version from the workspace `Cargo.toml`, `gh` and `git` on the wrapped `PATH`, and a dev shell with the `rust-toolchain.toml` toolchain. Keeps `packages.default`, `apps.default` and the dev shell. Prepared on `rust-main`, switched at cutover (#207). |
