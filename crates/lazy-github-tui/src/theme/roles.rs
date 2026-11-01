//! Semantic theme roles: what each color is for in this app, rather than which Textual variable it comes from.
//!
//! Every built-in theme's roles are filled from its generated Textual palette by [`Roles::from_palette`], each to
//! exactly what the widgets compute from the palette today, so moving a widget onto roles changes no cell. A role's
//! [`ThemeColor`] may carry alpha: it's composited over whatever it's drawn on ([`ThemeColor::over`]), as palette
//! colors are. Where a Textual CSS rule picks a different color or text style on light or terminal-color themes,
//! the role's value differs per theme, so widgets need not branch on the theme.
//!
//! The marks ([`Marks`]) and the diff line colors are the same in every built-in theme (or the same within dark,
//! light and terminal-color themes), as the Python app writes them as literal markup colors.

use ratatui::style::Modifier;

use super::accent::{Accent, RichAnsi};
use super::color::{AnsiColor, Rgb, TextStyle, ThemeColor, rgb};
use super::palette::{Palette, ThemeDef};

/// Every color and text style the app draws with, by what it's for. One per theme; see [`super::Theme::roles`].
#[derive(Debug, PartialEq)]
pub struct Roles {
    /// Backgrounds: the screen, panes, dialogs.
    pub surface: Surface,
    /// Text colors.
    pub text: Text,
    /// The theme's seed colors, as fills.
    pub brand: Brand,
    /// Status colors, as fills and borders: toasts, invalid inputs, link hints.
    pub status: Status,
    /// Box and rule colors.
    pub border: Border,
    /// The block cursor that highlights the current row of a list, table, tree or tab strip.
    pub cursor: Cursor,
    /// Scrollbars.
    pub scrollbar: Scrollbar,
    /// Text inputs and the text editor.
    pub input: Input,
    /// The key-binding footer.
    pub footer: Footer,
    /// Buttons, by variant.
    pub button: Button,
    /// The on/off switch.
    pub switch: Switch,
    /// The select (drop-down).
    pub select: Select,
    /// The selection list's check boxes.
    pub checkbox: Checkbox,
    /// Tree guide lines.
    pub tree: Tree,
    /// Tab strips.
    pub tabs: Tabs,
    /// Rendered markdown.
    pub markdown: Markdown,
    /// Syntax highlighting in markdown code blocks.
    pub syntax: Syntax,
    /// State colors by meaning: open, failed, merged, running, …
    pub marks: Marks,
    /// The diff viewer.
    pub diff: Diff,
    /// The command palette.
    pub command_palette: CommandPalette,
    /// Link hint labels.
    pub hints: Hints,
}

/// A foreground and background pair: `fg` is drawn over `bg`, and `bg` over what's under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    /// The text color, drawn over `bg`.
    pub fg: ThemeColor,
    /// The background, drawn over what's under it.
    pub bg: ThemeColor,
}

/// A [`Pair`] with a text style, such as a cursor row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    /// The text color, drawn over `bg`.
    pub fg: ThemeColor,
    /// The background, drawn over what's under it.
    pub bg: ThemeColor,
    /// The text style applied on top of the row's own.
    pub style: TextStyle,
}

/// A text color with a text style, such as a link.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    /// The text color.
    pub fg: ThemeColor,
    /// The text style.
    pub style: TextStyle,
}

/// Backgrounds.
#[derive(Debug, PartialEq)]
pub struct Surface {
    /// The screen, which everything else is drawn over (`$background`).
    pub screen: ThemeColor,
    /// A pane's or widget's own background: tables, trees, lists, inputs, switches (`$surface`).
    pub pane: ThemeColor,
    /// A table's header row (`$panel`).
    pub header: ThemeColor,
    /// Toasts, raised over the screen (`$panel-lighten-1`).
    pub raised: ThemeColor,
    /// A dialog's body, unless the dialog says otherwise (`$surface-lighten-3`).
    pub dialog: ThemeColor,
    /// The body of a dialog that shows one record, the user profile (`$surface-lighten-1`).
    pub card: ThemeColor,
    /// A lifted area inside a pane: the loading overlay, a block quote, the text editor's current line (`$boost`).
    pub boost: ThemeColor,
    /// What a modal screen dims the screens below it with, blended over every cell (`$background 60%`;
    /// transparent on terminal-color themes, which can't blend).
    pub scrim: ThemeColor,
    /// The tint a focused pane's background gets (`background-tint: $foreground 5%`), for [`super::Theme::tint`]
    /// at 100%.
    pub focus_tint: ThemeColor,
    /// The hatch pattern over an empty area (`$panel`).
    pub hatch: ThemeColor,
}

/// Text colors.
#[derive(Debug, PartialEq)]
pub struct Text {
    /// Body text with the theme's text contrast (`$text`).
    pub normal: ThemeColor,
    /// A widget's plain text color (`$foreground`).
    pub plain: ThemeColor,
    /// Secondary text: authors, headings of previews (`$text-muted`).
    pub muted: ThemeColor,
    /// Placeholder and unavailable text (`$text-disabled`).
    pub disabled: ThemeColor,
    /// Text in the primary color: help panel keys' descriptions (`$text-primary`).
    pub primary: ThemeColor,
    /// Text in the secondary color (`$text-secondary`).
    pub secondary: ThemeColor,
    /// Text in the accent color: help panel keys (`$text-accent`).
    pub accent: ThemeColor,
    /// Success text: an information toast's title (`$text-success`).
    pub success: ThemeColor,
    /// Warning text: a warning toast's title (`$text-warning`).
    pub warning: ThemeColor,
    /// Error text: an error toast's title (`$text-error`).
    pub error: ThemeColor,
}

