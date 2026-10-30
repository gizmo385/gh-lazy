# A bracketed paste on the main screen opens the paste link modal, prefilled

- Decision: A bracketed paste on the main screen, when no dialog is open and no text input (a table's search box,
  the diff viewer's file filter) has focus, opens the paste link modal with the pasted text in its input, as the
  input's own paste puts it there (the first line). A `ctrl+v` that arrives as a key still opens it empty, as in
  Python.
- Decided by: Chris, 2026-10-28
- Link: [comment on #197](https://github.com/gizmo385/gh-lazy/issues/197#issuecomment-6480951361)
- Rows: `markdown-links.paste-opens-link-modal`

Many terminals turn `ctrl+v` into a paste of the system clipboard (`terminal-caveats.ctrl-v-paste`), so
`link.paste` never reaches the app and the modal can't be opened there. In Python a bracketed paste on the main
screen goes to the focused table, which ignores it (a Python-only probe confirmed nothing happens). Opening the
modal with the pasted link in it gives those terminals a way in, and saves a step in the others.

Since the apps differ on purpose, no parity scenario compares them; unit tests on the Rust side pin it.
