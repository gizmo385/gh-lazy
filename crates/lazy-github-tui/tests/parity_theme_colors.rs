//! The parity harness's focus colors (`parity/themes.toml`, measured from Python screen dumps) are the built-in
//! themes' own, so a role edit that moves one fails here rather than as a puzzling focus miss in a scenario.
//!
//! Each entry names a theme as a config's `appearance.theme` does: a name the catalog doesn't know draws as the
//! default theme. The catalog has the built-in themes and the user themes of every seed
//! (`parity/seeds/*/home/.config/lazy-github/themes/`), so a scenario's user theme is checked like a built-in one. Its colors are the theme's roles composited over the pane at truecolor, written as the grid dumps
//! them: `#rrggbb`, or `idx:<n>` for one of the terminal's 16 colors.

use std::path::Path;

use lazy_github_tui::theme::{AnsiColor, Catalog, Paint, Theme, ThemeColor};

const THEMES: &str = include_str!("../../../parity/themes.toml");

/// The catalog of each seed that has user theme files, with any it skipped (seeds may hold broken files on purpose).
fn seed_catalogs() -> Vec<Catalog> {
    let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/seeds");
    let mut dirs: Vec<_> = std::fs::read_dir(seeds)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("home/.config/lazy-github/themes"))
        .filter(|dir| dir.is_dir())
        .collect();
    dirs.sort();
    dirs.iter().map(|dir| Catalog::with_user_themes(dir).0).collect()
}

/// The theme `name` stands for in the built-in catalog or a seed's, if any.
fn lookup(catalogs: &[Catalog], name: &str) -> Option<Theme> {
    Theme::by_name(name).or_else(|| catalogs.iter().find_map(|catalog| catalog.get(name)))
}

/// The color `role` paints over `theme`'s pane, as the grid dumps write it.
fn dumped(theme: &Theme, role: ThemeColor) -> String {
    let pane = theme.roles().surface.pane.over(theme.background());
    match role.over(pane) {
        Paint::Rgb(c) => format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
        Paint::Ansi(c) => {
            let index = match c {
                AnsiColor::Black => 0,
                AnsiColor::Red => 1,
                AnsiColor::Green => 2,
                AnsiColor::Yellow => 3,
                AnsiColor::Blue => 4,
                AnsiColor::Magenta => 5,
                AnsiColor::Cyan => 6,
                AnsiColor::White => 7,
                AnsiColor::BrightBlack => 8,
                AnsiColor::BrightRed => 9,
                AnsiColor::BrightGreen => 10,
                AnsiColor::BrightYellow => 11,
                AnsiColor::BrightBlue => 12,
                AnsiColor::BrightMagenta => 13,
                AnsiColor::BrightCyan => 14,
                AnsiColor::BrightWhite => 15,
                AnsiColor::Default => panic!("the terminal's default color has no index"),
            };
            format!("idx:{index}")
        }
    }
}

#[test]
fn the_harness_focus_colors_are_the_themes_own() {
    let entries: toml::Table = toml::from_str(THEMES).unwrap();
    assert!(!entries.is_empty());
    let catalogs = seed_catalogs();
    let mut checked = 0;
    for (name, entry) in &entries {
        let theme = lookup(&catalogs, name).unwrap_or_default();
        let roles = theme.roles();
        let entry = entry.as_table().unwrap_or_else(|| panic!("[{name}] is a table"));
        for (key, value) in entry {
            let role = match key.as_str() {
                "focus" => roles.border.pane_focused,
                "border" => roles.border.field_focused,
                "block_cursor" => roles.cursor.focused.bg,
                other => panic!("parity/themes.toml [{name}] has {other:?}; check it here"),
            };
            let want = value.as_str().unwrap_or_else(|| panic!("[{name}] {key} is a string"));
            assert_eq!(
                dumped(&theme, role),
                want.to_ascii_lowercase(),
                "[{name}] {key} (theme {})",
                theme.name
            );
            checked += 1;
        }
    }
    assert!(checked >= entries.len());
}

/// The fallback entry is checked against the default theme, not skipped.
#[test]
fn an_unknown_name_is_checked_against_the_default_theme() {
    let entries: toml::Table = toml::from_str(THEMES).unwrap();
    let catalogs = seed_catalogs();
    assert!(entries.keys().any(|name| lookup(&catalogs, name).is_none()));
}

/// The `user-theme` seed's theme is found, so its entry is checked against its own colors.
#[test]
fn a_seeds_user_theme_is_checked_as_itself() {
    let theme = lookup(&seed_catalogs(), "my-theme").expect("the user-theme seed's my-theme");
    assert_eq!(theme.name, "my-theme");
    assert_ne!(theme.roles(), Theme::default().roles());
}
