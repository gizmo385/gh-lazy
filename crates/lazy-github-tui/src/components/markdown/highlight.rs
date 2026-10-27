//! Syntax colors for fenced code, after Textual's `MarkdownFence.highlight`.
//!
//! Python lexes a fence with Pygments and styles each token type through `HighlightTheme.STYLES`, a table from
//! Pygments token types to theme variables (`ANSIDarkHighlightTheme` / `ANSILightHighlightTheme` on ANSI themes). We
//! tokenize with syntect's bundled grammars instead, map each scope to the Pygments token type it's nearest to, and
//! style that through the same tables. The grammars split text differently from Pygments' lexers, so some tokens are
//! colored differently, and a fence without a language stays plain where Python guesses one: both accepted as an
//! intentional diff (`docs/rust-rewrite/parity/decisions/markdown-code-highlighting.md`).
//!
//! The diff viewer's hunk panes (#237) share the grammars and the scope-to-token table, through [`syntax_for_file`]
//! and [`tokenize`], and color the tokens with Pygments' monokai style instead.

use std::ops::Range;
use std::sync::LazyLock;

use ratatui::style::{Modifier, Style};
use syntect::easy::ScopeRangeIterator;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use super::layout::Painter;
use crate::theme::{AnsiColor, Paint, Palette, ThemeColor};

/// bat's grammars (`two-face`): syntect's bundled ones and more, such as TOML and Dockerfile. Loaded once, on the
/// first fence with a language or the first hunk pane opened, and never if nothing needs them: with the first
/// highlight, about 30 ms in a release build and 0.4 s in a debug one (see `the_syntax_set_loads_once`).
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// The Pygments token types in `HighlightTheme.STYLES` that some scope maps to. Any other Pygments type would fall
/// back to one of these or to no style, as its parent does in Python. `Keyword.Type`, `Generic.Error` and
/// `Name.Function.Magic` are left out: no scope in the bundled grammars tells them apart from their parents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    Comment,
    Error,
    GenericStrong,
    GenericEmph,
    GenericHeading,
    GenericSubheading,
    Keyword,
    KeywordConstant,
    KeywordNamespace,
    Number,
    String,
    StringBacktick,
    StringDoc,
    StringDouble,
    Name,
    NameAttribute,
    NameBuiltin,
    NameBuiltinPseudo,
    NameClass,
    NameConstant,
    NameDecorator,
    NameFunction,
    NameTag,
    NameVariable,
    Operator,
    OperatorWord,
    /// Text Pygments leaves unstyled (`Text`, `Literal.Scalar.Plain`): plain `$text`, but known, so an outer scope
    /// doesn't style it.
    Text,
}

/// Scope prefixes and the token each stands for. The longest prefix that matches a scope wins, matching whole
/// dot-separated parts.
const SCOPES: &[(&str, Token)] = &[
    ("comment", Token::Comment),
    // Python docstrings and Rust doc comments, which Pygments reads as `String.Doc`.
    ("comment.block.documentation", Token::StringDoc),
    ("comment.line.documentation", Token::StringDoc),
    ("constant", Token::NameConstant),
    ("constant.character", Token::String),
    ("constant.language", Token::KeywordConstant),
    ("constant.numeric", Token::Number),
    // CSS hex colors, which Pygments reads as `Number.Hex`.
    ("constant.other.color", Token::Number),
    ("entity.name", Token::Name),
    ("entity.name.class", Token::NameClass),
    ("entity.name.enum", Token::NameClass),
    ("entity.name.function", Token::NameFunction),
    ("entity.name.struct", Token::NameClass),
    ("entity.name.tag", Token::NameTag),
    ("entity.name.trait", Token::NameClass),
    ("entity.name.type", Token::NameClass),
    ("entity.other.attribute-name", Token::NameAttribute),
    ("invalid", Token::Error),
    ("keyword", Token::Keyword),
    ("keyword.control.import", Token::KeywordNamespace),
    ("keyword.operator", Token::Operator),
    ("keyword.operator.logical.python", Token::OperatorWord),
    ("keyword.operator.word", Token::OperatorWord),
    ("markup.bold", Token::GenericStrong),
    // Pygments' Markdown lexer has `#` headings as `Generic.Heading` and deeper ones as `Generic.Subheading`.
    ("markup.heading", Token::GenericHeading),
    ("markup.heading.2", Token::GenericSubheading),
    ("markup.heading.3", Token::GenericSubheading),
    ("markup.heading.4", Token::GenericSubheading),
    ("markup.heading.5", Token::GenericSubheading),
    ("markup.heading.6", Token::GenericSubheading),
    ("markup.italic", Token::GenericEmph),
    // Markdown's inline code, Pygments' `String.Backtick`.
    ("markup.raw.inline", Token::StringBacktick),
    ("markup.subheading", Token::GenericSubheading),
    // JSON object keys, which Pygments reads as `Name.Tag`; this outer scope outranks the key's string scope. The
    // first is syntect's own JSON grammar, the second bat's.
    ("meta.mapping.key.json", Token::NameTag),
    ("meta.structure.dictionary.key.json", Token::NameTag),
    ("storage", Token::Keyword),
    ("string", Token::String),
    ("string.interpolated", Token::StringBacktick),
    ("string.quoted.double", Token::StringDouble),
    // YAML plain scalars and shell words: Pygments' `Literal.Scalar.Plain` and `Text`, both unstyled.
    ("string.unquoted", Token::Text),
    ("support.class", Token::NameClass),
    ("support.constant", Token::NameConstant),
    ("support.function", Token::NameBuiltin),
    ("support.type", Token::NameBuiltin),
    // CSS property names, which Pygments reads as `Keyword`.
    ("support.type.property-name", Token::Keyword),
    ("variable", Token::Name),
    ("variable.annotation", Token::NameDecorator),
    ("variable.language", Token::NameBuiltinPseudo),
    ("variable.other.readwrite.shell", Token::NameVariable),
    ("variable.other.readwrite.assignment.shell", Token::NameVariable),
];

