# PyPI stops at the last Python release

- Decision: The Rust app isn't published to PyPI. A final Python release, 0.6.7, whose README says the Python app is
  retired and lists the new install paths (`gh extension install gizmo385/gh-lazy`, `cargo install --locked
  lazy-github`, Nix), is the last version of `lazy-github` there. `uvx lazy-github` and `pip install lazy-github` keep
  working, with that last Python version.
- Decided by: Chris, 2026-10-30 (#203 D3, the recommendation)
- Link: [comment on #203](https://github.com/gizmo385/gh-lazy/issues/203#issuecomment-6534623936)
- Rows: `cli.pypi-install`

Publishing the Rust binary as wheels under the same name (maturin `bindings = "bin"`, one wheel per target) would be a
second packaging pipeline to keep working, and PLAN §8 already has the cutover delete `publish.yaml`. The order
matters: `publish.yaml` marks its GitHub release "latest", and gh decides between a binary and a script install from
the latest release, so 0.6.7 must be published before the first Rust release (1.0.0). A Python release after it would
turn new `gh extension install`s back into script installs.
