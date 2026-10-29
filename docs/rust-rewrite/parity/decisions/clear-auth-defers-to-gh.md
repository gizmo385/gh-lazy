# `clear-auth` defers to `gh auth logout`

- Decision: `lazy-github clear-auth` doesn't manage a login of its own. If the Python app's token file
  `~/.config/lazy-github/auth.text` exists, it deletes it and prints `Removed the token the Python app stored in
  <path>.`; then, always, it prints a pointer to `gh auth logout` on stdout and exits 0: `LazyGithub signs in
  through the GitHub CLI and stores no credentials of its own. To log out, run: gh auth logout`. A failed delete
  prints `Error: failed to delete <path>: <reason>` on stderr and exits 1, as `clear-config` does (Py bug B97).
- Decided by: Chris, 2026-10-25 (PLAN.md, merged in #61); the pointer chosen over a no-op, and deleting a leftover
  `auth.text`, on 2026-10-28
- Link: [PLAN.md §8](../../PLAN.md#8-milestones), M4; follows from [gh-only-backend](./gh-only-backend.md);
  [#199](https://github.com/gizmo385/gh-lazy/issues/199) (owner comment, 2026-10-28),
  [#308](https://github.com/gizmo385/gh-lazy/issues/308#issuecomment-6485042816)
- Rows: `cli.clear-auth`

In Python, `clear-auth` deletes the token the HTTPS backend stored. With `gh` as the only backend the Rust app holds
no credentials of its own, so there is nothing for it to clear.

A user who used the Python app's HTTPS backend has a plaintext token in `auth.text`, which the Rust app never reads;
`clear-auth` is where they'd expect it to go.
