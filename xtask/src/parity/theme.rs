//! The colors the harness looks for on screen, read from the app's own themes (`lazy_github_tui::theme`): the
//! focused pane's border, which structure extraction marks as focus, and the three focus markers `wait.focused`
//! reads. A scenario's theme is its config's `appearance.theme`, resolved the way the app resolves it: the built-in
//! themes and the user themes its seed puts in the config folder, an alias as its theme, an unknown name as the
//! default theme.

use std::path::Path;

use lazy_github_tui::theme::{AnsiColor, Catalog, Paint, Theme, ThemeColor};
use serde_json::Value;

use super::env::scenario_config;
use super::focus::FocusColors;
use super::scenario::{Roots, Scenario};
use super::structure::ThemeColors;
use super::{HarnessError, Result};

/// Where a seed keeps user theme files, relative to the seed: the config folder's `themes/`.
const SEED_THEMES: &str = "home/.config/lazy-github/themes";

/// What a scenario's run draws focus in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioColors {
    /// For structure extraction.
    pub theme: ThemeColors,
    /// For `wait.focused`.
    pub focus: FocusColors,
}

/// The colors of the scenario's theme.
pub fn scenario_colors(scenario: &Scenario, roots: &Roots) -> Result<ScenarioColors> {
    let config = scenario_config(scenario, roots)?;
    let name = match config.pointer("/appearance/theme") {
        None => lazy_github_core::config::DEFAULT_THEME,
        Some(Value::String(name)) => name.as_str(),
        Some(other) => {
            return Err(HarnessError(format!(
                "{}: appearance.theme is {other}, not a theme name",
                scenario.path.display()
            )));
        }
    };
    let seed_themes = scenario
        .env
        .seed
        .as_ref()
        .map(|seed| roots.seed_dir(seed).join(SEED_THEMES));
    Ok(colors(name, seed_themes.as_deref()))
}

/// The colors of theme `name`, with the user themes in `user_themes` (a seed's themes folder) alongside the built-in
/// ones.
pub fn colors(name: &str, user_themes: Option<&Path>) -> ScenarioColors {
    let theme = match user_themes.filter(|dir| dir.is_dir()) {
        // Parsing leaks each theme, as the app does once per process; a run reads a seed's themes once per size.
        Some(dir) => Catalog::with_user_themes(dir).0.resolve(name).theme_or_default(),
        None => Catalog::builtin().resolve(name).theme_or_default(),
    };
    let roles = theme.roles();
    let pane = grid_color(&theme, roles.border.pane_focused);
    ScenarioColors {
        theme: ThemeColors {
            name: name.to_owned(),
            focus: Some(pane.clone()),
        },
        focus: FocusColors {
            pane: Some(pane),
            border: Some(grid_color(&theme, roles.border.field_focused)),
            block_cursor: Some(grid_color(&theme, roles.cursor.focused.bg)),
        },
    }
}

/// The color `role` paints over `theme`'s pane at truecolor, as a grid dump writes it: `#rrggbb`, or `idx:<n>` for
/// one of the terminal's 16 colors. The terminal's default color has no index, so it's written as the dump writes an
/// unset color's absence, which matches nothing.
pub fn grid_color(theme: &Theme, role: ThemeColor) -> String {
    let pane = theme.roles().surface.pane.over(theme.background());
    match role.over(pane) {
        Paint::Rgb(c) => format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
        Paint::Ansi(c) => match ansi_index(c) {
            Some(index) => format!("idx:{index}"),
            None => "default".to_owned(),
        },
    }
}

fn ansi_index(color: AnsiColor) -> Option<u8> {
    Some(match color {
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
        AnsiColor::Default => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(name: &str, user_themes: Option<&Path>) -> [String; 3] {
        let colors = colors(name, user_themes);
        assert_eq!(colors.theme.name, name);
        assert_eq!(colors.theme.focus, colors.focus.pane);
        let FocusColors {
            pane,
            border,
            block_cursor,
        } = colors.focus;
        [pane, border, block_cursor].map(Option::unwrap)
    }

    /// The values `parity/themes.toml` had, measured from the Python app's screens, before the harness read the
    /// themes.
    #[test]
    fn a_truecolor_theme_gives_its_rgb_colors() {
        assert_eq!(focus("textual-dark", None), ["#4ebf71", "#0178d4", "#0178d4"]);
        assert_eq!(focus("catppuccin-latte", None)[0], "#40a02b");
    }

    #[test]
    fn an_ansi_theme_gives_indexed_colors() {
        assert_eq!(focus("ansi-dark", None)[0], "idx:2");
    }

    #[test]
    fn an_unknown_name_gives_the_default_theme() {
        assert_eq!(focus("no-such-theme", None), focus("textual-dark", None));
        assert_eq!(
            focus(lazy_github_core::config::DEFAULT_THEME, None),
            focus("textual-dark", None)
        );
    }

    #[test]
    fn a_seeds_user_theme_is_found() {
        let seed = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../parity/seeds/user-theme")
            .join(SEED_THEMES);
        assert_eq!(focus("my-theme", Some(&seed))[0], "#ff8800");
        // Without the seed's folder, the name is unknown.
        assert_eq!(focus("my-theme", None), focus("textual-dark", None));
    }
}
