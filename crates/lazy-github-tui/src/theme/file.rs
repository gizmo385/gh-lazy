//! Theme files: the TOML a theme's roles are written in, and the values in it.
//!
//! A file has three keys about the theme, then one table per group of roles (`[surface]`, `[cursor.focused]`), as
//! the files in `crates/lazy-github-tui/themes/` show. Values:
//!
//! - Colors: `#rrggbb`; `#rrggbbaa` or `#rrggbb N%` with alpha, composited over what's under; `auto` or `auto N%`,
//!   white or black, whichever contrasts with what's under (Textual's `auto`); `ansi:<name>`, one of the terminal's 16
//!   colors (`ansi:black` … `ansi:bright-white`); `default`, the terminal's default color; `transparent`. A syntax
//!   token's colors may also be `none`: the code block's own.
//! - Text styles: space-separated flags out of `bold dim italic underline reverse strike blink`, each turned on, or
//!   turned off with a leading `!` (`!underline`); `none` for no change.
//! - Flags: `true` or `false`.

use std::fmt;
use std::ops::Range;

use ratatui::style::Modifier;
use serde::de;
use serde::{Deserialize, Deserializer};
use toml::Spanned;
use toml::de::DeTable;

use super::color::{AnsiColor, Rgb, TextStyle, ThemeColor};
use super::roles::{MaybeColor, MissingRoles, Roles, RolesPatch};

/// Whether a theme is light or dark, as its file says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Light text on a dark background.
    Dark,
    /// Dark text on a light background.
    Light,
}

/// What a theme file says about the theme, besides its roles.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Header {
    /// The name Settings shows; Python's title case of the file's name when left out.
    pub(crate) display_name: Option<String>,
    pub(crate) appearance: Appearance,
    /// Whether the roles are the terminal's own colors, which can't be blended (`ansi-dark`, `ansi-light`).
    #[serde(default)]
    pub(crate) terminal_colors: bool,
}

/// [`Header`]'s keys, which sit beside the role tables at the top of a file.
const HEADER_KEYS: [&str; 3] = ["display_name", "appearance", "terminal_colors"];

/// A parsed theme file.
#[derive(Debug)]
pub(crate) struct ThemeFile {
    pub(crate) header: Header,
    /// The roles it writes, any of them left out.
    pub(crate) roles: RolesPatch,
    /// Its source, for placing the errors [`ThemeFile::complete`] finds.
    source: String,
    file: String,
}

impl ThemeFile {
    /// Parses `source`, the contents of the theme file called `file` (named in errors).
    pub(crate) fn parse(file: &str, source: &str) -> Result<ThemeFile, ThemeFileError> {
        let (header, roles) = parse_parts(file, source, &HEADER_KEYS)?;
        Ok(ThemeFile {
            header,
            roles,
            source: source.to_owned(),
            file: file.to_owned(),
        })
    }

    /// Its roles, if it lists every one, as a built-in theme's file must.
    pub(crate) fn complete(self) -> Result<(Header, Roles), ThemeFileError> {
        let ThemeFile {
            header,
            roles,
            source,
            file,
        } = self;
        match roles.complete() {
            Ok(roles) => Ok((header, roles)),
            Err(MissingRoles(missing)) => {
                let first = missing.first();
                let (line, column) = first
                    .and_then(|m| m.table.clone())
                    .map(|table| line_column(&source, table))
                    .unzip();
                let paths: Vec<&str> = missing.iter().map(|m| m.path.as_str()).collect();
                Err(ThemeFileError {
                    file,
                    line,
                    column,
                    role: first.map(|m| m.path.clone()),
                    message: format!("missing {}: {}", plural(paths.len()), paths.join(", ")),
                })
            }
        }
    }
}

/// What a user theme file (#380) says about the theme, besides its roles. `terminal_colors` isn't one of its keys:
/// it comes from the base theme, since a terminal-palette theme draws differently.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserHeader {
    /// The built-in theme (or alias) it starts from; `dark` when left out.
    pub(crate) extends: Option<Spanned<String>>,
    /// The name Settings shows; the title case of the file's name when left out.
    pub(crate) display_name: Option<String>,
    /// The base theme's when left out.
    pub(crate) appearance: Option<Appearance>,
}

/// [`UserHeader`]'s keys, plus `terminal_colors`, so that a user file writing it is told it's not a key here
/// rather than that it's not a role.
const USER_HEADER_KEYS: [&str; 4] = ["extends", "display_name", "appearance", "terminal_colors"];

