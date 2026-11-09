//! The themes the app knows by name: the built-in ones, each a TOML file in `crates/lazy-github-tui/themes/`, and
//! the user's own, each a TOML file in the config directory's `themes/` that extends a built-in one (#380).

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::LazyLock;

use super::Theme;
use super::depth::ColorMode;
use super::file::{ThemeFile, ThemeFileError, UserThemeFile};

/// The built-in themes, by name, with their files' text, in Textual's order, which is the order Settings lists them
/// in. Python users' configs name these, so the names stay as Textual has them, except for Textual's own two, which
/// [`ALIASES`] keeps working under their old names.
const BUILTIN: [(&str, &str); 21] = [
    ("dark", include_str!("../../themes/dark.toml")),
    ("light", include_str!("../../themes/light.toml")),
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

/// Other names a built-in theme answers to, as `(alias, name)`: the Textual-branded names of the two Textual themes,
/// which Python configs hold (`textual-dark` is Python's default). A config keeps an alias until the user picks a theme
/// in Settings, so the Python app reading the same file still finds a name it knows.
const ALIASES: &[(&str, &str)] = &[("textual-dark", "dark"), ("textual-light", "light")];

/// What a user theme extends when its file doesn't say.
pub(super) const DEFAULT_BASE: &str = "dark";

/// A set of themes, each with a unique name.
#[derive(Clone)]
pub struct Catalog {
    /// In the order they're listed: the built-in ones, then any user themes by name.
    themes: Vec<Theme>,
    /// How many of `themes`, at the end, are user themes.
    user_themes: usize,
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
            user_themes: 0,
            aliases: aliases.to_vec(),
        })
    }

    /// The built-in themes, then the user themes in `dir` (the config directory's `themes/`), with an error for
    /// each `*.toml` file there that was skipped. Reads the directory synchronously: it's done once, at startup,
    /// before the event loop starts.
    ///
    /// Every `*.toml` file directly in `dir` is a theme named after its file stem; other files and subdirectories
    /// are ignored. A missing directory has no themes, and one that can't be read is treated as missing, with a
    /// logged warning. Files are read in name order, so the errors come out in a stable order. Each theme's strings
    /// and roles are leaked, as the built-ins' are, so [`Theme`] stays `Copy`: call this once per process.
    pub fn with_user_themes(dir: &Path) -> (Catalog, Vec<ThemeFileError>) {
        let builtin = Catalog::builtin();
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    tracing::warn!("Can't read the themes directory {}: {err}", dir.display());
                }
                return (builtin.clone(), Vec::new());
            }
        };
        let mut files: Vec<(String, std::path::PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            // A directory called `x.toml` isn't a theme. One whose metadata can't be read (a dangling link) is
            // kept, so reading it reports why.
            .filter(|path| fs::metadata(path).map_or(true, |meta| meta.is_file()))
            .filter_map(|path| Some((path.file_name()?.to_string_lossy().into_owned(), path)))
            .collect();
        files.sort();
        let stems: BTreeSet<&str> = files
            .iter()
            .filter_map(|(name, _)| name.strip_suffix(".toml"))
            .collect();
        let mut themes = Vec::new();
        let mut errors = Vec::new();
        for (file, path) in &files {
            let result = fs::read_to_string(path)
                .map_err(|err| ThemeFileError::whole_file(file, format!("can't be read: {err}")))
                .and_then(|source| builtin.user_theme(file, &source, &stems));
            match result {
                Ok(theme) => themes.push(theme),
                Err(err) => {
                    tracing::warn!("Skipped the theme file {}: {err}", path.display());
                    errors.push(err);
                }
            }
        }
        themes.sort_by_key(|theme| theme.name);
        let mut catalog = builtin.clone();
        catalog.user_themes = themes.len();
        catalog.themes.extend(themes);
        (catalog, errors)
    }

    /// The user theme in the file called `file` with contents `source`, extending one of this catalog's themes.
    /// `stems` are the names of every theme file in its directory, to say why `extends` can't name one of them.
    fn user_theme(&self, file: &str, source: &str, stems: &BTreeSet<&str>) -> Result<Theme, ThemeFileError> {
        let stem = file.strip_suffix(".toml").unwrap_or(file);
        if stem.is_empty()
            || !stem
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(ThemeFileError::whole_file(
                file,
                format!("\"{stem}\" isn't a theme name: use lowercase letters, digits and -"),
            ));
        }
        if self.get(stem).is_some() {
            return Err(ThemeFileError::whole_file(
                file,
                format!("\"{stem}\" is a built-in theme's name"),
            ));
        }
        let UserThemeFile { header, roles } = UserThemeFile::parse(file, source)?;
        let base = match &header.extends {
            None => self.get(DEFAULT_BASE),
            Some(extends) => {
                let name = extends.get_ref();
                let base = self.get(name);
                if base.is_none() {
                    let message = if stems.contains(name.as_str()) {
                        format!("\"{name}\" is a user theme; a user theme can only extend a built-in one")
                    } else {
                        format!("no built-in theme is called \"{name}\"")
                    };
                    return Err(ThemeFileError::at_key(file, source, extends.span(), "extends", message));
                }
                base
            }
        };
        // `DEFAULT_BASE` is a built-in theme, which a unit test checks.
        let base = base.unwrap_or_default();
        let display_name = header.display_name.unwrap_or_else(|| title_case(stem));
        Ok(Theme {
            // Leaked once per process, as the built-ins are, so `Theme` stays a `Copy` handle.
            name: String::leak(stem.to_owned()),
            display_name: String::leak(display_name),
            appearance: header.appearance.unwrap_or(base.appearance),
            terminal_colors: base.terminal_colors,
            roles: Box::leak(Box::new(roles.apply(base.roles()))),
            mode: ColorMode::default(),
        })
    }

    /// How many of its themes are user themes.
    pub fn user_theme_count(&self) -> usize {
        self.user_themes
    }

    /// The theme for a config's `appearance.theme`, reduced to the terminal's color `mode`: an alias draws as its
    /// theme, and an unknown name as the default theme (the app warns about that once, at startup).
    pub fn theme_for_config(&self, name: &str, mode: ColorMode) -> Theme {
        self.resolve(name).theme_or_default().with_color_mode(mode)
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

    /// What a config's `appearance.theme` names: a theme by its own name, a theme by an alias, or nothing this
    /// catalog has. Exact and case-sensitive, as in Python.
    pub fn resolve(&self, name: &str) -> Resolved {
        match self.get(name) {
            Some(theme) if theme.name == name => Resolved::Exact(theme),
            Some(theme) => Resolved::Alias {
                theme,
                written: name.to_owned(),
            },
            None => Resolved::Unknown {
                written: name.to_owned(),
            },
        }
    }

    /// Every theme, in order.
    pub fn themes(&self) -> impl Iterator<Item = Theme> + '_ {
        self.themes.iter().copied()
    }
}

