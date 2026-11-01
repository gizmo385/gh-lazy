//! Checks [`ThemeColor::over`] against Textual's own compositing, for every built-in theme.
//!
//! `tests/fixtures/themes/composites.json` holds test vectors frozen from Textual 8.2.5: for each theme, Textual's
//! `Color.parse(bg) + Color.parse(fg)` for a handful of pairs the app draws, each operand keyed by the role that holds
//! that color. No generator exists any more; the vectors are data, like the theme files.

use ratatui::style::Color;
use serde_json::Value;

use super::roles::walk::{Leaf, Walk};
use super::{AnsiColor, Paint, Rgb, Theme, ThemeColor};

const FIXTURE: &str = include_str!("../../tests/fixtures/themes/composites.json");

/// The color of the role at `path`.
fn role(theme: &Theme, path: &str) -> ThemeColor {
    let mut leaves = Vec::new();
    theme.roles().walk("", &mut leaves);
    match leaves.into_iter().find(|(p, _)| p == path) {
        Some((_, Leaf::Color(c))) => c,
        other => panic!("composites.json names {path:?}, which is {other:?}, not a color role"),
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
            let under = role(&theme, bg).over(Paint::Rgb(Rgb::BLACK));
            let fg_color = role(&theme, fg);
            assert_eq!(fg_color.over(under), want, "{name}: {fg} over {bg}");
            let want_color = match want {
                Paint::Rgb(c) => Color::Rgb(c.r, c.g, c.b),
                Paint::Ansi(c) => Color::from(c),
            };
            assert_eq!(theme.color(fg_color, under), want_color, "{name}: {fg} over {bg}");
            match (want, under) {
                // The ANSI themes paint the terminal's own colors.
                (Paint::Ansi(_), _) => assert!(theme.terminal_colors, "{name}: {fg} over {bg} is ANSI"),
                // The focused DataTable/Log tint (`$foreground 5%`), through the theme's helper.
                (Paint::Rgb(want), Paint::Rgb(base)) if fg == "surface.focus_tint" => {
                    assert_eq!(
                        theme.tint(Paint::Rgb(base), theme.roles().text.plain, 5.0),
                        Paint::Rgb(want),
                        "{name}: tint"
                    );
                }
                (Paint::Rgb(_), _) => {}
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 21 * 9);
}
