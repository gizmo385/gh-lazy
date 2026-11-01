//! The themes the app knows by name: the built-in ones, each a TOML file in `crates/lazy-github-tui/themes/`.

use std::sync::LazyLock;

use super::Theme;
use super::depth::ColorMode;
use super::file::{ThemeFile, ThemeFileError};

/// The built-in themes, by name, with their files' text, in Textual's order, which is the order Settings lists them
/// in. Python users' configs name these, so the names stay as Textual has them.
const BUILTIN: [(&str, &str); 21] = [
    ("textual-dark", include_str!("../../themes/textual-dark.toml")),
    ("textual-light", include_str!("../../themes/textual-light.toml")),
    ("nord", include_str!("../../themes/nord.toml")),
    ("gruvbox", include_str!("../../themes/gruvbox.toml")),
    ("catppuccin-mocha", include_str!("../../themes/catppuccin-mocha.toml")),
    ("dracula", include_str!("../../themes/dracula.toml")),
    ("tokyo-night", include_str!("../../themes/tokyo-night.toml")),
    ("monokai", include_str!("../../themes/monokai.toml")),
    ("flexoki", include_str!("../../themes/flexoki.toml")),
    ("catppuccin-latte", include_str!("../../themes/catppuccin-latte.toml")),
    ("catppuccin-frappe", include_str!("../../themes/catppuccin-frappe.toml")),
    (
        "catppuccin-macchiato",
        include_str!("../../themes/catppuccin-macchiato.toml"),
    ),
    ("solarized-light", include_str!("../../themes/solarized-light.toml")),
    ("solarized-dark", include_str!("../../themes/solarized-dark.toml")),
    ("rose-pine", include_str!("../../themes/rose-pine.toml")),
    ("rose-pine-moon", include_str!("../../themes/rose-pine-moon.toml")),
    ("rose-pine-dawn", include_str!("../../themes/rose-pine-dawn.toml")),
    ("atom-one-dark", include_str!("../../themes/atom-one-dark.toml")),
    ("atom-one-light", include_str!("../../themes/atom-one-light.toml")),
    ("ansi-dark", include_str!("../../themes/ansi-dark.toml")),
    ("ansi-light", include_str!("../../themes/ansi-light.toml")),
];

/// Other names a built-in theme answers to, as `(alias, name)`: none yet. Renaming a theme (#379) keeps its old name
/// working here, so configs that name it still load.
const ALIASES: &[(&str, &str)] = &[];

/// A set of themes, each with a unique name.
pub struct Catalog {
    /// In the order they're listed.
    themes: Vec<Theme>,
    /// `(alias, name)`: each alias names one of `themes`.
    aliases: Vec<(&'static str, &'static str)>,
}

impl Catalog {
    /// The built-in themes, parsed on first use.
    pub fn builtin() -> &'static Catalog {
        static BUILTIN_CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
            // The built-in files are part of the binary, and a unit test parses every one of them, so a broken
            // file can't ship.
            Catalog::parse(&BUILTIN, ALIASES).unwrap_or_else(|err| panic!("a built-in theme file is broken: {err}"))
        });
        &BUILTIN_CATALOG
    }

    /// The catalog of `files`, `(name, text)`, each of which must list every role.
    fn parse(
        files: &[(&'static str, &str)],
        aliases: &[(&'static str, &'static str)],
    ) -> Result<Catalog, ThemeFileError> {
        let themes = files
            .iter()
            .map(|&(name, text)| {
                let (header, roles) = ThemeFile::parse(&format!("{name}.toml"), text)?.complete()?;
                let display_name = header.display_name.unwrap_or_else(|| title_case(name));
                Ok(Theme {
                    name,
                    // One small string and one `Roles` per theme, parsed once per process; leaking them keeps
                    // `Theme` a `Copy` handle.
                    display_name: String::leak(display_name),
                    appearance: header.appearance,
                    terminal_colors: header.terminal_colors,
                    roles: Box::leak(Box::new(roles)),
                    mode: ColorMode::default(),
                })
            })
            .collect::<Result<Vec<_>, ThemeFileError>>()?;
        Ok(Catalog {
            themes,
            aliases: aliases.to_vec(),
        })
    }

    /// The theme called `name`, or that `name` is an alias of. Exact and case-sensitive, as in Python.
    pub fn get(&self, name: &str) -> Option<Theme> {
        let name = self
            .aliases
            .iter()
            .find(|(alias, _)| *alias == name)
            .map_or(name, |&(_, theme)| theme);
        self.themes.iter().find(|theme| theme.name == name).copied()
    }

    /// Every theme, in order.
    pub fn themes(&self) -> impl Iterator<Item = Theme> + '_ {
        self.themes.iter().copied()
    }
}

