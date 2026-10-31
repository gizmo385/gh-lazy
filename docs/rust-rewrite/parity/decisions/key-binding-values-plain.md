# Key binding values are plain text, without Python's RichLog box

- Decision: In the Settings Key Bindings tab, Rust draws each binding's key as plain text on the tab's background.
  Python's two-row `$surface` box with a scrollbar track behind each key isn't copied.
- Decided by: Chris, 2026-10-30, on #366: an intentional diff; the tab's layout is left to #342's redesign.
- Link: #366, #342
- Rows: `modals.settings-key-rebinding`

Python shows each key value in a Textual `RichLog` (`KeySelectionInput.key_input`) inside a `height: 2` container.
`RichLog` has its own `$surface` background and an always-on vertical scrollbar, so every key sits in a darker
two-row bar with a black 2-column track at its right end. That comes from using a log widget as a one-line label,
not from a design choice, so it falls under the Textual-only rendering details PLAN §10 decision 8 leaves out of
parity.

Everything else in the tab matches: one row per binding sorted by id, the `<description or id>: <key>` text, the
focused row's three-row `solid $accent` box, key capture, `escape` to reset and the saved values.
