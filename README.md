[![Crates.io Version](https://img.shields.io/crates/v/lazy-github)](https://crates.io/crates/lazy-github)
[![GitHub Release](https://img.shields.io/github/v/release/gizmo385/gh-lazy)](https://github.com/gizmo385/gh-lazy/releases/latest)

LazyGithub is a terminal UI client for interacting with [GitHub](https://github.com). It draws heavy inspiration from the
[lazygit](https://github.com/jesseduffield/lazygit) project. It's written in Rust and does everything through the
[GitHub CLI](https://cli.github.com), so it uses the login, hosts and settings you already have there.

![Example screenshot](https://raw.githubusercontent.com/gizmo385/gh-lazy/main/images/lazy-github-conversation-ui.svg)

## Requirements

- The [GitHub CLI](https://cli.github.com) (`gh`), installed and logged in with `gh auth login`.
- `git`, for opening on the repository in the current directory and for creating pull requests from your branch.

If `gh` is missing or logged out, LazyGithub says so when it starts and tells you what to run.

## Installing

### As a GitHub CLI extension (recommended)

Install it with `gh extension install gizmo385/gh-lazy`, then run `gh lazy`. Upgrade with `gh extension upgrade lazy`.

If you installed the extension back when LazyGithub was written in Python, `gh extension upgrade lazy` switches you to
the new binary. If it doesn't, reinstall with `gh extension remove lazy && gh extension install gizmo385/gh-lazy`.

### Release binaries

Download the `gh-lazy-<os>-<arch>` binary for your platform from the
[latest release](https://github.com/gizmo385/gh-lazy/releases/latest) (`darwin-arm64`, `darwin-amd64`, `linux-amd64`
or `linux-arm64`; the Linux builds are static and run on any distribution). Make it executable with `chmod +x`, and put
it on your `PATH`, renamed to `lazy-github` if you like.

To verify a download, check it against the release's `SHA256SUMS` (`sha256sum --ignore-missing -c SHA256SUMS`, or
`shasum -a 256 --ignore-missing -c SHA256SUMS` on macOS) or its build attestation
(`gh attestation verify <file> -R gizmo385/gh-lazy`). On macOS, a binary downloaded with a browser is quarantined;
clear that with `xattr -d com.apple.quarantine <file>`.

### Cargo

`cargo install --locked lazy-github` builds and installs the `lazy-github` binary. It needs Rust 1.97 or newer.

### Nix

Run it with `nix run github:gizmo385/gh-lazy`, install it with `nix profile install github:gizmo385/gh-lazy`, or add the
flake as an input and use its `packages.<system>.default`.

### PyPI

The Python version of LazyGithub (0.6.x and earlier) is still on PyPI as `lazy-github` but is no longer developed.

## Features

- Repositories: your own and the ones you track, favorites, lookup by name, and the repository in the current directory
  selected on start.
- Pull requests: list filters, an overview with status checks and reviews, a split diff where you can leave review
  comments, the conversation, and creating, editing and merging pull requests.
- Issues: the list, details, creating, editing and commenting.
- Workflows: runs, their jobs and logs, triggering a workflow and re-running a run.
- Notifications, once turned on in Settings.
- The command palette (`ctrl+p`), link hints to follow any link on screen (`f`), and the keybindings for the focused
  section (`?`).
- Settings for everything in the config file, including rebinding keys.

## Reporting Bugs

If you run into any bugs while using LazyGithub, please let me know! You can do this by creating a
new [bug report](https://github.com/gizmo385/gh-lazy/issues/new?template=bug_report.md) or search
for `Open Bug Report` in the command palette from inside LazyGithub.

[![In-App Bug Reporter](https://asciinema.org/a/JiTZqOWyEjPzG4v1marSyTqUV.svg)](https://asciinema.org/a/JiTZqOWyEjPzG4v1marSyTqUV)

Please include the output of `gh lazy debug` (or `lazy-github debug`), or the contents of "Debug Info" from the command
palette.

## Customization

LazyGithub supports a number of customization options, all of which are stored in `~/.config/lazy-github/config.json`
on macOS and Linux. They can be edited by hand or in the Settings screen, which you open by searching for settings in
the command palette (`ctrl+p`).

`lazy-github dump-config` prints the configuration LazyGithub would load, and `lazy-github check-config` lists values
that stop it loading; `lazy-github check-config --fix` offers to reset each one to its default after saving a backup.
Run `lazy-github help` for the other commands.

![Settings screenshot](https://raw.githubusercontent.com/gizmo385/gh-lazy/main/images/lazy-github-settings-ui.svg)

## Migrating from the Python Version

- Your config file, the per-repository caches and pull request drafts carry over as they are, and both versions read
  and write the same files, so you can switch back and forth.
- Log in with `gh auth login`. LazyGithub uses the GitHub CLI's login only, so there's no device login, the
  `api.client_type` setting is ignored (it stays in your config file), and `run --auth-backend` is gone.
- `lazy-github clear-auth` now points you to `gh auth logout`, and deletes the token the Python version saved in
  `~/.config/lazy-github/auth.text` if it's still there.

## Development

From a clone of the repository:

- `cargo run` builds and starts the app; `cargo run -- <subcommand>` runs a subcommand, such as `cargo run -- debug`.
- `cargo test --workspace` runs the tests.
- `nix develop` opens a shell with the pinned Rust toolchain and `gh`.

[`docs/rust-rewrite/`](docs/rust-rewrite/) covers the architecture, the parity harness that compares the app with the
Python version, and how work on the rewrite is organized.
