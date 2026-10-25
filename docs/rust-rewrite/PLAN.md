# LazyGithub Rust Rewrite Plan

Status: in review · Owner: @gizmo385 · Trunk branch: `rust-main`

## 1. Goals and non-goals

**Goals**

1. Replace the Python/Textual implementation with a Rust implementation that a current user cannot tell apart in day-to-day
   use: same layout, same panes and tabs, same default keybindings, same config file, same workflows.
2. Write it as idiomatic Rust on native Rust libraries. We are not porting Textual's widget model, CSS engine or message
   bus; we are rebuilding the *behavior* on Rust-native foundations.
3. Run on macOS and Linux as first-class targets. Windows is a stretch goal that the chosen stack supports for free;
   we keep it building in CI and fix cheap breakages, but it does not block milestones.
4. Use the GitHub CLI (`gh`) for every GitHub interaction, including authentication.

**Non-goals (deliberate scope changes)**

- The raw-HTTPS backend and the OAuth device-flow login (`lib/github/auth.py`, `backends/hishel.py`) are dropped.
  `gh auth login` owns authentication; the first-start screen becomes a "gh missing / not logged in" check.
- The `hishel` HTTP cache is replaced by `gh api --cache <ttl>`, which gives us the same TTL semantics per request.
- Textual-specific features with no user-facing value (the `textual-dev` console, Textual's CSS) are not reproduced.
- New features are out of scope until parity is reached. New feature requests get labeled and parked.

## 2. Branching model

- `main` stays the Python app. Nothing from the rewrite lands on `main` until the cutover PR (Milestone 5).
- `rust-main` is the long-lived trunk for the rewrite, branched from `main` at `13b80e2` (v0.6.6).
- Work branches are `rust/<issue-number>-<short-slug>`, cut from `rust-main`, and merged back via PR into `rust-main`.
  (The trunk is `rust-main`, not `rust`, because git cannot hold both a `rust` branch and `rust/...` branches.)
- The Python code stays in place on `rust-main` as the parity reference and is only deleted in the cutover PR.
- Python bug fixes that land on `main` are merged into `rust-main` periodically (`main` → `rust-main`, never the reverse)
  so the reference implementation stays current.
- Squash-merge into `rust-main`; PR titles become the history. Auto-merge is allowed on PRs into `rust-main`.

## 3. Repository layout on `rust-main`

```
Cargo.toml                 # workspace
crates/
  lazy-github/             # the binary: main.rs, CLI (clap), app wiring
  lazy-github-core/        # models, config, gh client trait + gh CLI impl, cache, git helpers. No UI deps.
  lazy-github-tui/         # ratatui UI: app state, actions, components, theme, keymap
  fake-gh/                 # test double for the `gh` binary, replays recorded fixtures
xtask/                     # `cargo xtask parity`, `cargo xtask record-fixtures`, release helpers
parity/
  fixtures/                # recorded gh responses (JSON), scrubbed
  scenarios/               # keystroke scripts shared by both implementations
  reports/                 # generated, git-ignored
lazy_github/               # Python reference (unchanged until cutover)
docs/rust-rewrite/         # this plan, the parity matrix, process docs, specs
.claude/agents/            # agent role definitions
```

The split into a `core` crate keeps all GitHub/config logic testable without a terminal, and keeps the UI crate free of
process spawning.

## 4. Technology choices

| Concern | Choice | Why |
| --- | --- | --- |
| TUI rendering | `ratatui` | The de-facto Rust TUI library; immediate-mode rendering, `TestBackend` for snapshot tests. |
| Terminal backend | `crossterm` | Works on macOS, Linux and Windows; supports bracketed paste, mouse, key release events. |
| Async runtime | `tokio` | `tokio::process` to drive `gh`, timers for notification polling, channels for the event loop. |
| Text input | `tui-textarea` | Multi-line editors for issue/PR/comment bodies. Single-line inputs too. |
| Markdown | `pulldown-cmark` → our own ratatui renderer | Issue/PR bodies and comments. We own the renderer so links can be hint-targeted. |
| Diff highlighting | `syntect` (or `two-face` for bundled syntaxes) | Split diff viewer syntax colors. |
| Fuzzy matching | `nucleo-matcher` | Command palette, table search, file filter in diff view. |
| Serialization | `serde`, `serde_json` | Models and config. Unknown fields ignored, defaults everywhere. |
| CLI | `clap` (derive) | `run`, `dump-config`, `clear-config`, etc. mirror the Python `click` commands. |
| Errors | `thiserror` in `core`, `color-eyre` in the binary | Typed errors where they're handled, pretty reports where they aren't. |
| Logging | `tracing`, `tracing-subscriber`, `tracing-appender` | Rolling file log at the configured `logfile_path`; the in-app command log is a tracing layer. |
| Paths | `etcetera` | Lets us keep `~/.config/lazy-github` on macOS and Linux (config compatibility) and use the native dir on Windows. |
| Browser / clipboard | `open`, `arboard` | "Open in browser" and copying links. |
| Time | `jiff` (or `chrono`) | Relative timestamps in tables. |
| Tests | `insta` snapshots of `TestBackend` buffers, `rstest`, `assert_cmd` | Unit, snapshot and CLI tests. |
| PTY harness | `portable-pty` + `vt100` | Drives either app in a real pseudo-terminal for parity capture (see §6). |

These are recommendations for the M0 design issues to confirm; swapping one is a design-agent decision recorded in the
issue, not an implementer's ad-hoc call.

## 5. Architecture

**Event loop.** One `tokio` task owns the `App` state. Inputs arrive on an `mpsc` channel as `Event`s: terminal events
from a crossterm `EventStream`, ticks, and completions of background `gh` calls. The loop maps each event to an
`Action`, applies it with `App::update(&mut self, Action) -> Vec<Effect>`, then redraws. `Effect`s (run a `gh`
request, open a browser, save config) are executed by a runtime layer that sends results back as events. State updates
stay synchronous and unit-testable; side effects are data.

**Components.** Each pane (repos, PRs, issues, workflows, details tabs, command log, footer, status bar) and each
modal is a component with `handle_key`, `update` and `render`. Modals live on a screen stack, matching Textual's
`push_screen` behavior. Focus is an explicit enum, not a widget tree walk.

**Keymap.** Every binding is a `BindingId` (e.g. `app.quit`, `pull_request.merge`) with the same string ids and default
keys as `lazy_github/lib/bindings.py`, so existing `bindings.overrides` in config keep working. The footer and help panel
render from the same table.

**GitHub access.** `core` defines a `Github` trait (async methods such as `list_pull_requests`, `merge_pull_request`).
The real impl, `GhCli`, shells out to `gh api` with `-i` for status and headers, `--cache` for TTLs, `--paginate` where
the Python code pages manually, and `--input -` over stdin for request bodies instead of temp files. Responses
deserialize into typed `serde` models. Tests use an in-memory impl; parity runs use the real `GhCli` against `fake-gh`.

**Config.** `Config` mirrors `lazy_github/lib/config.py` field for field, reads and writes the same
`~/.config/lazy-github/config.json`, ignores fields it doesn't know (e.g. `api.client_type`), and preserves them on save
where practical. Theme names map Textual's built-in theme names to ratatui palettes so a saved `"theme": "nord"` still
works.

**Local caches and drafts.** The per-repo table caches (`lib/cache.py`) and PR drafts (`lib/pr_drafts.py`) keep the same
file names and JSON shape so switching implementations doesn't lose state.

## 6. UI parity strategy

Parity is checked at three levels, from cheapest to most faithful.

**6.1 The parity matrix (`docs/rust-rewrite/parity/`, one file per area).** One row per user-visible thing: every screen, pane, tab,
modal, table column, binding id, config field and CLI command in the Python app. Each row links to its issue and has a
status (`not started`, `in progress`, `parity`, `intentional diff`). A milestone is done when all its rows are `parity`
or an approved `intentional diff`. The design agent seeds it in M0 from the Python source.

**6.2 Shared fixtures through a fake `gh`.** Both implementations can talk to GitHub through `gh` (the Python app via
`--auth-backend GITHUB_CLI`). `crates/fake-gh` is a small binary named `gh` that answers `gh api ...` and
`gh auth status` from `parity/fixtures/`, keyed by method, path and query. Putting it first on `PATH` gives both apps
byte-identical data with no network. `cargo xtask record-fixtures` captures fresh fixtures from a real account and
scrubs tokens and private data.

**6.3 Side-by-side screen capture.** `cargo xtask parity <scenario>` launches each implementation in a pseudo-terminal of
fixed size (default 160×48, plus 100×30 for narrow layouts), with an isolated `HOME`, the fake `gh` and a fixed clock. It
replays a scenario from `parity/scenarios/*.toml` (a list of keystrokes and "capture" points), and at each capture dumps
the `vt100` screen grid (characters plus colors) for both apps. It then writes:

- a text diff of the two grids,
- a structural check that asserts the things we treat as must-match: pane titles and their order, tab names, table
  column headers, visible row text, footer bindings, focused pane,
- an HTML report with both screens rendered side by side for human or agent review.

Exact cell-for-cell equality is not the target, since Textual and ratatui draw borders and scrollbars differently. The
structural check is the gate in CI; the visual report is what the parity agent and Chris review.

**6.4 Rust-only snapshots.** Components get `insta` snapshots of `TestBackend` buffers so regressions show up in normal
`cargo test`, independent of the Python app.

## 7. Cross-platform

- CI matrix: `ubuntu-latest` and `macos-latest`, both required. Windows isn't built or tested until later in the
  project; `windows-latest` joins the matrix in M5.
- No shelling out except to `gh` and `git`; no `sh -c`. Paths via `std::path` and `etcetera`.
- `fake-gh` is Rust, not a shell script, so the parity harness runs on all three OSes.
- Windows-specific gaps get a `platform: windows` label instead of blocking a PR.

## 8. Milestones

Each milestone is a `milestone: M<n>` label plus a tracking issue, on issues labeled `rust`. Rough order; issues inside a milestone can run in
parallel once their dependencies land.

**M0 Foundations.** Cargo workspace and crate skeletons. CI workflow for `rust-main` (fmt, clippy `-D warnings`, test on
Linux and macOS). `core` config load/save with compatibility tests against the Python test fixtures. `Github` trait and
`GhCli` impl with auth check. tracing setup. `fake-gh` and fixture format. Parity harness skeleton running one trivial
scenario against both apps. Seed the parity matrix.

**M1 App shell.** Main layout: status summary bar (current repo, unread notification count), left selections pane,
details pane with tabs, optional command log, footer. Focus switching with `1`–`6`. Keymap with overrides, help panel
(`?`), maximize (`M`), quit. Theme system with Textual theme names. Searchable table component (`/`, `j/k/J/K/g/G/h/l`).
Toasts/notifications.

**M2 Read-only browsing.** Repositories table (favorites, tracked repos, lookup, current-directory default). Repo
overview with README. Pull requests list with state/owner filters, PR overview (checks, reviews), diff tab with split
diff viewer, conversation tab. Issues list, overview and conversation. Workflows tabs, run overview, jobs and logs.
Info pane. User profile modal. Local table caches.

**M3 Write actions.** New and edit issue. New comment and replies to comments and reviews. Reactions. Create and edit
PR (branch selection, remote branch check, reviewers, templates, drafts). Merge PR with preferred method. Diff review
comments and submit review. Trigger workflow, rerun and rerun failed jobs. Confirm dialog.

**M4 Remaining features.** Notifications modal (read/unread tabs, mark read, mark all read, polling). Command palette.
Link hints (`f`/`F`) and paste-link modal. Settings modal including key rebinding. Debug modal and bug report flow.
Preload PR for current commit. CLI subcommands (`dump-config`, `clear-config`, `clear-auth` becomes a no-op or a
pointer to `gh auth logout`).

**M5 Hardening and cutover.** Every parity matrix row signed off. Windows added to CI and its status decided. Packaging: release workflow
building binaries for `gh extension install` (precompiled gh extension, e.g. `cli/gh-extension-precompile`),
`cargo install`, and an updated Nix flake. README rewrite. Cutover PR `rust-main` → `main` that removes the Python
code, the PyPI publish workflow and `uv.lock`, with a final `python` release note.

## 9. Process

GitHub issues in this repo are the work queue. The labels, the issue lifecycle and the hand-offs between agents are
described in [PROCESS.md](./PROCESS.md); the agent roles are in `.claude/agents/`.

Summary of the lifecycle: `status: needs-triage` → triage agent → `status: needs-design` → design agent writes the
spec → `status: ready` → implementer opens a PR into `rust-main` → `status: in-review` → code reviewer and parity
reviewer → merged → parity matrix updated.

## 10. Decisions

Recorded 2026-10-25 by Chris:

1. The gh CLI is the only backend. The raw-HTTPS backend and device-flow auth are dropped.
2. The parity strategy in §6 is the starting point; it can be adjusted as we learn.
3. Branch naming is `rust-main` for the trunk and `rust/<issue>-<slug>` for work branches, as in §2.
4. The parity check runs on every PR into `rust-main`.
5. Windows is allowed to fail until later in the project. We don't build or test it in CI at the start; it gets added
   to the CI matrix in M5.
6. M0 and M1 issues are filed. Milestones are tracked with `milestone: M<n>` labels plus one tracking issue per
   milestone.
7. Auto-merge may be enabled on PRs into `rust-main` (squash). It's never used for PRs into `main`.
