# The debug info lists the Rust app's build and environment

- Decision: The "Library Versions" list in the debug info (the Debug Info modal, `lazy-github debug` and the bug
  report) shows the LazyGithub version, the rustc the binary was built with, the `gh` version, the OS and the
  terminal, instead of Python's LazyGithub, Textual, Click, Httpx, Hishel and Pydantic versions.
- Decided by: Chris, 2026-10-28
- Link: [#195](https://github.com/gizmo385/gh-lazy/issues/195) (owner comment, 2026-10-28)
- Rows: `modals.debug-library-versions`

The Python list names the libraries a bug in that app most likely comes from. The Rust app has none of them, and
its own dependencies are pinned by `Cargo.lock` for a given version, so the version already identifies them. What
varies between users' installs is the toolchain, the `gh` it drives, and where it runs, so those are listed instead.
The heading, the list format (`- <Name> Version: <value>`) and the "LazyGithub Config" section stay as in Python.