/// A parsed user theme file: a header and the roles it overrides.
#[derive(Debug)]
pub(crate) struct UserThemeFile {
    pub(crate) header: UserHeader,
    /// The roles it writes, which replace the base theme's.
    pub(crate) roles: RolesPatch,
}

impl UserThemeFile {
    /// Parses `source`, the contents of the user theme file called `file` (named in errors).
    pub(crate) fn parse(file: &str, source: &str) -> Result<UserThemeFile, ThemeFileError> {
        let (header, roles) = parse_parts(file, source, &USER_HEADER_KEYS)?;
        Ok(UserThemeFile { header, roles })
    }
}

/// `source`'s header (the top-level `header_keys`) and its role tables, each read on its own.
fn parse_parts<H: de::DeserializeOwned>(
    file: &str,
    source: &str,
    header_keys: &[&str],
) -> Result<(H, RolesPatch), ThemeFileError> {
    let error = |err: toml::de::Error, role: Option<String>| ThemeFileError::from_toml(file, source, &err, role);
    let document = DeTable::parse(source).map_err(|err| error(err, None))?;
    let span = document.span();
    // The header keys and the role tables share the top level; each half is read on its own, so both keep
    // rejecting keys they don't know.
    let (mut header, mut roles) = (DeTable::new(), DeTable::new());
    for (key, value) in document.into_inner() {
        if header_keys.contains(&key.get_ref().as_ref()) {
            header.insert(key, value);
        } else {
            roles.insert(key, value);
        }
    }
    let header: H = serde_path_to_error::deserialize(reader(span.clone(), header))
        .map_err(|err| error(err.inner().clone(), role_path(err.path())))?;
    let roles: RolesPatch = serde_path_to_error::deserialize(reader(span, roles))
        .map_err(|err| error(err.inner().clone(), role_path(err.path())))?;
    Ok((header, roles))
}

/// A deserializer for one half of a file's top level, which spans the whole file.
fn reader(span: Range<usize>, table: DeTable<'_>) -> toml::Deserializer<'_> {
    toml::Deserializer::from(Spanned::new(span, table))
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "role" } else { "roles" }
}

/// A theme file that can't be used: what's wrong, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeFileError {
    /// The file's name.
    pub file: String,
    /// The 1-based line the error is on, if it's on one.
    pub line: Option<usize>,
    /// The 1-based column on that line.
    pub column: Option<usize>,
    /// The dotted path of the role (or header key) it's about, if it's about one.
    pub role: Option<String>,
    /// What's wrong.
    pub message: String,
}

impl ThemeFileError {
    fn from_toml(file: &str, source: &str, err: &toml::de::Error, role: Option<String>) -> Self {
        let (line, column) = err.span().map(|span| line_column(source, span)).unzip();
        ThemeFileError {
            file: file.to_owned(),
            line,
            column,
            role,
            message: err.message().trim_end().to_owned(),
        }
    }

    /// An error about the whole file `file`, at no line.
    pub(crate) fn whole_file(file: &str, message: impl Into<String>) -> Self {
        ThemeFileError {
            file: file.to_owned(),
            line: None,
            column: None,
            role: None,
            message: message.into(),
        }
    }

    /// An error about the header key `key`, written at `span` of `source`.
    pub(crate) fn at_key(file: &str, source: &str, span: Range<usize>, key: &str, message: String) -> Self {
        let (line, column) = line_column(source, span);
        ThemeFileError {
            file: file.to_owned(),
            line: Some(line),
            column: Some(column),
            role: Some(key.to_owned()),
            message,
        }
    }

    /// What's wrong, without the file's name, as the "Theme file skipped" toast says it after the name: `line 3:
    /// surface.screen: …`. An unknown key's message leaves out the list of every key it could have been, which
    /// would fill the screen; `docs/themes.md` and the log have it.
    pub fn reason(&self) -> String {
        let message = self
            .message
            .split_once(", expected one of ")
            .map_or(self.message.as_str(), |(short, _)| short);
        let line = self.line.map(|line| format!("line {line}: ")).unwrap_or_default();
        match &self.role {
            Some(role) if !message.contains(role.as_str()) => format!("{line}{role}: {message}"),
            _ => format!("{line}{message}"),
        }
    }
}

impl fmt::Display for ThemeFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
            if let Some(column) = self.column {
                write!(f, ":{column}")?;
            }
        }
        match &self.role {
            Some(role) if !self.message.contains(role.as_str()) => write!(f, ": {role}: {}", self.message),
            _ => write!(f, ": {}", self.message),
        }
    }
}

