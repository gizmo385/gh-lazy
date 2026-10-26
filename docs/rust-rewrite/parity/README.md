# Parity Matrix

Every user-visible behavior of the Python app, and the state of its Rust equivalent. The design agent owns adding rows;
the parity reviewer owns moving a row to `parity`.

Generated from `lazy_github/` at v0.6.6 (completed in #71). Paths in "Python reference" are relative to `lazy_github/`.

The matrix is split into one file per area so that parallel PRs touch different files, or different lines of the same
file, and merge without conflicts. This README holds only the rules and the index; it has no rows and no counts.

## Layout

```
docs/rust-rewrite/parity/
├── README.md            # this file: rules and the area index
├── areas/<area>.md      # one table per area, one row per line
├── py-bugs/B<nn>.md     # one file per suspected Python bug
└── decisions/<slug>.md  # one file per intentional diff
```

## Status

A row's **Status** is one of:

- `not started`
- `in progress`
- `parity`: the parity reviewer signed it off.
- `intentional diff`: the Rust app differs on purpose. The row's Notes link the decision file
  (`Decision: [<slug>](../decisions/<slug>.md)`). A short qualifier in parentheses is allowed, e.g.
  `intentional diff (gh only)`.

What has to match is functionality and general UI structure, not Textual's rendering ([PLAN §10](../PLAN.md#10-decisions),
decision 8). A minor cosmetic difference that comes only from Textual (the pointer shape, exact glyphs, hover tints)
doesn't keep a row from `parity` and needs no decision file. Say so in the row's Notes instead.

## Columns

Every area table starts with **Id**. The other columns depend on the area (keybindings have Binding id, Default key,
Description and Context; config has Section, Field and Default; and so on), and most end with these:

- **Id**: `<area>.<name>`, kebab-case, unique across the matrix, e.g. `main-layout.status-current-repo`. The area
  prefix is the area file's name. Ids are stable: never rename or reuse one, because scenarios' `parity_rows`,
  py-bug files and decision files refer to them. If a row is split, keep the old id on one part and give the
  others new ids. Since the prefix must match the file and the id never changes, a row never moves to another
  area file. If a row really belongs elsewhere, add a new row there and change the old row's Notes to
  "Moved to `<new id>`." (and repoint any `parity_rows`, py-bug or decision references to the new id).
