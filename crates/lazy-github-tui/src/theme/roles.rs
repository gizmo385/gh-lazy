//! Semantic theme roles: what each color is for in this app, rather than which Textual variable it comes from.
//!
//! A role's [`ThemeColor`] may carry alpha: it's composited over whatever it's drawn on ([`ThemeColor::over`]).
//! Where a Textual CSS rule picks a different color or text style on light or terminal-color themes, the role's value
//! differs per theme, so widgets need not branch on the theme.
//!
//! The role list is written once, in the `roles!` invocation below, which generates the structs the widgets draw
//! with ([`Roles`] and its sections), the patch structs a theme file parses into ([`RolesPatch`], every role
//! optional), and [`Roles::keys`]. The values live in the theme files (`crates/lazy-github-tui/themes/`).

use std::ops::Range;
use std::sync::LazyLock;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::Spanned;

use super::color::{Rgb, TextStyle, ThemeColor};

/// A role, or a group of roles, as it's read from a theme file.
pub(crate) trait Role: Sized {
    /// What a theme file writes for it: the value itself for one role, a patch struct for a group.
    type Patch: DeserializeOwned;

    /// The value from a file that must list every role. Each role left out is recorded in `missing`.
    fn complete(patch: Self::Patch, missing: &mut Completion) -> Option<Self>;

    /// `base`, with the roles `patch` writes replaced.
    fn apply(patch: Option<Self::Patch>, base: &Self) -> Self;

    /// Every dotted role path under `path`, in file order.
    fn keys(path: &str, out: &mut Vec<String>);
}

