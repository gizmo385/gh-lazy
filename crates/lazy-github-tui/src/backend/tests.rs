//! The width facts the backend relies on, the cells it writes, and what a widening terminal (`vt100`, as the parity
//! harness reads) ends up showing.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::{Buffer, Cell, CellWidth as _};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, TerminalOptions, Viewport};

use super::{WideEmojiBackend, emoji_safe_cells, is_wide_emoji};

/// A fixed-size terminal drawing through [`WideEmojiBackend`] into a `vt100` parser, the way the parity harness reads
/// the app.
pub(crate) struct VtTerminal {
    terminal: Terminal<WideEmojiBackend<CrosstermBackend<VtWriter>>>,
    parser: Rc<RefCell<vt100::Parser>>,
}

/// Feeds what the backend writes to the shared parser.
struct VtWriter(Rc<RefCell<vt100::Parser>>);

impl io::Write for VtWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl VtTerminal {
    pub(crate) fn new(cols: u16, rows: u16) -> Self {
        let parser = Rc::new(RefCell::new(vt100::Parser::new(
            rows.try_into().unwrap(),
            cols.try_into().unwrap(),
            0,
        )));
        let backend = WideEmojiBackend::new(CrosstermBackend::new(VtWriter(Rc::clone(&parser))));
        let viewport = Viewport::Fixed(Rect::new(0, 0, cols, rows));
        let terminal = Terminal::with_options(backend, TerminalOptions { viewport }).unwrap();
        Self { terminal, parser }
    }

    /// Draws a frame and checks that the terminal shows exactly the buffer ratatui drew, row by row. Returns the
    /// terminal's rows.
    pub(crate) fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> Vec<String> {
        let buffer = self.terminal.draw(render).unwrap().buffer.clone();
        let shown = self.rows();
        assert_eq!(shown, buffer_rows(&buffer), "the terminal and the buffer disagree");
        shown
    }

    fn rows(&self) -> Vec<String> {
        let parser = self.parser.borrow();
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        (0..rows.get())
            .map(|row| {
                (0..cols.get())
                    .filter_map(|col| screen.cell(row, col))
                    .filter(|cell| !cell.is_wide_continuation())
                    .map(|cell| if cell.has_contents() { cell.contents() } else { " " }.to_owned())
                    .collect()
            })
            .collect()
    }
}

/// `buffer`'s rows as a terminal shows them: each wide symbol once, without the cells it covers.
fn buffer_rows(buffer: &Buffer) -> Vec<String> {
    let width = usize::from(buffer.area.width);
    buffer
        .content
        .chunks(width)
        .map(|row| {
            let mut text = String::new();
            let mut hidden = 0;
            for cell in row {
                if hidden > 0 {
                    hidden -= 1;
                    continue;
                }
                text.push_str(cell.symbol());
                hidden = usize::from(cell.cell_width()).saturating_sub(1);
            }
            text
        })
        .collect()
}

#[test]
fn an_emoji_presentation_sequence_is_two_cells_wide() {
    // The base characters are text-presentation (one cell) on their own; U+FE0F makes them two.
    assert_eq!("🗑".cell_width(), 1);
    assert_eq!("🗑️".cell_width(), 2);
    assert_eq!("❤".cell_width(), 1);
    assert_eq!("❤️".cell_width(), 2);
    // Emoji-presentation characters are two cells without a selector, and the image marker stays one.
    assert_eq!("📄".cell_width(), 2);
    assert_eq!("👍".cell_width(), 2);
    assert_eq!("🖼".cell_width(), 1);
}

#[test]
fn only_wide_cells_with_the_selector_are_wide_emoji() {
    assert!(is_wide_emoji(&Cell::new("🗑️")));
    assert!(is_wide_emoji(&Cell::new("❤️")));
    assert!(!is_wide_emoji(&Cell::new("📄")));
    assert!(!is_wide_emoji(&Cell::new("漢")));
    assert!(!is_wide_emoji(&Cell::new("🖼")));
    assert!(!is_wide_emoji(&Cell::new("a")));
}

