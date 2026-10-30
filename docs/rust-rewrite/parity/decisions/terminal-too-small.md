# A terminal below 40x15 shows only "Terminal too small"

- Decision: While the terminal is narrower than 40 columns or shorter than 15 rows, the app draws only the theme's
  background and a centered "Terminal too small (<w>×<h>)" (the size on a second line when the terminal is too narrow
  for one), and ignores every key, click and paste except a key that quits right then: `ctrl+q` always, and `q`
  where it would quit at full size (no dialog open, no text field focused). Background work (requests, timers, the
  notifications poll) carries on, and everything open (dialogs, typed text, focus, scroll positions, a maximized
  view, the help panel) is drawn as it was once the terminal grows back.
- Decided by: Chris, 2026-10-30, on #326: below the minimum size, draw only the centered "Terminal too small (w×h)"
  message and ignore every key except quit; the implementer picks the threshold near 40x12 from where the layout
  first becomes unusable. He then confirmed the chosen 40x15, and that `q` quits only where it would at full size
  while `ctrl+q` always quits.
- Link: #326 (owner comment of 2026-10-30), parent #201
- Rows: `main-layout.too-small`

Python has no minimum size: Textual lays out with CSS and clips, so at any size it draws whatever fits and every key
works, even on boxes that are no longer on screen.

The threshold is where the main screen first becomes unusable. With the default sections and a repo selected (its
Issues and Workflows empty, so three rows each), the focused Repositories section takes 40% of the rows below the
status box, and the Pull Requests section gets what's left: below 15 rows that's nothing, so its box isn't drawn at
all and `2` focuses a section that can't be seen. At 15 rows every section's title is on screen. The proposed 12
rows loses the Pull Requests box in that state, and on a fresh launch (no repo yet) the Pull Requests box loses
its bottom border. Width has no such edge: between 30 and 40 columns nothing vanishes, but the 40% section column
shrinks to a few letters of each title and table, so the proposed 40 columns stays. Dialogs taller than the terminal already scroll (Textual's
modal screen scrollbar), so they don't set the threshold.

`q` doesn't quit from a text field or a dialog while the terminal is small, so a `q` typed into an issue body while
a terminal multiplexer pane is briefly shrunk can't throw the text away.

The threshold doesn't guard a crash: with it set to 0x0, the property test `every_size_draws_without_panicking`
passes at every size from 0x0 up, every screen and dialog included (checked in #326, where the lookup, Add
Reaction and Trigger Workflow dialogs were fixed for frames taller than the terminal).
