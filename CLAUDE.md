# LazyGithub (rust-main)

This branch, `rust-main`, is the trunk for the Rust rewrite of LazyGithub. The Python/Textual app in `lazy_github/`
is the reference implementation and stays unchanged here until cutover.

Start with:

- `docs/rust-rewrite/PLAN.md`: goals, stack, architecture, parity strategy, milestones.
- `docs/rust-rewrite/PROCESS.md`: labels, issue lifecycle, spec format, rules for every agent.
- `docs/rust-rewrite/parity/README.md`: the parity matrix (rules and index; rows in `parity/areas/`).
- `.claude/agents/`: role definitions for triage, designer, implementer, code-reviewer and parity-reviewer.

Hard rules:

- Never commit to or open PRs into `main` for rewrite work. Branch `rust/<issue>-<slug>` from `rust-main` and PR back
  into `rust-main`.
- Auto-merge (squash) is allowed on PRs into `rust-main`. Never merge by hand, and never auto-merge into `main`.
- All GitHub interaction in the app goes through the `gh` CLI.
- Write idiomatic Rust; don't transliterate the Python.
- Before pushing: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