/// The theme's seed colors, as fills.
#[derive(Debug, PartialEq)]
pub struct Brand {
    /// The primary color: the loading dots, a markdown table's header, a selected check list item (`$primary`).
    pub primary: ThemeColor,
    /// The secondary color (`$secondary`).
    pub secondary: ThemeColor,
    /// The accent color (`$accent`).
    pub accent: ThemeColor,
}

/// Status colors, as fills and borders.
#[derive(Debug, PartialEq)]
pub struct Status {
    /// An information toast's border, a switch that's on (`$success`).
    pub success: ThemeColor,
    /// A warning toast's border, a link hint's label (`$warning`).
    pub warning: ThemeColor,
    /// An error toast's border, an invalid input's border, a removed file's notice (`$error`).
    pub error: ThemeColor,
}

/// Box and rule colors.
#[derive(Debug, PartialEq)]
pub struct Border {
    /// A focused pane or comment's box (`$success`).
    pub pane_focused: ThemeColor,
    /// An unfocused section's titled box (`$primary-lighten-3`).
    pub section_title: ThemeColor,
    /// A focused input, select, switch or editor's border (`$border`).
    pub field_focused: ThemeColor,
    /// An unfocused field's border, and a disabled button's (`$border-blurred`).
    pub field: ThemeColor,
    /// A horizontal rule: in dialogs, overviews and markdown (`$secondary`).
    pub rule: ThemeColor,
    /// The status bar's box (`$secondary`).
    pub status_bar: ThemeColor,
    /// A dialog's frame, unless the dialog says otherwise (`$background 80%`).
    pub dialog: ThemeColor,
    /// A box that draws attention: the diff viewer's review container, a key being captured in Settings, the
    /// Add Comment dialog's frame (`$accent`).
    pub emphasis: ThemeColor,
    /// An unfocused comment's box and a review thread's left bar in the conversation (`$secondary-background`).
    pub conversation: ThemeColor,
    /// The box around a review in the conversation (`$success-lighten-3`).
    pub review: ThemeColor,
    /// The help panel's left edge (`$foreground 30%`; `ansi_black` on terminal-color themes).
    pub help: ThemeColor,
}

/// The block cursor.
#[derive(Debug, PartialEq)]
pub struct Cursor {
    /// In a focused widget (`$block-cursor-*`).
    pub focused: Highlight,
    /// In an unfocused widget (`$block-cursor-blurred-*`).
    pub blurred: Highlight,
    /// A row under the mouse (`$block-hover-background`).
    pub hover_bg: ThemeColor,
}

/// Scrollbars.
#[derive(Debug, PartialEq)]
pub struct Scrollbar {
    /// The thumb, over the track (`$scrollbar`).
    pub thumb: ThemeColor,
    /// The track (`$scrollbar-background`).
    pub track: ThemeColor,
    /// The corner where two scrollbars meet (`$scrollbar-corner-color`).
    pub corner: ThemeColor,
}

/// Text inputs and the text editor.
#[derive(Debug, PartialEq)]
pub struct Input {
    /// An input's cursor (`$input-cursor-*`; `ansi_black` on `ansi_white` on terminal-color themes).
    pub cursor: Highlight,
    /// The text editor's cursor (`$input-cursor-*`; `$text 90%` on `$foreground 70%` on light themes; reversed on
    /// terminal-color themes).
    pub editor_cursor: Highlight,
    /// Selected text (`$input-selection-*`).
    pub selection: Pair,
    /// A placeholder or suggestion (`$text-disabled`).
    pub placeholder: ThemeColor,
    /// A placeholder's or suggestion's text style (dim on terminal-color themes).
    pub placeholder_style: TextStyle,
    /// A focused invalid input's border (`$error`).
    pub invalid: ThemeColor,
    /// An unfocused invalid input's border (`$error 60%`).
    pub invalid_blurred: ThemeColor,
}

/// The key-binding footer.
#[derive(Debug, PartialEq)]
pub struct Footer {
    /// The bar (`$footer-background`).
    pub bar: ThemeColor,
    /// A binding's background, over the bar (`$footer-item-background`).
    pub item: ThemeColor,
    /// A binding under the mouse, over the bar (`$block-hover-background`).
    pub hover: ThemeColor,
    /// A binding's key, over its item (`$footer-key-*`).
    pub key: Pair,
    /// A binding's description, over its item (`$footer-description-*`).
    pub description: Pair,
}

/// Buttons.
#[derive(Debug, PartialEq)]
pub struct Button {
    /// Whether buttons are drawn outlined in the terminal's own colors (`Button:ansi`): the terminal's background, a
    /// `tall` border in the variant's [`ButtonColors::border`] on all four sides, and a bold label.
    pub outlined: bool,
    /// The label's style while focused (`$button-focus-text-style`).
    pub focus_style: TextStyle,
    /// A disabled outlined button's border (`$border-blurred`).
    pub disabled_border: ThemeColor,
    /// The default button.
    pub default: ButtonColors,
    /// The primary button.
    pub primary: ButtonColors,
    /// The success (submit) button.
    pub success: ButtonColors,
    /// The warning button.
    pub warning: ButtonColors,
    /// The error (cancel) button.
    pub error: ButtonColors,
}

/// One button variant's colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonColors {
    /// The fill (`$surface`, `$primary`, …).
    pub bg: ThemeColor,
    /// The top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default).
    pub light: ThemeColor,
    /// The bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default).
    pub dark: ThemeColor,
    /// An outlined button's border (`$<variant>`; `$border-blurred` for default).
    pub border: ThemeColor,
    /// The label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined).
    pub label: ThemeColor,
    /// A disabled button's label (the label at `opacity: 0.6`).
    pub label_disabled: ThemeColor,
}

/// The on/off switch.
#[derive(Debug, PartialEq)]
pub struct Switch {
    /// The track the slider moves in (`$panel-darken-2`).
    pub track: ThemeColor,
    /// The slider while on (`$success`).
    pub on: ThemeColor,
    /// The slider while off (`$panel`; `$primary 15%` on light themes; `ansi_red` on the terminal-color dark theme).
    pub off: ThemeColor,
}