/// `path.name`, or `name` at the top.
fn child(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

/// A single role, read as one TOML value.
macro_rules! leaf_role {
    ($($ty:ty => $patch:ty),* $(,)?) => {$(
        impl Role for $ty {
            type Patch = $patch;

            fn complete(patch: $patch, _: &mut Completion) -> Option<Self> {
                Some(patch.into())
            }

            fn apply(patch: Option<$patch>, base: &Self) -> Self {
                patch.map_or(*base, Into::into)
            }

            fn keys(path: &str, out: &mut Vec<String>) {
                out.push(path.to_owned());
            }
        }
    )*};
}

leaf_role! {
    ThemeColor => ThemeColor,
    Option<ThemeColor> => MaybeColor,
    TextStyle => TextStyle,
    Rgb => Rgb,
    bool => bool,
}

/// A role that may have no color: `none` in a theme file. Parsed in `super::file`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MaybeColor(pub(crate) Option<ThemeColor>);

impl From<MaybeColor> for Option<ThemeColor> {
    fn from(c: MaybeColor) -> Self {
        c.0
    }
}

/// Defines each group of roles once: the struct widgets read, its patch struct (every role an `Option`, unknown
/// keys an error, each value with its place in the file), and their [`Role`] impl.
macro_rules! roles {
    ($(
        $(#[$meta:meta])*
        pub struct $name:ident / $patch:ident {
            $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),* $(,)?
        }
    )*) => {$(
        $(#[$meta])*
        pub struct $name {
            $( $(#[$fmeta])* pub $field: $ty, )*
        }

        #[doc = concat!("[`", stringify!($name), "`] as a theme file writes it, any role left out.")]
        #[derive(Debug, Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub(crate) struct $patch {
            $( $field: Option<Spanned<<$ty as Role>::Patch>>, )*
        }

        impl Role for $name {
            type Patch = $patch;

            fn complete(patch: $patch, missing: &mut Completion) -> Option<Self> {
                // Every field first, so one pass reports every missing role.
                $( let $field = missing.field::<$ty>(stringify!($field), patch.$field); )*
                Some(Self { $( $field: $field?, )* })
            }

            fn apply(patch: Option<$patch>, base: &Self) -> Self {
                let patch = patch.unwrap_or_default();
                Self {
                    $( $field: <$ty as Role>::apply(patch.$field.map(Spanned::into_inner), &base.$field), )*
                }
            }

            fn keys(path: &str, out: &mut Vec<String>) {
                $( <$ty as Role>::keys(&child(path, stringify!($field)), out); )*
            }
        }

        #[cfg(test)]
        impl walk::Walk for $name {
            fn walk(&self, path: &str, out: &mut Vec<(String, walk::Leaf)>) {
                $( walk::Walk::walk(&self.$field, &child(path, stringify!($field)), out); )*
            }
        }
    )*};
}

roles! {
    /// Every color and text style the app draws with, by what it's for. One per theme; see [`super::Theme::roles`].
    #[derive(Debug, PartialEq)]
    pub struct Roles / RolesPatch {
        /// Backgrounds: the screen, panes, dialogs.
        surface: Surface,
        /// Text colors.
        text: Text,
        /// The theme's seed colors, as fills.
        brand: Brand,
        /// Status colors, as fills and borders: toasts, invalid inputs, link hints.
        status: Status,
        /// Box and rule colors.
        border: Border,
        /// The block cursor that highlights the current row of a list, table, tree or tab strip.
        cursor: Cursor,
        /// Scrollbars.
        scrollbar: Scrollbar,
        /// Text inputs and the text editor.
        input: Input,
        /// The key-binding footer.
        footer: Footer,
        /// Buttons, by variant.
        button: Button,
        /// The on/off switch.
        switch: Switch,
        /// The select (drop-down).
        select: Select,
        /// The selection list's check boxes.
        checkbox: Checkbox,
        /// Tree guide lines.
        tree: Tree,
        /// Tab strips.
        tabs: Tabs,
        /// Rendered markdown.
        markdown: Markdown,
        /// Syntax highlighting in markdown code blocks.
        syntax: Syntax,
        /// State colors by meaning: open, failed, merged, running, …
        marks: Marks,
        /// The diff viewer.
        diff: Diff,
        /// The command palette.
        command_palette: CommandPalette,
        /// Link hint labels.
        hints: Hints,
        /// Tables (`DataTable`).
        table: Table,
        /// Disabled fields.
        disabled: Disabled,
        /// SVG screenshots.
        screenshot: Screenshot,
    }

    /// A foreground and background pair: `fg` is drawn over `bg`, and `bg` over what's under it.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Pair / PairPatch {
        /// The text color, drawn over `bg`.
        fg: ThemeColor,
        /// The background, drawn over what's under it.
        bg: ThemeColor,
    }

    /// A [`Pair`] with a text style, such as a cursor row.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Highlight / HighlightPatch {
        /// The text color, drawn over `bg`.
        fg: ThemeColor,
        /// The background, drawn over what's under it.
        bg: ThemeColor,
        /// The text style applied on top of the row's own.
        style: TextStyle,
    }

    /// A text color with a text style, such as a link.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Ink / InkPatch {
        /// The text color.
        fg: ThemeColor,
        /// The text style.
        style: TextStyle,
    }

    /// Backgrounds.
    #[derive(Debug, PartialEq)]
    pub struct Surface / SurfacePatch {
        /// The screen, which everything else is drawn over (`$background`).
        screen: ThemeColor,
        /// A pane's or widget's own background: tables, trees, lists, inputs, switches (`$surface`).
        pane: ThemeColor,
        /// A table's header row (`$panel`).
        header: ThemeColor,
        /// Toasts, raised over the screen (`$panel-lighten-1`).
        raised: ThemeColor,
        /// A dialog's body, unless the dialog says otherwise (`$surface-lighten-3`).
        dialog: ThemeColor,
        /// The body of a dialog that shows one record, the user profile (`$surface-lighten-1`).
        card: ThemeColor,
        /// A lifted area inside a pane: the loading overlay, a block quote, the text editor's current line (`$boost`).
        boost: ThemeColor,
        /// What a modal screen dims the screens below it with, blended over every cell (`$background 60%`;
        /// transparent on terminal-color themes, which can't blend).
        scrim: ThemeColor,
        /// The tint a focused pane's background gets (`background-tint: $foreground 5%`), for [`super::Theme::tint`]
        /// at 100%.
        focus_tint: ThemeColor,
        /// The hatch pattern over an empty area (`$panel`).
        hatch: ThemeColor,
    }

    /// Text colors.
    #[derive(Debug, PartialEq)]
    pub struct Text / TextPatch {
        /// Body text with the theme's text contrast (`$text`).
        normal: ThemeColor,
        /// A widget's plain text color (`$foreground`).
        plain: ThemeColor,
        /// Secondary text: authors, headings of previews (`$text-muted`).
        muted: ThemeColor,
        /// Placeholder and unavailable text (`$text-disabled`).
        disabled: ThemeColor,
        /// Text in the primary color: help panel keys' descriptions (`$text-primary`).
        primary: ThemeColor,
        /// Text in the secondary color (`$text-secondary`).
        secondary: ThemeColor,
        /// Text in the accent color: help panel keys (`$text-accent`).
        accent: ThemeColor,
        /// Success text: an information toast's title (`$text-success`).
        success: ThemeColor,
        /// Warning text: a warning toast's title (`$text-warning`).
        warning: ThemeColor,
        /// Error text: an error toast's title (`$text-error`).
        error: ThemeColor,
    }

    /// The theme's seed colors, as fills.
    #[derive(Debug, PartialEq)]
    pub struct Brand / BrandPatch {
        /// The primary color: the loading dots, a markdown table's header, a selected check list item (`$primary`).
        primary: ThemeColor,
        /// The secondary color (`$secondary`).
        secondary: ThemeColor,
        /// The accent color (`$accent`).
        accent: ThemeColor,
    }

    /// Status colors, as fills and borders.
    #[derive(Debug, PartialEq)]
    pub struct Status / StatusPatch {
        /// An information toast's border, a switch that's on (`$success`).
        success: ThemeColor,
        /// A warning toast's border, a link hint's label (`$warning`).
        warning: ThemeColor,
        /// An error toast's border, an invalid input's border, a removed file's notice (`$error`).
        error: ThemeColor,
    }

    /// Box and rule colors.
    #[derive(Debug, PartialEq)]
    pub struct Border / BorderPatch {
        /// A focused pane or comment's box (`$success`).
        pane_focused: ThemeColor,
        /// An unfocused section's titled box (`$primary-lighten-3`).
        section_title: ThemeColor,
        /// A focused input, select, switch or editor's border (`$border`).
        field_focused: ThemeColor,
        /// An unfocused field's border, and a disabled button's (`$border-blurred`).
        field: ThemeColor,
        /// A horizontal rule: in dialogs, overviews and markdown (`$secondary`).
        rule: ThemeColor,
        /// The status bar's box (`$secondary`).
        status_bar: ThemeColor,
        /// A dialog's frame, unless the dialog says otherwise (`$background 80%`).
        dialog: ThemeColor,
        /// A box that draws attention: the diff viewer's review container, a key being captured in Settings, the
        /// Add Comment dialog's frame (`$accent`).
        emphasis: ThemeColor,
        /// An unfocused comment's box and a review thread's left bar in the conversation (`$secondary-background`).
        conversation: ThemeColor,
        /// The box around a review in the conversation (`$success-lighten-3`).
        review: ThemeColor,
        /// The help panel's left edge (`$foreground 30%`; `ansi_black` on terminal-color themes).
        help: ThemeColor,
    }

    /// The block cursor.
    #[derive(Debug, PartialEq)]
    pub struct Cursor / CursorPatch {
        /// In a focused widget (`$block-cursor-*`).
        focused: Highlight,
        /// In an unfocused widget (`$block-cursor-blurred-*`).
        blurred: Highlight,
        /// A row under the mouse (`$block-hover-background`).
        hover_bg: ThemeColor,
    }

    /// Scrollbars.
    #[derive(Debug, PartialEq)]
    pub struct Scrollbar / ScrollbarPatch {
        /// The thumb, over the track (`$scrollbar`).
        thumb: ThemeColor,
        /// The track (`$scrollbar-background`).
        track: ThemeColor,
        /// The corner where two scrollbars meet (`$scrollbar-corner-color`).
        corner: ThemeColor,
    }

    /// Text inputs and the text editor.
    #[derive(Debug, PartialEq)]
    pub struct Input / InputPatch {
        /// An input's cursor (`$input-cursor-*`; `ansi_black` on `ansi_white` on terminal-color themes).
        cursor: Highlight,
        /// The text editor's cursor (`$input-cursor-*`; `$text 90%` on `$foreground 70%` on light themes; reversed on
        /// terminal-color themes).
        editor_cursor: Highlight,
        /// Selected text (`$input-selection-*`).
        selection: Pair,
        /// A placeholder or suggestion (`$text-disabled`).
        placeholder: ThemeColor,
        /// A placeholder's or suggestion's text style (dim on terminal-color themes).
        placeholder_style: TextStyle,
        /// A focused invalid input's border (`$error`).
        invalid: ThemeColor,
        /// An unfocused invalid input's border (`$error 60%`).
        invalid_blurred: ThemeColor,
        /// Whether inputs are drawn in the terminal's own colors (`Input:ansi`): its default text on its default
        /// background, whatever they're drawn over.
        terminal_colors: bool,
    }

    /// The key-binding footer.
    #[derive(Debug, PartialEq)]
    pub struct Footer / FooterPatch {
        /// The bar (`$footer-background`).
        bar: ThemeColor,
        /// A binding's background, over the bar (`$footer-item-background`).
        item: ThemeColor,
        /// A binding under the mouse, over the bar (`$block-hover-background`).
        hover: ThemeColor,
        /// A binding's key, over its item (`$footer-key-*`).
        key: Pair,
        /// A binding's description, over its item (`$footer-description-*`).
        description: Pair,
    }

    /// Buttons.
    #[derive(Debug, PartialEq)]
    pub struct Button / ButtonPatch {
        /// Whether buttons are drawn outlined in the terminal's own colors (`Button:ansi`): the terminal's background, a
        /// `tall` border in the variant's [`ButtonColors::border`] on all four sides, and a bold label.
        outlined: bool,
        /// The label's style while focused (`$button-focus-text-style`).
        focus_style: TextStyle,
        /// A disabled outlined button's border (`$border-blurred`).
        disabled_border: ThemeColor,
        /// The default button.
        default: ButtonColors,
        /// The primary button.
        primary: ButtonColors,
        /// The success (submit) button.
        success: ButtonColors,
        /// The warning button.
        warning: ButtonColors,
        /// The error (cancel) button.
        error: ButtonColors,
    }

    /// One button variant's colors.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ButtonColors / ButtonColorsPatch {
        /// The fill (`$surface`, `$primary`, …).
        bg: ThemeColor,
        /// The top edge, lit (`$<variant>-lighten-2`; `-lighten-3` for primary, `$surface-lighten-1` for default).
        light: ThemeColor,
        /// The bottom edge, in shadow (`$<variant>-darken-3`; `$surface-darken-1` for default).
        dark: ThemeColor,
        /// An outlined button's border (`$<variant>`; `$border-blurred` for default).
        border: ThemeColor,
        /// The label (`$button-color-foreground`, `$button-foreground` for default; the variant's color when outlined).
        label: ThemeColor,
        /// A disabled button's label (the label at `opacity: 0.6`).
        label_disabled: ThemeColor,
    }

    /// The on/off switch.
    #[derive(Debug, PartialEq)]
    pub struct Switch / SwitchPatch {
        /// The track the slider moves in (`$panel-darken-2`).
        track: ThemeColor,
        /// The slider while on (`$success`).
        on: ThemeColor,
        /// The slider while off (`$panel`; `$primary 15%` on light themes; `ansi_red` on the terminal-color dark theme).
        off: ThemeColor,
    }

    /// The select (drop-down).
    #[derive(Debug, PartialEq)]
    pub struct Select / SelectPatch {
        /// The closed select's prompt while nothing is chosen (`$foreground 50%`).
        muted: ThemeColor,
        /// Whether the closed select is drawn in the terminal's own colors (`SelectCurrent:ansi`): its default text on
        /// its default background, whatever it's drawn over, the prompt included.
        terminal_colors: bool,
    }

    /// The selection list's check boxes.
    #[derive(Debug, PartialEq)]
    pub struct Checkbox / CheckboxPatch {
        /// The box (`$panel`).
        bg: ThemeColor,
        /// The mark while unchecked, over the box (`$panel-darken-2`).
        off: ThemeColor,
        /// The mark while checked, over the box (`$text-success`).
        on: ThemeColor,
    }

    /// Tree guide lines.
    #[derive(Debug, PartialEq)]
    pub struct Tree / TreePatch {
        /// In an unfocused tree (`$surface-lighten-2`; `$surface-darken-1` on light themes; `$ansi-background` on
        /// terminal-color themes).
        guide: ThemeColor,
        /// In a focused tree (`$surface-lighten-3`, and as [`Self::guide`] on light and terminal-color themes).
        guide_focused: ThemeColor,
        /// The cursor row's guides (`$block-cursor-background`).
        guide_selected: ThemeColor,
    }

    /// Tab strips.
    #[derive(Debug, PartialEq)]
    pub struct Tabs / TabsPatch {
        /// An inactive tab's label (`$foreground 50%`).
        label: ThemeColor,
        /// An inactive tab's label style (dim on terminal-color themes).
        label_style: TextStyle,
        /// The active tab's label in an unfocused strip (`$foreground`).
        label_active: ThemeColor,
        /// The active tab's label style in an unfocused strip (bold on terminal-color themes).
        label_active_style: TextStyle,
        /// The underline bar of an unfocused strip (`$foreground 10%`; `$border-blurred` on terminal-color themes).
        bar: ThemeColor,
        /// The underline bar of a focused strip (`$foreground 30%`; `$border-blurred` on terminal-color themes).
        bar_focused: ThemeColor,
        /// The bar under the active tab (`$block-cursor-background`).
        bar_active: ThemeColor,
    }

    /// Rendered markdown. Dialog titles are drawn as [`Self::h1`].
    #[derive(Debug, PartialEq)]
    pub struct Markdown / MarkdownPatch {
        /// A level-1 heading, and a dialog's title (`$markdown-h1-*`).
        h1: Highlight,
        /// A level-2 heading (`$markdown-h2-*`).
        h2: Highlight,
        /// A level-3 heading (`$markdown-h3-*`).
        h3: Highlight,
        /// A level-4 heading (`$markdown-h4-*`).
        h4: Highlight,
        /// A level-5 heading (`$markdown-h5-*`).
        h5: Highlight,
        /// A level-6 heading (`$markdown-h6-*`).
        h6: Highlight,
        /// A link (`$link-color`, `$link-style`).
        link: Ink,
        /// A code block's background (`black 10%`; `white 30%` on light themes; transparent on terminal-color themes).
        fence_bg: ThemeColor,
        /// Whether a code block has its margin above and below, its side padding and a filled background
        /// (`MarkdownFence`; not on terminal-color themes, `MarkdownFence:ansi`).
        fence_inset: bool,
        /// Inline code's text (`$text-warning 95%`; `$text-error 95%` on light themes).
        code: ThemeColor,
        /// Inline code's background, drawn as a component background ([`ThemeColor::component_background`]):
        /// `$warning 10%`, `$error 5%` on light themes. Transparent on terminal-color themes, where the run keeps what's
        /// under it.
        code_bg: ThemeColor,
        /// A block quote's background (`$boost`).
        quote_bg: ThemeColor,
        /// A block quote's left bar (`$text-primary 50%`; `$text-secondary` on light themes).
        quote_bar: ThemeColor,
        /// A list's bullets and numbers (`$text-primary`; `$text-secondary` on light themes).
        bullet: ThemeColor,
        /// A table's background (transparent; `white 30%` on light themes).
        table_bg: ThemeColor,
        /// A table's header text (`$primary`).
        table_header: ThemeColor,
        /// A table's keylines (`$foreground 20%`).
        table_line: ThemeColor,
    }

    /// How one kind of syntax token looks: its colors replace the code block's, and its text style is added.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct TokenStyle / TokenStylePatch {
        /// The token's text color, or the code block's text color for `None`.
        fg: Option<ThemeColor>,
        /// The token's background, or the code block's for `None`.
        bg: Option<ThemeColor>,
        /// Text style added to the token.
        style: TextStyle,
    }

    /// Syntax highlighting in markdown code blocks: Textual's `HighlightTheme`, and its ANSI dark and light variants on
    /// terminal-color themes. One role per token kind the highlighter maps.
    #[derive(Debug, PartialEq)]
    pub struct Syntax / SyntaxPatch {
        /// A code block's text, under every token (`$text` at full strength).
        text: ThemeColor,
        /// Comments (`$text 60%`).
        comment: TokenStyle,
        /// Text the lexer couldn't read (`$text-error` on `$error-muted`).
        error: TokenStyle,
        /// Strong text in markup languages (bold).
        generic_strong: TokenStyle,
        /// Emphasized text in markup languages (italic).
        generic_emph: TokenStyle,
        /// Headings in markup languages (underlined `$text-primary`).
        generic_heading: TokenStyle,
        /// Subheadings in markup languages (`$text-primary`).
        generic_subheading: TokenStyle,
        /// Keywords (`$text-accent`).
        keyword: TokenStyle,
        /// Constant keywords such as `true` and `None` (bold `$text-success 80%`).
        keyword_constant: TokenStyle,
        /// Import keywords (`$text-error`).
        keyword_namespace: TokenStyle,
        /// Numbers (`$text-warning`).
        number: TokenStyle,
        /// Strings (`$text-success`).
        string: TokenStyle,
        /// Backtick strings (`$text 60%`).
        string_backtick: TokenStyle,
        /// Docstrings and doc comments (italic `$text-success 80%`).
        string_doc: TokenStyle,
        /// Double-quoted strings (`$text-success 90%`).
        string_double: TokenStyle,
        /// Names (`$text-primary`).
        name: TokenStyle,
        /// Attribute names (`$text-warning`).
        name_attribute: TokenStyle,
        /// Built-in names (`$text-accent`).
        name_builtin: TokenStyle,
        /// Built-in pseudo names such as `self` (italic).
        name_builtin_pseudo: TokenStyle,
        /// Class names (bold `$text-warning`).
        name_class: TokenStyle,
        /// Constant names (`$text-error`).
        name_constant: TokenStyle,
        /// Decorators (bold `$text-primary`).
        name_decorator: TokenStyle,
        /// Function names (underlined `$text-warning`).
        name_function: TokenStyle,
        /// Markup tags (bold `$text-primary`).
        name_tag: TokenStyle,
        /// Variable names (`$text-secondary`).
        name_variable: TokenStyle,
        /// Operators (bold).
        operator: TokenStyle,
        /// Word operators such as `and` and `not` (bold `$text-error`).
        operator_word: TokenStyle,
        /// Text the highlighter knows but leaves plain, so an outer scope doesn't style it.
        plain: TokenStyle,
    }

    /// State colors by meaning. Each is drawn as is, over anything.
    #[derive(Debug, PartialEq)]
    pub struct Marks / MarksPatch {
        /// A good state: open, passed, approved, succeeded, public, all notifications read (`greenyellow`).
        positive: ThemeColor,
        /// A bad state: closed unmerged, failed, changes requested, private, unread notifications, a branch that
        /// failed to push (`red`).
        negative: ThemeColor,
        /// Done and set aside: a merged pull request, a closed issue, an archived repository (`orchid`).
        merged: ThemeColor,
        /// Waiting or stopped: a pending check, a queued or cancelled run (`yellow`).
        pending: ThemeColor,
        /// In progress: a running workflow run (`cyan`).
        running: ThemeColor,
        /// A warning's lead (`Warning:`) and the name a warning or error is about (`yellow`).
        warning: ThemeColor,
        /// The name a message is about: the status bar's current repository, a pushed branch (`greenyellow`).
        name: ThemeColor,
        /// A pull request's added-lines count (`greenyellow`).
        count_added: ThemeColor,
        /// A pull request's removed-lines count (`red`).
        count_removed: ThemeColor,
        /// The authentication screen's box (`red`).
        auth_border: ThemeColor,
        /// The command the authentication screen asks for (`green`).
        auth_command: ThemeColor,
        /// A starred repository's star (Rich `green`).
        starred: ThemeColor,
        /// A failed job in the jobs tree (Rich `red`).
        job_failed: ThemeColor,
        /// A queued or cancelled job in the jobs tree (Rich `yellow`).
        job_pending: ThemeColor,
        /// A running job in the jobs tree (Rich `cyan`).
        job_running: ThemeColor,
    }

    /// The diff viewer.
    #[derive(Debug, PartialEq)]
    pub struct Diff / DiffPatch {
        /// An added line (`#0a3a0a`).
        added_bg: ThemeColor,
        /// A removed line (`#3a0a0a`).
        removed_bg: ThemeColor,
        /// The current line, when it's added (`#006400`).
        added_current_bg: ThemeColor,
        /// The current line, when it's removed (`#8b0000`).
        removed_current_bg: ThemeColor,
        /// The current line, when it's context (Rich `yellow`).
        context_current_bg: ThemeColor,
        /// A file or hunk's added count (Rich `green`).
        stat_added: ThemeColor,
        /// A file or hunk's removed count (Rich `red`).
        stat_removed: ThemeColor,
        /// A focused hunk pane's border (`$accent`).
        border_focused: ThemeColor,
        /// An unfocused hunk pane's border (`$primary-lighten-1`).
        border: ThemeColor,
        /// The current-line indicator (`$text` on `$accent`).
        indicator: Pair,
        /// The review panel's header (bold `$text` on `$accent`).
        review_header: Highlight,
        /// A pending review comment's body (`$panel`).
        review_body: ThemeColor,
        /// A pending review comment's box (`$secondary`).
        review_comment_border: ThemeColor,
    }

    /// The command palette.
    #[derive(Debug, PartialEq)]
    pub struct CommandPalette / CommandPalettePatch {
        /// The band behind the input and the hits (`$panel-darken-1`; `$surface` on light themes).
        band: ThemeColor,
        /// The rules above and below the input (`black 50%`).
        input_rule: ThemeColor,
        /// The rule below the hits (`black`).
        list_rule: ThemeColor,
        /// The search icon (`black`).
        icon: ThemeColor,
        /// The highlighted hit (the blurred block cursor; the focused one on terminal-color themes).
        highlight: Highlight,
    }

    /// Link hint labels.
    #[derive(Debug, PartialEq)]
    pub struct Hints / HintsPatch {
        /// A label (bold `black` on `$warning`).
        label: Highlight,
    }

    /// Tables (`DataTable`).
    #[derive(Debug, PartialEq)]
    pub struct Table / TablePatch {
        /// The header row (`$foreground` on `$panel`, tinted with the table while focused; `ansi_default` on
        /// `ansi_bright_blue` on terminal-color themes, `&:ansi > .datatable--header`, where the tint has no effect).
        header: Pair,
    }

    /// Disabled fields.
    #[derive(Debug, PartialEq)]
    pub struct Disabled / DisabledPatch {
        /// Whether a disabled input, switch or select is dimmed (SGR 2) rather than faded toward its parent's background
        /// (`opacity: 0.7`): on terminal-color themes, which can't blend.
        dimmed: bool,
    }

    /// SVG screenshots: what they draw the terminal's default colors as, where the theme leaves a color to the terminal
    /// (the terminal-color themes), since a screenshot can't ask the terminal. Exact RGB, as the SVG writes it.
    #[derive(Debug, PartialEq)]
    pub struct Screenshot / ScreenshotPatch {
        /// The terminal's default text color (`#D4D4D4` on dark themes, `#1E1E1E` on light ones).
        default_fg: Rgb,
        /// The terminal's default background (`#1E1E1E` on dark themes, `#FFFFFF` on light ones).
        default_bg: Rgb,
    }
}

