use proptest::prelude::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget as _;
use rstest::rstest;

use super::*;

/// The family emoji: three people joined by ZWJs, one grapheme two cells wide.
const FAMILY: &str = "👨\u{200d}👩\u{200d}👧";

#[rstest]
#[case::ascii("abc", 3)]
#[case::cjk("日本語", 6)]
#[case::hangul("한국어", 6)]
#[case::emoji("👍", 2)]
#[case::zwj_sequence(FAMILY, 2)]
#[case::flag("🇯🇵", 2)]
#[case::emoji_presentation("❤\u{fe0f}", 2)]
#[case::combining_mark("e\u{301}", 1)]
#[case::zero_width_space("a\u{200b}b", 2)]
#[case::right_to_left("שלום", 4)]
#[case::controls("a\x1b[31mb\x7f\u{9b}", 6)]
fn width_counts_cells(#[case] text: &str, #[case] cells: usize) {
    assert_eq!(width(text), cells);
}

#[test]
fn width_is_what_ratatui_draws() {
    for text in ["日本語 abc", FAMILY, "e\u{301}x", "a\x1bb", "🇯🇵🇫🇷", "שלום עולם"] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        let drawn = Span::raw(text);
        (&drawn).render(buf.area, &mut buf);
        // The last drawn grapheme's first cell, plus its width.
        let used = (0..30u16)
            .rev()
            .find(|&x| buf[(x, 0)].symbol() != " ")
            .map_or(0, |x| usize::from(x) + width(buf[(x, 0)].symbol()));
        assert_eq!(width(text), used, "{text:?}");
    }
}

#[rstest]
#[case::clean("plain 日本語", "plain 日本語")]
#[case::c0("a\x00b\x1bc\x07", "a\u{fffd}b\u{fffd}c\u{fffd}")]
#[case::del_and_c1("a\x7fb\u{85}c\u{9b}", "a\u{fffd}b\u{fffd}c\u{fffd}")]
#[case::ansi("\x1b[31mred\x1b[0m", "\u{fffd}[31mred\u{fffd}[0m")]
#[case::line_breaks_and_tabs("a\nb\tc\r", "a\u{fffd}b\u{fffd}c\u{fffd}")]
fn sanitize_replaces_every_control(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(sanitize(text), expected);
}

#[test]
fn sanitize_text_keeps_line_breaks_and_tabs() {
    assert_eq!(sanitize_text("a\r\n\tb\x1b[1m\x7f"), "a\r\n\tb\u{fffd}[1m\u{fffd}");
    assert!(matches!(sanitize_text("日本語\n"), Cow::Borrowed(_)));
}

#[test]
fn sanitize_line_keeps_styles() {
    let style = Style::new().bold();
    let line = sanitize_line(Line::from(vec![Span::raw("ok"), Span::styled("a\x1bb", style)]));
    assert_eq!(line.spans[1], Span::styled("a\u{fffd}b", style));
    assert_eq!(line.spans[0], Span::raw("ok"));
}

#[rstest]
#[case::fits("日本語", 6, "日本語")]
#[case::cut_ascii("abcdef", 4, "abc…")]
#[case::wide_fits_exactly("日本語です", 5, "日本…")]
#[case::wide_straddles("日本語です", 6, "日本 …")]
#[case::zwj_kept_whole(&format!("{FAMILY}{FAMILY}x"), 4, &format!("{FAMILY} …"))]
#[case::flag_kept_whole("🇯🇵🇫🇷!", 4, "🇯🇵 …")]
#[case::combining_kept_with_its_base("e\u{301}e\u{301}e\u{301}", 2, "e\u{301}…")]
#[case::one_cell("日本", 1, "…")]
#[case::zero("abc", 0, "")]
fn truncate_ends_with_an_ellipsis_inside_the_width(#[case] text: &str, #[case] cells: usize, #[case] expected: &str) {
    let cut = truncate(text, cells);
    assert_eq!(cut, expected);
    assert!(width(&cut) <= cells);
}

#[test]
fn cut_spans_keeps_whole_graphemes() {
    let spans = vec![
        Span::raw("ab"),
        Span::styled("日本", Style::new().bold()),
        Span::raw("x"),
    ];
    let (kept, left) = cut_spans(spans.clone(), 3);
    assert_eq!(kept, vec![Span::raw("ab")]);
    assert_eq!(left, 1);
    let (kept, left) = cut_spans(spans, 5);
    assert_eq!(kept, vec![Span::raw("ab"), Span::styled("日", Style::new().bold())]);
    assert_eq!(left, 1);
}

/// Arbitrary text: any `char`s, controls, unassigned and private-use code points included (`any::<String>()` leaves
/// out the "Other" categories).
fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..64).prop_map(String::from_iter)
}

proptest! {
    #[test]
    fn truncate_never_exceeds_the_width(text in text(), cells in 0usize..40) {
        let cut = truncate(&text, cells);
        prop_assert!(width(&cut) <= cells);
        if width(&text) > cells && cells > 0 {
            prop_assert!(cut.ends_with(ELLIPSIS));
        }
    }

    #[test]
    fn cut_spans_never_exceeds_the_width(a in text(), b in text(), cells in 0usize..40) {
        let (kept, left) = cut_spans([Span::raw(a), Span::raw(b)], cells);
        let used: usize = kept.iter().map(|span| width(&span.content)).sum();
        prop_assert!(used + left <= cells);
    }

    /// The printable-ASCII shortcut in `width` and `take_cells` (#325) counts what the grapheme walk counts.
    #[test]
    fn the_ascii_shortcut_matches_the_grapheme_walk(text in prop_oneof!["[ -~]{0,40}", "[ -~\t\r\n\x1b]{0,40}"], cells in 0usize..50) {
        let walked: usize = graphemes(&text).map(|(_, g)| grapheme_width(g)).sum();
        prop_assert_eq!(width(&text), walked);
        let mut used = 0;
        let mut end = 0;
        for (start, grapheme) in graphemes(&text) {
            if used + grapheme_width(grapheme) > cells {
                break;
            }
            used += grapheme_width(grapheme);
            end = start + grapheme.len();
        }
        prop_assert_eq!(take_cells(&text, cells), (&text[..end], used));
    }

    #[test]
    fn sanitized_text_has_no_controls(text in text()) {
        prop_assert!(!sanitize(&text).chars().any(char::is_control));
        prop_assert_eq!(width(&sanitize(&text)), Span::raw(sanitize(&text).into_owned()).width());
    }
}
