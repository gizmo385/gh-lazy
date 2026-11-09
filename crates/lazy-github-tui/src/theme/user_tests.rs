//! User theme files (#380): `*.toml` files in the config directory's `themes/` that extend a built-in theme.

use std::path::Path;

use super::{Appearance, Catalog, ColorMode, Resolved, Theme, ThemeColor, ThemeFileError};

/// A user theme overriding two roles of `nord`.
const MY_NORD: &str = r##"
extends = "nord"
display_name = "My Nord"

[border]
pane_focused = "#EBCB8B"

[marks]
merged = "#B48EAD"
"##;

fn rgb(r: u8, g: u8, b: u8) -> ThemeColor {
    ThemeColor::Rgba { r, g, b, a: 1.0 }
}

/// A themes directory holding `files`, `(file name, contents)`.
fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

fn load(files: &[(&str, &str)]) -> (Catalog, Vec<ThemeFileError>) {
    let dir = dir_with(files);
    Catalog::with_user_themes(dir.path())
}

fn builtin(name: &str) -> Theme {
    Catalog::builtin().get(name).unwrap()
}

fn user_names(catalog: &Catalog) -> Vec<&'static str> {
    catalog
        .themes()
        .skip(Catalog::builtin().themes().count())
        .map(|theme| theme.name)
        .collect()
}

#[test]
fn a_user_theme_overrides_only_the_roles_it_writes() {
    let (catalog, errors) = load(&[("my-nord.toml", MY_NORD)]);
    assert_eq!(errors, []);
    let theme = catalog.get("my-nord").unwrap();
    let base = builtin("nord");
    assert_eq!(theme.name, "my-nord");
    assert_eq!(theme.display_name(), "My Nord");
    assert_eq!(theme.appearance(), base.appearance());
    let (roles, base_roles) = (theme.roles(), base.roles());
    assert_eq!(roles.border.pane_focused, rgb(0xEB, 0xCB, 0x8B));
    assert_ne!(roles.border.pane_focused, base_roles.border.pane_focused);
    assert_eq!(roles.marks.merged, rgb(0xB4, 0x8E, 0xAD));
    assert_ne!(roles.marks.merged, base_roles.marks.merged);
    // Everything else is the base's.
    let mut expected = super::roles::RolesPatch::default().apply(base_roles);
    expected.border.pane_focused = roles.border.pane_focused;
    expected.marks.merged = roles.marks.merged;
    assert_eq!(*roles, expected);
    assert_eq!(catalog.user_theme_count(), 1);
    assert_eq!(catalog.resolve("my-nord"), Resolved::Exact(theme));
    assert_eq!(catalog.theme_for_config("my-nord", ColorMode::default()), theme);
}

#[test]
fn extends_defaults_to_dark_and_the_header_to_the_base() {
    let (catalog, errors) = load(&[("plain.toml", "[surface]\nscreen = \"#000000\"\n")]);
    assert_eq!(errors, []);
    let theme = catalog.get("plain").unwrap();
    let dark = builtin("dark");
    assert_eq!(theme.display_name(), "Plain");
    assert_eq!(theme.appearance(), Appearance::Dark);
    assert_eq!(theme.roles().surface.screen, rgb(0x00, 0x00, 0x00));
    assert_eq!(theme.roles().surface.pane, dark.roles().surface.pane);
    assert_eq!(theme.roles().text, dark.roles().text);
}

#[test]
fn extends_may_name_an_alias_and_appearance_may_be_overridden() {
    let text = "extends = \"textual-light\"\nappearance = \"dark\"\n";
    let (catalog, errors) = load(&[("light-ish.toml", text)]);
    assert_eq!(errors, []);
    let theme = catalog.get("light-ish").unwrap();
    assert_eq!(theme.roles(), builtin("light").roles());
    assert_eq!(theme.appearance(), Appearance::Dark);
    assert_eq!(theme.display_name(), "Light Ish");
}

/// A terminal-palette base keeps drawing as one: `terminal_colors` comes from it.
#[test]
fn terminal_colors_come_from_the_base() {
    let (catalog, _) = load(&[("my-ansi.toml", "extends = \"ansi-dark\"\n")]);
    assert!(catalog.get("my-ansi").unwrap().terminal_colors);
    let (catalog, _) = load(&[("mine.toml", "extends = \"nord\"\n")]);
    assert!(!catalog.get("mine").unwrap().terminal_colors);
}

/// Each bad file gives an error naming it and why; the good file beside it still loads.
#[test]
fn a_bad_file_is_skipped_with_its_reason_and_the_others_load() {
    let cases: &[(&str, &str, &str)] = &[
        ("chained.toml", "extends = \"my-nord\"\n", "\"my-nord\" is a user theme"),
        (
            "unknown-base.toml",
            "extends = \"nordic\"\n",
            "no built-in theme is called \"nordic\"",
        ),
        (
            "unknown-role.toml",
            "[border]\nshiny = \"#FFFFFF\"\n",
            "unknown field `shiny`",
        ),
        ("bad-color.toml", "[border]\npane_focused = \"#12345\"\n", "#12345"),
        ("My Theme.toml", "", "\"My Theme\" isn't a theme name"),
        ("nord.toml", "", "\"nord\" is a built-in theme's name"),
        ("textual-dark.toml", "", "\"textual-dark\" is a built-in theme's name"),
        (
            "terminal.toml",
            "terminal_colors = true\n",
            "unknown field `terminal_colors`",
        ),
        ("syntax.toml", "[border\n", "line 1"),
    ];
    for &(file, text, reason) in cases {
        let (catalog, errors) = load(&[("my-nord.toml", MY_NORD), (file, text)]);
        assert_eq!(errors.len(), 1, "{file}: {errors:?}");
        assert_eq!(errors[0].file, file);
        let shown = format!("{}: {}", errors[0].file, errors[0].reason());
        assert!(shown.contains(reason), "{file}: {shown}");
        assert_eq!(user_names(&catalog), ["my-nord"], "{file}");
    }
}