/// The select (drop-down).
#[derive(Debug, PartialEq)]
pub struct Select {
    /// The closed select's prompt while nothing is chosen (`$foreground 50%`).
    pub muted: ThemeColor,
}

/// The selection list's check boxes.
#[derive(Debug, PartialEq)]
pub struct Checkbox {
    /// The box (`$panel`).
    pub bg: ThemeColor,
    /// The mark while unchecked, over the box (`$panel-darken-2`).
    pub off: ThemeColor,
    /// The mark while checked, over the box (`$text-success`).
    pub on: ThemeColor,
}

/// Tree guide lines.
#[derive(Debug, PartialEq)]
pub struct Tree {
    /// In an unfocused tree (`$surface-lighten-2`; `$surface-darken-1` on light themes; `$ansi-background` on
    /// terminal-color themes).
    pub guide: ThemeColor,
    /// In a focused tree (`$surface-lighten-3`, and as [`Self::guide`] on light and terminal-color themes).
    pub guide_focused: ThemeColor,
    /// The cursor row's guides (`$block-cursor-background`).
    pub guide_selected: ThemeColor,
}

/// Tab strips.
#[derive(Debug, PartialEq)]
pub struct Tabs {
    /// An inactive tab's label (`$foreground 50%`).
    pub label: ThemeColor,
    /// An inactive tab's label style (dim on terminal-color themes).
    pub label_style: TextStyle,
    /// The active tab's label in an unfocused strip (`$foreground`).
    pub label_active: ThemeColor,
    /// The active tab's label style in an unfocused strip (bold on terminal-color themes).
    pub label_active_style: TextStyle,
    /// The underline bar of an unfocused strip (`$foreground 10%`; `$border-blurred` on terminal-color themes).
    pub bar: ThemeColor,
    /// The underline bar of a focused strip (`$foreground 30%`; `$border-blurred` on terminal-color themes).
    pub bar_focused: ThemeColor,
    /// The bar under the active tab (`$block-cursor-background`).
    pub bar_active: ThemeColor,
}

/// Rendered markdown. Dialog titles are drawn as [`Self::h1`].
#[derive(Debug, PartialEq)]
pub struct Markdown {
    /// A level-1 heading, and a dialog's title (`$markdown-h1-*`).
    pub h1: Highlight,
    /// A level-2 heading (`$markdown-h2-*`).
    pub h2: Highlight,
    /// A level-3 heading (`$markdown-h3-*`).
    pub h3: Highlight,
    /// A level-4 heading (`$markdown-h4-*`).
    pub h4: Highlight,
    /// A level-5 heading (`$markdown-h5-*`).
    pub h5: Highlight,
    /// A level-6 heading (`$markdown-h6-*`).
    pub h6: Highlight,
    /// A link (`$link-color`, `$link-style`).
    pub link: Ink,
    /// A code block's background (`black 10%`; `white 30%` on light themes; transparent on terminal-color themes).
    pub fence_bg: ThemeColor,
    /// Whether a code block has its margin above and below, its side padding and a filled background
    /// (`MarkdownFence`; not on terminal-color themes, `MarkdownFence:ansi`).
    pub fence_inset: bool,
    /// Inline code's text (`$text-warning 95%`; `$text-error 95%` on light themes).
    pub code: ThemeColor,
    /// Inline code's background, drawn as a component background ([`ThemeColor::component_background`]):
    /// `$warning 10%`, `$error 5%` on light themes. Transparent on terminal-color themes, where the run keeps what's
    /// under it.
    pub code_bg: ThemeColor,
    /// A block quote's background (`$boost`).
    pub quote_bg: ThemeColor,
    /// A block quote's left bar (`$text-primary 50%`; `$text-secondary` on light themes).
    pub quote_bar: ThemeColor,
    /// A list's bullets and numbers (`$text-primary`; `$text-secondary` on light themes).
    pub bullet: ThemeColor,
    /// A table's background (transparent; `white 30%` on light themes).
    pub table_bg: ThemeColor,
    /// A table's header text (`$primary`).
    pub table_header: ThemeColor,
    /// A table's keylines (`$foreground 20%`).
    pub table_line: ThemeColor,
}

/// How one kind of syntax token looks: its colors replace the code block's, and its text style is added.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TokenStyle {
    /// The token's text color, or the code block's text color for `None`.
    pub fg: Option<ThemeColor>,
    /// The token's background, or the code block's for `None`.
    pub bg: Option<ThemeColor>,
    /// Text style added to the token.
    pub style: TextStyle,
}

