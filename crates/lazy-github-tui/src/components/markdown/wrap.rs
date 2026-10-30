//! Word wrapping as Textual's `Content.wrap` does it with Rich's `divide_line`, over graphemes and cell widths.

use std::iter;
use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::Span;

use super::parse::is_space;

/// One grapheme of a line being wrapped.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cell<'a> {
    pub(super) symbol: &'a str,
    pub(super) width: usize,
    /// Which styled run it came from.
    pub(super) run: usize,
}

impl Cell<'_> {
    fn is_space(&self) -> bool {
        self.symbol.chars().all(is_space)
    }
}

/// The graphemes of `text`, all from `run`. Control characters are left out, as ratatui draws none.
pub(super) fn cells(text: &str, run: usize) -> Vec<Cell<'_>> {
    if crate::display::is_printable_ascii(text) {
        // Each byte is a grapheme one cell wide, and every index is a char boundary.
        return (0..text.len())
            .map(|i| Cell {
                symbol: &text[i..=i],
                width: 1,
                run,
            })
            .collect();
    }
    grapheme_cells(text, run)
}

/// [`cells`] by walking `text`'s graphemes.
fn grapheme_cells(text: &str, run: usize) -> Vec<Cell<'_>> {
    // Ratatui's grapheme split borrows the span, not the text, so each grapheme is taken back out of `text` by its
    // offset: a borrowed span's graphemes point into `text` itself.
    let span = Span::raw(text);
    let base = text.as_ptr().addr();
    span.styled_graphemes(Style::new())
        .filter_map(|g| {
            let start = g.symbol.as_ptr().addr().checked_sub(base)?;
            let symbol = text.get(start..start + g.symbol.len())?;
            Some(Cell {
                symbol,
                width: Span::raw(symbol).width(),
                run,
            })
        })
        .collect()
}

/// Greedy word wrap of one line of plain text to `width` cells, as Rich does it: a word that doesn't fit starts the
/// next row, a word wider than a row is split, and trailing spaces are dropped. Empty text gives no rows.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let cells = cells(text, 0);
    rows(&cells, width)
        .into_iter()
        .map(|row| cells[row].iter().map(|c| c.symbol).collect())
        .collect()
}

/// Splits one line into rows of at most `width` cells: Rich's `divide_line`, then Textual's clean-up of the pieces
/// (trailing whitespace off every row but the last, and anything past `width` cut).
pub(super) fn rows(cells: &[Cell<'_>], width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let breaks = break_positions(cells, width);
    let starts = iter::once(0).chain(breaks.iter().copied());
    let ends = breaks.iter().copied().chain(iter::once(cells.len()));
    let last = breaks.len();
    starts
        .zip(ends)
        .enumerate()
        .map(|(index, (start, mut end))| {
            if index < last {
                while end > start && cells[end - 1].is_space() {
                    end -= 1;
                }
            }
            let mut used = 0;
            let mut cut = start;
            while cut < end && used + cells[cut].width <= width {
                used += cells[cut].width;
                cut += 1;
            }
            start..cut
        })
        .collect()
}

/// Rich's `divide_line`: the indices where rows after the first start.
fn break_positions(cells: &[Cell<'_>], width: usize) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut offset = 0;
    for (start, end) in words(cells) {
        let word = &cells[start..end];
        let trimmed = word.iter().rposition(|c| !c.is_space()).map_or(0, |i| i + 1);
        let word_width = width_of(&word[..trimmed]);
        // Rich's offset counts trailing spaces, so it can pass `width`; then nothing more fits.
        if offset <= width && width - offset >= word_width {
            offset += width_of(word);
        } else if word_width > width {
            // Chop the word, trailing spaces included, into pieces of `width` cells; each starts a row.
            let pieces = chop(word, width);
            let mut piece_start = start;
            for (index, len) in pieces.iter().enumerate() {
                if piece_start > 0 {
                    breaks.push(piece_start);
                }
                if index + 1 == pieces.len() {
                    offset = width_of(&cells[piece_start..piece_start + len]);
                } else {
                    piece_start += len;
                }
            }
        } else if offset > 0 && start > 0 {
            breaks.push(start);
            offset = width_of(word);
        }
    }
    breaks
}

/// Rich's `words`: the spans of `\s*\S+\s*`, so leading whitespace belongs to the first word.
fn words(cells: &[Cell<'_>]) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut i = 0;
    loop {
        let start = i;
        while i < cells.len() && cells[i].is_space() {
            i += 1;
        }
        if i == cells.len() {
            break;
        }
        while i < cells.len() && !cells[i].is_space() {
            i += 1;
        }
        while i < cells.len() && cells[i].is_space() {
            i += 1;
        }
        words.push((start, i));
    }
    words
}

/// Rich's `chop_cells`: the lengths of the pieces `word` splits into at `width` cells. A grapheme wider than the
/// whole row still gets one (after an empty piece, as in Rich).
fn chop(word: &[Cell<'_>], width: usize) -> Vec<usize> {
    let mut pieces = Vec::new();
    let mut piece_start = 0;
    let mut piece_width = 0;
    for (i, cell) in word.iter().enumerate() {
        if piece_width + cell.width > width {
            pieces.push(i - piece_start);
            piece_start = i;
            piece_width = 0;
        }
        piece_width += cell.width;
    }
    if piece_width > 0 {
        pieces.push(word.len() - piece_start);
    }
    pieces
}

fn width_of(cells: &[Cell<'_>]) -> usize {
    cells.iter().map(|c| c.width).sum()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// The printable-ASCII shortcut in [`cells`] (#325) gives the cells the grapheme walk gives.
        #[test]
        fn the_ascii_shortcut_matches_the_grapheme_walk(text in prop_oneof!["[ -~]{0,40}", "[ -~\t\r\n\x1b]{0,40}"]) {
            let key = |cells: Vec<Cell<'_>>| -> Vec<(String, usize, usize)> {
                cells.into_iter().map(|c| (c.symbol.to_owned(), c.width, c.run)).collect()
            };
            prop_assert_eq!(key(cells(&text, 3)), key(grapheme_cells(&text, 3)));
        }
    }
}