#[test]
fn errors_on_a_line_say_which() {
    let (_, errors) = load(&[(
        "bad.toml",
        "extends = \"nord\"\n\n[border]\npane_focused = \"#12345\"\n",
    )]);
    assert_eq!(
        errors[0].reason(),
        "line 4: border.pane_focused: \"#12345\" is not a color: expected #rrggbb, #rrggbbaa, #rrggbb N%, auto, auto N%, ansi:<name>, default or transparent"
    );
    let (_, errors) = load(&[("bad.toml", "display_name = \"Bad\"\nextends = \"nordic\"\n")]);
    assert_eq!(
        errors[0].reason(),
        "line 2: extends: no built-in theme is called \"nordic\""
    );
}

/// The toast leaves out the list of every key an unknown one could have been.
#[test]
fn an_unknown_key_is_named_without_the_list_of_known_ones() {
    let (_, errors) = load(&[("bad.toml", "[border]\nshiny = \"#FFFFFF\"\n")]);
    assert_eq!(errors[0].reason(), "line 2: border.shiny: unknown field `shiny`");
    assert!(
        errors[0].to_string().contains("expected one of `pane_focused`"),
        "{}",
        errors[0]
    );
}

#[test]
fn an_unreadable_file_is_skipped() {
    let dir = dir_with(&[("my-nord.toml", MY_NORD)]);
    std::fs::write(dir.path().join("binary.toml"), [0xff, 0xfe, 0x00]).unwrap();
    let (catalog, errors) = Catalog::with_user_themes(dir.path());
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].file, "binary.toml");
    assert!(errors[0].reason().starts_with("can't be read: "), "{}", errors[0]);
    assert_eq!(user_names(&catalog), ["my-nord"]);
}

/// A link to nowhere can't be read either. Root reads any file regardless of its permissions, so a dangling link is
/// the unreadable file the tests can make everywhere.
#[cfg(unix)]
#[test]
fn a_dangling_link_is_skipped() {
    let dir = dir_with(&[]);
    std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("gone.toml")).unwrap();
    let (_, errors) = Catalog::with_user_themes(dir.path());
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].file, "gone.toml");
    assert!(errors[0].reason().starts_with("can't be read: "), "{}", errors[0]);
}

#[test]
fn a_missing_directory_has_no_themes_and_no_errors() {
    let dir = dir_with(&[]);
    let (catalog, errors) = Catalog::with_user_themes(&dir.path().join("themes"));
    assert_eq!(errors, []);
    assert_eq!(catalog.user_theme_count(), 0);
    assert!(
        catalog
            .themes()
            .map(|t| t.name)
            .eq(Catalog::builtin().themes().map(|t| t.name))
    );
    assert_eq!(catalog.get("textual-dark").map(|t| t.name), Some("dark"));
}

/// Only `*.toml` files directly in the directory are themes.
#[test]
fn other_files_and_subdirectories_are_ignored() {
    let dir = dir_with(&[("notes.txt", "x"), ("README", "x"), ("mine.toml", "")]);
    std::fs::create_dir(dir.path().join("dir.toml")).unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("nested").join("deep.toml"), "").unwrap();
    let (catalog, errors) = Catalog::with_user_themes(dir.path());
    assert_eq!(errors, []);
    assert_eq!(user_names(&catalog), ["mine"]);
}

/// The built-in themes first in their own order, then the user's by name; the errors in file name order.
#[test]
fn user_themes_follow_the_builtins_by_name() {
    let (catalog, errors) = load(&[
        ("zeta.toml", ""),
        ("a.toml", ""),
        ("a-b.toml", ""),
        ("z-bad.toml", "x"),
        ("b-bad.toml", "x"),
    ]);
    assert_eq!(user_names(&catalog), ["a", "a-b", "zeta"]);
    let builtins: Vec<_> = Catalog::builtin().themes().map(|t| t.name).collect();
    let names: Vec<_> = catalog.themes().map(|t| t.name).collect();
    assert_eq!(names[..builtins.len()], builtins[..]);
    assert_eq!(
        errors.iter().map(|e| e.file.as_str()).collect::<Vec<_>>(),
        ["b-bad.toml", "z-bad.toml"]
    );
}

#[test]
fn the_default_base_is_a_builtin_theme() {
    assert!(Catalog::builtin().get(super::catalog::DEFAULT_BASE).is_some());
}

/// `docs/themes.md` documents every role, so it can't fall behind them.
#[test]
fn the_themes_doc_lists_every_role() {
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/themes.md")).unwrap();
    let missing: Vec<&String> = super::Roles::keys()
        .iter()
        .filter(|key| !doc.contains(&format!("`{key}`")))
        .collect();
    assert!(missing.is_empty(), "docs/themes.md doesn't list {missing:?}");
}
