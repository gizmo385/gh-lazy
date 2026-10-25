# The Introduction shows the Rust crate version

- Decision: The Introduction tab's H1 shows the Rust crate version (`CARGO_PKG_VERSION`, currently 0.1.0) where
  Python shows its package version (0.6.6). This is expected until cutover, when the crate version is bumped past the
  last Python release and the Python app is removed.
- Decided by: accepted in the parity review of #125, 2026-10-25
- Link: [#125 review](https://github.com/gizmo385/gh-lazy/pull/125#pullrequestreview-5641762999)
- Rows: `repositories.introduction-version`

Faking the Python version would make the Rust app lie about what it is in bug reports. Parity scenarios that capture
the Introduction see the two version digits differ; the rest of the screen is cell-identical.