impl Roles {
    /// Every role's dotted path (`cursor.focused.bg`), each once, in the order the role structs declare them.
    pub fn keys() -> &'static [String] {
        static KEYS: LazyLock<Vec<String>> = LazyLock::new(|| {
            let mut keys = Vec::new();
            <Roles as Role>::keys("", &mut keys);
            keys
        });
        &KEYS
    }
}

impl RolesPatch {
    /// The roles, if the file listed every one: a built-in theme's file must.
    pub(crate) fn complete(self) -> Result<Roles, MissingRoles> {
        let mut completion = Completion::default();
        Roles::complete(self, &mut completion).ok_or(MissingRoles(completion.missing))
    }

    /// `base`, with the roles this file writes replaced.
    #[allow(dead_code, reason = "user theme files (#380) extend a built-in with it")]
    pub(crate) fn apply(self, base: &Roles) -> Roles {
        Roles::apply(Some(self), base)
    }
}

/// Where [`Role::complete`] is in the file, and what it found missing.
#[derive(Default)]
pub(crate) struct Completion {
    /// The path to the group being completed.
    path: Vec<&'static str>,
    /// The span of each enclosing table, innermost last.
    tables: Vec<Range<usize>>,
    missing: Vec<MissingRole>,
}

impl Completion {
    /// The role or group `name` in the group being completed, recorded as missing if `value` is `None`.
    fn field<T: Role>(&mut self, name: &'static str, value: Option<Spanned<T::Patch>>) -> Option<T> {
        self.path.push(name);
        let out = if let Some(value) = value {
            self.tables.push(value.span());
            let out = T::complete(value.into_inner(), self);
            self.tables.pop();
            out
        } else {
            self.missing.push(MissingRole {
                path: self.path.join("."),
                table: self.tables.last().cloned(),
            });
            None
        };
        self.path.pop();
        out
    }
}

/// The roles a theme file that must list every role left out.
#[derive(Debug)]
pub(crate) struct MissingRoles(pub(crate) Vec<MissingRole>);

/// One role a theme file left out.
#[derive(Debug)]
pub(crate) struct MissingRole {
    /// Its dotted path.
    pub(crate) path: String,
    /// The table it belongs in, if the file has that table: where the error points.
    pub(crate) table: Option<Range<usize>>,
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

    impl Walk for Rgb {
        fn walk(&self, path: &str, out: &mut Vec<(String, Leaf)>) {
            let Rgb { r, g, b } = *self;
            out.push((path.to_owned(), Leaf::Color(ThemeColor::Rgba { r, g, b, a: 1.0 })));
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
}