/// The name the Python app shows for a theme: `name.title().replace("-", " ")`, so `rose-pine-moon` is
/// "Rose Pine Moon".
pub(super) fn title_case(name: &str) -> String {
    // `str.title()` capitalizes a letter that follows anything other than a letter, and lowercases the rest.
    let mut after_letter = false;
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
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

/// A built-in theme file's text.
#[cfg(test)]
pub(super) fn source(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _)| *n == name).map(|&(_, text)| text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Roles;

    #[test]
    fn every_builtin_file_lists_every_role_and_nothing_else() {
        for (name, text) in BUILTIN {
            let file = ThemeFile::parse(&format!("{name}.toml"), text).unwrap_or_else(|err| panic!("{err}"));
            assert!(file.header.display_name.is_some(), "{name}");
            file.complete().unwrap_or_else(|err| panic!("{err}"));
        }
        assert_eq!(Catalog::builtin().themes().count(), BUILTIN.len());
    }

    /// The files are in `themes/`, one per entry, and nothing else is there.
    #[test]
    fn the_themes_directory_holds_exactly_the_builtin_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        files.sort();
        let mut names: Vec<String> = BUILTIN.iter().map(|(name, _)| format!("{name}.toml")).collect();
        names.sort();
        assert_eq!(files, names);
    }

    /// Settings shows Python's labels.
    #[test]
    fn builtin_display_names_are_pythons_title_case() {
        for theme in Catalog::builtin().themes() {
            assert_eq!(theme.display_name, title_case(theme.name), "{}", theme.name);
        }
    }

    #[test]
    fn title_case_capitalizes_after_any_non_letter() {
        assert_eq!(title_case("a3d-x_y"), "A3D X_Y");
        assert_eq!(title_case("rose-pine-moon"), "Rose Pine Moon");
    }

    #[test]
    fn keys_list_every_role_once() {
        let keys = Roles::keys();
        let mut unique = keys.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len());
        assert!(keys.iter().any(|k| k == "cursor.focused.bg"));
        assert!(keys.iter().any(|k| k == "syntax.keyword.fg"));
        // Every key is a value in every file: `nord` has exactly one line per key.
        let assignments = source("nord")
            .unwrap()
            .lines()
            .filter(|line| line.contains(" = ") && !line.starts_with('#'))
            .count();
        assert_eq!(assignments, keys.len() + 3, "the roles, and the three header keys");
    }

    #[test]
    fn get_finds_names_then_aliases() {
        let files = [("one", source("nord").unwrap()), ("two", source("gruvbox").unwrap())];
        let catalog = Catalog::parse(&files, &[("uno", "one")]).unwrap();
        assert_eq!(catalog.get("one").map(|t| t.name), Some("one"));
        assert_eq!(catalog.get("uno").map(|t| t.name), Some("one"));
        assert!(catalog.get("One").is_none());
        assert!(catalog.get("nord").is_none());
        assert_eq!(catalog.themes().map(|t| t.name).collect::<Vec<_>>(), ["one", "two"]);
    }

    #[test]
    fn a_broken_file_fails_the_catalog_with_its_name() {
        let broken = source("nord")
            .unwrap()
            .replace("[surface]\n", "[surface]\nshine = \"#FFFFFF\"\n");
        let Err(err) = Catalog::parse(&[("nord", source("nord").unwrap()), ("broken", &broken)], &[]) else {
            panic!("a broken file parsed");
        };
        assert_eq!(err.file, "broken.toml");
        assert_eq!(err.role.as_deref(), Some("surface.shine"));
    }

    /// A partial file, as a user theme (#380) will be, replaces only the roles it writes.
    #[test]
    fn a_patch_replaces_only_the_roles_it_writes() {
        use crate::theme::{AnsiColor, ThemeColor};

        let text = "appearance = \"dark\"\n\n[cursor.focused]\nbg = \"ansi:red\"\n\n[marks]\nstarred = \"default\"\n";
        let file = ThemeFile::parse("mine.toml", text).unwrap();
        assert!(file.header.display_name.is_none());
        let base = Catalog::builtin().get("nord").unwrap().roles();
        let roles = file.roles.apply(base);
        assert_eq!(roles.cursor.focused.bg, ThemeColor::Ansi(AnsiColor::Red));
        assert_eq!(roles.marks.starred, ThemeColor::Ansi(AnsiColor::Default));
        assert_eq!(roles.cursor.focused.fg, base.cursor.focused.fg);
        assert_eq!(roles.surface, base.surface);
        assert_eq!(roles.syntax, base.syntax);
    }
}