/// What [`Catalog::resolve`] found for a name.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// The theme of that name.
    Exact(Theme),
    /// The theme `written` is an alias of, such as `dark` for `textual-dark`.
    Alias {
        /// The theme the alias names.
        theme: Theme,
        /// The alias, as the config has it.
        written: String,
    },
    /// No theme has that name: it's drawn as the default theme, and left as `written` in the config.
    Unknown {
        /// The name, as the config has it.
        written: String,
    },
}

impl Resolved {
    /// The theme the name stands for, if any.
    pub fn theme(&self) -> Option<Theme> {
        match self {
            Resolved::Exact(theme) | Resolved::Alias { theme, .. } => Some(*theme),
            Resolved::Unknown { .. } => None,
        }
    }

    /// The theme the name stands for, or the default theme for an unknown name.
    pub fn theme_or_default(&self) -> Theme {
        self.theme().unwrap_or_default()
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
    fn resolve_tells_names_aliases_and_unknown_names_apart() {
        let catalog = Catalog::builtin();
        let dark = catalog.get("dark").unwrap();
        let light = catalog.get("light").unwrap();
        assert_eq!(catalog.resolve("dark"), Resolved::Exact(dark));
        assert_eq!(catalog.resolve("nord"), Resolved::Exact(catalog.get("nord").unwrap()));
        assert_eq!(
            catalog.resolve("textual-dark"),
            Resolved::Alias {
                theme: dark,
                written: "textual-dark".to_owned()
            }
        );
        assert_eq!(
            catalog.resolve("textual-light"),
            Resolved::Alias {
                theme: light,
                written: "textual-light".to_owned()
            }
        );
        for name in ["no-such-theme", "Nord", ""] {
            let resolved = catalog.resolve(name);
            assert_eq!(
                resolved,
                Resolved::Unknown {
                    written: name.to_owned()
                }
            );
            assert_eq!(resolved.theme(), None);
            assert_eq!(resolved.theme_or_default(), Theme::default());
        }
        assert_eq!(catalog.resolve("textual-light").theme_or_default(), light);
    }

    /// Every alias names a built-in theme, and none shadows one.
    #[test]
    fn aliases_name_builtin_themes() {
        for &(alias, name) in ALIASES {
            assert!(BUILTIN.iter().any(|(n, _)| *n == name), "{alias} -> {name}");
            assert!(BUILTIN.iter().all(|(n, _)| *n != alias), "{alias}");
        }
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
