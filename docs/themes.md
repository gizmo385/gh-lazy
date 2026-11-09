# Themes

LazyGithub ships 21 built-in themes, picked in Settings (Appearance → Theme) or for the session with the command
palette's "Theme" command. You can add your own: each is a small TOML file that starts from a built-in theme and
changes the colors you want.

## Where theme files go

Every `*.toml` file directly in the `themes` folder of the config directory is a theme:

- Linux and macOS: `~/.config/lazy-github/themes/`
- Windows: `%APPDATA%\lazy-github\themes\`

The theme is named after the file: `themes/my-nord.toml` is the theme `my-nord`, which is what Settings saves as
`appearance.theme` in `config.json`. A name must be lowercase letters, digits and `-`, and can't be a built-in
theme's name (or `textual-dark` or `textual-light`). Other files and subfolders are ignored.

Theme files are read once, at startup: after editing one, restart LazyGithub. A file that can't be used is skipped,
with a "Theme file skipped" warning naming the file and what's wrong (with the line, for a mistake on one); the
other themes still load. If your config names a theme that was skipped, LazyGithub draws the default theme (`dark`)
and keeps the name in `config.json`, so fixing the file brings it back.

Settings lists the built-in themes first, then yours by name. The command palette's theme list sorts them all
together by name.

## An example

```toml
# ~/.config/lazy-github/themes/my-nord.toml
extends = "nord"              # a built-in theme to start from; "dark" if left out
display_name = "My Nord"      # what Settings shows; "My Nord" is also the default for my-nord.toml
appearance = "dark"           # "dark" or "light"; the base theme's if left out

[border]
pane_focused = "#EBCB8B"      # the focused pane's box

[marks]
merged = "#B48EAD"            # merged pull requests
```

Everything the file doesn't set comes from the theme it extends.

## The file

At the top, before any `[table]`:

| Key | Value | Default |
| --- | --- | --- |
| `extends` | The name of a built-in theme (or `textual-dark` / `textual-light`). It can't name another of your themes. | `"dark"` |
| `display_name` | The name Settings shows. | The file name in title case: `my-nord` is "My Nord". |
| `appearance` | `"dark"` or `"light"`. | The base theme's. |

Then any of the roles below, grouped in tables by the part before the last dot: `border.pane_focused` is written as
`pane_focused = "…"` under `[border]`, and `cursor.focused.bg` as `bg = "…"` under `[cursor.focused]`. A key that
isn't a role, or a value that doesn't fit the role, skips the file.

Whether a theme draws in the terminal's own 16 colors (`ansi-dark`, `ansi-light`) comes from the theme it extends,
and a theme file can't set it (`terminal_colors`): such a theme draws differently, without blending.

The built-in themes' own files, which list every role, are in
[`crates/lazy-github-tui/themes/`](../crates/lazy-github-tui/themes/) and make good starting points.

## Values

- **color**:
  - `#rrggbb`, such as `"#EBCB8B"`;
  - `#rrggbbaa` or `#rrggbb N%`, with alpha: drawn over whatever is under it (`"#000000 10%"` darkens a little);
  - `auto` or `auto N%`: white or black, whichever contrasts with what's under it;
  - `ansi:<name>`: one of the terminal's 16 colors, `ansi:black`, `ansi:red`, `ansi:green`, `ansi:yellow`,
    `ansi:blue`, `ansi:magenta`, `ansi:cyan`, `ansi:white`, or one of those as `ansi:bright-<name>`;
  - `default`: the terminal's default color;
  - `transparent`.
- **color or `none`**: a color, or `none` for the code block's own (syntax highlighting only).
- **text style**: space-separated flags out of `bold dim italic underline reverse strike blink`, each turned on, or
  turned off with a leading `!` (`"bold !underline"`); `"none"` for no change.
- **`#rrggbb`**: an exact color with no alpha (SVG screenshots only).
- **`true` or `false`**.

## Roles

What each color is for. The `$name` in parentheses is the Textual variable a built-in theme's value comes from.

### `[surface]`

Backgrounds: the screen, panes, dialogs.

