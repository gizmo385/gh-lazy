//! Checks [`downsample`] against Rich's own `Color.downgrade`, for every opaque RGB variable of every built-in theme.
//!
//! `fixtures/themes/downgrade.json` is written by `src/theme/gen/dump_themes.py --downgrade`: for each theme variable,
//! its RGB and the xterm-256 and 16-color indices Rich picks for it.

use lazy_github_tui::theme::{ColorDepth, ColorMode, Rgb, Theme, downsample};
use ratatui::style::Color;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/themes/downgrade.json");

/// The 16 ANSI colors in index order, as ratatui names them.
const STANDARD: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

fn mode(depth: ColorDepth) -> ColorMode {
    ColorMode { depth, no_color: false }
}

fn rgb(hex: &str) -> Rgb {
    let n = u32::from_str_radix(hex.strip_prefix('#').unwrap(), 16).unwrap();
    let [_, r, g, b] = n.to_be_bytes();
    Rgb::new(r, g, b)
}

#[test]
fn downsampling_matches_rich_for_every_theme_color() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let themes = fixture["themes"].as_object().unwrap();
    assert_eq!(
        themes.keys().map(String::as_str).collect::<Vec<_>>(),
        Theme::all().map(|t| t.name).collect::<Vec<_>>()
    );

    let mut checked = 0;
    for (name, variables) in themes {
        for (var, row) in variables.as_object().unwrap() {
            let row = row.as_array().unwrap();
            let color = rgb(row[0].as_str().unwrap());
            let eight_bit = u8::try_from(row[1].as_u64().unwrap()).unwrap();
            let standard = STANDARD[usize::try_from(row[2].as_u64().unwrap()).unwrap()];
            assert_eq!(
                downsample(color, mode(ColorDepth::EightBit)),
                Color::Indexed(eight_bit),
                "{name}: ${var} {color:?} at 8-bit"
            );
            assert_eq!(
                downsample(color, mode(ColorDepth::Standard)),
                standard,
                "{name}: ${var} {color:?} at 16 colors"
            );
            assert_eq!(
                downsample(color, mode(ColorDepth::TrueColor)),
                Color::from(color),
                "{name}: ${var}"
            );
            checked += 1;
        }
    }
    // Every RGB theme has over a hundred opaque variables; the two ANSI themes have none.
    assert!(checked > 2000, "only {checked} colors checked");
}
