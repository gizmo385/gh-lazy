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
fn tables_have_keylines_between_every_cell() {
    assert_eq!(
        text("| a | b |\n|---|---|\n| [c](u) | d |\n\nafter", 20),
        [
            "  ┌──────┬───────┐",
            "  │ a    │ b     │",
            "  ├──────┼───────┤",
            "  │ c    │ d     │",
            "  └──────┴───────┘",
            "",
            "  after",
            "",
        ]
    );
}

#[test]
fn table_cells_keep_their_links_and_styles() {
    let rendered = render("| h |\n|---|\n| x [**c**](https://e.x) `k` |", 20);
    assert_eq!(rendered.links.len(), 1);
    assert_eq!(rendered.links[0].row, 3);
    // Two columns of padding, the keyline and a cell's padding before "x c".
    assert_eq!(rendered.links[0].columns, 6..7);
    assert_eq!(&*rendered.links[0].target, "https://e.x");
    let buf = buffer(&rendered, 20);
    // Keylines are `$foreground 20%`, the header `$primary`, body text `$foreground`.
    assert_eq!(buf[(2, 0)].fg, rgb(0x3B3B3B));
    assert_eq!(buf[(4, 1)].fg, rgb(0x0178D4));
    assert_eq!(buf[(4, 3)].fg, rgb(0xE0E0E0));
    assert_eq!(buf[(6, 3)].modifier, Modifier::BOLD | Modifier::UNDERLINED);
    // Inline code shows without its backticks.
    assert_eq!(buf[(8, 3)].symbol(), "k");
    assert_eq!(buf[(8, 3)].fg, rgb(0xF4BB6E));
}

#[test]
fn ragged_table_rows_are_padded_and_cut_to_the_header() {
    // As Textual 8.2.5 draws it.
    assert_eq!(
        text("| a | b |\n|---|---|\n| one |\n| p | q | r |", 20),
        [
            "  ┌────────┬─────┐",
            "  │ a      │ b   │",
            "  ├────────┼─────┤",
            "  │ one    │     │",
            "  ├────────┼─────┤",
            "  │ p      │ q   │",
            "  └────────┴─────┘",
            "",
        ]
    );
}

#[test]
fn a_wrapped_header_sits_at_the_bottom_of_its_row() {
    // Checked against Textual 8.2.5 at 30 columns: the long header gives up width down to 20, and `b` drops to the
    // header row's last line (`content-align: left bottom`).
    assert_eq!(
        text("| Long header that wraps around | b |\n| --- | --- |\n| x | y |", 30),
        [
            "  ┌────────────────────┬───┐",
            "  │ Long header that   │   │",
            "  │ wraps around       │ b │",
            "  ├────────────────────┼───┤",
            "  │ x                  │ y │",
            "  └────────────────────┴───┘",
            "",
        ]
    );
}

#[test]
fn light_tables_have_their_own_background() {
    let rendered = Markdown::parse("| Name | Value |\n|---|---|\n| a | 1 |").layout(30, &theme("textual-light"));
    let buf = buffer(&rendered, 30);
    // As Textual draws it: `white 30%` under the table, the keylines `$foreground 20%` over that.
    assert_eq!((buf[(2, 0)].fg, buf[(2, 0)].bg), (rgb(0xC0C0C0), rgb(0xE9E9E9)));
    assert_eq!((buf[(4, 1)].fg, buf[(4, 1)].bg), (rgb(0x004578), rgb(0xE9E9E9)));
    assert_eq!((buf[(4, 3)].fg, buf[(4, 3)].bg), (rgb(0x1F1F1F), rgb(0xE9E9E9)));
}

/// The hello-world README's two tables.
const README_TABLES: &str = "| Name | Value |
| ---- | ----- |
| a    | 1     |

| Option | Description |
| ------ | ----------- |
| `--verbose` | Prints every request and response while it runs, which is a lot of output for a long session. |
| `-q` | Quiet. |
";

