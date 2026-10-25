# `clear-auth` defers to `gh auth logout`

- Decision: `lazy-github clear-auth` doesn't delete a stored token. It becomes a no-op or a pointer to
  `gh auth logout`; the spec for the CLI issue picks the exact text.
- Decided by: Chris, 2026-10-25 (PLAN.md, merged in #61)
- Link: [PLAN.md §8](../../PLAN.md#8-milestones), M4; follows from [gh-only-backend](./gh-only-backend.md)
- Rows: `cli.clear-auth`

In Python, `clear-auth` deletes the token the HTTPS backend stored. With `gh` as the only backend the Rust app holds
no credentials of its own, so there is nothing for it to clear.