impl std::error::Error for ThemeFileError {}

/// The 1-based line and column (in characters) where `span` starts in `source`.
fn line_column(source: &str, span: Range<usize>) -> (usize, usize) {
    let before = source.get(..span.start).unwrap_or(source);
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = before.matches('\n').count() + 1;
    (line, before[line_start..].chars().count() + 1)
}

/// The dotted role path `serde_path_to_error` found, without `toml::Spanned`'s private wrapper keys.
fn role_path(path: &serde_path_to_error::Path) -> Option<String> {
    let segments: Vec<String> = path
        .iter()
        .map(ToString::to_string)
        .filter(|segment| !segment.starts_with("$__serde_spanned"))
        .collect();
    (!segments.is_empty()).then(|| segments.join("."))
}

/// The 16 colors' names after `ansi:`, in ANSI order.
const ANSI_NAMES: [(&str, AnsiColor); 16] = [
    ("black", AnsiColor::Black),
    ("red", AnsiColor::Red),
    ("green", AnsiColor::Green),
    ("yellow", AnsiColor::Yellow),
    ("blue", AnsiColor::Blue),
    ("magenta", AnsiColor::Magenta),
    ("cyan", AnsiColor::Cyan),
    ("white", AnsiColor::White),
    ("bright-black", AnsiColor::BrightBlack),
    ("bright-red", AnsiColor::BrightRed),
    ("bright-green", AnsiColor::BrightGreen),
    ("bright-yellow", AnsiColor::BrightYellow),
    ("bright-blue", AnsiColor::BrightBlue),
    ("bright-magenta", AnsiColor::BrightMagenta),
    ("bright-cyan", AnsiColor::BrightCyan),
    ("bright-white", AnsiColor::BrightWhite),
];

/// The text style flags, as written.
const STYLE_FLAGS: [(&str, Modifier); 7] = [
    ("bold", Modifier::BOLD),
    ("dim", Modifier::DIM),
    ("italic", Modifier::ITALIC),
    ("underline", Modifier::UNDERLINED),
    ("reverse", Modifier::REVERSED),
    ("strike", Modifier::CROSSED_OUT),
    ("blink", Modifier::SLOW_BLINK),
];

const COLOR_SYNTAX: &str =
    "expected #rrggbb, #rrggbbaa, #rrggbb N%, auto, auto N%, ansi:<name>, default or transparent";

/// `N%` as an alpha, exactly as the float `N / 100` is written: `52.2%` is `0.522`, not `52.2 / 100.0`, which can
/// differ in the last bit.
fn percent(text: &str) -> Result<f64, String> {
    let bad = || format!("{text:?} is not a percentage from 0% to 100%");
    let number = text.strip_suffix('%').ok_or_else(bad)?;
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(bad());
    }
    let alpha: f64 = format!("{number}e-2").parse().map_err(|_| bad())?;
    if (0.0..=1.0).contains(&alpha) {
        Ok(alpha)
    } else {
        Err(bad())
    }
}

/// `#rrggbb` or `#rrggbbaa`, the alpha as a fraction of 255.
fn hex(text: &str) -> Option<(Rgb, f64)> {
    let digits = text.strip_prefix('#')?;
    if !matches!(digits.len(), 6 | 8) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let rgb = Rgb::new(byte(0)?, byte(2)?, byte(4)?);
    let alpha = if digits.len() == 8 {
        f64::from(byte(6)?) / 255.0
    } else {
        1.0
    };
    Some((rgb, alpha))
}

/// A color as a theme file writes it.
fn parse_color(text: &str) -> Result<ThemeColor, String> {
    let text = text.trim();
    let bad = || format!("{text:?} is not a color: {COLOR_SYNTAX}");
    match text {
        "transparent" => return Ok(ThemeColor::Transparent),
        "default" => return Ok(ThemeColor::Ansi(AnsiColor::Default)),
        "auto" => return Ok(ThemeColor::Auto { alpha: 1.0 }),
        _ => {}
    }
    if let Some(name) = text.strip_prefix("ansi:") {
        return ANSI_NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, c)| ThemeColor::Ansi(c))
            .ok_or_else(|| format!("{text:?} is not one of the 16 ANSI colors (ansi:black … ansi:bright-white)"));
    }
    let (base, alpha) = match text.split_once(' ') {
        Some((base, alpha)) => (base, Some(percent(alpha.trim())?)),
        None => (text, None),
    };
    if base == "auto" {
        return Ok(ThemeColor::Auto {
            alpha: alpha.unwrap_or(1.0),
        });
    }
    let (Rgb { r, g, b }, a) = hex(base).ok_or_else(bad)?;
    // `#rrggbbaa N%` multiplies the two, as Textual's `$color N%` does.
    let a = match alpha {
        Some(alpha) if a < 1.0 => a * alpha,
        Some(alpha) => alpha,
        None => a,
    };
    Ok(ThemeColor::Rgba { r, g, b, a })
}

