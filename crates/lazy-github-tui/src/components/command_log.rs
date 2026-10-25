//! The command log: the [`LogLine`]s from the core's command-log layer, one row each, in a view that scrolls both
//! ways and never wraps. Python's `LazyGithubCommandLog`, a Textual `Log`.

use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use lazy_github_core::logging::LogLine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::components::info::Scroll;
use crate::components::scrollbar::{self, Axis, HORIZONTAL_HEIGHT, ScrollbarColors, ScrollbarState, VERTICAL_WIDTH};
use crate::theme::Theme;

/// Python's `str.expandtabs()` default.
const TAB_SIZE: usize = 8;
/// Textual's `App.scroll_sensitivity_x` and `scroll_sensitivity_y`: how far one wheel notch scrolls.
const WHEEL_COLUMNS: usize = 4;
const WHEEL_ROWS: usize = 2;
/// `Log:focus { background-tint: $foreground 5% }`.
const FOCUS_TINT_PERCENT: f64 = 5.0;

/// A mouse wheel notch over the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    Up,
    Down,
    Left,
    Right,
}

impl Wheel {
    /// The notch a mouse event stands for. As in Textual, ctrl or shift turns the vertical wheel horizontal.
    pub fn from_mouse(mouse: &MouseEvent) -> Option<Wheel> {
        let sideways = mouse.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        Some(match mouse.kind {
            MouseEventKind::ScrollUp if sideways => Wheel::Left,
            MouseEventKind::ScrollDown if sideways => Wheel::Right,
            MouseEventKind::ScrollUp => Wheel::Up,
            MouseEventKind::ScrollDown => Wheel::Down,
            MouseEventKind::ScrollLeft => Wheel::Left,
            MouseEventKind::ScrollRight => Wheel::Right,
            _ => return None,
        })
    }
}

/// The log's lines and scroll position.
#[derive(Debug, Default)]
pub struct CommandLog {
    /// Rows as drawn: split, tabs expanded and control characters replaced. Unbounded, like Python's
    /// `max_lines = None`.
    lines: Vec<String>,
    /// The widest row, in cells: how far the view scrolls right.
    width: usize,
    /// The first visible column and row.
    x: usize,
    y: usize,
    /// The text area inside the scrollbars at the last draw, as (columns, rows). Zero until the first draw.
    viewport: (usize, usize),
    visible: bool,
}

impl CommandLog {
    /// An empty log, shown or hidden.
    pub fn new(visible: bool) -> Self {
        Self {
            visible,
            ..Self::default()
        }
    }

    /// Appends a log line, with [`LogLine`]'s `Display` so the pane and the log file can't drift. A message with
    /// newlines adds several rows. When the view was at the bottom it follows the new rows; otherwise it stays put.
    pub fn push(&mut self, line: &LogLine) {
        let follow = self.at_bottom();
        for row in split_lines(&line.to_string()) {
            let row = process_row(row);
            self.width = self.width.max(Span::raw(row.as_str()).width());
            self.lines.push(row);
        }
        if follow {
            self.y = self.max_y();
        }
    }

    /// The rows, oldest first, as drawn.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The first visible (column, row).
    pub fn offset(&self) -> (usize, usize) {
        (self.x, self.y)
    }

    /// Shows or hides the log. Lines pushed while hidden are kept. The palette's "Toggle Command Log" (M4) calls
    /// this.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Whether the log is shown.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Textual's `is_vertical_scroll_end`, which is also true while the log has no size yet.
    fn at_bottom(&self) -> bool {
        self.viewport.1 == 0 || self.y >= self.max_y()
    }

    fn max_x(&self) -> usize {
        self.width.saturating_sub(self.viewport.0)
    }