/// Syntax highlighting in markdown code blocks: Textual's `HighlightTheme`, and its ANSI dark and light variants on
/// terminal-color themes. One role per token kind the highlighter maps.
#[derive(Debug, PartialEq)]
pub struct Syntax {
    /// A code block's text, under every token (`$text` at full strength).
    pub text: ThemeColor,
    /// Comments (`$text 60%`).
    pub comment: TokenStyle,
    /// Text the lexer couldn't read (`$text-error` on `$error-muted`).
    pub error: TokenStyle,
    /// Strong text in markup languages (bold).
    pub generic_strong: TokenStyle,
    /// Emphasized text in markup languages (italic).
    pub generic_emph: TokenStyle,
    /// Headings in markup languages (underlined `$text-primary`).
    pub generic_heading: TokenStyle,
    /// Subheadings in markup languages (`$text-primary`).
    pub generic_subheading: TokenStyle,
    /// Keywords (`$text-accent`).
    pub keyword: TokenStyle,
    /// Constant keywords such as `true` and `None` (bold `$text-success 80%`).
    pub keyword_constant: TokenStyle,
    /// Import keywords (`$text-error`).
    pub keyword_namespace: TokenStyle,
    /// Numbers (`$text-warning`).
    pub number: TokenStyle,
    /// Strings (`$text-success`).
    pub string: TokenStyle,
    /// Backtick strings (`$text 60%`).
    pub string_backtick: TokenStyle,
    /// Docstrings and doc comments (italic `$text-success 80%`).
    pub string_doc: TokenStyle,
    /// Double-quoted strings (`$text-success 90%`).
    pub string_double: TokenStyle,
    /// Names (`$text-primary`).
    pub name: TokenStyle,
    /// Attribute names (`$text-warning`).
    pub name_attribute: TokenStyle,
    /// Built-in names (`$text-accent`).
    pub name_builtin: TokenStyle,
    /// Built-in pseudo names such as `self` (italic).
    pub name_builtin_pseudo: TokenStyle,
    /// Class names (bold `$text-warning`).
    pub name_class: TokenStyle,
    /// Constant names (`$text-error`).
    pub name_constant: TokenStyle,
    /// Decorators (bold `$text-primary`).
    pub name_decorator: TokenStyle,
    /// Function names (underlined `$text-warning`).
    pub name_function: TokenStyle,
    /// Markup tags (bold `$text-primary`).
    pub name_tag: TokenStyle,
    /// Variable names (`$text-secondary`).
    pub name_variable: TokenStyle,
    /// Operators (bold).
    pub operator: TokenStyle,
    /// Word operators such as `and` and `not` (bold `$text-error`).
    pub operator_word: TokenStyle,
    /// Text the highlighter knows but leaves plain, so an outer scope doesn't style it.
    pub plain: TokenStyle,
}

/// State colors by meaning. Each is drawn as is, over anything.
#[derive(Debug, PartialEq)]
pub struct Marks {
    /// A good state: open, passed, approved, succeeded, public, all notifications read (`greenyellow`).
    pub positive: ThemeColor,
    /// A bad state: closed unmerged, failed, changes requested, private, unread notifications, a branch that
    /// failed to push (`red`).
    pub negative: ThemeColor,
    /// Done and set aside: a merged pull request, a closed issue, an archived repository (`orchid`).
    pub merged: ThemeColor,
    /// Waiting or stopped: a pending check, a queued or cancelled run (`yellow`).
    pub pending: ThemeColor,
    /// In progress: a running workflow run (`cyan`).
    pub running: ThemeColor,
    /// A warning's lead (`Warning:`) and the name a warning or error is about (`yellow`).
    pub warning: ThemeColor,
    /// The name a message is about: the status bar's current repository, a pushed branch (`greenyellow`).
    pub name: ThemeColor,
    /// A pull request's added-lines count (`greenyellow`).
    pub count_added: ThemeColor,
    /// A pull request's removed-lines count (`red`).
    pub count_removed: ThemeColor,
    /// The authentication screen's box (`red`).
    pub auth_border: ThemeColor,
    /// The command the authentication screen asks for (`green`).
    pub auth_command: ThemeColor,
    /// A starred repository's star (Rich `green`).
    pub starred: ThemeColor,
    /// A failed job in the jobs tree (Rich `red`).
    pub job_failed: ThemeColor,
    /// A queued or cancelled job in the jobs tree (Rich `yellow`).
    pub job_pending: ThemeColor,
    /// A running job in the jobs tree (Rich `cyan`).
    pub job_running: ThemeColor,
}

/// The diff viewer.
#[derive(Debug, PartialEq)]
pub struct Diff {
    /// An added line (`#0a3a0a`).
    pub added_bg: ThemeColor,
    /// A removed line (`#3a0a0a`).
    pub removed_bg: ThemeColor,
    /// The current line, when it's added (`#006400`).
    pub added_current_bg: ThemeColor,
    /// The current line, when it's removed (`#8b0000`).
    pub removed_current_bg: ThemeColor,
    /// The current line, when it's context (Rich `yellow`).
    pub context_current_bg: ThemeColor,
    /// A file or hunk's added count (Rich `green`).
    pub stat_added: ThemeColor,
    /// A file or hunk's removed count (Rich `red`).
    pub stat_removed: ThemeColor,
    /// A focused hunk pane's border (`$accent`).
    pub border_focused: ThemeColor,
    /// An unfocused hunk pane's border (`$primary-lighten-1`).
    pub border: ThemeColor,
    /// The current-line indicator (`$text` on `$accent`).
    pub indicator: Pair,
    /// The review panel's header (bold `$text` on `$accent`).
    pub review_header: Highlight,
    /// A pending review comment's body (`$panel`).
    pub review_body: ThemeColor,
}

/// The command palette.
#[derive(Debug, PartialEq)]
pub struct CommandPalette {
    /// The band behind the input and the hits (`$panel-darken-1`; `$surface` on light themes).
    pub band: ThemeColor,
    /// The rules above and below the input (`black 50%`).
    pub input_rule: ThemeColor,
    /// The rule below the hits (`black`).
    pub list_rule: ThemeColor,
    /// The search icon (`black`).
    pub icon: ThemeColor,
    /// The highlighted hit (the blurred block cursor; the focused one on terminal-color themes).
    pub highlight: Highlight,
}

/// Link hint labels.
#[derive(Debug, PartialEq)]
pub struct Hints {
    /// A label (bold `black` on `$warning`).
    pub label: Highlight,
}

const NO_STYLE: TextStyle = TextStyle {
    add: Modifier::empty(),
    sub: Modifier::empty(),
};

const fn add(add: Modifier) -> TextStyle {
    TextStyle {
        add,
        sub: Modifier::empty(),
    }
}

const fn opaque(c: Rgb) -> ThemeColor {
    rgb(c.r, c.g, c.b)
}

/// A built-in theme's palette, and the two facts Textual's per-theme CSS rules select on.
struct Source<'a> {
    p: &'a Palette,
    dark: bool,
    ansi: bool,
}

