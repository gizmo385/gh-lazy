// Colors are written as the hex Textual prints.
#![allow(clippy::unreadable_literal)]

use std::path::{Path, PathBuf};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;

fn dark() -> Theme {
    Theme::default()
}

fn theme(name: &str) -> Theme {
    Theme::by_name(name).unwrap()
}

fn render(source: &str, width: u16) -> Rendered {
    Markdown::parse(source).layout(width, &dark())
}

/// The rows as text, trailing spaces dropped.
fn text(source: &str, width: u16) -> Vec<String> {
    rows(&render(source, width))
}

fn rows(rendered: &Rendered) -> Vec<String> {
    rendered
        .lines
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect()
}

/// The rendered lines drawn into a buffer as wide as the layout and as tall as its lines.
fn buffer(rendered: &Rendered, width: u16) -> Buffer {
    let height = u16::try_from(rendered.lines.len()).unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    for (y, line) in rendered.lines.iter().enumerate() {
        buffer.set_line(0, u16::try_from(y).unwrap(), line, width);
    }
    buffer
}

fn rgb(hex: u32) -> Color {
    let [_, r, g, b] = hex.to_be_bytes();
    Color::Rgb(r, g, b)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The `repo-overview` scenario's hello-world README, as the fixture serves it.
fn hello_world_readme() -> String {
    let path = repo_root().join("parity/fixtures/repo-overview/repos/parity-bot/hello-world/readme/GET.raw.json");
    let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

#[test]
fn whitespace_collapses_to_one_space() {
    assert_eq!(text("a   b \t c", 40), ["  a b c", ""]);
    // Inline code keeps its spaces.
    assert_eq!(text("`a   b`", 40), ["  a   b", ""]);
}

#[test]
fn soft_breaks_are_spaces_and_hard_breaks_new_rows() {
    assert_eq!(text("one\ntwo", 40), ["  one two", ""]);
    assert_eq!(text("one  \ntwo\\\nthree", 40), ["  one", "  two", "  three", ""]);
}

#[test]
fn inline_styles() {
    let rendered = render("**b** *i* ~~s~~ `c` [l](https://x)", 40);
    let buf = buffer(&rendered, 40);
    let cell = |x: u16| &buf[(x, 0)];
    let fg = rgb(0xE0E0E0);
    assert_eq!(
        (cell(2).symbol(), cell(2).fg, cell(2).modifier),
        ("b", fg, Modifier::BOLD)
    );
    assert_eq!((cell(4).symbol(), cell(4).modifier), ("i", Modifier::ITALIC));
    assert_eq!((cell(6).symbol(), cell(6).modifier), ("s", Modifier::CROSSED_OUT));
    // Inline code: `$text-warning 95%` on `$warning 10%`, as Textual draws it.
    assert_eq!(
        (cell(8).symbol(), cell(8).fg, cell(8).bg),
        ("c", rgb(0xF4BB6E), rgb(0x241D13))
    );
    // A link is underlined in the text's color.
    assert_eq!(
        (cell(10).symbol(), cell(10).fg, cell(10).modifier),
        ("l", fg, Modifier::UNDERLINED)
    );
    // Styles nest.
    let buf = buffer(&render("***~~x~~***", 10), 10);
    assert_eq!(
        buf[(2, 0)].modifier,
        Modifier::BOLD | Modifier::ITALIC | Modifier::CROSSED_OUT
    );
}

#[test]
fn link_spans_follow_the_wrapped_text() {
    // Eight columns of text: "aa bb cc" then "dd ee", with the link over "bb cc" and "dd".
    let rendered = render("aa [bb cc dd](https://example.com/x) ee", 12);
    assert_eq!(rows(&rendered), ["  aa bb cc", "  dd ee", ""]);
    let target: Arc<str> = Arc::from("https://example.com/x");
    assert_eq!(
        rendered.links,
        [
            LinkSpan {
                row: 0,
                columns: 5..10,
                target: Arc::clone(&target)
            },
            LinkSpan {
                row: 1,
                columns: 2..4,
                target
            },
        ]
    );
    // Styled parts of one link make one span; two links to the same place stay two.
    let rendered = render("[a **b**](u)[c](u)", 20);
    let columns: Vec<_> = rendered.links.iter().map(|l| l.columns.clone()).collect();
    assert_eq!(columns, [2..5, 5..6]);
}

#[test]
fn images_show_their_alt_text_after_a_picture() {
    // Textual's own output: markdown-it leaves the image's `alt` attribute empty, so only its text follows.
    let rendered = render("![alt *text*](https://example.com/a.png) after", 40);
    assert_eq!(rows(&rendered), ["  🖼  alt text after", ""]);
    assert_eq!(rendered.links.len(), 1);
    assert_eq!(rendered.links[0].columns, 2..13);
    assert_eq!(&*rendered.links[0].target, "https://example.com/a.png");
    // The alt text's own styles don't apply.
    let buf = buffer(&rendered, 40);
    assert_eq!(buf[(10, 0)].modifier, Modifier::UNDERLINED);
}

#[test]
fn inline_html_drops_its_tags_and_keeps_its_text() {
    assert_eq!(text("a <b>bold</b> c", 40), ["  a bold c", ""]);
    // Each side of the tag is a text token of its own, so their spaces don't collapse together, as in Textual.
    assert_eq!(text("a <!-- note --> c", 40), ["  a  c", ""]);
}

#[test]
fn html_blocks_vanish() {
    assert_eq!(
        text("before\n\n<div>\nhidden\n</div>\n\nafter", 40),
        ["  before", "", "  after", ""]
    );
}

#[test]
fn only_a_double_tilde_strikes() {
    let rendered = render("~one~ ~~two~~", 40);
    assert_eq!(rows(&rendered), ["  ~one~ two", ""]);
    let buf = buffer(&rendered, 40);
    assert_eq!(buf[(3, 0)].modifier, Modifier::empty());
    assert_eq!(buf[(8, 0)].modifier, Modifier::CROSSED_OUT);
}

#[test]
fn bullets_change_with_depth_and_cycle() {
    let source = "- a\n  - b\n    - c\n      - d\n        - e\n          - f";
    assert_eq!(
        text(source, 40),
        [
            "  • a",
            "    ▪ b",
            "      ‣ c",
            "        ⭑ d",
            "          ◦ e",
            "            • f",
            ""
        ]
    );
    // Only bullet-list items count: a bullet list inside an ordered one starts again.
    assert_eq!(text("1. a\n   - b", 40), ["   1. a", "      • b", ""]);
    // The bullets are `$text-primary`.
    let buf = buffer(&render("- a", 10), 10);
    assert_eq!(buf[(2, 0)].fg, rgb(0x57A5E2));
}

#[test]
fn ordered_lists_count_from_their_start_right_aligned() {
    assert_eq!(text("9. nine\n10. ten", 40), ["    9. nine", "   10. ten", ""]);
    assert_eq!(text("3. a\n1. b", 40), ["   3. a", "   4. b", ""]);
}

#[test]
fn wrapped_items_hang_under_their_text() {
    assert_eq!(
        text("- one two three four five", 16),
        ["  • one two", "    three four", "    five", ""]
    );
    assert_eq!(
        text("10. one two three four", 16),
        ["   10. one two", "       three", "       four", ""]
    );
}

#[test]
fn lists_take_one_blank_row_after_and_nested_lists_none() {
    assert_eq!(
        text("- a\n  - b\n- c\n\nnext", 40),
        ["  • a", "    ▪ b", "  • c", "", "  next", ""]
    );
}

#[test]
fn quotes_have_a_bar_and_their_own_background() {
    let rendered = render("> quoted text", 20);
    assert_eq!(rows(&rendered), ["", "  ▌ quoted text", ""]);
    let buf = buffer(&rendered, 20);
    let boost = rgb(0x1B1B1B);
    let bar = &buf[(2, 1)];
    // `$text-primary 50%` over the background, on `$boost`.
    assert_eq!((bar.symbol(), bar.fg, bar.bg), ("▌", rgb(0x345B7A), boost));
    assert_eq!((buf[(4, 1)].fg, buf[(4, 1)].bg), (rgb(0xE0E0E0), boost));
    // The background runs to the right edge of the text area, and the padding stays outside it.
    assert_eq!(buf[(17, 1)].bg, boost);
    assert_eq!(buf[(18, 1)].bg, Color::Reset);
    // Inside, paragraphs have no margins and a nested quote has its own bar.
    assert_eq!(
        text("> a\n>\n> b\n>\n> > c", 20),
        ["", "  ▌ a", "  ▌ b", "  ▌", "  ▌ ▌ c", "  ▌", ""]
    );
}

#[test]
fn code_is_padded_and_cut_at_the_edge() {
    let rendered = render("```\n0123456789012345678\n\tx\n```", 20);
    assert_eq!(rows(&rendered), ["", "", "    01234567890123", "            x", "", ""]);
    let buf = buffer(&rendered, 20);
    let fence = rgb(0x101010);
    // The padding rows and columns are on the fence background, and the code is `$text` at full strength.
    for (x, y) in [(2, 1), (17, 1), (2, 2), (3, 2), (17, 3), (2, 4)] {
        assert_eq!(buf[(x, y)].bg, fence, "({x}, {y})");
    }
    assert_eq!((buf[(4, 2)].fg, buf[(4, 2)].bg), (rgb(0xFFFFFF), fence));
    assert_eq!(buf[(18, 2)].bg, Color::Reset);
    // Indented code is the same block.
    assert_eq!(text("    indented", 20), ["", "", "    indented", "", ""]);
}

#[test]
fn trailing_blank_lines_leave_a_fence() {
    assert_eq!(
        text("```\ncode\n\n\n```\nafter", 20),
        ["", "", "    code", "", "", "  after", ""]
    );
    // An empty fence is still a row between its padding.
    assert_eq!(text("```\n```", 20), ["", "", "", "", ""]);
}

#[test]
fn rules_are_a_blank_row_and_a_line() {
    let rendered = render("a\n\n---\n\nb", 12);
    assert_eq!(rows(&rendered), ["  a", "", "", "  ────────", "", "  b", ""]);
    let buf = buffer(&rendered, 12);
    assert_eq!(buf[(2, 3)].fg, rgb(0x004578));
}

#[test]
fn heading_margins_collapse_and_the_first_one_stays() {
    assert_eq!(
        text("## Two\n\npara\n\n# One\n\n### Three\n#### Four\n\nend", 20),
        [
            "",
            "",
            "  Two",
            "",
            "  para",
            "",
            "",
            "        One",
            "",
            "  Three",
            "",
            "  Four",
            "",
            "  end",
            "",
        ]
    );
    // H2 is underlined `$primary`; H1 is bold.
    let buf = buffer(&render("## Two", 20), 20);
    assert_eq!(
        (buf[(2, 2)].fg, buf[(2, 2)].modifier),
        (rgb(0x0178D4), Modifier::UNDERLINED)
    );
}

#[test]
fn h1_is_centered_with_its_whole_row_styled() {
    let rendered = render("# Odd", 20);
    // 16 columns of text: (16 - 3) / 2 = 6 spaces before.
    assert_eq!(rows(&rendered), ["", "", "        Odd", ""]);
    let buf = buffer(&rendered, 20);
    assert_eq!(buf[(2, 2)].modifier, Modifier::BOLD);
    assert_eq!(buf[(17, 2)].modifier, Modifier::BOLD);
    assert_eq!(buf[(1, 2)].modifier, Modifier::empty());
    // Each wrapped row is centered on its own.
    assert_eq!(text("# aaaa bb", 10), ["", "", "   aaaa", "    bb", ""]);
}

#[test]
fn tables_read_as_a_paragraph_per_row_until_216() {
    assert_eq!(
        text("| a | b |\n|---|---|\n| [c](u) | d |", 20),
        ["  a b", "", "  c d", ""]
    );
}

#[test]
fn empty_input_renders_nothing() {
    assert_eq!(render("", 40), Rendered::default());
    assert_eq!(render("  \n\n", 40), Rendered::default());
    assert_eq!(render("<!-- only html -->", 40), Rendered::default());
}

#[test]
fn tiny_widths() {
    let source = "# Title\n\nsome text\n\n> quote\n\n- item\n\n```\ncode\n```\n\n---";
    assert_eq!(render(source, 0), Rendered::default());
    for width in [1, 3, 4] {
        let rendered = render(source, width);
        // Too narrow for the padding: the text starts at the left edge.
        assert!(rows(&rendered).iter().any(|row| row.starts_with('s')), "{width}");
        for (row, line) in rendered.lines.iter().enumerate() {
            // Only containers inside a too-narrow width spill past it.
            assert!(line.width() <= usize::from(width) + 3, "{width} {row}: {line:?}");
        }
    }
    assert_eq!(text("some text", 4), ["some", "text", ""]);
    assert_eq!(
        text("some text", 5),
        ["  s", "  o", "  m", "  e", "", "  t", "  e", "  x", "  t", ""]
    );
}

#[test]
fn deep_nesting_and_odd_input_do_not_panic() {
    let deep_quote = format!("{} deep", ">".repeat(200));
    let deep_list: String = (0..100).map(|i| "  ".repeat(i) + "- item\n").collect();
    let samples = [
        deep_quote.as_str(),
        deep_list.as_str(),
        "- \n-\n- a",
        ">",
        "#",
        "![](x)",
        "[](x)",
        "**",
        "~~~",
        "| a |\n|---|\n",
        "\t\tcode\twith\ttabs",
        "中文字符和全角文字混合 very long text 🙂🙂🙂 e\u{301}",
        "a\u{0}b\u{7}c",
        "<details>\n\n- a\n\n</details>",
    ];
    for source in samples {
        let markdown = Markdown::parse(source);
        for width in 0..30 {
            markdown.layout(width, &dark());
        }
    }
    assert!(text(&deep_quote, 200).iter().any(|row| row.ends_with("deep")));
}

#[test]
fn wide_graphemes_wrap_by_cells() {
    // A folded word keeps its trailing space, which starts the next row, as in Rich.
    assert_eq!(text("中文中文 ab", 8), ["  中文", "  中文", "   ab", ""]);
    assert_eq!(wrap("e\u{301}e\u{301}e\u{301}", 2), ["e\u{301}e\u{301}", "e\u{301}"]);
}

/// The README part of a `repo-overview` capture: from the H1's row down to the rule above the table, or to the
/// bottom of the Details box.
fn python_readme(size: &str, width: usize) -> Vec<String> {
    let path = repo_root()
        .join("parity/references/repo-overview")
        .join(size)
        .join("readme.screen");
    let screen = std::fs::read_to_string(path).unwrap();
    let rows: Vec<Vec<char>> = screen
        .lines()
        .skip_while(|line| *line != "--- text")
        .skip(1)
        .take_while(|line| !line.starts_with("--- "))
        .map(|line| line.chars().collect())
        .collect();
    let left = rows
        .iter()
        .find_map(|row| {
            let line: String = row.iter().collect();
            let byte = line.find("┌─ [5]")?;
            Some(line[..byte].chars().count() + 1)
        })
        .unwrap();
    let pane: Vec<String> = rows
        .iter()
        .map(|row| {
            row.iter()
                .skip(left)
                .take(width)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let h1 = pane.iter().position(|row| row.trim() == "Hello World").unwrap();
    pane[h1..]
        .iter()
        // The box's bottom border; the README's own rule is indented.
        .take_while(|row| !row.starts_with('─'))
        .map(Clone::clone)
        .collect::<Vec<_>>()
        .into_iter()
        .scan(false, |past_rule, row| {
            // Stop after the rule: what follows is the table (#216).
            if *past_rule {
                return None;
            }
            *past_rule = row.trim_start().starts_with('─');
            Some(row)
        })
        .collect()
}

#[rstest::rstest]
#[case::wide("160x48", 92)]
#[case::narrow("100x30", 56)]
fn the_readme_matches_python(#[case] size: &str, #[case] width: u16) {
    let python = python_readme(size, usize::from(width));
    assert!(python.len() > 10, "{python:?}");
    let ours = text(&hello_world_readme(), width);
    // The README starts with its H1's two-row margin.
    assert_eq!(ours[..2], ["", ""]);
    assert_eq!(ours[2..2 + python.len()], python[..]);
}

#[test]
fn markdown_blocks_92() {
    let rendered = Markdown::parse(&hello_world_readme()).layout(92, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 92));
}

#[test]
fn markdown_blocks_56() {
    let rendered = Markdown::parse(&hello_world_readme()).layout(56, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 56));
}

/// One of everything, for the theme snapshots.
const SAMPLER: &str = "# Title

A paragraph with **bold**, *italic*, ~~struck~~, `code` and a [link](https://example.com).

## Section

- one
  - nested

1. first

> quoted `code`

```sh
echo hi
```

---
";

#[test]
fn markdown_inline_styles() {
    let source = "**bold** *italic* ~~struck~~ `code` [link](https://example.com) [`code link`](u) ![alt](i.png) \
                  ***~~all~~***";
    let rendered = Markdown::parse(source).layout(40, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 40));
}

#[test]
fn markdown_light_theme() {
    let rendered = Markdown::parse(SAMPLER).layout(40, &theme("textual-light"));
    insta::assert_debug_snapshot!(buffer(&rendered, 40));
}

#[test]
fn markdown_ansi_dark() {
    let rendered = Markdown::parse(SAMPLER).layout(40, &theme("ansi-dark"));
    insta::assert_debug_snapshot!(buffer(&rendered, 40));
}

#[test]
fn ansi_themes_drop_the_code_backgrounds_and_fence_margins() {
    let rendered = Markdown::parse("x `c`\n\n```\ncode\n```").layout(20, &theme("ansi-dark"));
    // No margin around the fence (the blank row is the paragraph's), and no left padding inside it.
    assert_eq!(rows(&rendered), ["  x c", "", "", "  code", ""]);
    let buf = buffer(&rendered, 20);
    assert_eq!((buf[(4, 0)].fg, buf[(4, 0)].bg), (Color::Yellow, Color::Reset));
    assert_eq!(buf[(2, 3)].bg, Color::Reset);
}

#[test]
fn a_theme_change_recolors_the_same_document() {
    let markdown = Markdown::parse("## Two");
    let dark = buffer(&markdown.layout(20, &dark()), 20);
    let nord = buffer(&markdown.layout(20, &theme("nord")), 20);
    assert_eq!(dark[(2, 2)].fg, rgb(0x0178D4));
    assert_eq!(nord[(2, 2)].fg, rgb(0x88C0D0));
}
