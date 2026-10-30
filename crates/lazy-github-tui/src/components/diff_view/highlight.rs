//! Syntax colors for a hunk pane, after Python's `UnifiedDiffPane._compute_syntax_highlighting`: the hunk's code is
//! tokenized by file type and drawn in Pygments' monokai style, whatever the app theme.
//!
//! The grammars and the scope-to-token table are the markdown fences' ([`highlight`]); only the colors differ. The
//! grammars split code differently from Pygments' lexers, so some tokens are colored differently than in Python,
//! and a file with no grammar is drawn plain: both accepted as an intentional diff
//! (`docs/rust-rewrite/parity/decisions/diff-syntax-highlighting.md`).

use std::ops::Range;

use lazy_github_core::diff::{ChangedFile, DiffLine, Hunk, LineKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::components::markdown::highlight::{self, Token};
use crate::theme::Rgb;

/// Rich's `Syntax` expands tabs to 4 columns before it tokenizes.
const TAB_SIZE: usize = 4;

/// Pygments' monokai style: its `background_color`, which context lines keep, and the colors its `styles` give the
/// token types [`Token`] stands for. An empty style in Pygments takes its parent's, so `Keyword.Constant` is
/// `Keyword`'s and `String.Double` is `String`'s.
mod monokai {
    use crate::theme::Rgb;

    pub const BACKGROUND: Rgb = Rgb::new(0x27, 0x28, 0x22);
    /// `Token`: everything without a color of its own.
    pub const TEXT: Rgb = Rgb::new(0xF8, 0xF8, 0xF2);
    /// `Comment` and `Generic.Subheading`.
    pub const COMMENT: Rgb = Rgb::new(0x95, 0x90, 0x77);
    /// `Keyword` and `Name.Constant`.
    pub const KEYWORD: Rgb = Rgb::new(0x66, 0xD9, 0xEF);
    /// `Operator`, `Keyword.Namespace` and `Name.Tag`.
    pub const OPERATOR: Rgb = Rgb::new(0xFF, 0x46, 0x89);
    /// `String`.
    pub const STRING: Rgb = Rgb::new(0xE6, 0xDB, 0x74);
    /// `Number`.
    pub const NUMBER: Rgb = Rgb::new(0xAE, 0x81, 0xFF);
    /// `Name.Function`, `Name.Class`, `Name.Attribute` and `Name.Decorator`.
    pub const NAME: Rgb = Rgb::new(0xA6, 0xE2, 0x2E);
    /// `Error`: this on [`ERROR_BACKGROUND`].
    pub const ERROR: Rgb = Rgb::new(0xED, 0x00, 0x7E);
    pub const ERROR_BACKGROUND: Rgb = Rgb::new(0x1E, 0x00, 0x10);
}

/// The code of each of `hunk`'s lines, without its `+`, `-` or space marker, tabs expanded: what the pane draws after
/// the gutter.
///
/// With a grammar for `file`'s type, guessed from its name, the lines are tokenized in order as one text (old and new
/// lines mixed, as Python does) and come back in monokai's colors, each line on monokai's background
/// ([`Line::style`]). The pane keeps that background on context lines and lays its own over the others. Without a
/// grammar they come back unstyled, so the pane's text color and backgrounds show.
pub fn code_spans(file: &ChangedFile, hunk: &Hunk) -> Vec<Line<'static>> {
    let texts: Vec<String> = hunk
        .lines
        .iter()
        .map(|line| crate::display::sanitize(&expand_tabs(&code_text(line))).into_owned())
        .collect();
    let Some(syntax) = highlight::syntax_for_file(&file.path) else {
        return texts.into_iter().map(Line::from).collect();
    };
    let tokens = highlight::tokenize(syntax, &texts);
    texts
        .iter()
        .zip(tokens)
        .map(|(text, tokens)| colored_line(text, &tokens))
        .collect()
}

/// A diff line's code, as Python takes it from the diff.
fn code_text(line: &DiffLine) -> String {
    match line.kind {
        // Python shows the marker line as it is in the diff.
        LineKind::NoNewline => format!("\\ {}", line.text),
        LineKind::Context | LineKind::Added | LineKind::Removed => line.text.clone(),
    }
}

