//! Checks [`ThemeColor::over`] against Textual's own compositing, for every built-in theme.
//!
//! `fixtures/themes/composites.json` is written by `src/theme/gen/dump_themes.py`, which also generates the palettes:
//! for each theme, Textual's `Color.parse(bg) + Color.parse(fg)` for a handful of pairs the app draws.

use lazy_github_tui::theme::{AnsiColor, Paint, Palette, Rgb, Theme, ThemeColor};
use ratatui::style::Color;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/themes/composites.json");

/// A pair's operand, as Textual CSS.
fn operand(palette: &Palette, css: &str) -> ThemeColor {
    match css {
        "$text" => palette.text,
        "$text-muted" => palette.text_muted,
        "$foreground-muted" => palette.foreground_muted,
        "$background" => palette.background,
        "$surface" => palette.surface,
        "$panel" => palette.panel,
        "$panel-lighten-1" => palette.panel_lighten_1,
        "$block-cursor-foreground" => palette.block_cursor_foreground,
        "$block-cursor-background" => palette.block_cursor_background,
        "$block-cursor-blurred-background" => palette.block_cursor_blurred_background,
        "$boost" => palette.boost,
        "$foreground 5%" => palette.foreground.multiply_alpha(5.0 / 100.0),
        other => panic!("composites.json names {other:?}; add it here"),
    }
}

/// Textual's result: `#RRGGBB`, or an ANSI color by name.
fn expected(result: &str) -> Paint {
    if let Some(hex) = result.strip_prefix('#') {
        let n = u32::from_str_radix(hex, 16).unwrap();
        let [_, r, g, b] = n.to_be_bytes();
        return Paint::Rgb(Rgb::new(r, g, b));
    }
    let names = [
        ("black", AnsiColor::Black),
        ("red", AnsiColor::Red),
        ("green", AnsiColor::Green),
        ("yellow", AnsiColor::Yellow),
        ("blue", AnsiColor::Blue),
        ("magenta", AnsiColor::Magenta),
        ("cyan", AnsiColor::Cyan),
        ("white", AnsiColor::White),
        ("bright_black", AnsiColor::BrightBlack),
        ("bright_red", AnsiColor::BrightRed),
        ("bright_green", AnsiColor::BrightGreen),
        ("bright_yellow", AnsiColor::BrightYellow),
        ("bright_blue", AnsiColor::BrightBlue),
        ("bright_magenta", AnsiColor::BrightMagenta),
        ("bright_cyan", AnsiColor::BrightCyan),
        ("bright_white", AnsiColor::BrightWhite),
        ("default", AnsiColor::Default),
    ];
    let name = result
        .strip_prefix("ansi_")
        .unwrap_or_else(|| panic!("unexpected result {result:?}"));
    let (_, ansi) = names.into_iter().find(|(n, _)| *n == name).unwrap();
    Paint::Ansi(ansi)
}

#[test]
fn compositing_matches_textual_for_every_theme() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let themes = fixture["themes"].as_object().unwrap();
    assert_eq!(
        themes.keys().map(String::as_str).collect::<Vec<_>>(),
        Theme::all().map(|t| t.name).collect::<Vec<_>>()
    );

    let mut checked = 0;
    for (name, pairs) in themes {
        let theme = Theme::by_name(name).unwrap();
        for pair in pairs.as_array().unwrap() {
            let (fg, bg) = (pair["fg"].as_str().unwrap(), pair["bg"].as_str().unwrap());
            let want = expected(pair["result"].as_str().unwrap());
            // Textual parses a transparent background as black with zero alpha, and blends with that black.
            let under = operand(theme.palette, bg).over(Paint::Rgb(Rgb::BLACK));
            let fg_color = operand(theme.palette, fg);
            assert_eq!(fg_color.over(under), want, "{name}: {fg} over {bg}");
            let want_color = match want {
                Paint::Rgb(c) => Color::Rgb(c.r, c.g, c.b),
                Paint::Ansi(c) => Color::from(c),
            };
            assert_eq!(theme.color(fg_color, under), want_color, "{name}: {fg} over {bg}");
            match (want, under) {
                // The ANSI themes paint the terminal's own colors.
                (Paint::Ansi(_), _) => assert!(theme.ansi, "{name}: {fg} over {bg} is ANSI"),
                // The focused DataTable/Log tint, through the theme's helper.
                (Paint::Rgb(want), Paint::Rgb(base)) if fg == "$foreground 5%" => {
                    assert_eq!(
                        theme.tint(Paint::Rgb(base), theme.palette.foreground, 5.0),
                        Paint::Rgb(want),
                        "{name}: tint"
                    );
                }
                (Paint::Rgb(_), _) => {}
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 21 * 10);
}