impl Source<'_> {
    /// Picks by Textual's `:ansi`, `:dark` and `:light` rules, the terminal-color themes first.
    fn pick(&self, on_ansi: ThemeColor, on_dark: ThemeColor, on_light: ThemeColor) -> ThemeColor {
        if self.ansi {
            on_ansi
        } else if self.dark {
            on_dark
        } else {
            on_light
        }
    }

    /// A text style only terminal-color themes add.
    fn on_ansi(&self, m: Modifier) -> TextStyle {
        if self.ansi { add(m) } else { NO_STYLE }
    }

    /// A Rich ANSI color: the terminal's own on terminal-color themes, Textual's terminal theme (MONOKAI or
    /// ALABASTER) otherwise.
    fn rich(&self, c: RichAnsi) -> ThemeColor {
        if self.ansi {
            ThemeColor::Ansi(match c {
                RichAnsi::Green => AnsiColor::Green,
                RichAnsi::Red => AnsiColor::Red,
                RichAnsi::Yellow => AnsiColor::Yellow,
                RichAnsi::Cyan => AnsiColor::Cyan,
            })
        } else if self.dark {
            opaque(c.dark())
        } else {
            opaque(c.light())
        }
    }

    fn surface(&self) -> Surface {
        let p = self.p;
        Surface {
            screen: p.background,
            pane: p.surface,
            header: p.panel,
            raised: p.panel_lighten_1,
            dialog: p.surface_lighten_3,
            card: p.surface_lighten_1,
            boost: p.boost,
            scrim: self.pick(
                ThemeColor::Transparent,
                p.background.multiply_alpha(0.6),
                p.background.multiply_alpha(0.6),
            ),
            focus_tint: p.foreground.multiply_alpha(0.05),
            hatch: p.panel,
        }
    }

    fn text(&self) -> Text {
        let p = self.p;
        Text {
            normal: p.text,
            plain: p.foreground,
            muted: p.text_muted,
            disabled: p.text_disabled,
            primary: p.text_primary,
            secondary: p.text_secondary,
            accent: p.text_accent,
            success: p.text_success,
            warning: p.text_warning,
            error: p.text_error,
        }
    }

    fn border(&self) -> Border {
        let p = self.p;
        let help = p.foreground.multiply_alpha(0.3);
        Border {
            pane_focused: p.success,
            section_title: p.primary_lighten_3,
            field_focused: p.border,
            field: p.border_blurred,
            rule: p.secondary,
            status_bar: p.secondary,
            dialog: p.background.multiply_alpha(0.8),
            emphasis: p.accent,
            conversation: p.secondary_background,
            review: p.success_lighten_3,
            help: self.pick(ThemeColor::Ansi(AnsiColor::Black), help, help),
        }
    }

    fn input(&self) -> Input {
        let p = self.p;
        let cursor = Highlight {
            fg: p.input_cursor_foreground,
            bg: p.input_cursor_background,
            style: p.input_cursor_text_style,
        };
        Input {
            cursor: if self.ansi {
                Highlight {
                    fg: ThemeColor::Ansi(AnsiColor::Black),
                    bg: ThemeColor::Ansi(AnsiColor::White),
                    style: NO_STYLE,
                }
            } else {
                cursor
            },
            editor_cursor: if self.ansi {
                Highlight {
                    style: add(Modifier::REVERSED),
                    ..cursor
                }
            } else if self.dark {
                cursor
            } else {
                Highlight {
                    fg: p.text.multiply_alpha(0.9),
                    bg: p.foreground.multiply_alpha(0.7),
                    style: p.input_cursor_text_style,
                }
            },
            selection: Pair {
                fg: p.input_selection_foreground,
                bg: p.input_selection_background,
            },
            placeholder: p.text_disabled,
            placeholder_style: self.on_ansi(Modifier::DIM),
            invalid: p.error,
            invalid_blurred: p.error.multiply_alpha(0.6),
        }
    }

    fn footer(&self) -> Footer {
        let p = self.p;
        Footer {
            bar: p.footer_background,
            item: p.footer_item_background,
            hover: p.block_hover_background,
            key: Pair {
                fg: p.footer_key_foreground,
                bg: p.footer_key_background,
            },
            description: Pair {
                fg: p.footer_description_foreground,
                bg: p.footer_description_background,
            },
        }
    }

    fn button(&self) -> Button {
        let p = self.p;
        let colored = |bg: ThemeColor, light, dark| {
            let label = if self.ansi { bg } else { p.button_color_foreground };
            ButtonColors {
                bg,
                light,
                dark,
                border: bg,
                label,
                label_disabled: label.multiply_alpha(BUTTON_DISABLED_OPACITY),
            }
        };
        Button {
            outlined: self.ansi,
            focus_style: p.button_focus_text_style,
            disabled_border: p.border_blurred,
            default: ButtonColors {
                bg: p.surface,
                light: p.surface_lighten_1,
                dark: p.surface_darken_1,
                border: p.border_blurred,
                label: p.button_foreground,
                label_disabled: p.button_foreground.multiply_alpha(BUTTON_DISABLED_OPACITY),
            },
            primary: colored(p.primary, p.primary_lighten_3, p.primary_darken_3),
            success: colored(p.success, p.success_lighten_2, p.success_darken_3),
            warning: colored(p.warning, p.warning_lighten_2, p.warning_darken_3),
            error: colored(p.error, p.error_lighten_2, p.error_darken_3),
        }
    }

    fn switch(&self) -> Switch {
        let p = self.p;
        Switch {
            track: p.panel_darken_2,
            on: p.success,
            // `:light` comes after `:ansi`, so the terminal-color light theme takes the light rule.
            off: if !self.dark {
                p.primary.multiply_alpha(0.15)
            } else if self.ansi {
                ThemeColor::Ansi(AnsiColor::Red)
            } else {
                p.panel
            },
        }
    }

    fn tabs(&self) -> Tabs {
        let p = self.p;
        let bar = |alpha: f64| {
            let on_rgb = p.foreground.multiply_alpha(alpha);
            self.pick(p.border_blurred, on_rgb, on_rgb)
        };
        Tabs {
            label: p.foreground.multiply_alpha(0.5),
            label_style: self.on_ansi(Modifier::DIM),
            label_active: p.foreground,
            label_active_style: self.on_ansi(Modifier::BOLD),
            bar: bar(0.1),
            bar_focused: bar(0.3),
            bar_active: p.block_cursor_background,
        }
    }

    fn markdown(&self) -> Markdown {
        let p = self.p;
        let heading = |fg, bg, style| Highlight { fg, bg, style };
        let dark = self.dark;
        Markdown {
            h1: heading(p.markdown_h1_color, p.markdown_h1_background, p.markdown_h1_text_style),
            h2: heading(p.markdown_h2_color, p.markdown_h2_background, p.markdown_h2_text_style),
            h3: heading(p.markdown_h3_color, p.markdown_h3_background, p.markdown_h3_text_style),
            h4: heading(p.markdown_h4_color, p.markdown_h4_background, p.markdown_h4_text_style),
            h5: heading(p.markdown_h5_color, p.markdown_h5_background, p.markdown_h5_text_style),
            h6: heading(p.markdown_h6_color, p.markdown_h6_background, p.markdown_h6_text_style),
            link: Ink {
                fg: p.link_color,
                style: p.link_style,
            },
            fence_bg: self.pick(ThemeColor::Transparent, BLACK_10, WHITE_30),
            fence_inset: !self.ansi,
            code: if dark { p.text_warning } else { p.text_error }.multiply_alpha(0.95),
            code_bg: self.pick(
                ThemeColor::Transparent,
                p.warning.multiply_alpha(0.1),
                p.error.multiply_alpha(0.05),
            ),
            quote_bg: p.boost,
            quote_bar: if dark {
                p.text_primary.multiply_alpha(0.5)
            } else {
                p.text_secondary
            },
            bullet: if dark { p.text_primary } else { p.text_secondary },
            // `MarkdownTable:light`, which the terminal-color light theme takes too.
            table_bg: if dark { ThemeColor::Transparent } else { WHITE_30 },
            table_header: p.primary,
            table_line: p.foreground.multiply_alpha(0.2),
        }
    }

    fn marks(&self) -> Marks {
        let fixed = |a: Accent| opaque(a.rgb());
        Marks {
            positive: fixed(Accent::GreenYellow),
            negative: fixed(Accent::Red),
            merged: fixed(Accent::Orchid),
            pending: fixed(Accent::Yellow),
            running: fixed(Accent::Cyan),
            warning: fixed(Accent::Yellow),
            name: fixed(Accent::GreenYellow),
            count_added: fixed(Accent::GreenYellow),
            count_removed: fixed(Accent::Red),
            auth_border: fixed(Accent::Red),
            auth_command: fixed(Accent::Green),
            starred: self.rich(RichAnsi::Green),
            job_failed: self.rich(RichAnsi::Red),
            job_pending: self.rich(RichAnsi::Yellow),
            job_running: self.rich(RichAnsi::Cyan),
        }
    }

    fn diff(&self) -> Diff {
        let p = self.p;
        let fixed = |a: Accent| opaque(a.rgb());
        Diff {
            added_bg: fixed(Accent::DiffAdded),
            removed_bg: fixed(Accent::DiffRemoved),
            added_current_bg: fixed(Accent::DiffAddedCurrent),
            removed_current_bg: fixed(Accent::DiffRemovedCurrent),
            context_current_bg: self.rich(RichAnsi::Yellow),
            stat_added: self.rich(RichAnsi::Green),
            stat_removed: self.rich(RichAnsi::Red),
            border_focused: p.accent,
            border: p.primary_lighten_1,
            indicator: Pair {
                fg: p.text,
                bg: p.accent,
            },
            review_header: Highlight {
                fg: p.text,
                bg: p.accent,
                style: add(Modifier::BOLD),
            },
            review_body: p.panel,
        }
    }
}

