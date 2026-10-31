# Disabled fields stay dim on the ANSI themes

- Decision: On the ANSI themes (`ansi-dark`, `ansi-light`) a disabled input, switch or select is drawn with the
  terminal's dim attribute (SGR 2), where Python draws it exactly like an enabled one. On every other theme Rust
  fades it as Python does (Textual's `opacity: 0.7`).
- Decided by: Chris, 2026-10-30, on #363: keep Rust's dim on disabled fields on the ANSI themes.
- Link: #363 (owner comment of 2026-10-30)
- Rows: `modals.edit-pr`, `modals.settings-widgets`

Textual marks a disabled widget that could otherwise take focus with `*:disabled:can-focus { opacity: 0.7 }`, which
blends the widget's colors toward its parent's background. Textual can't blend the terminal's own colors, so on the
ANSI themes the opacity changes nothing and a disabled field looks the same as an enabled one: Edit PR's base and
compare inputs and its Draft switch, and Settings' read-only `client_type` select, give no sign that they can't be
changed.

Rust keeps the same cue on the ANSI themes with SGR 2, the only fade the terminal's colors allow, so a disabled
field reads as disabled on every theme. The fields are still out of the focus ring and ignore keys and clicks, as in
Python. Buttons aren't affected: Textual's own `Button:ansi:disabled` is `bold dim`, which Rust already matches.
