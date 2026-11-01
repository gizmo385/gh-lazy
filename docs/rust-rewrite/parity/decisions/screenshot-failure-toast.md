# A screenshot that can't be saved says where and why

- Decision: When the palette's "Screenshot" can't write its file, the error toast titled "Screenshot" names the path
  it tried and the I/O error: "Failed to save screenshot to '<path>': <I/O error>", where Textual says only "Failed
  to save screenshot". The directory is the same Downloads directory, and neither app creates it. The error is also
  logged as a warning.
- Decided by: Chris, 2026-10-31 (Py bug B170, the recommendation; D2 option 1 on #291)
- Link: [comment on #291](https://github.com/gizmo385/gh-lazy/issues/291)
- Rows: `toasts.screenshot-failed`

Textual saves screenshots to platformdirs' Downloads directory and doesn't create it, so on most servers and SSH hosts
(and in the parity harness's isolated `HOME`) the save fails, and Textual's toast gives no hint of where it tried or
why; the details go to its devtools log. The Rust toast follows `toasts.settings-save-failed`: the path as Python's
`repr` and the I/O error's text. The success toast and everything else about the command match Python. The
`command-palette-screenshot-error` scenario's `failed` capture checks the toast's text per app with `[expect]`, which
both apps' texts contain, and leaves its box out of the cross-app check.