const OPTIONS_DESCRIPTION: &str =
    "Prints every request and response while it runs, which is a lot of output for a long session.";

#[rstest::rstest]
// The 2 x 2 table: natural widths 6 and 7, spread over the 85 or 49 columns between the keylines.
#[case::small_wide(&[6, 7], &[6, 7], 88, &[39, 46])]
#[case::small_narrow(&[6, 7], &[6, 7], 52, &[22, 27])]
// The Options table: the description gives up width and wraps, the first column keeps its natural 11.
#[case::options_wide(&[11, OPTIONS_DESCRIPTION.len() + 2], &[11, 13], 88, &[11, 74])]
#[case::options_narrow(&[11, OPTIONS_DESCRIPTION.len() + 2], &[11, 13], 52, &[11, 38])]
#[case::one_column(&[5], &[5], 20, &[18])]
#[case::exact_fit(&[6, 7], &[6, 7], 16, &[6, 7])]
// Every column over its minimum: the first gives up what it can, then all shrink in proportion.
#[case::below_minimums(&[10, 10], &[8, 8], 15, &[6, 6])]
#[case::narrower_than_padding(&[6, 7], &[6, 7], 3, &[0, 0])]
#[case::no_width(&[6, 7], &[6, 7], 0, &[0, 0])]
fn column_widths_resolve_as_textual(
    #[case] natural: &[usize],
    #[case] minimums: &[usize],
    #[case] width: usize,
    #[case] expected: &[usize],
) {
    assert_eq!(table::column_widths(natural, minimums, width), expected);
}

#[test]
fn narrow_tables_do_not_panic() {
    let markdown = Markdown::parse(README_TABLES);
    for width in 0..30 {
        markdown.layout(width, &dark());
    }
}

/// The tables part of a `repo-overview` `scrolled` capture: from the first table's top keyline to the second's
/// bottom one.
fn python_tables(size: &str, width: usize) -> Vec<String> {
    let path = repo_root()
        .join("parity/references/repo-overview")
        .join(size)
        .join("scrolled.screen");
    let screen = std::fs::read_to_string(path).unwrap();
    let rows: Vec<&str> = screen
        .lines()
        .skip_while(|line| *line != "--- text")
        .skip(1)
        .take_while(|line| !line.starts_with("--- "))
        .collect();
    let left = rows
        .iter()
        .find_map(|row| Some(row[..row.find("┌─ [5]")?].chars().count() + 1))
        .unwrap();
    let pane: Vec<String> = rows
        .iter()
        .map(|row| {
            row.chars()
                .skip(left)
                .take(width)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let first = pane.iter().position(|row| row.starts_with("  ┌")).unwrap();
    let last = pane.iter().rposition(|row| row.starts_with("  └")).unwrap();
    pane[first..=last].to_vec()
}

#[rstest::rstest]
#[case::wide("160x48", 92)]
#[case::narrow("100x30", 56)]
fn the_readme_tables_match_python(#[case] size: &str, #[case] width: u16) {
    let python = python_tables(size, usize::from(width));
    assert!(python.len() > 10, "{python:?}");
    let ours = text(&hello_world_readme(), width);
    let first = ours.iter().position(|row| row.starts_with("  ┌")).unwrap();
    assert_eq!(ours[first..first + python.len()], python[..]);
}

#[test]
fn markdown_table_92() {
    let rendered = Markdown::parse(README_TABLES).layout(92, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 92));
}

#[test]
fn markdown_table_56() {
    let rendered = Markdown::parse(README_TABLES).layout(56, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 56));
}

/// Alignment colons are ignored: every cell is left-aligned, as in Textual.
const WRAPPING_TABLE: &str =
    "| Long header that wraps | `k` |\n| :-- | --: |\n| a [link that wraps](https://example.com) | **b** |";

