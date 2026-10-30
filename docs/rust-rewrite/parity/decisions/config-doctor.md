# `check-config`: a config doctor Python doesn't have

- Decision: The Rust app adds `lazy-github check-config [--fix [--no-confirm]]`, a config doctor that lists every
  invalid field in `config.json` (unknown keys as notes) and can reset the invalid ones to their defaults after
  backing the file up to `config.json.bak`. `--fix` asks before writing unless `--no-confirm` is given, and the
  command exits non-zero while problems remain. The broken-config error points at `check-config --fix`
  ([broken-config-error](./broken-config-error.md)). Python has no such command: it crashes, or stops at the first
  error, on a bad `config.json`.
- Decided by: Chris, 2026-10-28 (#208, folded into #199), details 2026-10-29 (#310: no second hint line for invalid
  JSON).
- Link: #208, #199, #310; recorded from the matrix audit in #351
- Rows: `cli.check-config`
