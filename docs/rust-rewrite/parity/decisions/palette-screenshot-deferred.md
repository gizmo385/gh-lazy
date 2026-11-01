# The palette's "Screenshot" command is deferred

- Decision: Superseded by #291 (Chris, 2026-10-31): "Screenshot" is back in the Rust palette, saving an SVG as
  Textual does, so `command-palette.screenshot` no longer differs. Kept for its history. It was: the Rust command
  palette has no "Screenshot" command for now; capturing the screen will come back, though not necessarily as
  Textual's SVG export, in #291, which isn't needed for cutover.
- Decided by: Chris, 2026-10-28
- Link: [comment on #191](https://github.com/gizmo385/gh-lazy/issues/191#issuecomment-6480950939); tracked in #291
- Rows: none since #291 (was `command-palette.screenshot`)

Textual's system command "Screenshot" ("Save an SVG 'screenshot' of the current screen") writes an SVG of the
screen through Rich's console export. ratatui has no SVG export, and how the Rust app should capture its screen is
an open design question, so the palette ships without the command. Until #291 lands, the palette's opening list is
"Keys", "Maximize" or "Minimize", "Quit" and "Theme", one entry shorter than Python's, and queries that would match
"Screenshot" don't list it.