#[test]
fn wrapped_cells_grow_their_row_and_keep_their_links() {
    // As Textual 8.2.5 draws it at 24 columns.
    let rendered = render(WRAPPING_TABLE, 24);
    assert_eq!(
        rows(&rendered),
        [
            "  ┌──────────────┬───┐",
            "  │ Long header  │   │",
            "  │ that wraps   │ k │",
            "  ├──────────────┼───┤",
            "  │ a link that  │ b │",
            "  │ wraps        │   │",
            "  └──────────────┴───┘",
            "",
        ]
    );
    let spans: Vec<_> = rendered.links.iter().map(|l| (l.row, l.columns.clone())).collect();
    assert_eq!(spans, [(4, 6..15), (5, 4..9)]);
}

#[test]
fn markdown_table_wrapping() {
    let rendered = Markdown::parse(WRAPPING_TABLE).layout(24, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 24));
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
    // Past the depth limit the text is flattened, scheme URLs as written.
    let deep_url = format!("{} see http://a.com/*b*", ">".repeat(200));
    assert!(
        text(&deep_url, 300)
            .iter()
            .any(|row| row.ends_with("see http://a.com/*b*"))
    );
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

/// A row's cells in `columns`: `(symbol, fg, modifier)` for each.
fn code_cells(
    rendered: &Rendered,
    width: u16,
    row: u16,
    columns: std::ops::Range<u16>,
) -> Vec<(String, Color, Modifier)> {
    let buf = buffer(rendered, width);
    columns
        .map(|x| {
            let cell = &buf[(x, row)];
            (cell.symbol().to_owned(), cell.fg, cell.modifier)
        })
        .collect()
}

#[test]
fn sh_fences_color_echo_as_python_does() {
    // `parity/references/repo-overview/160x48/readme.screen`: `echo` is `#ffc473` and its argument `#ffffff`.
    let rendered = render("```sh\necho hello\n```", 40);
    let cells = code_cells(&rendered, 40, 2, 4..14);
    assert!(cells[..4].iter().all(|(_, fg, _)| *fg == rgb(0xFFC473)), "{cells:?}");
    assert!(cells[4..].iter().all(|(_, fg, _)| *fg == rgb(0xFFFFFF)), "{cells:?}");
    let buf = buffer(&rendered, 40);
    assert_eq!(buf[(4, 2)].bg, rgb(0x101010));
}

#[test]
fn python_fences_take_the_highlight_themes_colors() {
    // Each color as Textual 8.2.5 draws the same token on `textual-dark`.
    let source = "```python\n# note\ndef foo(x):\n    return \"s\" + 'c' and None\n```";
    let rendered = render(source, 60);
    let buf = buffer(&rendered, 60);
    let at = |x: u16, y: u16| (buf[(x, y)].symbol().to_owned(), buf[(x, y)].fg, buf[(x, y)].modifier);
    // `$text 60%`.
    assert_eq!(at(4, 2), ("#".to_owned(), rgb(0x9F9F9F), Modifier::empty()));
    // `$text-accent`, then `$text-warning underline`, then `$text-primary`, and `$text` between.
    assert_eq!(at(4, 3), ("d".to_owned(), rgb(0xFFC473), Modifier::empty()));
    assert_eq!(at(8, 3), ("f".to_owned(), rgb(0xFFC473), Modifier::UNDERLINED));
    assert_eq!(at(12, 3), ("x".to_owned(), rgb(0x57A5E2), Modifier::empty()));
    assert_eq!(at(11, 3), ("(".to_owned(), rgb(0xFFFFFF), Modifier::empty()));
    // `$text-success 90%` for double quotes, `$text-success` for single, `bold $text-error` for `and`, and
    // `bold $text-success 80%` for `None`.
    assert_eq!(at(15, 4), ("\"".to_owned(), rgb(0x7DC092), Modifier::empty()));
    assert_eq!(at(21, 4), ("'".to_owned(), rgb(0x8AD4A1), Modifier::empty()));
    assert_eq!(at(25, 4), ("a".to_owned(), rgb(0xD17E92), Modifier::BOLD));
    assert_eq!(at(29, 4), ("N".to_owned(), rgb(0x71AC84), Modifier::BOLD));
}