fn diff(prev: &Buffer, next: &Buffer) -> Vec<(u16, u16, String)> {
    emoji_safe_cells(prev.diff(next).into_iter())
        .into_iter()
        .map(|(x, y, cell)| (x, y, cell.symbol().to_owned()))
        .collect()
}

#[test]
fn the_trailing_cell_is_written_before_the_emoji_and_never_after() {
    let area = Rect::new(0, 0, 6, 1);
    let mut prev = Buffer::empty(area);
    prev.set_string(0, 0, "abcdef", Style::new());
    let mut next = Buffer::empty(area);
    next.set_string(0, 0, "🗑️ xyz", Style::new());
    // ratatui's own diff writes the trailing blank right after the emoji, at the cursor a widening terminal has
    // already moved past it.
    let ratatui: Vec<_> = prev
        .diff(&next)
        .into_iter()
        .map(|(x, _, cell)| (x, cell.symbol().to_owned()))
        .collect();
    assert_eq!(&ratatui[..2], [(0, "🗑️".to_owned()), (1, " ".to_owned())]);
    assert_eq!(
        diff(&prev, &next),
        [
            (1, 0, " ".to_owned()),
            (0, 0, "🗑️".to_owned()),
            (2, 0, " ".to_owned()),
            (3, 0, "x".to_owned()),
            (4, 0, "y".to_owned()),
            (5, 0, "z".to_owned()),
        ]
    );
}

#[test]
fn the_blank_keeps_the_emoji_style() {
    let area = Rect::new(0, 0, 3, 1);
    let prev = Buffer::empty(area);
    let mut next = Buffer::empty(area);
    let style = Style::new().fg(Color::Red).bg(Color::Blue);
    next.set_string(0, 0, "❤️", style);
    let cells = emoji_safe_cells(prev.diff(&next).into_iter());
    assert_eq!(cells[0].0, 1);
    assert_eq!(cells[0].2.style(), cells[1].2.style());
    assert_eq!(cells[1].2.symbol(), "❤️");
}

#[test]
fn other_cells_pass_through_unchanged() {
    let area = Rect::new(0, 0, 6, 1);
    let prev = Buffer::empty(area);
    let mut next = Buffer::empty(area);
    next.set_string(0, 0, "📄 漢a", Style::new());
    let ratatui: Vec<_> = prev
        .diff(&next)
        .into_iter()
        .map(|(x, y, cell)| (x, y, cell.symbol().to_owned()))
        .collect();
    assert_eq!(diff(&prev, &next), ratatui);
}

/// A bordered box with `title` on its first inner row.
fn boxed(title: &'static str) -> impl FnOnce(&mut Frame<'_>) {
    move |frame| {
        let area = frame.area();
        frame.render_widget(Paragraph::new(title).block(Block::bordered()), area);
    }
}

#[test]
fn a_deleted_file_title_keeps_the_box_border_in_a_widening_terminal() {
    let mut terminal = VtTerminal::new(30, 3);
    // Something else on the row first, so ratatui's diff writes the emoji's trailing cell.
    terminal.draw(boxed("dddddddddddddddd"));
    let rows = terminal.draw(boxed("▶ 🗑️  old.md (deleted)"));
    assert_eq!(rows[1], "│▶ 🗑️  old.md (deleted)      │");
    // And the next frame leaves nothing behind.
    let rows = terminal.draw(boxed("Review"));
    assert_eq!(rows[1], "│Review                      │");
}

#[test]
fn an_unchanged_emoji_with_changes_around_it_stays_in_place() {
    let mut terminal = VtTerminal::new(12, 1);
    terminal.draw(|frame| frame.render_widget(Paragraph::new("a❤️ 1"), frame.area()));
    let rows = terminal.draw(|frame| frame.render_widget(Paragraph::new("b❤️ 2"), frame.area()));
    assert_eq!(rows[0], "b❤️ 2       ");
}
