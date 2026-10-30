//! Text as the terminal draws it: cell widths, control characters and cutting to a width.
//!
//! Every width here is in terminal cells and is measured grapheme by grapheme, the way ratatui's buffer lays text out,
//! so a CJK character or an emoji (ZWJ sequences and flags included) counts two cells, a combining mark or a
//! zero-width character none, and a cut never lands inside a grapheme.
//!
//! ratatui silently drops control characters when it draws a span, but `Span::width` still counts them, so a title
//! holding an ESC would push every later column of its row one cell left. Text from GitHub, `gh` or `git` therefore
//! goes through [`sanitize`] (or [`sanitize_text`] when it has lines) before it's drawn: every control character
//! becomes U+FFFD, as the command log has always shown them (Py bug B34).

use std::borrow::Cow;

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr;

/// What a cut string ends in.
pub const ELLIPSIS: &str = "…";

/// What a control character is drawn as.
const REPLACEMENT: char = '\u{FFFD}';

/// Whether `c` is drawn as U+FFFD: a C0 control, DEL or a C1 control.
fn is_control(c: char) -> bool {
    c.is_control()
}

/// The cells `s` takes once drawn. Control characters, which ratatui doesn't draw, take none.
pub fn width(s: &str) -> usize {
    if is_printable_ascii(s) {
        return s.len();
    }
    graphemes(s).map(|(_, g)| grapheme_width(g)).sum()
}

/// Whether `s` is all printable ASCII (space to `~`): then each byte is a grapheme one cell wide, and the grapheme
/// walk, the slow part of laying out a large body or log, can be skipped.
pub(crate) fn is_printable_ascii(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b' '..=b'~'))
}

/// The cells one grapheme takes, as ratatui's buffer counts them.
fn grapheme_width(grapheme: &str) -> usize {
    UnicodeWidthStr::width(grapheme)
}

/// `s`'s graphemes and their byte offsets, without the ones holding a control character, which ratatui skips when
/// it draws a span.
fn graphemes(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.grapheme_indices(true).filter(|(_, g)| !g.contains(is_control))
}

/// `c` as [`sanitize`] draws it: U+FFFD for a control character, else `c`.
pub fn sanitize_char(c: char) -> char {
    if is_control(c) { REPLACEMENT } else { c }
}

/// One line of text from GitHub, `gh` or `git`, ready to draw: every control character, line breaks and tabs
/// included, replaced with U+FFFD. Borrowed when there's none.
pub fn sanitize(s: &str) -> Cow<'_, str> {
    replace_controls(s, |_| false)
}

/// Multi-line text from GitHub, `gh` or `git` (a body, a diff, a log), ready to lay out: like [`sanitize`], but line
/// breaks (`\n`, `\r`) and tabs stay for the layout to handle.
pub fn sanitize_text(s: &str) -> Cow<'_, str> {
    replace_controls(s, |c| matches!(c, '\n' | '\r' | '\t'))
}

fn replace_controls(s: &str, keep: impl Fn(char) -> bool) -> Cow<'_, str> {
    if !s.chars().any(|c| is_control(c) && !keep(c)) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| if is_control(c) && !keep(c) { REPLACEMENT } else { c })
            .collect(),
    )
}

/// `line` with each span [`sanitize`]d.
pub fn sanitize_line(line: Line<'static>) -> Line<'static> {
    if !line.spans.iter().any(|span| span.content.chars().any(is_control)) {
        return line;
    }
    let Line {
        spans,
        style,
        alignment,
    } = line;
    Line {
        spans: spans
            .into_iter()
            .map(|span| match sanitize(&span.content) {
                Cow::Borrowed(_) => span,
                Cow::Owned(text) => Span::styled(text, span.style),
            })
            .collect(),
        style,
        alignment,
    }
}

/// `s` fitted to `width` cells: whole if it fits, else cut to `width - 1` cells and ended with [`ELLIPSIS`]. As in
/// Rich's `set_cell_size`, a wide grapheme that would straddle the cut is dropped and its cell left blank, so the
/// ellipsis is always the last cell. Empty for a width of 0.
pub fn truncate(s: &str, width: usize) -> Cow<'_, str> {
    if self::width(s) <= width {
        return Cow::Borrowed(s);
    }
    let Some(room) = width.checked_sub(1) else {
        return Cow::Borrowed("");
    };
    let (kept, used) = take_cells(s, room);
    let mut out = String::with_capacity(kept.len() + ELLIPSIS.len() + room - used);
    out.push_str(kept);
    out.extend(std::iter::repeat_n(' ', room - used));
    out.push_str(ELLIPSIS);
    Cow::Owned(out)
}

/// The longest prefix of `s` made of whole graphemes that fits in `cells`, and the cells it takes.
fn take_cells(s: &str, cells: usize) -> (&str, usize) {
    if is_printable_ascii(s) {
        let end = s.len().min(cells);
        return (&s[..end], end);
    }
    let mut used = 0;
    let mut end = 0;
    for (start, grapheme) in graphemes(s) {
        let w = grapheme_width(grapheme);
        if used + w > cells {
            break;
        }
        used += w;
        end = start + grapheme.len();
    }
    (&s[..end], used)
}

/// `spans` cut to at most `width` cells, with no ellipsis: whole spans up to the one that crosses the edge, and as
/// many whole graphemes of that one as fit. Returns the spans and the cells left over, which is more than 0 when a
/// wide grapheme didn't fit in the last cell.
pub fn cut_spans<'a>(spans: impl IntoIterator<Item = Span<'a>>, width: usize) -> (Vec<Span<'a>>, usize) {
    let mut left = width;
    let mut kept = Vec::new();
    for span in spans {
        if left == 0 {
            break;
        }
        let span_width = self::width(&span.content);
        if span_width <= left {
            left -= span_width;
            kept.push(span);
            continue;
        }
        let (text, used) = take_cells(&span.content, left);
        left -= used;
        if !text.is_empty() {
            kept.push(Span::styled(text.to_owned(), span.style));
        }
        break;
    }
    (kept, left)
}

#[cfg(test)]
mod tests;