#[test]
fn unknown_unlabeled_and_indented_code_is_plain_text() {
    for source in [
        "```klingon\nfn main() { 1 }\n```",
        "```\nfn main() { 1 }\n```",
        "    fn main() { 1 }",
        // Pygments takes the whole info string as the language name.
        "```rust ignore\nfn main() { 1 }\n```",
    ] {
        let cells = code_cells(&render(source, 40), 40, 2, 4..19);
        assert!(
            cells.iter().all(|(_, fg, m)| *fg == rgb(0xFFFFFF) && m.is_empty()),
            "{source}: {cells:?}"
        );
    }
}

#[test]
fn tabs_expand_before_highlighting() {
    let rendered = render("```sh\n\techo x\n```", 40);
    assert_eq!(rows(&rendered)[2], "            echo x");
    let cells = code_cells(&rendered, 40, 2, 12..16);
    assert!(cells.iter().all(|(_, fg, _)| *fg == rgb(0xFFC473)), "{cells:?}");
}

#[test]
fn ansi_themes_use_the_ansi_highlight_themes() {
    // As Textual 8.2.5 draws it on both ANSI themes.
    let source = "```python\ndef f(): return 'c' + \"d\" + 1\n```";
    for (name, number) in [
        ("ansi-dark", (Color::Yellow, Modifier::empty())),
        ("ansi-light", (Color::Blue, Modifier::BOLD)),
    ] {
        let rendered = Markdown::parse(source).layout(40, &theme(name));
        let buf = buffer(&rendered, 40);
        // No fence padding or margin: the code starts at the document's padding, on the first row.
        assert_eq!(buf[(2, 1)].symbol(), "d", "{name}");
        assert_eq!(
            (buf[(2, 1)].fg, buf[(2, 1)].modifier),
            (Color::Magenta, Modifier::BOLD),
            "{name}"
        );
        // `ansi_greenb` doesn't parse, so single-quoted strings are plain; double-quoted ones are green.
        assert_eq!(buf[(18, 1)].fg, Color::Reset, "{name}");
        assert_eq!(buf[(24, 1)].fg, Color::Green, "{name}");
        assert_eq!((buf[(30, 1)].fg, buf[(30, 1)].modifier), number, "{name}");
    }
}

#[test]
fn a_theme_change_recolors_the_highlighting() {
    let markdown = Markdown::parse("```sh\necho hello\n```");
    let dark = buffer(&markdown.layout(20, &dark()), 20);
    let light = buffer(&markdown.layout(20, &theme("textual-light")), 20);
    assert_eq!(dark[(4, 2)].fg, rgb(0xFFC473));
    assert_ne!(light[(4, 2)].fg, dark[(4, 2)].fg);
    assert_ne!(light[(4, 2)].fg, light[(9, 2)].fg);
}

/// Each link's text and target, in order.
fn link_texts(rendered: &Rendered) -> Vec<(String, String)> {
    let mut links: Vec<(String, String)> = Vec::new();
    let mut last: Option<(usize, u16, Arc<str>)> = None;
    for link in &rendered.links {
        let line = rendered.lines[link.row].to_string();
        let text: String = line
            .chars()
            .skip(usize::from(link.columns.start))
            .take(usize::from(link.columns.end - link.columns.start))
            .collect();
        // A link that wraps continues on the next row from the left padding.
        let continues = last
            .as_ref()
            .is_some_and(|(row, _, target)| *row + 1 == link.row && Arc::ptr_eq(target, &link.target));
        match links.last_mut() {
            Some((previous, _)) if continues => previous.push_str(&text),
            _ => links.push((text, link.target.to_string())),
        }
        last = Some((link.row, link.columns.end, Arc::clone(&link.target)));
    }
    links
}