| Key | Value | What it's for |
| --- | --- | --- |
| `surface.screen` | color | The screen, which everything else is drawn over (`$background`). |
| `surface.pane` | color | A pane's or widget's own background: tables, trees, lists, inputs, switches (`$surface`). |
| `surface.header` | color | A table's header row (`$panel`). |
| `surface.raised` | color | Toasts, raised over the screen (`$panel-lighten-1`). |
| `surface.dialog` | color | A dialog's body, unless the dialog says otherwise (`$surface-lighten-3`). |
| `surface.card` | color | The body of a dialog that shows one record, the user profile (`$surface-lighten-1`). |
| `surface.boost` | color | A lifted area inside a pane: the loading overlay, a block quote, the text editor's current line (`$boost`). |
| `surface.scrim` | color | What a modal screen dims the screens below it with, blended over every cell (`$background 60%`; transparent on terminal-color themes, which can't blend). |
| `surface.focus_tint` | color | The tint a focused pane's background gets (`background-tint: $foreground 5%`), for `tint` at 100%. |
| `surface.hatch` | color | The hatch pattern over an empty area (`$panel`). |

### `[text]`

Text colors.

| Key | Value | What it's for |
| --- | --- | --- |
| `text.normal` | color | Body text with the theme's text contrast (`$text`). |
| `text.plain` | color | A widget's plain text color (`$foreground`). |
| `text.muted` | color | Secondary text: authors, headings of previews (`$text-muted`). |
| `text.disabled` | color | Placeholder and unavailable text (`$text-disabled`). |
| `text.primary` | color | Text in the primary color: help panel keys' descriptions (`$text-primary`). |
| `text.secondary` | color | Text in the secondary color (`$text-secondary`). |
| `text.accent` | color | Text in the accent color: help panel keys (`$text-accent`). |
| `text.success` | color | Success text: an information toast's title (`$text-success`). |
| `text.warning` | color | Warning text: a warning toast's title (`$text-warning`). |
| `text.error` | color | Error text: an error toast's title (`$text-error`). |

### `[brand]`

The theme's seed colors, as fills.

| Key | Value | What it's for |
| --- | --- | --- |
| `brand.primary` | color | The primary color: the loading dots, a markdown table's header, a selected check list item (`$primary`). |
| `brand.secondary` | color | The secondary color (`$secondary`). |
| `brand.accent` | color | The accent color (`$accent`). |

### `[status]`

Status colors, as fills and borders: toasts, invalid inputs, link hints.

| Key | Value | What it's for |
| --- | --- | --- |
| `status.success` | color | An information toast's border, a switch that's on (`$success`). |
| `status.warning` | color | A warning toast's border, a link hint's label (`$warning`). |
| `status.error` | color | An error toast's border, an invalid input's border, a removed file's notice (`$error`). |

### `[border]`

Box and rule colors.

| Key | Value | What it's for |
| --- | --- | --- |
| `border.pane_focused` | color | A focused pane or comment's box (`$success`). |
| `border.section_title` | color | An unfocused section's titled box (`$primary-lighten-3`). |
| `border.field_focused` | color | A focused input, select, switch or editor's border (`$border`). |
| `border.field` | color | An unfocused field's border, and a disabled button's (`$border-blurred`). |
| `border.rule` | color | A horizontal rule: in dialogs, overviews and markdown (`$secondary`). |
| `border.status_bar` | color | The status bar's box (`$secondary`). |
| `border.dialog` | color | A dialog's frame, unless the dialog says otherwise (`$background 80%`). |
| `border.emphasis` | color | A box that draws attention: the diff viewer's review container, a key being captured in Settings, the Add Comment dialog's frame (`$accent`). |
| `border.conversation` | color | An unfocused comment's box and a review thread's left bar in the conversation (`$secondary-background`). |
| `border.review` | color | The box around a review in the conversation (`$success-lighten-3`). |
| `border.help` | color | The help panel's left edge (`$foreground 30%`; `ansi_black` on terminal-color themes). |

### `[cursor]`

The block cursor that highlights the current row of a list, table, tree or tab strip.

| Key | Value | What it's for |
| --- | --- | --- |
| `cursor.focused.fg` | color | In a focused widget (`$block-cursor-*`): the text color, drawn over `bg`. |
| `cursor.focused.bg` | color | In a focused widget (`$block-cursor-*`): the background, drawn over what's under it. |
| `cursor.focused.style` | text style | In a focused widget (`$block-cursor-*`): the text style applied on top of the row's own. |
| `cursor.blurred.fg` | color | In an unfocused widget (`$block-cursor-blurred-*`): the text color, drawn over `bg`. |
| `cursor.blurred.bg` | color | In an unfocused widget (`$block-cursor-blurred-*`): the background, drawn over what's under it. |
| `cursor.blurred.style` | text style | In an unfocused widget (`$block-cursor-blurred-*`): the text style applied on top of the row's own. |
| `cursor.hover_bg` | color | A row under the mouse (`$block-hover-background`). |

### `[scrollbar]`

Scrollbars.

| Key | Value | What it's for |
| --- | --- | --- |
| `scrollbar.thumb` | color | The thumb, over the track (`$scrollbar`). |
| `scrollbar.track` | color | The track (`$scrollbar-background`). |
| `scrollbar.corner` | color | The corner where two scrollbars meet (`$scrollbar-corner-color`). |

### `[input]`

Text inputs and the text editor.

| Key | Value | What it's for |
| --- | --- | --- |
| `input.cursor.fg` | color | An input's cursor (`$input-cursor-*`; `ansi_black` on `ansi_white` on terminal-color themes): the text color, drawn over `bg`. |
| `input.cursor.bg` | color | An input's cursor (`$input-cursor-*`; `ansi_black` on `ansi_white` on terminal-color themes): the background, drawn over what's under it. |
| `input.cursor.style` | text style | An input's cursor (`$input-cursor-*`; `ansi_black` on `ansi_white` on terminal-color themes): the text style applied on top of the row's own. |
| `input.editor_cursor.fg` | color | The text editor's cursor (`$input-cursor-*`; `$text 90%` on `$foreground 70%` on light themes; reversed on terminal-color themes): the text color, drawn over `bg`. |
| `input.editor_cursor.bg` | color | The text editor's cursor (`$input-cursor-*`; `$text 90%` on `$foreground 70%` on light themes; reversed on terminal-color themes): the background, drawn over what's under it. |
| `input.editor_cursor.style` | text style | The text editor's cursor (`$input-cursor-*`; `$text 90%` on `$foreground 70%` on light themes; reversed on terminal-color themes): the text style applied on top of the row's own. |
| `input.selection.fg` | color | Selected text (`$input-selection-*`): the text color, drawn over `bg`. |
| `input.selection.bg` | color | Selected text (`$input-selection-*`): the background, drawn over what's under it. |
| `input.placeholder` | color | A placeholder or suggestion (`$text-disabled`). |
| `input.placeholder_style` | text style | A placeholder's or suggestion's text style (dim on terminal-color themes). |
| `input.invalid` | color | A focused invalid input's border (`$error`). |
| `input.invalid_blurred` | color | An unfocused invalid input's border (`$error 60%`). |
| `input.terminal_colors` | `true` or `false` | Whether inputs are drawn in the terminal's own colors (`Input:ansi`): its default text on its default background, whatever they're drawn over. |

### `[footer]`

The key-binding footer.

| Key | Value | What it's for |
| --- | --- | --- |
| `footer.bar` | color | The bar (`$footer-background`). |
| `footer.item` | color | A binding's background, over the bar (`$footer-item-background`). |
| `footer.hover` | color | A binding under the mouse, over the bar (`$block-hover-background`). |
| `footer.key.fg` | color | A binding's key, over its item (`$footer-key-*`): the text color, drawn over `bg`. |
| `footer.key.bg` | color | A binding's key, over its item (`$footer-key-*`): the background, drawn over what's under it. |
| `footer.description.fg` | color | A binding's description, over its item (`$footer-description-*`): the text color, drawn over `bg`. |
| `footer.description.bg` | color | A binding's description, over its item (`$footer-description-*`): the background, drawn over what's under it. |

### `[button]`

Buttons, by variant.

| Key | Value | What it's for |
| --- | --- | --- |
| `button.outlined` | `true` or `false` | Whether buttons are drawn outlined in the terminal's own colors (`Button:ansi`): the terminal's background, a `tall` border in the variant's `ButtonColors::border` on all four sides, and a bold label. |
| `button.focus_style` | text style | The label's style while focused (`$button-focus-text-style`). |
| `button.disabled_border` | color | A disabled outlined button's border (`$border-blurred`). |
| `button.default.bg` | color | The default button: the fill (`$surface`, `$primary`, …). |
| `button.default.light` | color | The default button: the top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default). |
| `button.default.dark` | color | The default button: the bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default). |
| `button.default.border` | color | The default button: an outlined button's border (`$<variant>`; `$border-blurred` for default). |
| `button.default.label` | color | The default button: the label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined). |
| `button.default.label_disabled` | color | The default button: a disabled button's label (the label at `opacity: 0.6`). |
| `button.primary.bg` | color | The primary button: the fill (`$surface`, `$primary`, …). |
| `button.primary.light` | color | The primary button: the top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default). |
| `button.primary.dark` | color | The primary button: the bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default). |
| `button.primary.border` | color | The primary button: an outlined button's border (`$<variant>`; `$border-blurred` for default). |
| `button.primary.label` | color | The primary button: the label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined). |
| `button.primary.label_disabled` | color | The primary button: a disabled button's label (the label at `opacity: 0.6`). |
| `button.success.bg` | color | The success (submit) button: the fill (`$surface`, `$primary`, …). |
| `button.success.light` | color | The success (submit) button: the top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default). |
| `button.success.dark` | color | The success (submit) button: the bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default). |
| `button.success.border` | color | The success (submit) button: an outlined button's border (`$<variant>`; `$border-blurred` for default). |
| `button.success.label` | color | The success (submit) button: the label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined). |
| `button.success.label_disabled` | color | The success (submit) button: a disabled button's label (the label at `opacity: 0.6`). |
| `button.warning.bg` | color | The warning button: the fill (`$surface`, `$primary`, …). |
| `button.warning.light` | color | The warning button: the top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default). |
| `button.warning.dark` | color | The warning button: the bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default). |
| `button.warning.border` | color | The warning button: an outlined button's border (`$<variant>`; `$border-blurred` for default). |
| `button.warning.label` | color | The warning button: the label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined). |
| `button.warning.label_disabled` | color | The warning button: a disabled button's label (the label at `opacity: 0.6`). |
| `button.error.bg` | color | The error (cancel) button: the fill (`$surface`, `$primary`, …). |
| `button.error.light` | color | The error (cancel) button: the top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default). |
| `button.error.dark` | color | The error (cancel) button: the bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default). |
| `button.error.border` | color | The error (cancel) button: an outlined button's border (`$<variant>`; `$border-blurred` for default). |
| `button.error.label` | color | The error (cancel) button: the label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined). |
| `button.error.label_disabled` | color | The error (cancel) button: a disabled button's label (the label at `opacity: 0.6`). |

