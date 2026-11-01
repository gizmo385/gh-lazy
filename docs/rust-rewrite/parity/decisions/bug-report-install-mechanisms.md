# The bug report's install placeholder names the Rust install paths

- Decision: At cutover, the "**Installation Mechanism:**" placeholder in Open Bug Report's new-issue body reads
  `[gh extension, release binary, cargo, nix, etc]` instead of Python's `[Github CLI, nix, PyPi, etc]`, and the
  GitHub issue template (`.github/ISSUE_TEMPLATE/bug_report.md`) changes the same way. Lands in #402, the cutover
  prep; until then the app shows Python's text.
- Decided by: Chris, 2026-10-30 (#206 D1, the recommendation)
- Link: [comment on #206](https://github.com/gizmo385/gh-lazy/issues/206#issuecomment-6534625798)
- Rows: `modals.bug-report-template`

The Rust app isn't published to PyPI (decision [pypi-final-python-release](./pypi-final-python-release.md)), so
"PyPi" would ask reporters for an install path that no longer exists, and "Github CLI" doesn't say whether it's the
old script extension or the new binary. The new list matches the README's install section (#206): the gh extension,
a downloaded release binary, `cargo install` and Nix. The row's Status becomes `intentional diff` when #402 changes
the template; the rest of the template is unchanged.