impl Roles {
    /// The roles for a built-in theme: each exactly what the widgets compute from its palette today, Textual's
    /// per-theme CSS rules included.
    pub(super) fn from_palette(def: &ThemeDef) -> Roles {
        let s = Source {
            p: &def.palette,
            dark: def.dark,
            ansi: def.ansi,
        };
        let p = s.p;
        Roles {
            surface: s.surface(),
            text: s.text(),
            brand: Brand {
                primary: p.primary,
                secondary: p.secondary,
                accent: p.accent,
            },
            status: Status {
                success: p.success,
                warning: p.warning,
                error: p.error,
            },
            border: s.border(),
            cursor: Cursor {
                focused: focused_cursor(p),
                blurred: blurred_cursor(p),
                hover_bg: p.block_hover_background,
            },
            scrollbar: Scrollbar {
                thumb: p.scrollbar,
                track: p.scrollbar_background,
                corner: p.scrollbar_corner_color,
            },
            input: s.input(),
            footer: s.footer(),
            button: s.button(),
            switch: s.switch(),
            select: Select {
                muted: p.foreground.multiply_alpha(0.5),
            },
            checkbox: Checkbox {
                bg: p.panel,
                off: p.panel_darken_2,
                on: p.text_success,
            },
            tree: Tree {
                guide: s.pick(p.ansi_background, p.surface_lighten_2, p.surface_darken_1),
                guide_focused: s.pick(p.ansi_background, p.surface_lighten_3, p.surface_darken_1),
                guide_selected: p.block_cursor_background,
            },
            tabs: s.tabs(),
            markdown: s.markdown(),
            syntax: if s.ansi {
                ansi_syntax(!s.dark)
            } else {
                textual_syntax(p)
            },
            marks: s.marks(),
            diff: s.diff(),
            command_palette: CommandPalette {
                band: if s.dark { p.panel_darken_1 } else { p.surface },
                input_rule: ThemeColor::BLACK.multiply_alpha(0.5),
                list_rule: ThemeColor::BLACK,
                icon: ThemeColor::BLACK,
                // `CommandList:ansi` highlights with the focused cursor.
                highlight: if s.ansi { focused_cursor(p) } else { blurred_cursor(p) },
            },
            hints: Hints {
                label: Highlight {
                    fg: ThemeColor::BLACK,
                    bg: p.warning,
                    style: add(Modifier::BOLD),
                },
            },
        }
    }

