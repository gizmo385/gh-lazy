# Themes are the app's own data: Textual's two themes are renamed, and an unknown name is kept and warned about

- Decision: The app ships its themes as data instead of taking them from Textual, and owns their names.
  - D1: all 21 of Textual 8.2.5's built-in themes ship, frozen as data files (#378). The 17 that port a named color
    scheme (everything but `dark`, `light`, `ansi-dark` and `ansi-light`) were then audited against their upstream
    palettes (#392): exact seed colors, body and muted text, surfaces, syntax and diff lines from the scheme itself,
    with each file's header citing its source and saying which upstream color each role takes. Those 17 no longer
    match Python's colors; `dark`, `light` and the ANSI pair still do.
  - D2 (b): `textual-dark` is renamed `dark` ("Dark") and `textual-light` is renamed `light` ("Light"). `dark` is the
    default for a new config and what an unknown name draws as. The old names stay valid aliases forever: they load
    and draw exactly as `dark` and `light`, the command palette's theme list and the Settings select don't list them,
    and the stored value is never rewritten on its own, only when the user picks a theme in Settings and saves. The
    other 19 names are the color schemes' own and stay as they are.
  - D3 (a): user themes are `~/.config/lazy-github/themes/<name>.toml` files that extend a built-in (#380).
  - D4 (a): theme files are TOML.
  - D5 (a): an unknown `appearance.theme` string (a typo, a deleted user theme, a theme from a newer version) is kept
    in `config.json` as written. The app draws `dark` and, once at startup when the first screen after the startup
    one appears (auth or main), shows a warning toast "Unknown theme", `No theme is called "<name>". Using "dark".`
    Matching is exact and case-sensitive, as in Python (`Nord` is unknown). The Settings theme select shows its blank
    option for it, and Save leaves the name alone unless the user picks a theme. A number, bool or `null` still loads
    as `dark` and is written back on save, as Python does; a list or object is still the broken-config error (Py bug
    B39).
- Decided by: Chris, 2026-11-01 (#347)
- Link: https://github.com/gizmo385/gh-lazy/issues/347#issuecomment-6554309331
- Rows: `main-layout.themes`, `main-layout.theme-unknown-warning`, `config.appearance-theme`, `command-palette.theme`,
  `modals.settings-widgets`

Python takes its themes from Textual's `BUILTIN_THEMES`: a name outside them, or any non-string scalar, loads as
`textual-dark`, silently, and the next save writes `textual-dark` back (`lib/config.py`
`AppearanceSettings.validate_theme`). That would erase a user theme's name the first time its file was missing or
broken, so the Rust app keeps the name and says what it did instead.

A Python app reading a config the Rust app saved with `dark` or `light` doesn't know the name and falls back to
`textual-dark`, which looks the same as `dark`. A config Python wrote keeps working unchanged, since `textual-dark`
and `textual-light` stay aliases and are never rewritten unasked.

`ansi-dark` and `ansi-light` keep their names: Textual calls them that, with no Textual branding. The config doctor
(`check-config`) doesn't report an unknown theme name, as it can't know the user's theme files; the startup toast
covers it.
