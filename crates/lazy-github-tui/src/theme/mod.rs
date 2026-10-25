//! Themes: Textual's 21 built-in themes, by name, with the exact colors Textual paints.
//!
//! Textual derives its CSS variables (`$panel`, `$primary-lighten-3`, …) in Python at runtime. Rather than port that,
//! `gen/dump_themes.py` dumps Textual 8.2.5's results into the generated, committed `palette` module. Colors with
//! alpha or `auto` are composited over what they're drawn on with Textual's arithmetic ([`ThemeColor::over`]), so a
//! cell gets the same RGB in both apps. Components take every color from here.

mod accent;
mod color;
#[rustfmt::skip]
#[allow(clippy::enum_glob_use, clippy::unreadable_literal)]
mod palette;

use std::fmt;

use ratatui::style::{Color, Style};

pub use accent::{Accent, RichAnsi};
pub use color::{AnsiColor, Paint, Rgb, TextStyle, ThemeColor};
pub use palette::Palette;

use palette::{BUILTIN, ThemeDef};

/// The theme Python falls back to for an unknown name, and the default.
const DEFAULT: &str = "textual-dark";

/// One of Textual's built-in themes. A small `Copy` handle onto its static palette.
#[derive(Clone, Copy, PartialEq)]
pub struct Theme {
    /// Textual's name for it, as written in the config (`textual-dark`).
    pub name: &'static str,
    /// Whether it's a dark theme.
    pub dark: bool,
    /// Whether it uses the terminal's own 16 colors (`ansi-dark`, `ansi-light`).
    pub ansi: bool,
    /// Its colors and text styles.
    pub palette: &'static Palette,
}

impl Theme {
    fn from_def(def: &'static ThemeDef) -> Self {
        Self {
            name: def.name,
            dark: def.dark,
            ansi: def.ansi,
            palette: &def.palette,
        }
    }

    /// The built-in theme called exactly `name`.
    pub fn by_name(name: &str) -> Option<Theme> {
        BUILTIN.iter().find(|def| def.name == name).map(Self::from_def)
    }

    /// The theme for `appearance.theme`: an unknown name renders as `textual-dark`, silently, as in Python.
    pub fn from_config(name: &str) -> Theme {
        Self::by_name(name).unwrap_or_else(|| {
            tracing::debug!("unknown theme {name:?}, using {DEFAULT}");
            Self::default()
        })
    }

    /// Every built-in theme, in Textual's order.
    pub fn all() -> impl Iterator<Item = Theme> {
        BUILTIN.iter().map(Self::from_def)
    }

    /// The name the Python app shows for this theme: `name.title().replace("-", " ")`, so `rose-pine-moon` is
    /// "Rose Pine Moon".
    pub fn display_name(&self) -> String {
        // `str.title()` capitalizes a letter that follows anything other than a letter, and lowercases the rest.
        let mut after_letter = false;
        let mut out = String::with_capacity(self.name.len());
        for c in self.name.chars() {
            if !c.is_alphabetic() {
                out.push(if c == '-' { ' ' } else { c });
            } else if after_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            after_letter = c.is_alphabetic();
        }
        out
    }

    /// What the screen is painted with (`$background`), which everything else is drawn over.
    pub fn background(&self) -> Paint {
        self.palette.background.over(Paint::Ansi(AnsiColor::Default))
    }

    /// The terminal color for `c` drawn over `under`.
    pub fn color(&self, c: ThemeColor, under: Paint) -> Color {
        c.over(under).into()
    }

    /// A style for `fg` text on a `bg` background, with `bg` itself drawn over `under`: "`$text` on `$panel`".
    pub fn fg_on(&self, fg: ThemeColor, bg: ThemeColor, under: Paint) -> Style {
        let bg = bg.over(under);
        Style::new().fg(fg.over(bg).into()).bg(bg.into())
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

    /// A fixed markup color, the same in every theme.
    pub fn accent(&self, a: Accent) -> Color {
        a.rgb().into()
    }

    /// A Rich ANSI color in a table cell: the terminal's own color under the ANSI themes, and Textual's terminal theme
    /// otherwise.
    pub fn rich_ansi(&self, c: RichAnsi) -> Color {
        if self.ansi {
            match c {
                RichAnsi::Green => Color::Green,
            }
        } else if self.dark {
            c.dark().into()
        } else {
            c.light().into()
        }
    }
}

impl Default for Theme {
    /// `textual-dark`, Textual's default theme.
    fn default() -> Self {
        Self::from_def(&BUILTIN[0])
    }
}

impl fmt::Debug for Theme {
    // The palette has 166 entries; the name says which one it is.
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
        "textual-dark",
        "textual-light",
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

    fn hex(text: &str) -> ThemeColor {
        let value = u32::from_str_radix(text.trim_start_matches('#'), 16).unwrap();
        let [_, r, g, b] = value.to_be_bytes();
        ThemeColor::Rgba { r, g, b, a: 1.0 }
    }

    #[test]
    fn all_lists_textuals_themes_in_order() {
        assert_eq!(Theme::all().map(|t| t.name).collect::<Vec<_>>(), NAMES);
        assert_eq!(Theme::default().name, "textual-dark");
    }

    #[test]
    fn by_name_finds_every_builtin_theme() {
        for name in NAMES {
            assert_eq!(Theme::by_name(name).map(|t| t.name), Some(name));
            assert_eq!(Theme::from_config(name).name, name);
        }
    }

    #[rstest]
    #[case("Nord")]
    #[case("")]
    #[case("no-such-theme")]
    fn unknown_names_render_as_textual_dark(#[case] name: &str) {
        assert_eq!(Theme::by_name(name), None);
        assert_eq!(Theme::from_config(name), theme("textual-dark"));
    }