    /// Which role an [`Accent`] stands for, while callers still name accents (until #377).
    pub(super) fn accent(&self, a: Accent) -> ThemeColor {
        match a {
            Accent::GreenYellow => self.marks.positive,
            Accent::Red => self.marks.negative,
            Accent::Orchid => self.marks.merged,
            Accent::Yellow => self.marks.pending,
            Accent::Cyan => self.marks.running,
            Accent::Green => self.marks.auth_command,
            Accent::DiffRemoved => self.diff.removed_bg,
            Accent::DiffAdded => self.diff.added_bg,
            Accent::DiffRemovedCurrent => self.diff.removed_current_bg,
            Accent::DiffAddedCurrent => self.diff.added_current_bg,
        }
    }

    /// Which role a [`RichAnsi`] color stands for, while callers still name them (until #377).
    pub(super) fn rich_ansi(&self, c: RichAnsi) -> ThemeColor {
        match c {
            RichAnsi::Green => self.marks.starred,
            RichAnsi::Red => self.marks.job_failed,
            RichAnsi::Yellow => self.marks.job_pending,
            RichAnsi::Cyan => self.marks.job_running,
        }
    }
}

/// A disabled button's `opacity`, on its label.
const BUTTON_DISABLED_OPACITY: f64 = 0.6;

/// `black 10%`, a code block's background on dark themes.
const BLACK_10: ThemeColor = ThemeColor::Rgba {
    r: 0,
    g: 0,
    b: 0,
    a: 0.1,
};

/// `white 30%`, a code block's and a table's background on light themes.
const WHITE_30: ThemeColor = ThemeColor::Rgba {
    r: 255,
    g: 255,
    b: 255,
    a: 0.3,
};

fn focused_cursor(p: &Palette) -> Highlight {
    Highlight {
        fg: p.block_cursor_foreground,
        bg: p.block_cursor_background,
        style: p.block_cursor_text_style,
    }
}

fn blurred_cursor(p: &Palette) -> Highlight {
    Highlight {
        fg: p.block_cursor_blurred_foreground,
        bg: p.block_cursor_blurred_background,
        style: p.block_cursor_blurred_text_style,
    }
}

/// `$text` as the highlighter resolves it: an `auto` color at full strength, whatever the theme's alpha.
fn full_text(text: ThemeColor) -> ThemeColor {
    match text {
        ThemeColor::Auto { .. } => ThemeColor::Auto { alpha: 1.0 },
        other => other,
    }
}

const fn token(fg: Option<ThemeColor>, m: Modifier) -> TokenStyle {
    TokenStyle {
        fg,
        bg: None,
        style: add(m),
    }
}

const NONE: Modifier = Modifier::empty();

/// `HighlightTheme.STYLES`. `Token.String` and `Token.Literal.String` are one type in Pygments, so the dict's later
/// `Token.String: "$text-success"` is the one that applies, not `"$text-success 90%"`.
fn textual_syntax(p: &Palette) -> Syntax {
    let text = full_text(p.text);
    let var = |c: ThemeColor| Some(c);
    let at = |c: ThemeColor, alpha: f64| Some(c.multiply_alpha(alpha));
    let comment = token(at(text, 0.6), NONE);
    let primary = token(var(p.text_primary), NONE);
    let strong = token(None, Modifier::BOLD);
    let emph = token(None, Modifier::ITALIC);
    let accent = token(var(p.text_accent), NONE);
    let error = token(var(p.text_error), NONE);
    let warning = token(var(p.text_warning), NONE);
    let decorator = token(var(p.text_primary), Modifier::BOLD);
    Syntax {
        text,
        comment,
        error: TokenStyle {
            fg: var(p.text_error),
            bg: var(p.error_muted),
            style: NO_STYLE,
        },
        generic_strong: strong,
        generic_emph: emph,
        generic_heading: token(var(p.text_primary), Modifier::UNDERLINED),
        generic_subheading: primary,
        keyword: accent,
        keyword_constant: token(at(p.text_success, 0.8), Modifier::BOLD),
        keyword_namespace: error,
        number: warning,
        string: token(var(p.text_success), NONE),
        string_backtick: comment,
        string_doc: token(at(p.text_success, 0.8), Modifier::ITALIC),
        string_double: token(at(p.text_success, 0.9), NONE),
        name: primary,
        name_attribute: warning,
        name_builtin: accent,
        name_builtin_pseudo: emph,
        name_class: token(var(p.text_warning), Modifier::BOLD),
        name_constant: error,
        name_decorator: decorator,
        name_function: token(var(p.text_warning), Modifier::UNDERLINED),
        name_tag: decorator,
        name_variable: token(var(p.text_secondary), NONE),
        operator: strong,
        operator_word: token(var(p.text_error), Modifier::BOLD),
        plain: token(None, NONE),
    }
}

