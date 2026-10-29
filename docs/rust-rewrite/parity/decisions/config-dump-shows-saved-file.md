# `dump-config` and the debug info show the config as Rust saves it

- Decision: `lazy-github dump-config`, `lazy-github debug`, the Debug Info modal and the bug report show the config
  exactly as the Rust app would save it: keys it doesn't know are shown after the known ones (Rust keeps them on
  save), and an `api` section without `client_type` stays without it.
- Decided by: Chris, 2026-10-28
- Link: [#308](https://github.com/gizmo385/gh-lazy/issues/308#issuecomment-6485042816)
- Rows: `cli.dump-unknown-keys`

Python's pydantic models ignore unknown keys, so its dump and debug text leave them out, and fill a missing
`api.client_type` with its default `"RAW_HTTP"`. The Rust config keeps unknown keys and writes them back
([PLAN §5](../../PLAN.md#5-architecture)), so the text it shows is the file it will write, which is what a user
checking their config needs to see. For a file without unknown keys and with `client_type` set (every file Python
saves), the output is the same.
