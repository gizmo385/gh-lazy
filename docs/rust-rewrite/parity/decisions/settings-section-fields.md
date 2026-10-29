# Settings saves each section's own fields

- Decision: The Settings modal writes every field to its own section, so the Issues tab's State Filter and Owner
  Filter save to `issues.*`, where Python copies the Pull Requests tab's values into them (Py bug B1). Values are
  saved typed (Py bug B2), and text that isn't a valid value is refused with an "Invalid setting" toast (Py bug B90).
- Decided by: Chris, 2026-10-28
- Link: https://github.com/gizmo385/gh-lazy/issues/193#issuecomment-6485805292
- Rows: `modals.settings-values`

Python looks each field's widget up by an id built from the field name alone (`adjust_state_filter_input`), and the
first match in the modal is the Pull Requests tab's, so the Issues filters can't be changed from Settings. The Rust
modal is built from per-field descriptors (#304), and each descriptor reads and writes its own section. What the user
sees on screen is unchanged; only what Save writes differs, and the result loads in both apps.