/// `ANSIDarkHighlightTheme.STYLES` (`light` false) and `ANSILightHighlightTheme.STYLES`. Their misspelled colors
/// (`ansi_greenb` for strings, `ansi_yelllow`, `ansi_blow`, and the light theme's `ans_red`) don't parse, so Textual
/// draws those tokens unstyled, and so do we.
fn ansi_syntax(light: bool) -> Syntax {
    use AnsiColor::{Blue, BrightBlack, Cyan, Default, Green, Magenta, Red, Yellow};
    let ink = |c| Some(ThemeColor::Ansi(c));
    let plain = token(None, NONE);
    let red = token(ink(Red), NONE);
    let blue = token(ink(Blue), NONE);
    let cyan = token(ink(Cyan), NONE);
    let magenta = token(ink(Magenta), NONE);
    let default = token(ink(Default), NONE);
    let decorator = token(ink(Blue), Modifier::BOLD);
    let number = if light {
        token(ink(Blue), Modifier::BOLD)
    } else {
        token(ink(Yellow), NONE)
    };
    Syntax {
        text: ThemeColor::Ansi(Default),
        comment: token(None, Modifier::DIM | Modifier::ITALIC),
        error: if light { plain } else { red },
        generic_strong: token(None, Modifier::BOLD),
        generic_emph: token(None, Modifier::ITALIC),
        generic_heading: token(ink(Blue), Modifier::UNDERLINED),
        generic_subheading: blue,
        keyword: token(ink(Magenta), Modifier::BOLD),
        keyword_constant: cyan,
        keyword_namespace: magenta,
        number,
        string: plain,
        string_backtick: token(ink(BrightBlack), NONE),
        string_doc: token(ink(Green), Modifier::ITALIC),
        string_double: token(ink(Green), NONE),
        name: default,
        name_attribute: plain,
        name_builtin: cyan,
        name_builtin_pseudo: token(None, Modifier::ITALIC),
        name_class: number,
        name_constant: red,
        name_decorator: decorator,
        name_function: blue,
        name_tag: decorator,
        name_variable: default,
        operator: default,
        operator_word: magenta,
        plain,
    }
}

/// Every role as a `(path, value)` leaf, for the snapshots.
#[cfg(test)]
pub(super) mod walk {
    use super::*;

    /// One role's value.
    #[derive(Clone, Copy, Debug)]
    pub(in crate::theme) enum Leaf {
        Color(ThemeColor),
        MaybeColor(Option<ThemeColor>),
        Style(TextStyle),
        Flag(bool),
    }

    pub(in crate::theme) trait Walk {
        fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>);
    }

    impl Walk for ThemeColor {
        fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>) {
            out.push((path.to_owned(), Leaf::Color(*self)));
        }
    }

    impl Walk for Option<ThemeColor> {
        fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>) {
            out.push((path.to_owned(), Leaf::MaybeColor(*self)));
        }
    }

    impl Walk for TextStyle {
        fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>) {
            out.push((path.to_owned(), Leaf::Style(*self)));
        }
    }

    impl Walk for bool {
        fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>) {
            out.push((path.to_owned(), Leaf::Flag(*self)));
        }
    }

    macro_rules! walk_fields {
        ($($ty:ident { $($field:ident),* $(,)? })*) => {$(
            impl Walk for $ty {
                fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>) {
                    // Destructured, so a role left out here doesn't compile.
                    let $ty { $($field),* } = self;
                    $(
                        let child = if path.is_empty() {
                            stringify!($field).to_owned()
                        } else {
                            format!("{path}.{}", stringify!($field))
                        };
                        $field.walk(&child, out);
                    )*
                }
            }
        )*};
    }

    walk_fields! {
        Roles {
            surface, text, brand, status, border, cursor, scrollbar, input, footer, button, switch, select,
            checkbox, tree, tabs, markdown, syntax, marks, diff, command_palette, hints,
        }
        Pair { fg, bg }
        Highlight { fg, bg, style }
        Ink { fg, style }
        Surface { screen, pane, header, raised, dialog, card, boost, scrim, focus_tint, hatch }
        Text { normal, plain, muted, disabled, primary, secondary, accent, success, warning, error }
        Brand { primary, secondary, accent }
        Status { success, warning, error }
        Border {
            pane_focused, section_title, field_focused, field, rule, status_bar, dialog, emphasis, conversation,
            review, help,
        }
        Cursor { focused, blurred, hover_bg }
        Scrollbar { thumb, track, corner }
        Input { cursor, editor_cursor, selection, placeholder, placeholder_style, invalid, invalid_blurred }
        Footer { bar, item, hover, key, description }
        Button { outlined, focus_style, disabled_border, default, primary, success, warning, error }
        ButtonColors { bg, light, dark, border, label, label_disabled }
        Switch { track, on, off }
        Select { muted }
        Checkbox { bg, off, on }
        Tree { guide, guide_focused, guide_selected }
        Tabs { label, label_style, label_active, label_active_style, bar, bar_focused, bar_active }
        Markdown {
            h1, h2, h3, h4, h5, h6, link, fence_bg, fence_inset, code, code_bg, quote_bg, quote_bar, bullet,
            table_bg, table_header, table_line,
        }
        TokenStyle { fg, bg, style }
        Syntax {
            text, comment, error, generic_strong, generic_emph, generic_heading, generic_subheading, keyword,
            keyword_constant, keyword_namespace, number, string, string_backtick, string_doc, string_double, name,
            name_attribute, name_builtin, name_builtin_pseudo, name_class, name_constant, name_decorator,
            name_function, name_tag, name_variable, operator, operator_word, plain,
        }
        Marks {
            positive, negative, merged, pending, running, warning, name, count_added, count_removed, auth_border,
            auth_command, starred, job_failed, job_pending, job_running,
        }
        Diff {
            added_bg, removed_bg, added_current_bg, removed_current_bg, context_current_bg, stat_added,
            stat_removed, border_focused, border, indicator, review_header, review_body,
        }
        CommandPalette { band, input_rule, list_rule, icon, highlight }
        Hints { label }
    }
}
