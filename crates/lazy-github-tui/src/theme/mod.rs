//! Themes: the 21 built-in themes Python users know by name, with the exact colors Textual paints for them.
//!
//! Each theme is a data file in `crates/lazy-github-tui/themes/` ([`Catalog`]) that gives every role ([`Roles`]): what
//! each color is for in this app. The built-in files hold Textual 8.2.5's colors, frozen. Colors with alpha or `auto`
//! are composited over what they're drawn on with Textual's arithmetic ([`ThemeColor::over`]), so a cell gets the same
//! RGB in both apps. The result is then reduced to what the terminal shows ([`ColorMode`]), as Rich does it.
//! Components take every color from [`Theme::roles`].

mod catalog;
mod color;
#[cfg(test)]
mod composites_tests;
mod depth;
mod file;
mod roles;

use std::fmt;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

pub use catalog::{Catalog, Resolved};
pub use color::{AnsiColor, Paint, Rgb, TextStyle, ThemeColor};
pub use depth::{ColorDepth, ColorMode, DepthReason, DetectedDepth, TerminalColors, downsample};
pub use file::{Appearance, ThemeFileError};
pub use roles::{
    Border, Brand, Button, ButtonColors, Checkbox, CommandPalette, Cursor, Diff, Footer, Highlight, Hints, Ink, Input,
    Markdown, Marks, Pair, Roles, Screenshot, Scrollbar, Select, Status, Surface, Switch, Syntax, Tabs, Text,
    TokenStyle, Tree,
};

/// The default theme, and what an unknown name is drawn as. Python's `textual-dark`, renamed (#379).
const DEFAULT: &str = "dark";

/// The alpha Textual draws Rich's `dim` text color at: blended 66% of the way from what's under it.
const RICH_DIM_ALPHA: f64 = 0.66;

/// A theme from the [`Catalog`]. A small `Copy` handle onto its static roles.
#[derive(Clone, Copy, PartialEq)]
pub struct Theme {
    /// Its name, as a config writes it (`dark`).
    pub name: &'static str,
    /// The name Settings shows (`Dark`).
    display_name: &'static str,
    /// Whether it's a light or a dark theme.
    appearance: Appearance,
    /// Whether it uses the terminal's own 16 colors (`ansi-dark`, `ansi-light`), which Textual can't blend.
    terminal_colors: bool,
    /// Its colors and text styles by what they're for.
    roles: &'static Roles,
    /// What the terminal shows, which every color is reduced to. Set by [`Theme::from_config`]; truecolor for
    /// [`Theme::by_name`], [`Theme::all`] and [`Theme::default`].
    mode: ColorMode,
}

