# Non-ASCII keys are saved as the character itself

- Decision: When a Key Bindings row in Settings records a non-ASCII punctuation or symbol key (`£`, `§`, `°`), the
  Rust app shows and saves the character itself, where Python writes Unicode's character name (`pound_sign`). ASCII
  keys keep Textual's names (`question_mark`, `slash`).
- Decided by: Chris, 2026-10-28
- Link: https://github.com/gizmo385/gh-lazy/issues/194#issuecomment-6485805658
- Rows: `modals.settings-key-names-non-ascii`

Reproducing Textual's names for these keys needs a Unicode name table (a crate of about 0.5 MB) for keys few keymaps
use. Both apps read the character back: Textual turns a one-character key into its name when it loads a keymap, and
the Rust keymap parser accepts single characters (#73). The only visible difference is the row's text for such a key.