/// Scopes that decide the token for everything inside them, ahead of the inner scopes. A Markdown heading is one
/// token in Pygments, its `#`s and its text alike.
const ENCLOSING: &[&str] = &[
    "markup.heading",
    "meta.mapping.key.json",
    "meta.structure.dictionary.key.json",
];

/// Fence languages Pygments knows by a name syntect doesn't, as the name syntect knows.
const ALIASES: &[(&str, &str)] = &[
    ("shell", "sh"),
    ("py3", "py"),
    ("python3", "py"),
    ("golang", "go"),
    ("c++", "cpp"),
];

/// The token a scope stands for: the longest matching prefix in [`SCOPES`], or `None` for a scope we know nothing
/// about.
fn scope_token(scope: &str) -> Option<Token> {
    SCOPES
        .iter()
        .filter(|(prefix, _)| {
            scope
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|&(_, token)| token)
}

/// The token for text under `scopes` (outermost first): an enclosing scope's, or else the innermost scope's that
/// maps to anything. `None` is plain `$text`.
fn stack_token(scopes: &[String]) -> Option<Token> {
    if let Some(scope) = scopes
        .iter()
        .find(|scope| ENCLOSING.iter().any(|e| scope.starts_with(e)))
    {
        return scope_token(scope).filter(|token| *token != Token::Text);
    }
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope_token(scope))
        .filter(|token| *token != Token::Text)
}

/// The grammar for a fence's info string, as Pygments' `get_lexer_by_name` takes the whole string, case-blind.
fn syntax_for(language: &str) -> Option<&'static SyntaxReference> {
    let name = language.to_lowercase();
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name.as_str(), |(_, known)| known);
    SYNTAXES.find_syntax_by_token(name)
}

/// The grammar for a file, from its path, as Pygments' `get_lexer_for_filename` picks a lexer: by the file's whole
/// name first (`Makefile`, `Dockerfile`), then by its extension. `None` for a file there's no grammar for.
pub(crate) fn syntax_for_file(path: &str) -> Option<&'static SyntaxReference> {
    // Diff paths always use `/`, whatever the platform.
    let name = path.rsplit('/').next().unwrap_or(path);
    SYNTAXES.find_syntax_by_extension(name).or_else(|| {
        let (_, extension) = name.rsplit_once('.')?;
        SYNTAXES.find_syntax_by_extension(extension)
    })
}

/// The tokens of each line of a fence, as byte ranges into the line. A line's ranges cover it in order; text with no
/// token is left out. Without a language, or with one there's no grammar for, there are no tokens.
pub(super) fn highlight(language: Option<&str>, lines: &[String]) -> Vec<Vec<(Range<usize>, Token)>> {
    match language.and_then(syntax_for) {
        Some(syntax) => tokenize(syntax, lines),
        None => vec![Vec::new(); lines.len()],
    }
}