fn parse_maybe_color(text: &str) -> Result<MaybeColor, String> {
    if text.trim() == "none" {
        Ok(MaybeColor(None))
    } else {
        parse_color(text).map(|c| MaybeColor(Some(c)))
    }
}

fn parse_rgb(text: &str) -> Result<Rgb, String> {
    match hex(text.trim()) {
        Some((rgb, a)) if a >= 1.0 => Ok(rgb),
        _ => Err(format!("{text:?} is not an opaque color: expected #rrggbb")),
    }
}

fn parse_style(text: &str) -> Result<TextStyle, String> {
    let mut style = TextStyle {
        add: Modifier::empty(),
        sub: Modifier::empty(),
    };
    if text.trim() == "none" {
        return Ok(style);
    }
    for word in text.split_whitespace() {
        let (off, name) = match word.strip_prefix('!') {
            Some(name) => (true, name),
            None => (false, word),
        };
        let Some(&(_, flag)) = STYLE_FLAGS.iter().find(|(n, _)| *n == name) else {
            return Err(format!(
                "{word:?} is not a text style: expected none, or bold, dim, italic, underline, reverse, strike or \
                 blink, each with an optional leading !"
            ));
        };
        if off {
            style.sub |= flag;
        } else {
            style.add |= flag;
        }
    }
    Ok(style)
}

/// Deserializes a string with `parse`.
fn from_text<'de, D: Deserializer<'de>, T>(d: D, parse: fn(&str) -> Result<T, String>) -> Result<T, D::Error> {
    struct Text<T>(fn(&str) -> Result<T, String>);

    impl<T> de::Visitor<'_> for Text<T> {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a string")
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<T, E> {
            (self.0)(text).map_err(E::custom)
        }
    }

    d.deserialize_str(Text(parse))
}

impl<'de> Deserialize<'de> for ThemeColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        from_text(d, parse_color)
    }
}

impl<'de> Deserialize<'de> for MaybeColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        from_text(d, parse_maybe_color)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        from_text(d, parse_rgb)
    }
}

