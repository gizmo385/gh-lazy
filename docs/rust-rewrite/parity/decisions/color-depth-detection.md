# Color depth: truecolor terminals detected without `COLORTERM`, and an `appearance.color_depth` setting

- Decision: Keep `TEXTUAL_COLOR_SYSTEM` as the first word on the color depth. After it comes a new Rust-only setting,
  `appearance.color_depth` (`"auto"`, the default, `"truecolor"`, `"256"` or `"16"`), shown in Settings → Appearance
  as a select ("Auto", "Truecolor", "256 colors", "16 colors") and applied on Save without a restart. `auto` goes
  further than Rich: `COLORTERM` of `truecolor`/`24bit` is still truecolor; `TERM_PROGRAM=Apple_Terminal` is 256
  whatever else is set; a terminal known to show 24-bit color is truecolor (`WT_SESSION` set, `TERM_PROGRAM` of
  `iTerm.app`, `WezTerm`, `vscode`, `ghostty`, `Hyper` or `rio`, `KITTY_WINDOW_ID` set, `TERM` of `xterm-kitty`,
  `alacritty`, `xterm-ghostty` or `wezterm`, `KONSOLE_VERSION` set, or `VTE_VERSION` ≥ 3600); otherwise Rich's rules
  as before. The debug info lists the depth and why, as `- Color Depth: truecolor (WT_SESSION)`.
- Decided by: Chris, 2026-11-03
- Link: [#409](https://github.com/gizmo385/gh-lazy/issues/409) (spec and owner decision in the issue body)
- Rows: `config.appearance-color-depth`, `main-layout.color-depth-detection`

Windows Terminal (WSL included) runs with `TERM=xterm-256color` and no `COLORTERM`, so Rich, and the Rust app
following it, drew every theme in the xterm-256 palette. That palette has few dark colors: Solarized Dark's `#002B36`
screen became index 17 (`#00005F`), its `#073642` panes index 23 (`#005F5F`), and Rose Pine Moon's panes index 59, so
the dark themes all looked like one navy-and-teal theme in a terminal that shows 24-bit color. The Python app does
the same there.

The known-terminal list is the variables those terminals set and that reach the app, usually even under tmux or
screen (tmux replaces `TERM_PROGRAM`, but `WT_SESSION` and the like come through). tmux isn't queried; a tmux user
whose outer terminal sets none of them sets `COLORTERM=truecolor` or the setting. `Apple_Terminal` comes after
`COLORTERM`, which nothing sets in Terminal.app unless the user does, so on macOS 26, where Terminal.app shows
24-bit color, setting it (or the setting) still gets truecolor; before `WT_SESSION` and the rest, so a variable
inherited from another terminal doesn't turn Terminal.app's 256 colors into truecolor it can't show.

The setting's key is last in the `appearance` section, after the keys Python shares, so Settings lists Python's
fields in Python's order and every scenario that tabs through the Appearance tab is unchanged; a new config, and
every save, writes it. Python's loader ignores the key and drops it on its next save, which then loads as `auto`
(golden `config`, `python-rust-only-keys.json`). A value that isn't one of the four loads as `auto` rather than
stopping the load, as every other bad enum does, because a bad color depth costs only colors: the command log shows
a warning once the main screen is up, `check-config` lists the value as a problem and `--fix` resets it to `auto`.

`NO_COLOR` is unchanged. The environment is read once at startup; a Settings save picks the depth again from that
reading with the new setting. The parity harness pins `COLORTERM=truecolor`, which wins over everything but
`TEXTUAL_COLOR_SYSTEM`, so every cross-app scenario still compares truecolor screens.