### `[switch]`

The on/off switch.

| Key | Value | What it's for |
| --- | --- | --- |
| `switch.track` | color | The track the slider moves in (`$panel-darken-2`). |
| `switch.on` | color | The slider while on (`$success`). |
| `switch.off` | color | The slider while off (`$panel`; `$primary 15%` on light themes; `ansi_red` on the terminal-color dark theme). |

### `[select]`

The select (drop-down).

| Key | Value | What it's for |
| --- | --- | --- |
| `select.muted` | color | The closed select's prompt while nothing is chosen (`$foreground 50%`). |
| `select.terminal_colors` | `true` or `false` | Whether the closed select is drawn in the terminal's own colors (`SelectCurrent:ansi`): its default text on its default background, whatever it's drawn over, the prompt included. |

### `[checkbox]`

The selection list's check boxes.

| Key | Value | What it's for |
| --- | --- | --- |
| `checkbox.bg` | color | The box (`$panel`). |
| `checkbox.off` | color | The mark while unchecked, over the box (`$panel-darken-2`). |
| `checkbox.on` | color | The mark while checked, over the box (`$text-success`). |

### `[tree]`

Tree guide lines.

| Key | Value | What it's for |
| --- | --- | --- |
| `tree.guide` | color | In an unfocused tree (`$surface-lighten-2`; `$surface-darken-1` on light themes; `$ansi-background` on terminal-color themes). |
| `tree.guide_focused` | color | In a focused tree (`$surface-lighten-3`, and as `Self::guide` on light and terminal-color themes). |
| `tree.guide_selected` | color | The cursor row's guides (`$block-cursor-background`). |

