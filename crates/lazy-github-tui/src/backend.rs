//! The terminal backend the app draws through: ratatui's backend with emoji presentation sequences written so every
//! terminal ends up with the cells ratatui's buffer holds (#249).
//!
//! An emoji followed by the variation selector U+FE0F (`🗑️`, `❤️`) takes two cells in ratatui's buffer, as `unicode-width`
//! measures the sequence, and in Python's layout. Terminals disagree: some (kitty, `WezTerm`, Windows Terminal, the
//! `vt100` parser the parity harness reads) widen the base character to two cells when the selector arrives, others
//! (VTE, xterm) keep it one cell wide. ratatui 0.30 papers over the second kind by writing the trailing cell right
//! after the emoji, without moving the cursor. On the first kind that write lands one cell too far right, so the rest
//! of the row is shifted, its last cell (a box border) falls off, and the shifted cells outlive the frame, since
//! ratatui only redraws what changed.
//!
//! [`WideEmojiBackend`] makes the cursor position explicit instead: it writes the trailing cells first, then the
//! emoji over them, and drops ratatui's own trailing writes. A widening terminal draws the emoji over both cells, a
//! narrow one leaves the blank after it, and the next write moves the cursor, so neither shifts the row.

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Cell, CellWidth as _};
use ratatui::layout::{Position, Size};

/// The variation selector that asks for an emoji's emoji (two-cell) presentation.
const VS16: char = '\u{FE0F}';

/// Wraps a backend so emoji presentation sequences can't shift the row they're on. See the module docs.
#[derive(Debug)]
pub struct WideEmojiBackend<B> {
    inner: B,
}

impl<B> WideEmojiBackend<B> {
    /// Wraps `inner`.
    pub const fn new(inner: B) -> Self {
        Self { inner }
    }

    /// The wrapped backend.
    pub const fn inner(&self) -> &B {
        &self.inner
    }

    /// The wrapped backend, to write to it directly (the hyperlink sequences, `crate::hyperlink`).
    pub const fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }
}

/// Whether the terminal may draw `cell` narrower than ratatui counts it: a wide cell holding U+FE0F.
fn is_wide_emoji(cell: &Cell) -> bool {
    cell.cell_width() > 1 && cell.symbol().contains(VS16)
}

/// The cells to write for ratatui's `content`, in order, with every wide emoji sequence preceded by blanks over its
/// trailing cells (in its style) and followed by nothing on them.
///
/// A blank written before the emoji can't shift anything: the emoji's write moves the cursor back to it, and the
/// next cell after the emoji is never adjacent to it, so the backend moves the cursor there too.
fn emoji_safe_cells<'a>(content: impl Iterator<Item = (u16, u16, &'a Cell)>) -> Vec<(u16, u16, Cell)> {
    let mut out = Vec::new();
    // The row and columns the last wide emoji covers past its first cell.
    let mut covered: Option<(u16, std::ops::Range<u16>)> = None;
    for (x, y, cell) in content {
        if covered
            .as_ref()
            .is_some_and(|(row, columns)| *row == y && columns.contains(&x))
        {
            continue;
        }
        if is_wide_emoji(cell) {
            let end = x.saturating_add(cell.cell_width());
            for column in x.saturating_add(1)..end {
                let mut blank = cell.clone();
                blank.set_symbol(" ");
                out.push((column, y, blank));
            }
            out.push((x, y, cell.clone()));
            covered = Some((y, x.saturating_add(1)..end));
        } else {
            out.push((x, y, cell.clone()));
        }
    }
    out
}

impl<B: Backend> Backend for WideEmojiBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let cells = emoji_safe_cells(content);
        self.inner.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// A terminal for tests that sends what the app draws through this backend into the `vt100` parser.
#[cfg(test)]
pub(crate) mod tests;