    fn max_y(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport.1)
    }

    /// Scrolls for a `textual.scroll.*` binding, clamped to the content as of the last draw.
    pub fn scroll(&mut self, scroll: Scroll) {
        let (columns, rows) = self.viewport;
        let (x, y) = (self.x, self.y);
        let (x, y) = match scroll {
            Scroll::Up => (x, y.saturating_sub(1)),
            Scroll::Down => (x, y + 1),
            Scroll::Left => (x.saturating_sub(1), y),
            Scroll::Right => (x + 1, y),
            Scroll::PageUp => (x, y.saturating_sub(rows)),
            Scroll::PageDown => (x, y + rows),
            Scroll::PageLeft => (x.saturating_sub(columns), y),
            Scroll::PageRight => (x + columns, y),
            // Textual's `action_scroll_home` also goes to the first column, but only when already at the top.
            Scroll::Home => (if y == 0 { 0 } else { x }, 0),
            // `action_scroll_end` passes `x_axis=self.scroll_y == self.is_vertical_scroll_end`, comparing the row
            // with a bool: the first column only when at row 1 and the end, or at row 0 and not the end.
            Scroll::End => {
                let at_end = y == self.max_y();
                (if y == usize::from(at_end) { 0 } else { x }, self.max_y())
            }
        };
        self.x = x.min(self.max_x());
        self.y = y.min(self.max_y());
    }

    /// Scrolls for a wheel notch, by Textual's scroll sensitivity.
    pub fn wheel(&mut self, wheel: Wheel) {
        match wheel {
            Wheel::Up => self.y = self.y.saturating_sub(WHEEL_ROWS),
            Wheel::Down => self.y = (self.y + WHEEL_ROWS).min(self.max_y()),
            Wheel::Left => self.x = self.x.saturating_sub(WHEEL_COLUMNS),
            Wheel::Right => self.x = (self.x + WHEEL_COLUMNS).min(self.max_x()),
        }
    }

    /// Draws the log into `area`: the rows on `$surface` (tinted while focused), with both scrollbar tracks always
    /// drawn, as Textual's `overflow: scroll` does, and a thumb only on the axis that overflows.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        let palette = theme.palette;
        let surface = palette.surface.over(theme.background());
        let bg = if focused {
            theme.tint(surface, palette.foreground, FOCUS_TINT_PERCENT)
        } else {
            surface
        };
        let fg = theme.color(palette.text, bg);
        let style = Style::new().fg(fg).bg(theme.paint(bg));

        let bar_width = VERTICAL_WIDTH.min(area.width);
        let bar_height = HORIZONTAL_HEIGHT.min(area.height);
        let text = Rect {
            width: area.width - bar_width,
            height: area.height - bar_height,
            ..area
        };
        self.viewport = (usize::from(text.width), usize::from(text.height));
        self.x = self.x.min(self.max_x());
        self.y = self.y.min(self.max_y());

        let rows: Vec<Line<'_>> = self
            .lines
            .iter()
            .skip(self.y)
            .take(self.viewport.1)
            .map(|row| Line::raw(row.as_str()))
            .collect();
        // Lines wider than u16::MAX cells can't be scrolled to their very end; no log line is that long.
        let x = u16::try_from(self.x).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(rows).style(style).scroll((0, x)), text);

        let back = palette.scrollbar_background.over(bg);
        let colors = ScrollbarColors {
            bar: theme.paint(palette.scrollbar.over(back)),
            back: theme.paint(back),
            fg,
            bg: theme.paint(bg),
        };
        let vertical = Rect {
            x: text.right(),
            width: bar_width,
            ..text
        };
        let horizontal = Rect {
            y: text.bottom(),
            height: bar_height,
            ..text
        };
        let buf = frame.buffer_mut();
        scrollbar::render(
            buf,
            vertical,
            Axis::Vertical,
            bar_state(self.lines.len(), self.viewport.1, self.y),
            colors,
        );
        scrollbar::render(
            buf,
            horizontal,
            Axis::Horizontal,
            bar_state(self.width, self.viewport.0, self.x),
            colors,
        );
        // Textual's `ScrollBarCorner`: a blank in `$scrollbar-corner-color` with no foreground.
        let corner = Rect {
            x: text.right(),
            y: text.bottom(),
            width: bar_width,
            height: bar_height,
        };
        let corner_bg = theme.color(palette.scrollbar_corner_color, bg);
        frame.render_widget(Clear, corner);
        frame.render_widget(Block::new().style(Style::new().bg(corner_bg)), corner);
    }
}

/// Textual's `ScrollBar.render`: no thumb unless the content is bigger than the window.
fn bar_state(content: usize, window: usize, position: usize) -> ScrollbarState {
    ScrollbarState {
        content,
        window: if window < content { window } else { 0 },
        position,
    }
}

/// Python's `str.splitlines()`: splits at every line boundary Python knows and drops a trailing empty line.
fn split_lines(text: &str) -> Vec<&str> {
    let mut rows = Vec::new();
    let mut rest = text;
    while let Some((i, c)) = rest.char_indices().find(|&(_, c)| is_line_boundary(c)) {
        rows.push(&rest[..i]);
        let mut next = i + c.len_utf8();
        if c == '\r' && rest[next..].starts_with('\n') {
            next += 1;
        }
        rest = &rest[next..];
    }
    if !rest.is_empty() {
        rows.push(rest);
    }
    rows
}

/// The boundaries `str.splitlines()` splits at.
fn is_line_boundary(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Textual's `Log._process_line`: tabs expanded to 8-column stops, then U+0000–U+0014 replaced with U+FFFD.
///
/// The other C0 controls and DEL are replaced too, which Textual doesn't do: drawn raw, an escape would reach the
/// terminal and corrupt the screen.
fn process_row(row: &str) -> String {
    let mut out = String::with_capacity(row.len());
    let mut column = 0;
    for c in row.chars() {
        if c == '\t' {
            let spaces = TAB_SIZE - column % TAB_SIZE;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
            continue;
        }
        out.push(if c.is_ascii_control() { '\u{FFFD}' } else { c });
        // Python counts characters, not cells, for tab stops.
        column += 1;
    }
    out
}

#[cfg(test)]
mod tests;