- **Milestone**: where the row lands, per [PLAN.md §8](../PLAN.md#8-milestones). M0 is only used for infrastructure
  rows.
- **Status**: see above.
- **Issue**: the issue(s) that implement the row, as `#n` or `#n, #m`, where one is filed. Rows with no issue are the
  backlog the milestone issues get filed from.
- **Notes**: details a spec needs, `TERM:` for terminal caveats (see
  [Terminal caveats](./areas/terminal-caveats.md)), and `Py bug Bn` where the Python behavior looks like a bug (see
  [`py-bugs/`](./py-bugs/)). Rows with a `Py bug` note must not be copied silently: the spec for that row follows
  Chris's answer, recorded in the bug's file.

Wording in quotes is the exact text the Python app shows. Toast rows give severity and title as
`severity / "title"`; no title means Textual's default (none).

## Editing the matrix

- A PR edits only the area file(s) that hold its rows. Keep each row on one line. Two PRs that change different,
  non-adjacent rows of the same file merge cleanly. Edits to adjacent rows, or two rows inserted at the same spot,
  still conflict, because git merges neighbouring lines as one hunk; that conflict is a one-line resolve (keep
  both rows).
- The design agent adds rows, with a new id, the milestone and the issue link. Add a row next to the rows it relates
  to rather than at the end of the table: appends from parallel PRs all land on the same spot and always
  conflict, while inserts next to related rows usually land in different places.
- The parity reviewer moves the rows a PR completes to `parity`, and back from `parity` when a gap is found. That is
  usually, but not always, every id in the PR's scenarios' `parity_rows`: an `intentional diff` row stays one, and
  a row the scenario only covers in part (e.g. `main-layout.command-log` before #80) stays where it is.
- The same goes for a row named in a golden manifest's `parity_rows` (`parity/goldens/`, see
  [`parity/README.md`](../../../parity/README.md#python-goldens)). The reviewer moves it to `parity` when the
  manifest's `test` passes in `cargo test` and its `fresh_test` passed in the PR's CI run. The exceptions are the
  same as for scenarios: an `intentional diff` row stays as it is, and so does a row whose Notes say the golden
  covers it only in part, or one with an open `Py bug` whose recommendation the Rust side doesn't follow yet.
- A new suspected Python bug is a new file `py-bugs/B<nn>.md` with the next free number (look at `rust-main` and open
  PRs). If another PR takes the number first, renumber yours. The file starts with a `# B<n>: <title>` heading and
  these lines, then the behavior as prose:

  ```
  - Id: B<n>
  - Status: open | answered | withdrawn
  - Answer: <Chris's answer, once given>
  - Python reference: <files and functions>
  - Recommendation: <what Rust should do>
  - Rows: `<row id>`, `<row id>`
  ```

  The affected rows say `Py bug B<n>` in their Notes.
- A new intentional diff is a new file `decisions/<slug>.md`: a `# <title>` heading, then these lines and the
  rationale as prose. The rows' Status becomes `intentional diff` and their Notes link the file.

  ```
  - Decision: <what the Rust app does instead>
  - Decided by: <who, and when>
  - Link: <issue, comment or thread where it was decided>
  - Rows: `<row id>`, `<row id>`
  ```

- Only edit this README's index when you add or remove an area file.
- Don't keep counts by hand. `cargo xtask parity-matrix` checks the matrix and prints the rows per area and status,
  then per area and coverage: Scenario (named by a scenario and no golden), Golden (by a golden and no scenario),
  Both, and Neither. `--markdown` prints both as tables to paste into an issue or PR. `--uncovered` prints the
  Neither rows, except intentional diffs, as `<id>  <status>` lines in index order: the backlog of rows nothing
  checks yet.

`cargo test --workspace` runs the same check, so CI fails when an id is duplicated or malformed, a status isn't one
of the values above, a `Py bug Bn` note has no file, an `intentional diff` row doesn't link a decision, a py-bug or
decision file names a row that doesn't exist, a scenario's `parity_rows` entry isn't a row id, or an area file is
missing from the index below. It also checks the golden manifests in `parity/goldens/`: each parses with no unknown
keys and `version = 1`, its `parity_rows` are row ids, its `generator`, `golden` and `test` files exist, its
`test` file has a `#[test]` that isn't `#[ignore]`d, and its `fresh_test` is an `#[ignore]`d `#[test]` fn in that
file.

## Index

| Area file | Covers |
| --- | --- |
| [infrastructure](./areas/infrastructure.md) | Infrastructure (M0 and shell plumbing) |
| [startup-auth](./areas/startup-auth.md) | Startup and authentication |
| [main-layout](./areas/main-layout.md) | Main screen layout and shell |
| [searchable-tables](./areas/searchable-tables.md) | Searchable tables (shared component) |
| [repositories](./areas/repositories.md) | Repositories |
| [pull-requests](./areas/pull-requests.md) | Pull requests |
| [diff-viewer](./areas/diff-viewer.md) | Split diff viewer and review |
| [issues](./areas/issues.md) | Issues |
| [workflows](./areas/workflows.md) | Workflows |
| [markdown-links](./areas/markdown-links.md) | Markdown and links |
| [command-palette](./areas/command-palette.md) | Command palette |
| [modals](./areas/modals.md) | Modals |
| [toasts](./areas/toasts.md) | Toasts |
| [keybindings](./areas/keybindings.md) | Keybindings |
| [config](./areas/config.md) | Config |
| [caches-drafts](./areas/caches-drafts.md) | Local caches and drafts |
| [cli](./areas/cli.md) | CLI |
| [terminal-caveats](./areas/terminal-caveats.md) | Terminal caveats (no status column) |

Suspected Python bugs are in [`py-bugs/`](./py-bugs/), listed for Chris in
[this comment on #71](https://github.com/gizmo385/gh-lazy/issues/71#issuecomment-6392375110). Intentional diffs are in
[`decisions/`](./decisions/).