/// The tokens of each of `lines` under `syntax`, tokenized in order as one text, so a string or comment one line
/// opens carries on to the next. Each line's tokens are byte ranges into it, in order; text with no token is left
/// out. If the grammar fails on some line, that line and the rest have no tokens.
pub(crate) fn tokenize(syntax: &SyntaxReference, lines: &[String]) -> Vec<Vec<(Range<usize>, Token)>> {
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut failed = false;
    lines
        .iter()
        .map(|line| {
            if failed {
                return Vec::new();
            }
            // The newline-terminated grammars want each line with its `\n`.
            let text = format!("{line}\n");
            let Ok(ops) = state.parse_line(&text, &SYNTAXES) else {
                // A grammar that fails on some input: the rest of the fence stays plain.
                failed = true;
                return Vec::new();
            };
            let mut tokens: Vec<(Range<usize>, Token)> = Vec::new();
            for (range, op) in ScopeRangeIterator::new(&ops, &text) {
                if stack.apply(op).is_err() {
                    failed = true;
                    return Vec::new();
                }
                let range = range.start.min(line.len())..range.end.min(line.len());
                if range.is_empty() {
                    continue;
                }
                let scopes: Vec<String> = stack.as_slice().iter().map(|s| s.build_string()).collect();
                if let Some(token) = stack_token(&scopes) {
                    match tokens.last_mut() {
                        Some((last, previous)) if *previous == token && last.end == range.start => {
                            last.end = range.end;
                        }
                        _ => tokens.push((range, token)),
                    }
                }
            }
            tokens
        })
        .collect()
}

/// A color in a highlight theme.
#[derive(Debug, Clone, Copy)]
enum Ink {
    /// `$text` at an alpha.
    Text(f64),
    /// A theme variable at an alpha.
    Var(fn(&Palette) -> ThemeColor, f64),
    /// A terminal color, in the ANSI highlight themes.
    Ansi(AnsiColor),
}

/// What a token looks like: `HighlightTheme.STYLES`' value for it.
#[derive(Debug, Clone, Copy, Default)]
struct Look {
    fg: Option<Ink>,
    bg: Option<Ink>,
    add: Modifier,
}

const fn look(fg: Option<Ink>, add: Modifier) -> Look {
    Look { fg, bg: None, add }
}

const NONE: Modifier = Modifier::empty();

/// `HighlightTheme.STYLES`. `Token.String` and `Token.Literal.String` are one type in Pygments, so the dict's later
/// `Token.String: "$text-success"` is the one that applies, not `"$text-success 90%"`.
fn textual(token: Token) -> Look {
    use Ink::{Text, Var};
    let var = |f: fn(&Palette) -> ThemeColor| Some(Var(f, 1.0));
    match token {
        Token::Comment | Token::StringBacktick => look(Some(Text(0.6)), NONE),
        Token::Error => Look {
            fg: var(|p| p.text_error),
            bg: var(|p| p.error_muted),
            add: NONE,
        },
        Token::GenericStrong | Token::Operator => look(None, Modifier::BOLD),
        Token::GenericEmph | Token::NameBuiltinPseudo => look(None, Modifier::ITALIC),
        Token::GenericHeading => look(var(|p| p.text_primary), Modifier::UNDERLINED),
        Token::GenericSubheading | Token::Name => look(var(|p| p.text_primary), NONE),
        Token::Keyword | Token::NameBuiltin => look(var(|p| p.text_accent), NONE),
        Token::KeywordConstant => look(Some(Var(|p| p.text_success, 0.8)), Modifier::BOLD),
        Token::KeywordNamespace | Token::NameConstant => look(var(|p| p.text_error), NONE),
        Token::Number | Token::NameAttribute => look(var(|p| p.text_warning), NONE),
        Token::String => look(var(|p| p.text_success), NONE),
        Token::StringDoc => look(Some(Var(|p| p.text_success, 0.8)), Modifier::ITALIC),
        Token::StringDouble => look(Some(Var(|p| p.text_success, 0.9)), NONE),
        Token::NameClass => look(var(|p| p.text_warning), Modifier::BOLD),
        Token::NameDecorator | Token::NameTag => look(var(|p| p.text_primary), Modifier::BOLD),
        Token::NameFunction => look(var(|p| p.text_warning), Modifier::UNDERLINED),
        Token::NameVariable => look(var(|p| p.text_secondary), NONE),
        Token::OperatorWord => look(var(|p| p.text_error), Modifier::BOLD),
        Token::Text => Look::default(),
    }
}