#[test]
fn bare_urls_become_links() {
    let rendered = render(
        "see https://example.com. or www.github.com, mail foo@bar.com and <a@b.co>",
        80,
    );
    assert_eq!(
        link_texts(&rendered),
        [
            ("https://example.com".to_owned(), "https://example.com".to_owned()),
            ("www.github.com".to_owned(), "http://www.github.com".to_owned()),
            ("foo@bar.com".to_owned(), "mailto:foo@bar.com".to_owned()),
            ("a@b.co".to_owned(), "mailto:a@b.co".to_owned()),
        ]
    );
    // Underlined like any link.
    let buf = buffer(&rendered, 80);
    assert_eq!(buf[(6, 0)].modifier, Modifier::UNDERLINED);
    assert_eq!(buf[(25, 0)].modifier, Modifier::empty());
}

#[test]
fn linkify_skips_code_fences_and_existing_links() {
    let rendered = render(
        "`https://code.com` [www.inside.com](https://u.com) www.after.com\n\n```\nhttps://fence.com\n```\n\n    www.indented.com",
        80,
    );
    assert_eq!(
        link_texts(&rendered),
        [
            ("www.inside.com".to_owned(), "https://u.com".to_owned()),
            ("www.after.com".to_owned(), "http://www.after.com".to_owned()),
        ]
    );
}

#[test]
fn links_are_found_in_styled_text_headings_and_tables() {
    let rendered = render(
        "# see foo.io\n\n**https://bold.com** *www.em.com*\n\n| a |\n|---|\n| x.org |",
        60,
    );
    let targets: Vec<String> = link_texts(&rendered).into_iter().map(|(_, target)| target).collect();
    assert_eq!(
        targets,
        ["http://foo.io", "https://bold.com", "http://www.em.com", "http://x.org"]
    );
    let buf = buffer(&render("**https://bold.com**", 40), 40);
    assert_eq!(buf[(2, 0)].modifier, Modifier::BOLD | Modifier::UNDERLINED);
}

#[test]
fn link_targets_and_autolink_text_are_normalized() {
    let rendered = render(
        "<http://xn--4ca.com/%C3%A4> [t](http://ä.com/ä) ![i](http://a.com/[x].png) <a@bär.com> www.xn--bcher-kva.de",
        120,
    );
    assert_eq!(
        link_texts(&rendered),
        [
            ("http://ä.com/ä".to_owned(), "http://xn--4ca.com/%C3%A4".to_owned()),
            ("t".to_owned(), "http://xn--4ca.com/%C3%A4".to_owned()),
            ("🖼  i".to_owned(), "http://a.com/%5Bx%5D.png".to_owned()),
            ("a@bär.com".to_owned(), "mailto:a@xn--br-via.com".to_owned()),
            ("www.bücher.de".to_owned(), "http://www.xn--bcher-kva.de".to_owned()),
        ]
    );
}

#[test]
fn a_link_markdown_it_rejects_shows_its_markup() {
    let rendered = render(
        "a [*t*](javascript:x) www.b.com <javascript:y> ![i](file:z) [](vbscript:v)",
        100,
    );
    assert_eq!(
        rows(&rendered),
        [
            "  a [t](javascript:x) www.b.com <javascript:y> ![i](file:z) [](vbscript:v)",
            ""
        ]
    );
    // The link text is still parsed, and the text around it still linkified.
    assert_eq!(
        link_texts(&rendered),
        [("www.b.com".to_owned(), "http://www.b.com".to_owned())]
    );
    let buf = buffer(&rendered, 100);
    assert!(buf[(5, 0)].modifier.contains(Modifier::ITALIC));
    assert!(!buf[(4, 0)].modifier.contains(Modifier::ITALIC));
}

#[test]
fn a_wrapped_url_is_one_link_over_both_rows() {
    let rendered = render("see https://example.com/a/long/path", 24);
    assert_eq!(
        rows(&rendered),
        ["  see", "  https://example.com/", "  a/long/path", ""]
    );
    let target: Arc<str> = Arc::from("https://example.com/a/long/path");
    let spans: Vec<_> = rendered
        .links
        .iter()
        .map(|link| (link.row, link.columns.clone(), *link.target == *target))
        .collect();
    assert_eq!(spans, [(1, 2..22, true), (2, 2..13, true)]);
    assert_eq!(link_texts(&rendered), [(target.to_string(), target.to_string())]);
}