### `[tabs]`

Tab strips.

| Key | Value | What it's for |
| --- | --- | --- |
| `tabs.label` | color | An inactive tab's label (`$foreground 50%`). |
| `tabs.label_style` | text style | An inactive tab's label style (dim on terminal-color themes). |
| `tabs.label_active` | color | The active tab's label in an unfocused strip (`$foreground`). |
| `tabs.label_active_style` | text style | The active tab's label style in an unfocused strip (bold on terminal-color themes). |
| `tabs.bar` | color | The underline bar of an unfocused strip (`$foreground 10%`; `$border-blurred` on terminal-color themes). |
| `tabs.bar_focused` | color | The underline bar of a focused strip (`$foreground 30%`; `$border-blurred` on terminal-color themes). |
| `tabs.bar_active` | color | The bar under the active tab (`$block-cursor-background`). |

### `[markdown]`

Rendered markdown.

| Key | Value | What it's for |
| --- | --- | --- |
| `markdown.h1.fg` | color | A level-1 heading, and a dialog's title (`$markdown-h1-*`): the text color, drawn over `bg`. |
| `markdown.h1.bg` | color | A level-1 heading, and a dialog's title (`$markdown-h1-*`): the background, drawn over what's under it. |
| `markdown.h1.style` | text style | A level-1 heading, and a dialog's title (`$markdown-h1-*`): the text style applied on top of the row's own. |
| `markdown.h2.fg` | color | A level-2 heading (`$markdown-h2-*`): the text color, drawn over `bg`. |
| `markdown.h2.bg` | color | A level-2 heading (`$markdown-h2-*`): the background, drawn over what's under it. |
| `markdown.h2.style` | text style | A level-2 heading (`$markdown-h2-*`): the text style applied on top of the row's own. |
| `markdown.h3.fg` | color | A level-3 heading (`$markdown-h3-*`): the text color, drawn over `bg`. |
| `markdown.h3.bg` | color | A level-3 heading (`$markdown-h3-*`): the background, drawn over what's under it. |
| `markdown.h3.style` | text style | A level-3 heading (`$markdown-h3-*`): the text style applied on top of the row's own. |
| `markdown.h4.fg` | color | A level-4 heading (`$markdown-h4-*`): the text color, drawn over `bg`. |
| `markdown.h4.bg` | color | A level-4 heading (`$markdown-h4-*`): the background, drawn over what's under it. |
| `markdown.h4.style` | text style | A level-4 heading (`$markdown-h4-*`): the text style applied on top of the row's own. |
| `markdown.h5.fg` | color | A level-5 heading (`$markdown-h5-*`): the text color, drawn over `bg`. |
| `markdown.h5.bg` | color | A level-5 heading (`$markdown-h5-*`): the background, drawn over what's under it. |
| `markdown.h5.style` | text style | A level-5 heading (`$markdown-h5-*`): the text style applied on top of the row's own. |
| `markdown.h6.fg` | color | A level-6 heading (`$markdown-h6-*`): the text color, drawn over `bg`. |
| `markdown.h6.bg` | color | A level-6 heading (`$markdown-h6-*`): the background, drawn over what's under it. |
| `markdown.h6.style` | text style | A level-6 heading (`$markdown-h6-*`): the text style applied on top of the row's own. |
| `markdown.link.fg` | color | A link (`$link-color`, `$link-style`): the text color. |
| `markdown.link.style` | text style | A link (`$link-color`, `$link-style`): the text style. |
| `markdown.fence_bg` | color | A code block's background (`black 10%`; `white 30%` on light themes; transparent on terminal-color themes). |
| `markdown.fence_inset` | `true` or `false` | Whether a code block has its margin above and below, its side padding and a filled background (`MarkdownFence`; not on terminal-color themes, `MarkdownFence:ansi`). |
| `markdown.code` | color | Inline code's text (`$text-warning 95%`; `$text-error 95%` on light themes). |
| `markdown.code_bg` | color | Inline code's background, drawn as a component background (`ThemeColor::component_background`): `$warning 10%`, `$error 5%` on light themes. Transparent on terminal-color themes, where the run keeps what's under it. |
| `markdown.quote_bg` | color | A block quote's background (`$boost`). |
| `markdown.quote_bar` | color | A block quote's left bar (`$text-primary 50%`; `$text-secondary` on light themes). |
| `markdown.bullet` | color | A list's bullets and numbers (`$text-primary`; `$text-secondary` on light themes). |
| `markdown.table_bg` | color | A table's background (transparent; `white 30%` on light themes). |
| `markdown.table_header` | color | A table's header text (`$primary`). |
| `markdown.table_line` | color | A table's keylines (`$foreground 20%`). |

