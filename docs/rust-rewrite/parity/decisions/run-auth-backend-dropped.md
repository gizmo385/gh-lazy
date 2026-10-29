# `run --auth-backend` is dropped

- Decision: `lazy-github run` takes no `--auth-backend` option and doesn't read `AUTH_BACKEND`. Passing the option
  is a usage error (clap's "unexpected argument", exit 2), like any unknown option.
- Decided by: Chris, 2026-10-28
- Link: [#199](https://github.com/gizmo385/gh-lazy/issues/199) (owner comment, 2026-10-28); follows from
  [gh-only-backend](./gh-only-backend.md)
- Rows: `cli.run`

In Python, `run --auth-backend GITHUB_CLI|RAW_HTTP` (or `AUTH_BACKEND` in the environment) writes `api.client_type`
to the config before starting. The Rust app only talks to GitHub through `gh` and doesn't act on
`api.client_type` (it keeps the saved value as it is), so the option would change nothing the user can see.