    #[test]
    fn palettes_match_textuals_output() {
        let dark = theme("textual-dark").palette;
        assert_eq!(dark.primary_lighten_3, hex("#6DB2FF"));
        assert_eq!(dark.success, hex("#4EBF71"));
        assert_eq!(dark.secondary, hex("#004578"));
        assert_eq!(dark.panel, hex("#242F38"));
        assert_eq!(dark.background, hex("#121212"));

        let nord = theme("nord").palette;
        assert_eq!(
            nord.block_cursor_text_style,
            TextStyle {
                add: ratatui::style::Modifier::empty(),
                sub: ratatui::style::Modifier::empty()
            }
        );

        let ansi = theme("ansi-dark");
        assert!(ansi.ansi && ansi.dark);
        assert_eq!(ansi.palette.primary, ThemeColor::Ansi(AnsiColor::Blue));
        assert_eq!(ansi.palette.surface, ThemeColor::Transparent);
        assert_eq!(ansi.background(), Paint::Ansi(AnsiColor::Default));
        assert_eq!(ansi.color(ansi.palette.primary, ansi.background()), Color::Blue);
    }

    #[test]
    fn tint_passes_ansi_through() {
        let ansi = theme("ansi-dark");
        let surface = ansi.palette.surface.over(ansi.background());
        assert_eq!(surface, Paint::Ansi(AnsiColor::Default));
        assert_eq!(ansi.tint(surface, ansi.palette.foreground, 5.0), surface);
        // An ANSI tint over an RGB base also leaves the base alone, unlike `over`.
        let base = Paint::Rgb(Rgb::new(30, 30, 30));
        assert_eq!(ansi.tint(base, ThemeColor::Ansi(AnsiColor::Red), 5.0), base);
        assert_eq!(
            theme("textual-dark").tint(base, hex("#E0E0E0"), 5.0),
            Paint::Rgb(Rgb::new(39, 39, 39))
        );
    }

    #[rstest]
    #[case("textual-dark", "Textual Dark")]
    #[case("catppuccin-mocha", "Catppuccin Mocha")]
    #[case("rose-pine-moon", "Rose Pine Moon")]
    #[case("ansi-light", "Ansi Light")]
    fn display_name_is_pythons_title_case(#[case] name: &str, #[case] display: &str) {
        assert_eq!(theme(name).display_name(), display);
    }

    #[test]
    fn display_name_capitalizes_after_any_non_letter() {
        let odd = Theme {
            name: "a3d-x_y",
            ..Theme::default()
        };
        assert_eq!(odd.display_name(), "A3D X_Y");
    }

    #[test]
    fn fixed_colors_ignore_the_theme() {
        let dark = theme("textual-dark");
        let light = theme("textual-light");
        for t in [dark, light, theme("nord")] {
            assert_eq!(t.accent(Accent::GreenYellow), Color::Rgb(0xAD, 0xFF, 0x2F));
            assert_eq!(t.accent(Accent::Orchid), Color::Rgb(0xDA, 0x70, 0xD6));
            assert_eq!(t.accent(Accent::Red), Color::Rgb(0xFF, 0x00, 0x00));
            assert_eq!(t.accent(Accent::Yellow), Color::Rgb(0xFF, 0xFF, 0x00));
            assert_eq!(t.accent(Accent::Cyan), Color::Rgb(0x00, 0xFF, 0xFF));
            assert_eq!(t.accent(Accent::Green), Color::Rgb(0x00, 0x80, 0x00));
        }
        assert_eq!(dark.rich_ansi(RichAnsi::Green), Color::Rgb(0x98, 0xE0, 0x24));
        assert_eq!(light.rich_ansi(RichAnsi::Green), Color::Rgb(0x44, 0x8C, 0x27));
        assert_eq!(theme("ansi-light").rich_ansi(RichAnsi::Green), Color::Green);
    }

    /// A bordered block with a focused and an unfocused border, a `$text` on `$panel` line and a blurred cursor row.
    fn themed_widget(theme: &Theme) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
        terminal
            .draw(|frame| {
                let p = theme.palette;
                let screen = theme.background();
                frame.render_widget(
                    Block::new().style(Style::new().fg(theme.color(p.foreground, screen)).bg(screen.into())),
                    frame.area(),
                );
                let surface = p.surface.over(screen);
                let [focused, blurred] = Layout::vertical([Constraint::Length(4); 2]).areas(frame.area());
                for (area, border, row) in [
                    (
                        focused,
                        p.success,
                        Line::styled("text on panel", theme.fg_on(p.text, p.panel, surface)),
                    ),
                    (
                        blurred,
                        p.primary_lighten_3,
                        Line::styled(
                            "blurred cursor",
                            p.block_cursor_blurred_text_style.apply(theme.fg_on(
                                p.block_cursor_blurred_foreground,
                                p.block_cursor_blurred_background,
                                surface,
                            )),
                        ),
                    ),
                ] {
                    let block = Block::bordered()
                        .border_style(Style::new().fg(theme.color(border, surface)))
                        .style(Style::new().bg(surface.into()));
                    frame.render_widget(Paragraph::new(row).block(block), area);
                }
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[rstest]
    #[case("textual-dark")]
    #[case("nord")]
    #[case("textual-light")]
    #[case("ansi-dark")]
    fn themed_widget_snapshot(#[case] name: &str) {
        insta::assert_debug_snapshot!(format!("themed_widget_{name}"), themed_widget(&theme(name)));
    }
}