/// `text` split at its tokens' edges, each piece in its token's monokai style, on monokai's background.
fn colored_line(text: &str, tokens: &[(Range<usize>, Token)]) -> Line<'static> {
    let mut spans = Vec::new();
    let mut at = 0;
    let mut push = |range: Range<usize>, token: Option<Token>| {
        if !range.is_empty() {
            spans.push(Span::styled(text[range].to_owned(), token_style(token)));
        }
    };
    for (range, token) in tokens {
        push(at..range.start, None);
        push(range.clone(), Some(*token));
        at = range.end;
    }
    push(at..text.len(), None);
    Line::from(spans).style(Style::new().bg(color(monokai::BACKGROUND)))
}

/// How monokai draws `token`, or text with no token.
fn token_style(token: Option<Token>) -> Style {
    use monokai::{COMMENT, KEYWORD, NAME, NUMBER, OPERATOR, STRING, TEXT};
    let fg = |rgb| Style::new().fg(color(rgb));
    let Some(token) = token else {
        return fg(TEXT);
    };
    match token {
        Token::Comment | Token::GenericSubheading => fg(COMMENT),
        Token::Error => fg(monokai::ERROR).bg(color(monokai::ERROR_BACKGROUND)),
        Token::GenericStrong => fg(TEXT).add_modifier(Modifier::BOLD),
        Token::GenericEmph => fg(TEXT).add_modifier(Modifier::ITALIC),
        Token::Keyword | Token::KeywordConstant | Token::NameConstant => fg(KEYWORD),
        Token::KeywordNamespace | Token::NameTag | Token::Operator | Token::OperatorWord => fg(OPERATOR),
        Token::Number => fg(NUMBER),
        // `String.Doc`, `String.Backtick` and `String.Double` are all `String`'s.
        Token::String | Token::StringBacktick | Token::StringDoc | Token::StringDouble => fg(STRING),
        Token::NameAttribute | Token::NameClass | Token::NameDecorator | Token::NameFunction => fg(NAME),
        Token::GenericHeading
        | Token::Name
        | Token::NameBuiltin
        | Token::NameBuiltinPseudo
        | Token::NameVariable
        | Token::Text => fg(TEXT),
    }
}

/// A monokai color as exact RGB; the pane reduces it to the terminal's colors when it draws.
const fn color(rgb: Rgb) -> Color {
    Color::Rgb(rgb.r, rgb.g, rgb.b)
}