### `[syntax]`

Syntax highlighting in markdown code blocks.

| Key | Value | What it's for |
| --- | --- | --- |
| `syntax.text` | color | A code block's text, under every token (`$text` at full strength). |
| `syntax.comment.fg` | color or `none` | Comments (`$text 60%`): the token's text color, or the code block's text color for `None`. |
| `syntax.comment.bg` | color or `none` | Comments (`$text 60%`): the token's background, or the code block's for `None`. |
| `syntax.comment.style` | text style | Comments (`$text 60%`): text style added to the token. |
| `syntax.error.fg` | color or `none` | Text the lexer couldn't read (`$text-error` on `$error-muted`): the token's text color, or the code block's text color for `None`. |
| `syntax.error.bg` | color or `none` | Text the lexer couldn't read (`$text-error` on `$error-muted`): the token's background, or the code block's for `None`. |
| `syntax.error.style` | text style | Text the lexer couldn't read (`$text-error` on `$error-muted`): text style added to the token. |
| `syntax.generic_strong.fg` | color or `none` | Strong text in markup languages (bold): the token's text color, or the code block's text color for `None`. |
| `syntax.generic_strong.bg` | color or `none` | Strong text in markup languages (bold): the token's background, or the code block's for `None`. |
| `syntax.generic_strong.style` | text style | Strong text in markup languages (bold): text style added to the token. |
| `syntax.generic_emph.fg` | color or `none` | Emphasized text in markup languages (italic): the token's text color, or the code block's text color for `None`. |
| `syntax.generic_emph.bg` | color or `none` | Emphasized text in markup languages (italic): the token's background, or the code block's for `None`. |
| `syntax.generic_emph.style` | text style | Emphasized text in markup languages (italic): text style added to the token. |
| `syntax.generic_heading.fg` | color or `none` | Headings in markup languages (underlined `$text-primary`): the token's text color, or the code block's text color for `None`. |
| `syntax.generic_heading.bg` | color or `none` | Headings in markup languages (underlined `$text-primary`): the token's background, or the code block's for `None`. |
| `syntax.generic_heading.style` | text style | Headings in markup languages (underlined `$text-primary`): text style added to the token. |
| `syntax.generic_subheading.fg` | color or `none` | Subheadings in markup languages (`$text-primary`): the token's text color, or the code block's text color for `None`. |
| `syntax.generic_subheading.bg` | color or `none` | Subheadings in markup languages (`$text-primary`): the token's background, or the code block's for `None`. |
| `syntax.generic_subheading.style` | text style | Subheadings in markup languages (`$text-primary`): text style added to the token. |
| `syntax.keyword.fg` | color or `none` | Keywords (`$text-accent`): the token's text color, or the code block's text color for `None`. |
| `syntax.keyword.bg` | color or `none` | Keywords (`$text-accent`): the token's background, or the code block's for `None`. |
| `syntax.keyword.style` | text style | Keywords (`$text-accent`): text style added to the token. |
| `syntax.keyword_constant.fg` | color or `none` | Constant keywords such as `true` and `None` (bold `$text-success 80%`): the token's text color, or the code block's text color for `None`. |
| `syntax.keyword_constant.bg` | color or `none` | Constant keywords such as `true` and `None` (bold `$text-success 80%`): the token's background, or the code block's for `None`. |
| `syntax.keyword_constant.style` | text style | Constant keywords such as `true` and `None` (bold `$text-success 80%`): text style added to the token. |
| `syntax.keyword_namespace.fg` | color or `none` | Import keywords (`$text-error`): the token's text color, or the code block's text color for `None`. |
| `syntax.keyword_namespace.bg` | color or `none` | Import keywords (`$text-error`): the token's background, or the code block's for `None`. |
| `syntax.keyword_namespace.style` | text style | Import keywords (`$text-error`): text style added to the token. |
| `syntax.number.fg` | color or `none` | Numbers (`$text-warning`): the token's text color, or the code block's text color for `None`. |
| `syntax.number.bg` | color or `none` | Numbers (`$text-warning`): the token's background, or the code block's for `None`. |
| `syntax.number.style` | text style | Numbers (`$text-warning`): text style added to the token. |
| `syntax.string.fg` | color or `none` | Strings (`$text-success`): the token's text color, or the code block's text color for `None`. |
| `syntax.string.bg` | color or `none` | Strings (`$text-success`): the token's background, or the code block's for `None`. |
| `syntax.string.style` | text style | Strings (`$text-success`): text style added to the token. |
| `syntax.string_backtick.fg` | color or `none` | Backtick strings (`$text 60%`): the token's text color, or the code block's text color for `None`. |
| `syntax.string_backtick.bg` | color or `none` | Backtick strings (`$text 60%`): the token's background, or the code block's for `None`. |
| `syntax.string_backtick.style` | text style | Backtick strings (`$text 60%`): text style added to the token. |
| `syntax.string_doc.fg` | color or `none` | Docstrings and doc comments (italic `$text-success 80%`): the token's text color, or the code block's text color for `None`. |
| `syntax.string_doc.bg` | color or `none` | Docstrings and doc comments (italic `$text-success 80%`): the token's background, or the code block's for `None`. |
| `syntax.string_doc.style` | text style | Docstrings and doc comments (italic `$text-success 80%`): text style added to the token. |
| `syntax.string_double.fg` | color or `none` | Double-quoted strings (`$text-success 90%`): the token's text color, or the code block's text color for `None`. |
| `syntax.string_double.bg` | color or `none` | Double-quoted strings (`$text-success 90%`): the token's background, or the code block's for `None`. |
| `syntax.string_double.style` | text style | Double-quoted strings (`$text-success 90%`): text style added to the token. |
| `syntax.name.fg` | color or `none` | Names (`$text-primary`): the token's text color, or the code block's text color for `None`. |
| `syntax.name.bg` | color or `none` | Names (`$text-primary`): the token's background, or the code block's for `None`. |
| `syntax.name.style` | text style | Names (`$text-primary`): text style added to the token. |
| `syntax.name_attribute.fg` | color or `none` | Attribute names (`$text-warning`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_attribute.bg` | color or `none` | Attribute names (`$text-warning`): the token's background, or the code block's for `None`. |
| `syntax.name_attribute.style` | text style | Attribute names (`$text-warning`): text style added to the token. |
| `syntax.name_builtin.fg` | color or `none` | Built-in names (`$text-accent`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_builtin.bg` | color or `none` | Built-in names (`$text-accent`): the token's background, or the code block's for `None`. |
| `syntax.name_builtin.style` | text style | Built-in names (`$text-accent`): text style added to the token. |
| `syntax.name_builtin_pseudo.fg` | color or `none` | Built-in pseudo names such as `self` (italic): the token's text color, or the code block's text color for `None`. |
| `syntax.name_builtin_pseudo.bg` | color or `none` | Built-in pseudo names such as `self` (italic): the token's background, or the code block's for `None`. |
| `syntax.name_builtin_pseudo.style` | text style | Built-in pseudo names such as `self` (italic): text style added to the token. |
| `syntax.name_class.fg` | color or `none` | Class names (bold `$text-warning`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_class.bg` | color or `none` | Class names (bold `$text-warning`): the token's background, or the code block's for `None`. |
| `syntax.name_class.style` | text style | Class names (bold `$text-warning`): text style added to the token. |
| `syntax.name_constant.fg` | color or `none` | Constant names (`$text-error`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_constant.bg` | color or `none` | Constant names (`$text-error`): the token's background, or the code block's for `None`. |
| `syntax.name_constant.style` | text style | Constant names (`$text-error`): text style added to the token. |
| `syntax.name_decorator.fg` | color or `none` | Decorators (bold `$text-primary`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_decorator.bg` | color or `none` | Decorators (bold `$text-primary`): the token's background, or the code block's for `None`. |
| `syntax.name_decorator.style` | text style | Decorators (bold `$text-primary`): text style added to the token. |
| `syntax.name_function.fg` | color or `none` | Function names (underlined `$text-warning`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_function.bg` | color or `none` | Function names (underlined `$text-warning`): the token's background, or the code block's for `None`. |
| `syntax.name_function.style` | text style | Function names (underlined `$text-warning`): text style added to the token. |
| `syntax.name_tag.fg` | color or `none` | Markup tags (bold `$text-primary`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_tag.bg` | color or `none` | Markup tags (bold `$text-primary`): the token's background, or the code block's for `None`. |
| `syntax.name_tag.style` | text style | Markup tags (bold `$text-primary`): text style added to the token. |
| `syntax.name_variable.fg` | color or `none` | Variable names (`$text-secondary`): the token's text color, or the code block's text color for `None`. |
| `syntax.name_variable.bg` | color or `none` | Variable names (`$text-secondary`): the token's background, or the code block's for `None`. |
| `syntax.name_variable.style` | text style | Variable names (`$text-secondary`): text style added to the token. |
| `syntax.operator.fg` | color or `none` | Operators (bold): the token's text color, or the code block's text color for `None`. |
| `syntax.operator.bg` | color or `none` | Operators (bold): the token's background, or the code block's for `None`. |
| `syntax.operator.style` | text style | Operators (bold): text style added to the token. |
| `syntax.operator_word.fg` | color or `none` | Word operators such as `and` and `not` (bold `$text-error`): the token's text color, or the code block's text color for `None`. |
| `syntax.operator_word.bg` | color or `none` | Word operators such as `and` and `not` (bold `$text-error`): the token's background, or the code block's for `None`. |
| `syntax.operator_word.style` | text style | Word operators such as `and` and `not` (bold `$text-error`): text style added to the token. |
| `syntax.plain.fg` | color or `none` | Text the highlighter knows but leaves plain, so an outer scope doesn't style it: the token's text color, or the code block's text color for `None`. |
| `syntax.plain.bg` | color or `none` | Text the highlighter knows but leaves plain, so an outer scope doesn't style it: the token's background, or the code block's for `None`. |
| `syntax.plain.style` | text style | Text the highlighter knows but leaves plain, so an outer scope doesn't style it: text style added to the token. |

### `[marks]`

State colors by meaning: open, failed, merged, running, …

| Key | Value | What it's for |
| --- | --- | --- |
| `marks.positive` | color | A good state: open, passed, approved, succeeded, public, all notifications read (`greenyellow`). |
| `marks.negative` | color | A bad state: closed unmerged, failed, changes requested, private, unread notifications, a branch that failed to push (`red`). |
| `marks.merged` | color | Done and set aside: a merged pull request, a closed issue, an archived repository (`orchid`). |
| `marks.pending` | color | Waiting or stopped: a pending check, a queued or cancelled run (`yellow`). |
| `marks.running` | color | In progress: a running workflow run (`cyan`). |
| `marks.warning` | color | A warning's lead (`Warning:`) and the name a warning or error is about (`yellow`). |
| `marks.name` | color | The name a message is about: the status bar's current repository, a pushed branch (`greenyellow`). |
| `marks.count_added` | color | A pull request's added-lines count (`greenyellow`). |
| `marks.count_removed` | color | A pull request's removed-lines count (`red`). |
| `marks.auth_border` | color | The authentication screen's box (`red`). |
| `marks.auth_command` | color | The command the authentication screen asks for (`green`). |
| `marks.starred` | color | A starred repository's star (Rich `green`). |
| `marks.job_failed` | color | A failed job in the jobs tree (Rich `red`). |
| `marks.job_pending` | color | A queued or cancelled job in the jobs tree (Rich `yellow`). |
| `marks.job_running` | color | A running job in the jobs tree (Rich `cyan`). |

### `[diff]`

The diff viewer.

| Key | Value | What it's for |
| --- | --- | --- |
| `diff.added_bg` | color | An added line (`#0a3a0a`). |
| `diff.removed_bg` | color | A removed line (`#3a0a0a`). |
| `diff.added_current_bg` | color | The current line, when it's added (`#006400`). |
| `diff.removed_current_bg` | color | The current line, when it's removed (`#8b0000`). |
| `diff.context_current_bg` | color | The current line, when it's context (Rich `yellow`). |
| `diff.stat_added` | color | A file or hunk's added count (Rich `green`). |
| `diff.stat_removed` | color | A file or hunk's removed count (Rich `red`). |
| `diff.indicator.fg` | color | The current-line indicator (`$text` on `$accent`): the text color, drawn over `bg`. |
| `diff.indicator.bg` | color | The current-line indicator (`$text` on `$accent`): the background, drawn over what's under it. |
| `diff.review_header.fg` | color | The review panel's header (bold `$text` on `$accent`): the text color, drawn over `bg`. |
| `diff.review_header.bg` | color | The review panel's header (bold `$text` on `$accent`): the background, drawn over what's under it. |
| `diff.review_header.style` | text style | The review panel's header (bold `$text` on `$accent`): the text style applied on top of the row's own. |
| `diff.review_body` | color | A pending review comment's body (`$panel`). |
| `diff.review_comment_border` | color | A pending review comment's box (`$secondary`). |

### `[command_palette]`

The command palette.

| Key | Value | What it's for |
| --- | --- | --- |
| `command_palette.band` | color | The band behind the input and the hits (`$panel-darken-1`; `$surface` on light themes). |
| `command_palette.input_rule` | color | The rules above and below the input (`black 50%`). |
| `command_palette.list_rule` | color | The rule below the hits (`black`). |
| `command_palette.icon` | color | The search icon (`black`). |
| `command_palette.highlight.fg` | color | The highlighted hit (the blurred block cursor; the focused one on terminal-color themes): the text color, drawn over `bg`. |
| `command_palette.highlight.bg` | color | The highlighted hit (the blurred block cursor; the focused one on terminal-color themes): the background, drawn over what's under it. |
| `command_palette.highlight.style` | text style | The highlighted hit (the blurred block cursor; the focused one on terminal-color themes): the text style applied on top of the row's own. |

### `[hints]`

Link hint labels.

| Key | Value | What it's for |
| --- | --- | --- |
| `hints.label.fg` | color | A label (bold `black` on `$warning`): the text color, drawn over `bg`. |
| `hints.label.bg` | color | A label (bold `black` on `$warning`): the background, drawn over what's under it. |
| `hints.label.style` | text style | A label (bold `black` on `$warning`): the text style applied on top of the row's own. |

### `[table]`

Tables (`DataTable`).

| Key | Value | What it's for |
| --- | --- | --- |
| `table.header.fg` | color | The header row (`$foreground` on `$panel`, tinted with the table while focused; `ansi_default` on `ansi_bright_blue` on terminal-color themes, `&:ansi > .datatable--header`, where the tint has no effect): the text color, drawn over `bg`. |
| `table.header.bg` | color | The header row (`$foreground` on `$panel`, tinted with the table while focused; `ansi_default` on `ansi_bright_blue` on terminal-color themes, `&:ansi > .datatable--header`, where the tint has no effect): the background, drawn over what's under it. |

### `[disabled]`

Disabled fields.

| Key | Value | What it's for |
| --- | --- | --- |
| `disabled.dimmed` | `true` or `false` | Whether a disabled input, switch or select is dimmed (SGR 2) rather than faded toward its parent's background (`opacity: 0.7`): on terminal-color themes, which can't blend. |

### `[screenshot]`

SVG screenshots.

| Key | Value | What it's for |
| --- | --- | --- |
| `screenshot.default_fg` | `#rrggbb` | The terminal's default text color (`#D4D4D4` on dark themes, `#1E1E1E` on light ones). |
| `screenshot.default_bg` | `#rrggbb` | The terminal's default background (`#1E1E1E` on dark themes, `#FFFFFF` on light ones). |