impl<'de> Deserialize<'de> for TextStyle {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        from_text(d, parse_style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_in_every_syntax() {
        let rgba = |r, g, b, a| ThemeColor::Rgba { r, g, b, a };
        assert_eq!(parse_color("#2E3440"), Ok(rgba(0x2E, 0x34, 0x40, 1.0)));
        assert_eq!(parse_color("#2e344099"), Ok(rgba(0x2E, 0x34, 0x40, 0.6)));
        assert_eq!(parse_color("#FFFFFF0A"), Ok(rgba(255, 255, 255, 10.0 / 255.0)));
        assert_eq!(parse_color("#2E3440 60%"), Ok(rgba(0x2E, 0x34, 0x40, 0.6)));
        assert_eq!(parse_color("#000000 0%"), Ok(rgba(0, 0, 0, 0.0)));
        // Exactly the float 0.522 (0.6 * 0.87), which 52.2 / 100.0 isn't.
        assert_eq!(parse_color("auto 52.2%"), Ok(ThemeColor::Auto { alpha: 0.522 }));
        assert_eq!(parse_color("auto"), Ok(ThemeColor::Auto { alpha: 1.0 }));
        assert_eq!(
            parse_color("ansi:bright-blue"),
            Ok(ThemeColor::Ansi(AnsiColor::BrightBlue))
        );
        assert_eq!(parse_color("default"), Ok(ThemeColor::Ansi(AnsiColor::Default)));
        assert_eq!(parse_color("transparent"), Ok(ThemeColor::Transparent));
        assert_eq!(parse_maybe_color("none"), Ok(MaybeColor(None)));
        assert!(parse_color("none").is_err());
    }

    #[test]
    fn bad_colors_say_what_is_expected() {
        for bad in [
            "#12345",
            "#1234567",
            "#GGGGGG",
            "red",
            "ansi:purple",
            "#123456 150%",
            "#123456 50",
            "auto x",
        ] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
        assert!(parse_rgb("#12345678").is_err());
    }

    #[test]
    fn text_styles_add_and_remove_flags() {
        let style = parse_style("bold !underline reverse").unwrap();
        assert_eq!(style.add, Modifier::BOLD | Modifier::REVERSED);
        assert_eq!(style.sub, Modifier::UNDERLINED);
        assert_eq!(parse_style("none").unwrap().add, Modifier::empty());
        assert!(parse_style("bold blinky").is_err());
    }

    /// A complete file: the built-in `nord`.
    fn nord() -> String {
        super::super::catalog::source("nord").unwrap().to_owned()
    }

    fn error(source: &str) -> ThemeFileError {
        match ThemeFile::parse("nord.toml", source) {
            Ok(file) => file.complete().unwrap_err(),
            Err(err) => err,
        }
    }

    fn line_of(source: &str, needle: &str) -> usize {
        source.lines().position(|l| l.starts_with(needle)).unwrap() + 1
    }

    #[test]
    fn a_missing_role_names_its_table() {
        let source = nord().replacen("bg = \"#88C0D0\"\n", "", 1);
        let err = error(&source);
        assert_eq!(err.role.as_deref(), Some("cursor.focused.bg"));
        assert_eq!(err.line, Some(line_of(&source, "[cursor.focused]")));
        assert_eq!(
            err.to_string(),
            format!("nord.toml:{}:1: missing role: cursor.focused.bg", err.line.unwrap())
        );

        // A whole group left out is named once. With no table to point at, the error names only the file.
        let err = error("appearance = \"dark\"\n");
        assert_eq!((err.line, err.role.as_deref()), (None, Some("surface")));
        assert!(
            err.to_string()
                .starts_with("nord.toml: missing roles: surface, text, brand,"),
            "{err}"
        );
    }

    #[test]
    fn an_unknown_role_is_an_error() {
        let source = nord().replace("[cursor.focused]\n", "[cursor.focused]\nshadow = \"#000000\"\n");
        let err = error(&source);
        assert_eq!(err.role.as_deref(), Some("cursor.focused.shadow"));
        assert_eq!(err.line, Some(line_of(&source, "shadow")));
        assert!(err.message.contains("unknown field `shadow`"), "{err}");
        let err = error(&format!("{}\n[shadows]\nx = 1\n", nord()));
        assert_eq!(err.role.as_deref(), Some("shadows"));
        assert!(err.message.contains("unknown field `shadows`"), "{err}");
    }

    #[test]
    fn a_bad_value_names_the_role_and_line() {
        let base = nord();
        for (from, to, needle) in [
            ("bg = \"#88C0D0\"", "bg = \"#12345\"", "#12345"),
            ("bg = \"#88C0D0\"", "bg = \"ansi:purple\"", "ansi:purple"),
            ("bg = \"#88C0D0\"", "bg = 3", "string"),
        ] {
            let source = base.replacen(from, to, 1);
            let err = error(&source);
            assert_eq!(err.role.as_deref(), Some("cursor.focused.bg"), "{to}");
            assert_eq!(err.line, Some(line_of(&source, to)), "{to}");
            assert!(err.message.contains(needle), "{to}: {err}");
        }
        let source = base.replacen("style = \"bold\"", "style = \"bold blinky\"", 1);
        let err = error(&source);
        assert!(
            err.role.as_deref().and_then(|r| r.rsplit('.').next()) == Some("style"),
            "{err}"
        );
        assert_eq!(err.line, Some(line_of(&source, "style = \"bold blinky\"")));
        assert!(err.message.contains("\"blinky\" is not a text style"), "{err}");
    }

    #[test]
    fn header_errors_name_the_key() {
        let err = error(&nord().replace("appearance = \"dark\"", "appearance = \"dusk\""));
        assert_eq!(err.role.as_deref(), Some("appearance"));
        assert_eq!(err.line, Some(4));
        let err = error(&nord().replace("appearance = \"dark\"\n", ""));
        assert!(err.message.contains("missing field `appearance`"), "{err}");
        let err = error("appearance = \"dark\"\nname = 3\n");
        assert!(err.message.contains("unknown field `name`"), "{err}");
    }

    #[test]
    fn a_toml_syntax_error_has_a_line() {
        let err = error("display_name = \"Nord\"\nappearance = \n");
        assert_eq!((err.file.as_str(), err.line), ("nord.toml", Some(2)));
    }
}