/// `ANSIDarkHighlightTheme.STYLES` (`light` false) and `ANSILightHighlightTheme.STYLES`. Their misspelled colors
/// (`ansi_greenb` for strings, `ansi_yelllow`, `ansi_blow`, and the light theme's `ans_red`) don't parse, so Textual
/// draws those tokens unstyled, and so do we.
fn ansi(token: Token, light: bool) -> Look {
    use AnsiColor::{Blue, BrightBlack, Cyan, Default, Green, Magenta, Red, Yellow};
    let ink = |c| Some(Ink::Ansi(c));
    match token {
        Token::Comment => look(None, Modifier::DIM | Modifier::ITALIC),
        Token::Error if light => Look::default(),
        Token::Error | Token::NameConstant => look(ink(Red), NONE),
        Token::GenericStrong => look(None, Modifier::BOLD),
        Token::GenericEmph | Token::NameBuiltinPseudo => look(None, Modifier::ITALIC),
        Token::GenericHeading => look(ink(Blue), Modifier::UNDERLINED),
        Token::GenericSubheading | Token::NameFunction => look(ink(Blue), NONE),
        Token::Keyword => look(ink(Magenta), Modifier::BOLD),
        Token::KeywordConstant | Token::NameBuiltin => look(ink(Cyan), NONE),
        Token::KeywordNamespace | Token::OperatorWord => look(ink(Magenta), NONE),
        Token::Number | Token::NameClass if light => look(ink(Blue), Modifier::BOLD),
        Token::Number | Token::NameClass => look(ink(Yellow), NONE),
        Token::StringBacktick => look(ink(BrightBlack), NONE),
        Token::String | Token::NameAttribute | Token::Text => Look::default(),
        Token::StringDoc => look(ink(Green), Modifier::ITALIC),
        Token::StringDouble => look(ink(Green), NONE),
        Token::Name | Token::NameVariable | Token::Operator => look(ink(Default), NONE),
        Token::NameDecorator | Token::NameTag => look(ink(Blue), Modifier::BOLD),
    }
}

impl Painter<'_> {
    /// `base` (the fence's `$text` on its background) with `token`'s highlight style, its colors drawn over the
    /// fence's `background`.
    pub(super) fn token_style(&self, token: Token, base: Style, background: Paint) -> Style {
        let look = if self.theme.ansi {
            ansi(token, !self.theme.dark)
        } else {
            textual(token)
        };
        let p = self.palette();
        let color = |ink: Ink| {
            let c = match ink {
                Ink::Text(alpha) => full_text(p.text).multiply_alpha(alpha),
                Ink::Var(var, alpha) => var(p).multiply_alpha(alpha),
                Ink::Ansi(c) => ThemeColor::Ansi(c),
            };
            self.theme.color(c, background)
        };
        let mut style = base.add_modifier(look.add);
        if let Some(fg) = look.fg {
            style = style.fg(color(fg));
        }
        if let Some(bg) = look.bg {
            style = style.bg(color(bg));
        }
        style
    }
}

