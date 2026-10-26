//! The bordered boxes of the main screen. Python's `LazyGithubContainer`.

use std::borrow::Cow;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Block;

use crate::theme::Theme;

/// The cells a title row spends around the title itself: two corners, plus one `─` and one blank on each side.
const TITLE_RESERVED: usize = 6;

/// What a compact section's title ends in: Python's `set_compact` suffix `" [dim](empty)[/dim]"`, drawn after a
/// blank.
pub(super) const EMPTY_SUFFIX: &str = "(empty)";

/// A box `width` cells wide titled `┌─ {title} ─…┐`, its border and title in `$success` when focus is within it
/// and `$primary-lighten-3` otherwise. A `dim` suffix follows the title after a blank, dimmed, as a compact
/// section's ` (empty)` is.
///
/// Like Textual, a title too long for the box is cut to `width - 6` cells ending in `…`, keeping the `─` before
/// the closing corner, and a box with no room for any title cell draws none. The title and its suffix are cut as
/// one string, as Textual cuts the whole border title.
pub(super) fn titled_box<'a>(title: &str, dim: Option<&str>, width: u16, focused: bool, theme: &Theme) -> Block<'a> {
    let slot = if focused {
        theme.palette.success
    } else {
        theme.palette.primary_lighten_3
    };
    let border = Style::new().fg(theme.color(slot, theme.background()));
    let block = Block::bordered().border_style(border);
    let full = match dim {
        Some(suffix) => Cow::Owned(format!("{title} {suffix}")),
        None => Cow::Borrowed(title),
    };
    let Some(fitted) = fit_title(&full, usize::from(width)) else {
        return block;
    };
    // Everything from the suffix's first cell on is dim, including an ellipsis that cut into it.
    let dim_from = if dim.is_some() {
        cell_width(title) + 1
    } else {
        usize::MAX
    };
    let (plain, dimmed) = split_at_cell(&fitted, dim_from);
    let mut spans = vec![Span::styled("─", border), Span::styled(format!(" {plain}"), border)];
    if !dimmed.is_empty() {
        spans.push(Span::styled(dimmed.to_owned(), border.add_modifier(Modifier::DIM)));
    }
    spans.push(Span::styled(" ", border));
    block.title(Line::from(spans))
}

/// `s` split before the first character that starts at or after cell `cell`.
fn split_at_cell(s: &str, cell: usize) -> (&str, &str) {
    let mut used = 0;
    for (i, ch) in s.char_indices() {
        if used >= cell {
            return s.split_at(i);
        }
        used += cell_width(ch.encode_utf8(&mut [0; 4]));
    }
    (s, "")
}

/// `title` as Textual draws it in a box `width` cells wide: whole if it fits, else cut with `…`, and `None` if
/// the box has no room past its corners, rules and blanks.
fn fit_title(title: &str, width: usize) -> Option<Cow<'_, str>> {
    let room = width.checked_sub(TITLE_RESERVED).filter(|room| *room > 0)?;
    if title.is_empty() {
        return None;
    }
    if cell_width(title) <= room {
        return Some(Cow::Borrowed(title));
    }
    // Keep whole characters up to `room - 1` cells and the ellipsis takes the last one. As in Rich's
    // `set_cell_size`, a wide character that would straddle the cut is dropped and its cell left blank.
    let mut kept = String::new();
    let mut used = 0;
    for ch in title.chars() {
        let w = cell_width(ch.encode_utf8(&mut [0; 4]));
        if used + w > room - 1 {
            break;
        }
        kept.push(ch);
        used += w;
    }
    kept.extend(std::iter::repeat_n(' ', room - 1 - used));
    kept.push('…');
    Some(Cow::Owned(kept))
}

fn cell_width(s: &str) -> usize {
    Span::raw(s).width()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn top_buffer(title: &str, dim: Option<&str>, width: u16) -> ratatui::buffer::Buffer {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(titled_box(title, dim, width, false, &theme), frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn top_row(title: &str, width: u16) -> String {
        dim_row(title, None, width).0
    }

    /// The top row, and under it a `d` for each dim cell.
    fn dim_row(title: &str, dim: Option<&str>, width: u16) -> (String, String) {
        let buffer = top_buffer(title, dim, width);
        let row = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
        let dims = (0..width)
            .map(|x| {
                if buffer[(x, 0)].modifier.contains(Modifier::DIM) {
                    'd'
                } else {
                    ' '
                }
            })
            .collect::<String>();
        (row, dims.trim_end().to_owned())
    }

    #[test]
    fn a_compact_title_ends_in_a_dim_empty() {
        let (row, dims) = dim_row("[2] Pull Requests", Some(EMPTY_SUFFIX), 40);
        assert_eq!(row, "┌─ [2] Pull Requests (empty) ──────────┐");
        assert_eq!(dims, format!("{}ddddddd", " ".repeat(21)));
    }

    #[test]
    fn a_compact_title_is_cut_as_one_string() {
        // 21 cells of room: the cut falls inside the suffix, and the ellipsis is dim with it.
        let (row, dims) = dim_row("[2] Pull Requests", Some(EMPTY_SUFFIX), 27);
        assert_eq!(row, "┌─ [2] Pull Requests (e… ─┐");
        assert_eq!(dims, format!("{}ddd", " ".repeat(21)));
        // 12 cells: the cut falls inside the base title, so nothing is dim.
        let (row, dims) = dim_row("[2] Pull Requests", Some(EMPTY_SUFFIX), 18);
        assert_eq!(row, "┌─ [2] Pull Re… ─┐");
        assert_eq!(dims, "");
    }

    #[test]
    fn a_title_that_fits_is_drawn_whole() {
        insta::assert_snapshot!(top_row("[3] Issues", 16), @"┌─ [3] Issues ─┐");
    }

    #[test]
    fn a_title_one_cell_too_long_ends_in_an_ellipsis() {
        insta::assert_snapshot!(top_row("[3] Issues!", 16), @"┌─ [3] Issue… ─┐");
    }

    #[test]
    fn a_long_title_is_cut_like_textual_at_twenty_cells() {
        insta::assert_snapshot!(top_row("[1] Repositories", 20), @"┌─ [1] Repositor… ─┐");
    }

    #[test]
    fn a_box_with_room_for_one_title_cell_draws_only_the_ellipsis() {
        insta::assert_snapshot!(top_row("[1] Repositories", 7), @"┌─ … ─┐");
    }

    #[test]
    fn a_box_too_narrow_for_any_title_draws_none() {
        insta::assert_snapshot!(top_row("[1] Repositories", 6), @"┌────┐");
    }

    #[test]
    fn an_empty_title_draws_none() {
        insta::assert_snapshot!(top_row("", 10), @"┌────────┐");
    }

    #[test]
    fn wide_characters_are_never_split() {
        // 11 cells of room at 17: `[1] 日本語` fills the 10 before the ellipsis exactly.
        assert_eq!(fit_title("[1] 日本語日本語", 17).as_deref(), Some("[1] 日本語…"));
        // 12 at 18: the fourth wide character would straddle the cut, so its first cell is left blank.
        assert_eq!(fit_title("[1] 日本語日本語", 18).as_deref(), Some("[1] 日本語 …"));
    }
}
