# The gh CLI is the only backend

- Decision: The Rust app talks to GitHub only through the `gh` CLI. The raw-HTTPS backend, the first-start backend
  picker and device-flow auth are dropped. `api.client_type` is preserved in `config.json` but not acted on.
- Decided by: Chris, 2026-10-25
- Link: [PLAN.md §10](../../PLAN.md#10-decisions), decision 1
- Rows: `startup-auth.gh-auth-check`, `startup-auth.backend-picker`, `config.api-client-type`

The Python app can authenticate either through `gh` or directly over HTTPS with a device-flow token, and asks which on
first start. The rewrite keeps one backend so there is one auth story, one HTTP cache (`gh api --cache`) and one set
of parity fixtures (`fake-gh`). `core.first_start` is still read, and written as `false` after the `gh` auth check, so
switching between the two apps keeps working.

Related rows that keep their own status because only part of them differs: the not-logged-in screen
(`startup-auth.not-logged-in-screen`, no device-flow branch), the Settings `api.client_type` select
(`modals.settings-widgets`) and `run --auth-backend` (`cli.run`).