/// `text` with each tab replaced by spaces up to the next tab stop.
pub(super) fn expand_tabs(text: &str) -> String {
    if !text.contains('\t') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.chars() {
        if c == '\t' {
            let spaces = TAB_SIZE - column % TAB_SIZE;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(c);
            column += Span::raw(c.encode_utf8(&mut [0; 4]) as &str).width();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use lazy_github_core::diff;

    use super::*;

    /// The code spans of `path`'s one hunk, whose lines are `lines` (each with its marker).
    fn spans(path: &str, lines: &[&str]) -> Vec<Line<'static>> {
        let body: String = lines.iter().flat_map(|line| [*line, "\n"]).collect();
        let removed = lines.iter().filter(|l| !l.starts_with('+')).count();
        let added = lines.iter().filter(|l| !l.starts_with('-')).count();
        let text =
            format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,{removed} +1,{added} @@\n{body}");
        let parsed = diff::parse(&text).unwrap();
        let file = &parsed.files[0];
        code_spans(file, &file.hunks[0])
    }

    /// The foreground of the first span on `line` whose text is `text`.
    fn fg_of(lines: &[Line<'_>], line: usize, text: &str) -> Option<Color> {
        lines[line]
            .spans
            .iter()
            .find(|span| span.content.trim() == text)
            .and_then(|span| span.style.fg)
    }

    #[test]
    fn rust_tokens_take_monokai_colors() {
        let lines = spans(
            "src/viewer.rs",
            &[
                " fn main() {",
                "-    let s = \"old\"; // gone",
                "+    let n = 42 + 1;",
                " }",
            ],
        );
        assert_eq!(fg_of(&lines, 0, "fn"), Some(color(monokai::KEYWORD)));
        assert_eq!(fg_of(&lines, 0, "main"), Some(color(monokai::NAME)));
        assert_eq!(fg_of(&lines, 1, "let"), Some(color(monokai::KEYWORD)));
        assert_eq!(fg_of(&lines, 1, "\"old\""), Some(color(monokai::STRING)));
        assert_eq!(fg_of(&lines, 1, "// gone"), Some(color(monokai::COMMENT)));
        assert_eq!(fg_of(&lines, 2, "42"), Some(color(monokai::NUMBER)));
        assert_eq!(fg_of(&lines, 2, "+"), Some(color(monokai::OPERATOR)));
        // Text with no token is monokai's text color, not unstyled.
        assert_eq!(fg_of(&lines, 3, "}"), Some(color(monokai::TEXT)));
        for line in &lines {
            assert_eq!(line.style.bg, Some(color(monokai::BACKGROUND)));
        }
    }

    #[test]
    fn markdown_css_and_toml_tokens_take_monokai_colors() {
        let md = spans("README.md", &[" ## Usage", "-Run `viewer FILE`."]);
        assert_eq!(fg_of(&md, 0, "## Usage"), Some(color(monokai::COMMENT)));
        assert_eq!(fg_of(&md, 1, "Run"), Some(color(monokai::TEXT)));
        assert_eq!(fg_of(&md, 1, "`viewer FILE`"), Some(color(monokai::STRING)));

        let css = spans("assets/app.css", &["+body { color: #fff; margin: 0 }"]);
        assert_eq!(fg_of(&css, 0, "body"), Some(color(monokai::OPERATOR)));
        assert_eq!(fg_of(&css, 0, "color"), Some(color(monokai::KEYWORD)));
        assert_eq!(fg_of(&css, 0, "0"), Some(color(monokai::NUMBER)));

        let toml = spans("Cargo.toml", &["+name = \"viewer\" # the crate", "+edition = 2024"]);
        assert_eq!(fg_of(&toml, 0, "name"), Some(color(monokai::OPERATOR)));
        assert_eq!(fg_of(&toml, 0, "\"viewer\""), Some(color(monokai::STRING)));
        assert_eq!(fg_of(&toml, 0, "# the crate"), Some(color(monokai::COMMENT)));
        assert_eq!(fg_of(&toml, 1, "2024"), Some(color(monokai::NUMBER)));
    }

    #[test]
    fn a_file_with_no_grammar_stays_plain() {
        let lines = spans("notes.klingon", &[" fn main() {", "+    1"]);
        for line in &lines {
            assert_eq!(line.style, Style::new());
            assert!(line.spans.iter().all(|span| span.style == Style::new()), "{line:?}");
        }
        assert_eq!(lines[1].to_string(), "    1");
    }

    #[test]
    fn a_block_comment_carries_on_to_the_lines_after_it() {
        let lines = spans(
            "src/lib.rs",
            &["+/* opened here", " still inside", "-and here */ fn after() {}"],
        );
        assert_eq!(fg_of(&lines, 0, "/* opened here"), Some(color(monokai::COMMENT)));
        assert_eq!(fg_of(&lines, 1, "still inside"), Some(color(monokai::COMMENT)));
        assert_eq!(fg_of(&lines, 2, "and here */"), Some(color(monokai::COMMENT)));
        assert_eq!(fg_of(&lines, 2, "fn"), Some(color(monokai::KEYWORD)));
    }

    #[test]
    fn files_are_typed_by_name_then_extension() {
        let name = |path: &str| highlight::syntax_for_file(path).map(|s| s.name.as_str());
        assert_eq!(name("src/viewer.rs"), Some("Rust"));
        assert_eq!(name("README.md"), Some("Markdown"));
        assert_eq!(name("assets/app.css"), Some("CSS"));
        assert_eq!(name("Cargo.toml"), Some("TOML"));
        assert_eq!(name("Makefile"), Some("Makefile"));
        assert_eq!(name("docs/notes.klingon"), None);
        assert_eq!(name("LICENSE"), None);
    }

    #[test]
    fn tabs_expand_to_four_columns_before_tokenizing() {
        assert_eq!(expand_tabs("\tx\ty"), "    x   y");
        assert_eq!(expand_tabs("plain"), "plain");
        let lines = spans("src/lib.rs", &["+\tlet x = 1;"]);
        assert_eq!(lines[0].to_string(), "    let x = 1;");
        assert_eq!(fg_of(&lines, 0, "let"), Some(color(monokai::KEYWORD)));
    }
}