/// `$text` as the highlighter resolves it: an `auto` color at full strength, whatever the theme's alpha.
pub(super) fn full_text(text: ThemeColor) -> ThemeColor {
    match text {
        ThemeColor::Auto { .. } => ThemeColor::Auto { alpha: 1.0 },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn the_longest_scope_prefix_wins() {
        assert_eq!(scope_token("keyword.control.flow.python"), Some(Token::Keyword));
        assert_eq!(scope_token("keyword.operator.arithmetic.python"), Some(Token::Operator));
        assert_eq!(
            scope_token("keyword.operator.logical.python"),
            Some(Token::OperatorWord)
        );
        assert_eq!(scope_token("string.quoted.single.python"), Some(Token::String));
        assert_eq!(scope_token("string.quoted.double.rust"), Some(Token::StringDouble));
        assert_eq!(scope_token("string.unquoted.plain.out.yaml"), Some(Token::Text));
        // Whole parts only: `stringy` isn't a `string`.
        assert_eq!(scope_token("stringy.thing"), None);
        assert_eq!(scope_token("source.python"), None);
        assert_eq!(scope_token("punctuation.definition.string.begin.python"), None);
    }

    #[test]
    fn the_innermost_known_scope_decides_and_unknown_ones_are_plain() {
        let stack = |scopes: &[&str]| scopes.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        // A string's quote takes the string's token.
        assert_eq!(
            stack_token(&stack(&[
                "source.python",
                "string.quoted.double.python",
                "punctuation.definition.string.begin.python"
            ])),
            Some(Token::StringDouble)
        );
        assert_eq!(
            stack_token(&stack(&["source.python", "meta.generic-name.python"])),
            None
        );
        // A YAML key's tag outranks its plain scalar.
        assert_eq!(
            stack_token(&stack(&[
                "source.yaml",
                "string.unquoted.plain.out.yaml",
                "entity.name.tag.yaml"
            ])),
            Some(Token::NameTag)
        );
        // A JSON key is a tag, though its innermost scope is a string.
        assert_eq!(
            stack_token(&stack(&[
                "source.json",
                "meta.structure.dictionary.key.json",
                "string.quoted.double.json"
            ])),
            Some(Token::NameTag)
        );
    }

    fn tokens(language: &str, code: &str) -> Vec<Vec<(String, Token)>> {
        let lines: Vec<String> = code.lines().map(str::to_owned).collect();
        highlight(Some(language), &lines)
            .into_iter()
            .zip(&lines)
            .map(|(tokens, line)| {
                tokens
                    .into_iter()
                    .map(|(range, token)| (line[range].to_owned(), token))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn languages_are_found_by_their_fence_names() {
        for language in [
            "sh", "bash", "shell", "rust", "rs", "python", "py", "Python", "json", "yaml", "yml",
        ] {
            assert!(syntax_for(language).is_some(), "{language}");
        }
        assert!(syntax_for("no-such-language").is_none());
        assert!(syntax_for("rust ignore").is_none());
    }

    #[test]
    fn sh_rust_python_json_and_yaml_get_tokens() {
        let token = |t: &[Vec<(String, Token)>], text: &str| {
            t.iter().flatten().find(|(s, _)| s == text).map(|(_, token)| *token)
        };
        let sh = tokens("sh", "echo hello $HOME # c");
        assert_eq!(sh[0][0], ("echo".to_owned(), Token::NameBuiltin));
        assert_eq!(token(&sh, "HOME"), Some(Token::NameVariable));
        assert_eq!(token(&sh, "# c"), Some(Token::Comment));
        let rust = tokens("rust", "fn main() { let s = \"x\"; 1 }");
        assert_eq!(token(&rust, "fn"), Some(Token::Keyword));
        assert_eq!(token(&rust, "main"), Some(Token::NameFunction));
        assert_eq!(token(&rust, "\"x\""), Some(Token::StringDouble));
        assert_eq!(token(&rust, "1"), Some(Token::Number));
        let python = tokens("python", "def f():\n    return 'a' and None");
        assert_eq!(token(&python, "def"), Some(Token::Keyword));
        assert_eq!(token(&python, "'a'"), Some(Token::String));
        assert_eq!(token(&python, "and"), Some(Token::OperatorWord));
        assert_eq!(token(&python, "None"), Some(Token::KeywordConstant));
        let json = tokens("json", "{\"a\": [1, true]}");
        assert_eq!(token(&json, "\"a\""), Some(Token::NameTag));
        assert_eq!(token(&json, "true"), Some(Token::KeywordConstant));
        let yaml = tokens("yaml", "key: value");
        assert_eq!(yaml[0], [("key".to_owned(), Token::NameTag)]);
    }

    #[test]
    fn unknown_languages_have_no_tokens() {
        let lines = vec!["fn main() {}".to_owned()];
        assert_eq!(highlight(Some("klingon"), &lines), [Vec::new()]);
        assert_eq!(highlight(None, &lines), [Vec::new()]);
    }

    #[test]
    fn the_syntax_set_loads_once() {
        let lines = vec!["echo hi".to_owned()];
        let start = Instant::now();
        highlight(Some("sh"), &lines);
        let first = start.elapsed();
        let start = Instant::now();
        highlight(Some("sh"), &lines);
        let second = start.elapsed();
        // Measured: about 0.4 s for the first in a debug build (30 ms in release), under 1 ms after. Another
        // test may have loaded the set already, so only the second call's speed is certain.
        eprintln!("first highlight {first:?}, second {second:?}");
        assert!(second.as_millis() < 100, "{second:?}");
    }
}