impl Theme {
    /// Its colors and text styles by what they're for in this app.
    pub fn roles(&self) -> &'static Roles {
        self.roles
    }

    /// Whether it's a light or a dark theme.
    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// This theme, with its colors reduced to `mode`. The mode belongs to the terminal, not the theme: switching
    /// themes at runtime must carry it over, which [`Theme::from_config`] makes explicit.
    #[must_use]
    pub fn with_color_mode(self, mode: ColorMode) -> Self {
        Self { mode, ..self }
    }

    /// What the terminal shows, which every color is reduced to.
    pub fn color_mode(&self) -> ColorMode {
        self.mode
    }

    /// The built-in theme called exactly `name`, or that `name` is an alias of (`textual-dark`).
    pub fn by_name(name: &str) -> Option<Theme> {
        Catalog::builtin().get(name)
    }

    /// The theme for `appearance.theme`, reduced to the terminal's color `mode`: an alias draws as its theme, and
    /// an unknown name as the default theme (the app warns about that once, at startup). To switch themes at
    /// runtime, pass the current theme's [`Theme::color_mode`].
    pub fn from_config(name: &str, mode: ColorMode) -> Theme {
        Catalog::builtin()
            .resolve(name)
            .theme_or_default()
            .with_color_mode(mode)
    }

    /// Every built-in theme, in Textual's order.
    pub fn all() -> impl Iterator<Item = Theme> {
        Catalog::builtin().themes()
    }

    /// The name Settings shows for this theme, as its file gives it: the Python app's, `name.title().replace("-",
    /// " ")`, so `rose-pine-moon` is "Rose Pine Moon".
    pub fn display_name(&self) -> String {
        self.display_name.to_owned()
    }

    /// What the screen is painted with (`$background`), which everything else is drawn over.
    pub fn background(&self) -> Paint {
        self.roles.surface.screen.over(Paint::Ansi(AnsiColor::Default))
    }

    /// The terminal color for `c` drawn over `under`.
    pub fn color(&self, c: ThemeColor, under: Paint) -> Color {
        self.paint(c.over(under))
    }

    /// The terminal color for an already composited paint, such as [`Theme::background`], reduced to the color mode.
    /// Use this rather than `Color::from(paint)`, which always gives exact RGB.
    pub fn paint(&self, paint: Paint) -> Color {
        self.mode.paint(paint)
    }

    /// A style for `fg` text on a `bg` background, with `bg` itself drawn over `under`: "`$text` on `$panel`".
    pub fn fg_on(&self, fg: ThemeColor, bg: ThemeColor, under: Paint) -> Style {
        let bg = bg.over(under);
        Style::new().fg(self.paint(fg.over(bg))).bg(self.paint(bg))
    }

    /// `style`, whose text is `fg` drawn over `under`, with Rich's `dim` (`[dim]…[/dim]` markup) applied as Textual
    /// resolves it: `fg` at 66% alpha over `under`. Textual can't blend the terminal's own colors, so an ANSI `fg`
    /// keeps its color and gets SGR 2, leaving the dimming to the terminal.
    ///
    /// This resolves the dim eagerly, which is only needed where the color must be known while drawing. Setting
    /// `Modifier::DIM` gives the same cell, since [`Theme::resolve_dim`] blends it at the end of every frame.
    #[must_use]
    pub fn rich_dim(&self, style: Style, fg: ThemeColor, under: Paint) -> Style {
        match fg {
            ThemeColor::Ansi(_) => style.add_modifier(Modifier::DIM),
            fg => style.fg(self.color(fg.multiply_alpha(RICH_DIM_ALPHA), under)),
        }
    }

    /// Textual's `ANSIToTruecolor` dim step, over the cells of `area` in a drawn frame: on a non-ANSI theme every
    /// cell with `Modifier::DIM` gets its fg blended 66% of the way from its bg toward its fg, and loses `DIM`. A
    /// cell whose bg is the terminal's default blends toward [`Theme::background`], as Textual falls back to the
    /// screen's background. A no-op on the ANSI themes, where Textual leaves `dim` to the terminal as SGR 2.
    ///
    /// So `Modifier::DIM` in a style means Textual's `dim` (Rich `[dim]`, `style="dim"` or TCSS `text-style: dim`)
    /// on every theme. A cell whose fg (or bg) has no RGB, such as the terminal's default color, keeps `DIM`.
    pub fn resolve_dim(&self, buf: &mut Buffer, area: Rect) {
        if self.terminal_colors {
            return;
        }
        let screen = match self.background() {
            Paint::Rgb(rgb) => Some(rgb),
            Paint::Ansi(_) => None,
        };
        for position in area.intersection(buf.area).positions() {
            let cell = &mut buf[position];
            if !cell.modifier.contains(Modifier::DIM) {
                continue;
            }
            let bg = match cell.bg {
                Color::Reset => screen,
                bg => depth::rgb_of(bg),
            };
            let (Some(bg), Some(fg)) = (bg, depth::rgb_of(cell.fg)) else {
                continue;
            };
            cell.fg = self.paint(Paint::Rgb(bg.blend(fg, RICH_DIM_ALPHA)));
            cell.modifier.remove(Modifier::DIM);
        }
    }

    /// Textual's `background-tint: <tint> <percent>%` on `base`, as focused `DataTable`s and `Log`s use
    /// (`$foreground 5%`). Like Textual's `Color.tint`, an ANSI base or an ANSI tint leaves `base` unchanged.
    ///
    /// Textual tints the widget's own background before compositing it over its parent; tinting the composited
    /// `base` gives the same result only while that background is opaque, as every built-in one is.
    pub fn tint(&self, base: Paint, tint: ThemeColor, percent: f64) -> Paint {
        match base {
            Paint::Ansi(_) => base,
            Paint::Rgb(_) => match tint.multiply_alpha(percent / 100.0).over(base) {
                tinted @ Paint::Rgb(_) => tinted,
                Paint::Ansi(_) => base,
            },
        }
    }

    /// `color`, already drawn, seen through a modal screen's `background: $background 60%`: blended `factor` of the
    /// way toward [`Theme::background`]. Unchanged on the ANSI themes, whose modal background is `transparent`, and
    /// for the terminal's default color, which Textual can't blend.
    ///
    /// Textual blends before Rich reduces colors to the terminal's depth; a reduced color is blended as the RGB it
    /// stands for, and reduced again.
    pub fn dim(&self, color: Color, factor: f64) -> Color {
        if self.terminal_colors {
            return color;
        }
        let (Some(rgb), Paint::Rgb(background)) = (depth::rgb_of(color), self.background()) else {
            return color;
        };
        self.paint(Paint::Rgb(rgb.blend(background, factor)))
    }

    /// `color`, already drawn, under Textual's `opacity: <opacity>` on a widget over `under`, its parent's background:
    /// blended `1 - opacity` of the way toward `under`. As with [`Theme::dim`], the terminal's default color and an
    /// ANSI `under` can't be blended and stay as they are.
    pub fn fade(&self, color: Color, under: Paint, opacity: f64) -> Color {
        let (Some(rgb), Paint::Rgb(under)) = (depth::rgb_of(color), under) else {
            return color;
        };
        self.paint(Paint::Rgb(rgb.blend(under, 1.0 - opacity)))
    }
}

