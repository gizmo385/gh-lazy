//! The markdown renderer links, normalizes and emphasizes text as markdown-it-py's `gfm-like` preset does under
//! Textual: `fixtures/generate_linkify.py` dumps the inline tokens markdown-it makes of each case in
//! `fixtures/linkify_corpus.toml` to `fixtures/linkify.json`, and this lays each case out and compares, character by
//! character, the text, whether it's in italics, and where it links to.

mod common;

use lazy_github_tui::components::markdown::Markdown;
use lazy_github_tui::theme::Theme;
use ratatui::style::Modifier;
use serde::Deserialize;

const GOLDEN: &str = include_str!("fixtures/linkify.json");

/// Wide enough that no case wraps.
const WIDTH: u16 = 2000;

/// The cases Rust doesn't match yet, by how their source starts, each with the issue that does.
const KNOWN_GAPS: &[(&str, &str)] = &[
    // markdown-it links a scheme URL on the source before emphasis and code spans: `http://example.com/*wiki*` is
    // one link, and so is ``http://a.com/`x` ``.
    ("Pull requests for the PR details tabs", "#290"),
    ("http://a.com/`x`", "#290"),
];

#[derive(Debug, Deserialize)]
struct Case {
    source: String,
    /// Each inline token's children: a paragraph's or heading's text.
    blocks: Vec<Vec<Token>>,
}

#[derive(Debug, Deserialize)]
struct Token {
    #[serde(rename = "type")]
    kind: String,
    content: String,
    #[serde(default)]
    href: Option<String>,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    children: Vec<String>,
}

/// One drawn character: itself, whether it's in italics, and its link target.
type Cell = (char, bool, Option<String>);

/// A run of characters in one style: its text, italics, and link target.
type Run = (String, bool, Option<String>);

fn cases() -> Vec<Case> {
    serde_json::from_str(GOLDEN).unwrap()
}

/// What Textual draws for a block's tokens (`_token_to_content`), one cell per character.
fn expected(tokens: &[Token]) -> Vec<Cell> {
    let mut cells = Vec::new();
    let mut emphasis = 0_usize;
    let mut links: Vec<String> = Vec::new();
    let add = |cells: &mut Vec<Cell>, text: &str, emphasis: usize, link: Option<&String>| {
        cells.extend(text.chars().map(|c| (c, emphasis > 0, link.cloned())));
    };
    for token in tokens {
        match token.kind.as_str() {
            "text" => {
                // `re.sub(r"\s+", " ", content)`.
                let mut collapsed = String::new();
                for c in token.content.chars() {
                    if !c.is_whitespace() {
                        collapsed.push(c);
                    } else if !collapsed.ends_with(' ') {
                        collapsed.push(' ');
                    }
                }
                add(&mut cells, &collapsed, emphasis, links.last());
            }
            "code_inline" => add(&mut cells, &token.content, emphasis, links.last()),
            "softbreak" => add(&mut cells, " ", emphasis, links.last()),
            "hardbreak" => add(&mut cells, "\n", emphasis, links.last()),
            "em_open" => emphasis += 1,
            "em_close" => emphasis = emphasis.saturating_sub(1),
            "link_open" => links.push(token.href.clone().unwrap_or_default()),
            "link_close" => {
                links.pop();
            }
            "image" => {
                let src = token.src.clone().unwrap_or_default();
                let text = format!("🖼  {}", token.children.concat());
                add(&mut cells, &text, emphasis, Some(&src));
            }
            _ => {}
        }
    }
    cells
}

/// What the Rust renderer draws for `source`: each non-blank row's cells, trailing spaces dropped.
fn rendered(source: &str) -> Vec<Vec<Cell>> {
    let rendered = Markdown::parse(source).layout_unpadded(WIDTH, &Theme::default());
    let mut rows = Vec::new();
    for (row, line) in rendered.lines.iter().enumerate() {
        let mut cells = Vec::new();
        let mut column = 0_u16;
        for span in &line.spans {
            let italic = span.style.add_modifier.contains(Modifier::ITALIC);
            for c in span.content.chars() {
                let target = rendered
                    .links
                    .iter()
                    .find(|link| link.row == row && link.columns.contains(&column))
                    .map(|link| link.target.to_string());
                cells.push((c, italic, target));
                let width = ratatui::text::Span::raw(c.to_string()).width();
                column += u16::try_from(width).unwrap();
            }
        }
        while cells.last().is_some_and(|(c, _, link)| *c == ' ' && link.is_none()) {
            cells.pop();
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    rows
}

fn runs(cells: &[Cell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (c, italic, link) in cells {
        match runs.last_mut() {
            Some((text, i, l)) if i == italic && l == link => text.push(*c),
            _ => runs.push((c.to_string(), *italic, link.clone())),
        }
    }
    runs
}

/// The case's mismatch, if Rust draws it differently from Python.
fn mismatch(case: &Case) -> Option<String> {
    let expected: Vec<Vec<Run>> = case
        .blocks
        .iter()
        .map(|tokens| runs(&expected(tokens)))
        .filter(|runs| !runs.is_empty())
        .collect();
    let actual: Vec<Vec<Run>> = rendered(&case.source).iter().map(|cells| runs(cells)).collect();
    (expected != actual).then(|| format!("{:?}\n  python: {expected:?}\n    rust: {actual:?}", case.source))
}

fn known_gap(case: &Case) -> Option<&'static str> {
    KNOWN_GAPS
        .iter()
        .find(|(start, _)| case.source.starts_with(start))
        .map(|&(_, issue)| issue)
}

#[test]
fn links_match_markdown_it() {
    let mut failures = Vec::new();
    for case in cases() {
        match (mismatch(&case), known_gap(&case)) {
            (Some(diff), None) => failures.push(diff),
            (None, Some(issue)) => failures.push(format!(
                "{:?} matches now: take it out of KNOWN_GAPS ({issue})",
                case.source
            )),
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn the_golden_has_every_case() {
    let cases = cases();
    assert!(cases.len() > 100, "{} cases", cases.len());
    assert!(cases.iter().all(|case| !case.blocks.is_empty()));
}

/// Regenerates `linkify.json` with markdown-it-py and checks the committed copy matches (the golden manifest is
/// `parity/goldens/linkify.toml`).
#[test]
#[ignore = "needs uv and the Python app"]
fn linkify_golden_is_fresh() {
    let home = tempfile::tempdir().unwrap();
    common::assert_goldens_fresh(
        "crates/lazy-github-tui/tests/fixtures/generate_linkify.py",
        &["crates/lazy-github-tui/tests/fixtures/linkify.json"],
        home.path(),
        "uv run python crates/lazy-github-tui/tests/fixtures/generate_linkify.py",
    );
}