#[test]
fn markdown_linkify() {
    let source = "Visit https://example.com/a_(b), www.github.com or example.com/docs?x=1#y. Mail foo@bar.com or \
                  <a@b.co>. Not config.json, v1.2, x.dev, x@y, skype:me or 1.2.3.4, nor `https://code.com`; see \
                  README.md and [t](https://u.com) https://after.com.";
    let rendered = Markdown::parse(source).layout(60, &dark());
    insta::assert_debug_snapshot!((buffer(&rendered, 60), &rendered.links));
}

#[test]
fn markdown_code_rust() {
    let source = "```rust\n/// Doc.\n#[derive(Debug)]\npub struct Point { x: i32 }\n\nfn main() {\n    let p = \"point\"; // note\n    println!(\"{p} {}\", 42);\n}\n```";
    let rendered = Markdown::parse(source).layout(50, &dark());
    insta::assert_debug_snapshot!(buffer(&rendered, 50));
}

#[test]
fn markdown_code_sh_ansi_dark() {
    let source = "```sh\n# install\nexport PATH=\"$HOME/bin:$PATH\"\nif [ -f x ]; then echo 'ok' 42; fi\n```";
    let rendered = Markdown::parse(source).layout(50, &theme("ansi-dark"));
    insta::assert_debug_snapshot!(buffer(&rendered, 50));
}

/// markdown-it links `http://a.com/*b` before it pairs emphasis: no italics on the link, a plain `*`, and `c` in
/// italics.
#[test]
fn markdown_linkify_before_emphasis() {
    let rendered = Markdown::parse("see http://a.com/*b* and *c*").layout(40, &dark());
    insta::assert_debug_snapshot!((buffer(&rendered, 40), &rendered.links));
}

mod never_panics {
    use proptest::prelude::*;

    use super::*;

    /// Arbitrary text: any `char`s, controls, unassigned and private-use code points included (`any::<String>()` leaves
    /// out the "Other" categories).
    fn text() -> impl Strategy<Value = String> {
        proptest::collection::vec(any::<char>(), 0..64).prop_map(String::from_iter)
    }

    /// Markdown-shaped text: block and inline syntax mixed with arbitrary characters, controls, wide and combining
    /// characters included.
    fn source() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            text(),
            Just("\n".to_owned()),
            Just("\n\n".to_owned()),
            Just("# ".to_owned()),
            Just("- ".to_owned()),
            Just("1. ".to_owned()),
            Just("> ".to_owned()),
            Just("```\n".to_owned()),
            Just("| a | 日本 |\n| --- | --- |\n| 👨\u{200d}👩\u{200d}👧 | e\u{301} |\n".to_owned()),
            Just("**".to_owned()),
            Just("`".to_owned()),
            Just("[x](http://a.com) www.b.org ".to_owned()),
            Just("\t".to_owned()),
            Just("\x1b[31m".to_owned()),
            Just("全角文字".to_owned()),
        ];
        proptest::collection::vec(piece, 0..16).prop_map(|pieces| pieces.concat())
    }

    proptest! {
        #[test]
        fn rendered_lines_fit_the_width(source in source(), width in 0u16..60) {
            let rendered = render(&source, width);
            for line in &rendered.lines {
                prop_assert!(line.width() <= usize::from(width), "{line:?} is wider than {width}");
                prop_assert!(!line.spans.iter().any(|span| span.content.contains(char::is_control)));
            }
        }

        #[test]
        fn plain_text_fits_the_width(text in text(), width in 0u16..60) {
            let rendered = wrap_text(&[TextRun::new(text, Style::new())], width);
            for line in &rendered.lines {
                prop_assert!(line.width() <= usize::from(width), "{line:?} is wider than {width}");
            }
        }
    }
}