impl Default for Theme {
    /// `dark`: Textual's default theme, `textual-dark`, renamed.
    fn default() -> Self {
        Catalog::builtin()
            .get(DEFAULT)
            // A unit test checks the default is a built-in theme.
            .unwrap_or_else(|| panic!("the default theme {DEFAULT:?} is a built-in theme"))
    }
}

impl fmt::Debug for Theme {
    // The roles are hundreds of values; the name says which theme it is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Theme").field(&self.name).finish()
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::{Constraint, Layout};
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Paragraph};
    use rstest::rstest;

    use super::*;

    const NAMES: [&str; 21] = [
        "dark",
        "light",
        "nord",
        "gruvbox",
        "catppuccin-mocha",
        "dracula",
        "tokyo-night",
        "monokai",
        "flexoki",
        "catppuccin-latte",
        "catppuccin-frappe",
        "catppuccin-macchiato",
        "solarized-light",
        "solarized-dark",
        "rose-pine",
        "rose-pine-moon",
        "rose-pine-dawn",
        "atom-one-dark",
        "atom-one-light",
        "ansi-dark",
        "ansi-light",
    ];

    fn theme(name: &str) -> Theme {
        Theme::by_name(name).unwrap()
    }

    /// The terminal color of the mark `pick` chooses, drawn as is over the screen.
    fn mark(theme: &Theme, pick: impl FnOnce(&Marks) -> ThemeColor) -> Color {
        theme.color(pick(&theme.roles().marks), theme.background())
    }

    fn hex(text: &str) -> ThemeColor {
        let value = u32::from_str_radix(text.trim_start_matches('#'), 16).unwrap();
        let [_, r, g, b] = value.to_be_bytes();
        ThemeColor::Rgba { r, g, b, a: 1.0 }
    }

    #[test]
    fn all_lists_textuals_themes_in_order() {
        assert_eq!(Theme::all().map(|t| t.name).collect::<Vec<_>>(), NAMES);
        assert_eq!(Theme::default().name, "dark");
    }

    /// A new config names the default theme.
    #[test]
    fn the_config_default_is_the_default_theme() {
        assert_eq!(Theme::default().name, lazy_github_core::config::DEFAULT_THEME);
    }

    #[test]
    fn by_name_finds_every_builtin_theme() {
        for name in NAMES {
            assert_eq!(Theme::by_name(name).map(|t| t.name), Some(name));
            assert_eq!(Theme::from_config(name, ColorMode::default()).name, name);
        }
    }

    #[rstest]
    #[case("Nord")]
    #[case("")]
    #[case("no-such-theme")]
    fn unknown_names_render_as_dark(#[case] name: &str) {
        assert_eq!(Theme::by_name(name), None);
        assert_eq!(Theme::from_config(name, ColorMode::default()), theme("dark"));
        let mode = ColorMode {
            depth: ColorDepth::Standard,
            no_color: true,
        };
        assert_eq!(Theme::from_config(name, mode).color_mode(), mode);
    }

    /// Spot checks against Textual's output, which the theme files were frozen from.
    #[test]
    fn roles_match_textuals_output() {
        let dark = theme("dark");
        let r = dark.roles();
        assert_eq!(r.border.section_title, hex("#6DB2FF"));
        assert_eq!(r.status.success, hex("#4EBF71"));
        assert_eq!(r.brand.secondary, hex("#004578"));
        assert_eq!(r.surface.header, hex("#242F38"));
        assert_eq!(r.surface.screen, hex("#121212"));
        assert_eq!(dark.appearance(), Appearance::Dark);
        assert!(!dark.terminal_colors);

        assert_eq!(
            theme("nord").roles().cursor.focused.style,
            TextStyle {
                add: ratatui::style::Modifier::empty(),
                sub: ratatui::style::Modifier::empty()
            }
        );
        assert_eq!(theme("catppuccin-latte").appearance(), Appearance::Light);

        let ansi = theme("ansi-dark");
        let r = ansi.roles();
        assert!(ansi.terminal_colors);
        assert_eq!(r.brand.primary, ThemeColor::Ansi(AnsiColor::Blue));
        assert_eq!(r.surface.pane, ThemeColor::Transparent);
        assert_eq!(ansi.background(), Paint::Ansi(AnsiColor::Default));
        assert_eq!(ansi.color(r.brand.primary, ansi.background()), Color::Blue);
    }

    #[test]
    fn tint_passes_ansi_through() {
        let ansi = theme("ansi-dark");
        let surface = ansi.roles().surface.pane.over(ansi.background());
        assert_eq!(surface, Paint::Ansi(AnsiColor::Default));
        assert_eq!(ansi.tint(surface, ansi.roles().text.plain, 5.0), surface);
        // An ANSI tint over an RGB base also leaves the base alone, unlike `over`.
        let base = Paint::Rgb(Rgb::new(30, 30, 30));
        assert_eq!(ansi.tint(base, ThemeColor::Ansi(AnsiColor::Red), 5.0), base);
        assert_eq!(
            theme("dark").tint(base, hex("#E0E0E0"), 5.0),
            Paint::Rgb(Rgb::new(39, 39, 39))
        );
    }

    #[rstest]
    #[case("dark", "Dark")]
    #[case("light", "Light")]
    #[case("catppuccin-mocha", "Catppuccin Mocha")]
    #[case("rose-pine-moon", "Rose Pine Moon")]
    #[case("ansi-light", "Ansi Light")]
    fn display_name_is_pythons_title_case(#[case] name: &str, #[case] display: &str) {
        assert_eq!(theme(name).display_name(), display);
    }

    #[test]
    fn fixed_colors_ignore_the_theme() {
        let dark = theme("dark");
        let light = theme("light");
        for t in [dark, light, theme("nord")] {
            assert_eq!(mark(&t, |m| m.positive), Color::Rgb(0xAD, 0xFF, 0x2F));
            assert_eq!(mark(&t, |m| m.merged), Color::Rgb(0xDA, 0x70, 0xD6));
            assert_eq!(mark(&t, |m| m.negative), Color::Rgb(0xFF, 0x00, 0x00));
            assert_eq!(mark(&t, |m| m.pending), Color::Rgb(0xFF, 0xFF, 0x00));
            assert_eq!(mark(&t, |m| m.running), Color::Rgb(0x00, 0xFF, 0xFF));
            assert_eq!(mark(&t, |m| m.auth_command), Color::Rgb(0x00, 0x80, 0x00));
        }
        assert_eq!(mark(&dark, |m| m.starred), Color::Rgb(0x98, 0xE0, 0x24));
        assert_eq!(mark(&light, |m| m.starred), Color::Rgb(0x44, 0x8C, 0x27));
        assert_eq!(mark(&theme("ansi-light"), |m| m.starred), Color::Green);
        // Rich's `cyan` (the Jobs tree's `⋯`): MONOKAI's and ALABASTER's color 6.
        assert_eq!(mark(&dark, |m| m.job_running), Color::Rgb(0x58, 0xD1, 0xEB));
        assert_eq!(mark(&light, |m| m.job_running), Color::Rgb(0x00, 0x83, 0xB2));
        assert_eq!(mark(&theme("ansi-dark"), |m| m.job_running), Color::Cyan);
    }

    /// A bordered block with a focused and an unfocused border, a `$text` on `$panel` line and a blurred cursor row.
    fn themed_widget(theme: &Theme) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
        terminal
            .draw(|frame| {
                let r = theme.roles();
                let screen = theme.background();
                frame.render_widget(
                    Block::new().style(
                        Style::new()
                            .fg(theme.color(r.text.plain, screen))
                            .bg(theme.paint(screen)),
                    ),
                    frame.area(),
                );
                let surface = r.surface.pane.over(screen);
                let [focused, blurred] = Layout::vertical([Constraint::Length(4); 2]).areas(frame.area());
                let cursor = r.cursor.blurred;
                for (area, border, row) in [
                    (
                        focused,
                        r.border.pane_focused,
                        Line::styled("text on panel", theme.fg_on(r.text.normal, r.surface.header, surface)),
                    ),
                    (
                        blurred,
                        r.border.section_title,
                        Line::styled(
                            "blurred cursor",
                            cursor.style.apply(theme.fg_on(cursor.fg, cursor.bg, surface)),
                        ),
                    ),
                ] {
                    let block = Block::bordered()
                        .border_style(Style::new().fg(theme.color(border, surface)))
                        .style(Style::new().bg(theme.paint(surface)));
                    frame.render_widget(Paragraph::new(row).block(block), area);
                }
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[rstest]
    #[case("dark")]
    #[case("nord")]
    #[case("light")]
    #[case("ansi-dark")]
    fn themed_widget_snapshot(#[case] name: &str) {
        insta::assert_debug_snapshot!(format!("themed_widget_{name}"), themed_widget(&theme(name)));
    }

    #[rstest]
    #[case("eight_bit", ColorDepth::EightBit)]
    #[case("standard", ColorDepth::Standard)]
    fn themed_widget_snapshot_at_depth(#[case] label: &str, #[case] depth: ColorDepth) {
        let mode = ColorMode { depth, no_color: false };
        let theme = theme("dark").with_color_mode(mode);
        insta::assert_debug_snapshot!(format!("themed_widget_dark_{label}"), themed_widget(&theme));
    }

    #[test]
    fn color_mode_applies_after_compositing() {
        let truecolor = theme("dark");
        assert_eq!(truecolor.color_mode(), ColorMode::default());
        let eight_bit = truecolor.with_color_mode(ColorMode {
            depth: ColorDepth::EightBit,
            no_color: false,
        });
        let r = eight_bit.roles();
        // The blurred cursor (#0178D4, 30% alpha) over the pane is #153854, then index 23.
        let surface = r.surface.pane.over(eight_bit.background());
        assert_eq!(
            truecolor.color(r.cursor.blurred.bg, surface),
            Color::Rgb(0x15, 0x38, 0x54)
        );
        assert_eq!(
            eight_bit.color(r.cursor.blurred.bg, surface),
            downsample(Rgb::new(0x15, 0x38, 0x54), eight_bit.color_mode())
        );
        assert_eq!(mark(&eight_bit, |m| m.negative), Color::Indexed(196));
        assert_eq!(mark(&eight_bit, |m| m.starred), Color::Indexed(112));
    }

    #[test]
    fn no_color_grays_rgb_themes_and_resets_ansi_themes() {
        let no_color = ColorMode {
            depth: ColorDepth::TrueColor,
            no_color: true,
        };
        let dark = theme("dark").with_color_mode(no_color);
        assert_eq!(
            dark.color(dark.roles().brand.primary, dark.background()),
            Color::Rgb(101, 101, 101)
        );
        let eight_bit = dark.with_color_mode(ColorMode {
            depth: ColorDepth::EightBit,
            no_color: true,
        });
        assert_eq!(
            eight_bit.color(eight_bit.roles().brand.primary, eight_bit.background()),
            Color::Indexed(241)
        );

        let ansi = theme("ansi-dark").with_color_mode(no_color);
        let r = ansi.roles();
        for c in [r.brand.primary, r.status.error, r.text.plain, r.surface.pane] {
            assert_eq!(ansi.color(c, ansi.background()), Color::Reset);
        }
        assert_eq!(mark(&ansi, |m| m.starred), Color::Reset);
    }

    #[test]
    fn ansi_themes_pass_through_at_every_depth() {
        for depth in [ColorDepth::TrueColor, ColorDepth::EightBit, ColorDepth::Standard] {
            let ansi = theme("ansi-dark").with_color_mode(ColorMode { depth, no_color: false });
            assert_eq!(ansi.color(ansi.roles().brand.primary, ansi.background()), Color::Blue);
            assert_eq!(ansi.paint(ansi.background()), Color::Reset);
        }
    }

    fn rgb(hex: u32) -> Color {
        let [_, r, g, b] = hex.to_be_bytes();
        Color::Rgb(r, g, b)
    }

    /// One cell of `fg` on `bg` with `modifier`, through [`Theme::resolve_dim`].
    fn resolved(theme: &Theme, fg: Color, bg: Color, modifier: Modifier) -> (Color, Color, Modifier) {
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        buf[(0, 0)]
            .set_symbol("x")
            .set_style(Style::new().fg(fg).bg(bg).add_modifier(modifier));
        theme.resolve_dim(&mut buf, area);
        let cell = &buf[(0, 0)];
        (cell.fg, cell.bg, cell.modifier)
    }

    /// Textual's `dim_color`, checked against its output (a probe on Textual 8.2.5).
    #[rstest]
    #[case("dark", 0xff_a6_2b, 0x24_2f_38, 0xb4_7d_2f)]
    #[case("dark", 0xe0_e0_e0, 0x1e_1e_1e, 0x9e_9e_9e)]
    #[case("dark", 0xe0_e0_e0, 0x27_27_27, 0xa1_a1_a1)]
    #[case("dark", 0xdd_ed_f9, 0x01_78_d4, 0x92_c5_ec)]
    #[case("nord", 0x2e_34_40, 0x88_c0_d0, 0x4c_63_70)]
    fn resolve_dim_blends_the_fg_from_the_cells_bg(
        #[case] name: &str,
        #[case] fg: u32,
        #[case] bg: u32,
        #[case] dimmed: u32,
    ) {
        assert_eq!(
            resolved(&theme(name), rgb(fg), rgb(bg), Modifier::DIM),
            (rgb(dimmed), rgb(bg), Modifier::empty())
        );
    }

    #[test]
    fn resolve_dim_keeps_the_other_modifiers_and_skips_undimmed_cells() {
        let dark = theme("dark");
        let (fg, bg) = (rgb(0xff_a6_2b), rgb(0x24_2f_38));
        assert_eq!(
            resolved(&dark, fg, bg, Modifier::DIM | Modifier::BOLD | Modifier::ITALIC),
            (rgb(0xb4_7d_2f), bg, Modifier::BOLD | Modifier::ITALIC)
        );
        assert_eq!(resolved(&dark, fg, bg, Modifier::BOLD), (fg, bg, Modifier::BOLD));
    }

    #[test]
    fn resolve_dim_falls_back_to_the_screen_background_and_keeps_dim_without_an_fg() {
        let dark = theme("dark");
        // `#e0e0e0` over the screen's `#121212`.
        assert_eq!(
            resolved(&dark, rgb(0xe0_e0_e0), Color::Reset, Modifier::DIM),
            (rgb(0x99_99_99), Color::Reset, Modifier::empty())
        );
        let bg = rgb(0x1e_1e_1e);
        assert_eq!(
            resolved(&dark, Color::Reset, bg, Modifier::DIM),
            (Color::Reset, bg, Modifier::DIM)
        );
    }

    #[test]
    fn resolve_dim_leaves_the_ansi_themes_to_the_terminal() {
        for name in ["ansi-dark", "ansi-light"] {
            let ansi = theme(name);
            for (fg, bg) in [
                (Color::Black, Color::Gray),
                (Color::Reset, Color::Reset),
                (rgb(0xe0_e0_e0), rgb(0x1e_1e_1e)),
            ] {
                let modifier = Modifier::DIM | Modifier::BOLD;
                assert_eq!(resolved(&ansi, fg, bg, modifier), (fg, bg, modifier), "{name}");
            }
        }
    }

    /// Reduced colors are blended as the RGB they stand for and reduced again.
    #[test]
    fn resolve_dim_reduces_the_blend_to_the_color_mode() {
        let eight_bit = theme("dark").with_color_mode(ColorMode {
            depth: ColorDepth::EightBit,
            no_color: false,
        });
        let (fg, _, modifier) = resolved(&eight_bit, Color::Indexed(254), Color::Indexed(234), Modifier::DIM);
        assert!(matches!(fg, Color::Indexed(_)), "{fg:?}");
        assert_eq!(modifier, Modifier::empty());
    }

    /// A role color as written: `#RRGGBB`, with its alpha if it has one, `auto`, an ANSI name or `transparent`.
    fn raw(c: ThemeColor) -> String {
        match c {
            ThemeColor::Rgba { r, g, b, a } if a >= 1.0 => format!("#{r:02X}{g:02X}{b:02X}"),
            ThemeColor::Rgba { r, g, b, a } => format!("#{r:02X}{g:02X}{b:02X} a={a}"),
            ThemeColor::Auto { alpha } => format!("auto a={alpha}"),
            ThemeColor::Ansi(c) => ansi_name(c),
            ThemeColor::Transparent => "transparent".to_owned(),
        }
    }

    /// Textual's name for an ANSI color: `ansi_bright_black`.
    fn ansi_name(c: AnsiColor) -> String {
        let mut name = "ansi".to_owned();
        for ch in format!("{c:?}").chars() {
            if ch.is_uppercase() {
                name.push('_');
            }
            name.extend(ch.to_lowercase());
        }
        name
    }

    fn paint_name(paint: Paint) -> String {
        match paint {
            Paint::Rgb(Rgb { r, g, b }) => format!("#{r:02X}{g:02X}{b:02X}"),
            Paint::Ansi(c) => ansi_name(c),
        }
    }

    fn style_name(style: TextStyle) -> String {
        match (style.add.is_empty(), style.sub.is_empty()) {
            (true, true) => "none".to_owned(),
            (false, true) => format!("add {:?}", style.add),
            (true, false) => format!("sub {:?}", style.sub),
            (false, false) => format!("add {:?}, sub {:?}", style.add, style.sub),
        }
    }

    /// Every role of `theme`, one per line: its path, the color resolved over [`Theme::background`] at truecolor
    /// with the value as written, the text style, or the flag.
    fn roles_listing(theme: &Theme) -> String {
        use std::fmt::Write as _;

        use roles::walk::{Leaf, Walk};

        let mut leaves = Vec::new();
        theme.roles().walk("", &mut leaves);
        let screen = theme.background();
        let color = |c: ThemeColor| format!("{}  ({})", paint_name(c.over(screen)), raw(c));
        let mut out = String::new();
        for (path, leaf) in leaves {
            let value = match leaf {
                Leaf::Color(c) | Leaf::MaybeColor(Some(c)) => color(c),
                Leaf::MaybeColor(None) => "none".to_owned(),
                Leaf::Style(style) => style_name(style),
                Leaf::Flag(flag) => flag.to_string(),
            };
            writeln!(out, "{path:<38} {value}").unwrap();
        }
        out
    }

    /// The regression guard for the role data: every role of every built-in theme.
    #[test]
    fn roles_snapshot() {
        for theme in Theme::all() {
            insta::assert_snapshot!(format!("roles_{}", theme.name), roles_listing(&theme));
        }
    }

    /// Python's names for Textual's own themes draw exactly as their renamed themes, from the same roles.
    #[rstest]
    #[case("textual-dark", "dark")]
    #[case("textual-light", "light")]
    fn aliases_draw_as_their_theme(#[case] alias: &str, #[case] name: &str) {
        let drawn = Theme::from_config(alias, ColorMode::default());
        assert_eq!(drawn.name, name);
        assert!(std::ptr::eq(drawn.roles(), theme(name).roles()));
    }

    #[test]
    fn roles_are_per_theme() {
        assert!(std::ptr::eq(
            theme("nord").roles(),
            Theme::by_name("nord").unwrap().roles()
        ));
        assert!(!std::ptr::eq(theme("nord").roles(), theme("dark").roles()));
        assert_eq!(Theme::default().roles(), theme("dark").roles());
    }

    /// The marks stay the same in every built-in theme, as the Python app's literal markup colors do.
    #[test]
    fn marks_are_fixed_across_themes() {
        let first = Theme::default().roles();
        for t in Theme::all() {
            let r = t.roles();
            assert_eq!(r.marks.positive, first.marks.positive, "{}", t.name);
            assert_eq!(r.marks.negative, first.marks.negative, "{}", t.name);
            assert_eq!(r.marks.merged, first.marks.merged, "{}", t.name);
            assert_eq!(r.marks.pending, first.marks.pending, "{}", t.name);
        }
    }
}
