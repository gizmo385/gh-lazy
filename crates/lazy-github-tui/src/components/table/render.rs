//! Drawing the table as Textual's `DataTable` draws it in row-cursor mode.
//!
//! Not ratatui's `Table`, which can't scroll horizontally: every row is one [`Line`] of padded cells, drawn with
//! `Paragraph::scroll` over the visible rows, under a header drawn the same way.

use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use super::model::{Cell, Column, RowEntry};
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::display;
use crate::theme::{Paint, Theme};

/// Textual's `DataTable.cell_padding`: one space each side of every cell.
const CELL_PADDING: usize = 1;

/// Each column's width over `rows`: its widest cell or header, plus the padding. Columns are never stretched, and only
/// a [`Column::shrinks`] one is ever cut ([`fit_widths`]).
pub(crate) fn column_widths<K>(columns: &[Column], rows: &[RowEntry<K>]) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            rows.iter()
                .filter_map(|row| row.cells.get(i))
                .map(cell_width)
                .fold(column.header.width() + 2 * CELL_PADDING, usize::max)
        })
        .collect()
}

/// The width a column needs for `cell`, padding included.
pub(crate) fn cell_width(cell: &Cell) -> usize {
    cell.display.width() + 2 * CELL_PADDING
}

/// `widths` fitted to a table of `rows` rows in `area`: when they're wider than the columns left of any vertical
/// scrollbar, the [`Column::shrinks`] columns give up the difference, one cell at a time from whichever is widest
/// (the rightmost on a tie), so they end up as even as they can, and none narrower than its header. Without such a
/// column, or when they fit, they're `widths` as they are; past every floor the rest still overflows and scrolls.
pub(crate) fn fit_widths<'a>(columns: &[Column], widths: &'a [usize], area: Rect, rows: usize) -> Cow<'a, [usize]> {
    let floors: Vec<(usize, usize)> = columns
        .iter()
        .enumerate()
        .filter(|(i, column)| column.shrinks && *i < widths.len())
        .map(|(i, column)| (i, column.header.width() + 2 * CELL_PADDING))
        .collect();
    // The rows alone decide the vertical scrollbar here: fitted, the columns only overflow past every floor.
    let vbar = if 1 + rows > usize::from(area.height) {
        usize::from(scrollbar::VERTICAL_WIDTH)
    } else {
        0
    };
    let viewport = usize::from(area.width).saturating_sub(vbar);
    let mut excess = widths.iter().sum::<usize>().saturating_sub(viewport);
    if floors.is_empty() || excess == 0 {
        return Cow::Borrowed(widths);
    }
    let mut fitted = widths.to_vec();
    while excess > 0 {
        let widest = floors
            .iter()
            .filter(|&&(i, floor)| fitted[i] > floor)
            .max_by_key(|&&(i, _)| fitted[i]);
        let Some(&(i, _)) = widest else { break };
        fitted[i] -= 1;
        excess -= 1;
    }
    Cow::Owned(fitted)
}

/// Where everything goes for a table of `content_width` columns and `rows` rows drawn in an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    /// The table itself: `height: auto`, so header plus rows plus a horizontal scrollbar, capped at the area.
    pub(crate) table: Rect,
    /// The columns the cells are drawn in, left of any vertical scrollbar.
    pub(crate) viewport_width: usize,
    /// The rows that fit below the header and above any horizontal scrollbar.
    pub(crate) body_height: usize,
    /// Whether the rows overflow, which shows a vertical scrollbar.
    pub(crate) vertical_bar: bool,
    /// Whether the columns overflow, which shows a horizontal scrollbar.
    pub(crate) horizontal_bar: bool,
}

impl Layout {
    /// Textual's box model for an auto-height scrolling widget. Each scrollbar takes space from the viewport, and
    /// each can make the other necessary, so the check runs twice.
    pub(crate) fn new(area: Rect, content_width: usize, rows: usize) -> Self {
        let (width, height) = (usize::from(area.width), usize::from(area.height));
        let lines = 1 + rows;
        let vbar = usize::from(scrollbar::VERTICAL_WIDTH);
        let hbar = usize::from(scrollbar::HORIZONTAL_HEIGHT);
        let horizontal = content_width > width;
        let vertical = lines + if horizontal { hbar } else { 0 } > height;
        let horizontal = content_width > width.saturating_sub(if vertical { vbar } else { 0 });
        let vertical = lines + if horizontal { hbar } else { 0 } > height;

        let table_height = height.min(lines + if horizontal { hbar } else { 0 });
        let content_height = table_height.saturating_sub(if horizontal { hbar } else { 0 });
        Self {
            table: Rect {
                // `table_height` is at most the area's height, a u16.
                height: u16::try_from(table_height).unwrap_or(area.height),
                ..area
            },
            viewport_width: width.saturating_sub(if vertical { vbar } else { 0 }),
            body_height: content_height.saturating_sub(1),
            vertical_bar: vertical,
            horizontal_bar: horizontal,
        }
    }

    /// The header row. A horizontal scrollbar takes the last row first, so in a table one row high it hides the
    /// header, as in Textual.
    pub(crate) fn header(self) -> Rect {
        Rect {
            height: self.content_height().min(1),
            width: self.viewport_width_u16(),
            ..self.table
        }
    }

    /// The `i`th visible body row, counted from the top of the body.
    pub(crate) fn body_row(self, i: usize) -> Rect {
        Rect {
            // Callers pass `i < body_height`, which fits the table's u16 height.
            y: self.table.y + 1 + u16::try_from(i).unwrap_or(0),
            height: 1,
            width: self.viewport_width_u16(),
            ..self.table
        }
    }

    fn viewport_width_u16(self) -> u16 {
        u16::try_from(self.viewport_width).unwrap_or(self.table.width)
    }

    /// The rows above any horizontal scrollbar: the header and the body.
    fn content_height(self) -> u16 {
        let hbar = if self.horizontal_bar {
            scrollbar::HORIZONTAL_HEIGHT
        } else {
            0
        };
        self.table.height.saturating_sub(hbar)
    }

    fn vertical_bar_rect(self) -> Rect {
        Rect {
            x: self.table.x + self.viewport_width_u16(),
            width: self.table.width - self.viewport_width_u16(),
            height: self.content_height(),
            ..self.table
        }
    }

    fn horizontal_bar_rect(self) -> Rect {
        Rect {
            y: self.table.y + self.content_height(),
            height: self.table.height - self.content_height(),
            width: self.viewport_width_u16(),
            ..self.table
        }
    }
}

/// The table's styles for one frame, from `DataTable.DEFAULT_CSS`.
pub(crate) struct Styles {
    /// The table's own background and text (`$surface`, tinted while focused).
    pub(crate) base: Style,
    pub(crate) header: Style,
    pub(crate) cursor: Style,
    /// `cursor` with the cursor's foreground, which overrides the cells' own (`cursor_foreground_priority = "css"`).
    cursor_fg: Color,
    pub(crate) hover: Style,
    scrollbar: ScrollbarColors,
    corner: Style,
}

impl Styles {
    pub(crate) fn new(theme: &Theme, focused: bool) -> Self {
        let roles = theme.roles();
        let surface = roles.surface.pane.over(theme.background());
        // `&:focus { background-tint: $foreground 5% }`, on the table and, separately, on its header.
        let tint = |paint: Paint| {
            if focused {
                theme.tint(paint, roles.surface.focus_tint, 100.0)
            } else {
                paint
            }
        };
        let base_bg = tint(surface);
        let base = Style::new()
            .fg(theme.color(roles.text.plain, base_bg))
            .bg(theme.paint(base_bg));

        // The tint leaves the terminal-color themes' header as it is.
        let header_bg = tint(roles.table.header.bg.over(surface));
        let header = Style::new()
            .fg(theme.color(roles.table.header.fg, header_bg))
            .bg(theme.paint(header_bg))
            .add_modifier(Modifier::BOLD);

        let (cursor_paint, cursor_ink, cursor_text) = if focused {
            let c = roles.cursor.focused;
            let bg = c.bg.over(base_bg);
            (bg, c.fg.over(bg), c.style)
        } else {
            let c = roles.cursor.blurred;
            let bg = c.bg.over(surface);
            (bg, c.fg.over(bg), c.style)
        };
        let cursor_text_color = theme.paint(cursor_ink);
        let cursor = cursor_text.apply(Style::new().fg(cursor_text_color).bg(theme.paint(cursor_paint)));

        let hover = base.bg(theme.color(roles.cursor.hover_bg, base_bg));

        let back = roles.scrollbar.track.over(surface);
        let scrollbar = ScrollbarColors {
            bar: theme.color(roles.scrollbar.thumb, back),
            back: theme.paint(back),
            fg: theme.color(roles.text.plain, base_bg),
            bg: theme.paint(base_bg),
        };
        let corner = Style::new()
            .fg(Color::Reset)
            .bg(theme.color(roles.scrollbar.corner, surface));
        Self {
            base,
            header,
            cursor,
            cursor_fg: cursor_text_color,
            hover,
            scrollbar,
            corner,
        }
    }
}

/// Which style a body row is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    Plain,
    Hover,
    Cursor,
}

/// What [`draw`] needs from the table.
pub(crate) struct Frame<'a, K> {
    pub(crate) columns: &'a [Column],
    pub(crate) rows: &'a [RowEntry<K>],
    pub(crate) widths: &'a [usize],
    pub(crate) layout: Layout,
    pub(crate) scroll_x: usize,
    pub(crate) scroll_y: usize,
    pub(crate) cursor: usize,
    pub(crate) hover: Option<usize>,
}

/// Draws the header, the visible rows and the scrollbars. Below the table, the parent's background shows.
pub(crate) fn draw<K>(buf: &mut Buffer, frame: &Frame<'_, K>, styles: &Styles) {
    let layout = frame.layout;
    let scroll_x = u16::try_from(frame.scroll_x).unwrap_or(u16::MAX);
    buf.set_style(layout.table.intersection(buf.area), styles.base);

    let header_cells = frame.columns.iter().map(|c| &c.header);
    draw_line(
        buf,
        layout.header(),
        padded(header_cells, frame.widths, styles.header, None),
        styles.header,
        scroll_x,
    );

    let visible = frame
        .rows
        .iter()
        .enumerate()
        .skip(frame.scroll_y)
        .take(layout.body_height);
    for (i, (index, row)) in visible.enumerate() {
        let kind = if index == frame.cursor {
            RowKind::Cursor
        } else if frame.hover == Some(index) {
            RowKind::Hover
        } else {
            RowKind::Plain
        };
        let (style, fg) = match kind {
            RowKind::Plain => (styles.base, None),
            RowKind::Hover => (styles.hover, None),
            RowKind::Cursor => (styles.cursor, Some(styles.cursor_fg)),
        };
        let line = padded(row.cells.iter().map(|c| &c.display), frame.widths, style, fg);
        draw_line(buf, layout.body_row(i), line, style, scroll_x);
    }

    let content_lines = 1 + frame.rows.len();
    let content_width = frame.widths.iter().sum();
    if layout.vertical_bar {
        let state = ScrollbarState {
            content: content_lines,
            window: layout.body_height + 1,
            position: frame.scroll_y,
        };
        scrollbar::render(buf, layout.vertical_bar_rect(), Axis::Vertical, state, styles.scrollbar);
    }
    if layout.horizontal_bar {
        let state = ScrollbarState {
            content: content_width,
            window: layout.viewport_width,
            position: frame.scroll_x,
        };
        scrollbar::render(
            buf,
            layout.horizontal_bar_rect(),
            Axis::Horizontal,
            state,
            styles.scrollbar,
        );
    }
    if layout.vertical_bar && layout.horizontal_bar {
        let corner = Rect {
            x: layout.vertical_bar_rect().x,
            width: layout.vertical_bar_rect().width,
            ..layout.horizontal_bar_rect()
        };
        for pos in corner.intersection(buf.area).positions() {
            buf[pos].set_symbol(" ").set_style(styles.corner);
        }
    }
}

/// One row's cells laid out at the column widths: each cell's spans over the row style, padded with the row style.
/// `fg` replaces every span's foreground, as the cursor does.
fn padded<'a>(
    cells: impl Iterator<Item = &'a Line<'static>>,
    widths: &[usize],
    style: Style,
    fg: Option<Color>,
) -> Line<'static> {
    let mut spans = Vec::new();
    let pad = |n: usize| Span::styled(" ".repeat(n), style);
    for (cell, &width) in cells.zip(widths) {
        spans.push(pad(CELL_PADDING));
        let room = width.saturating_sub(2 * CELL_PADDING);
        let cell_style = |span_style: Style| {
            let span_style = style.patch(cell.style).patch(span_style);
            fg.map_or(span_style, |fg| span_style.fg(fg))
        };
        let mut used = 0;
        if cell.width() <= room {
            for span in &cell.spans {
                spans.push(Span::styled(span.content.clone(), cell_style(span.style)));
            }
            used = cell.width();
        } else if let Some(kept) = room.checked_sub(1) {
            // A column fitted narrower than its cells: cut, and end with an ellipsis in the last span's style.
            let (cut, left) = display::cut_spans(cell.spans.iter().cloned(), kept);
            let last = cell.spans.last().map_or(Style::new(), |span| span.style);
            for span in cut {
                spans.push(Span::styled(span.content, cell_style(span.style)));
            }
            spans.push(pad(left));
            spans.push(Span::styled(display::ELLIPSIS, cell_style(last)));
            used = room;
        }
        spans.push(pad(width.saturating_sub(used + CELL_PADDING)));
    }
    Line::from(spans)
}

/// Draws `line` into the one-row `area`, scrolled `scroll_x` columns. The rest of the row, past the last column, is
/// filled with `style`: Textual extends the header, cursor and hover styles across the whole row.
fn draw_line(buf: &mut Buffer, area: Rect, line: Line<'static>, style: Style, scroll_x: u16) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    for pos in area.positions() {
        buf[pos].set_symbol(" ").set_style(style);
    }
    Paragraph::new(line).scroll((0, scroll_x)).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_without_overflow_is_as_tall_as_the_rows() {
        let layout = Layout::new(Rect::new(0, 0, 40, 10), 30, 3);
        assert_eq!(layout.table, Rect::new(0, 0, 40, 4));
        assert_eq!((layout.viewport_width, layout.body_height), (40, 3));
        assert!(!layout.vertical_bar && !layout.horizontal_bar);
    }

    #[test]
    fn layout_with_both_scrollbars() {
        let layout = Layout::new(Rect::new(0, 0, 40, 8), 60, 12);
        assert_eq!(layout.table, Rect::new(0, 0, 40, 8));
        assert_eq!((layout.viewport_width, layout.body_height), (38, 6));
        assert!(layout.vertical_bar && layout.horizontal_bar);
    }

    #[test]
    fn a_horizontal_scrollbar_can_bring_a_vertical_one() {
        // 4 lines fit in 4 rows, until the horizontal scrollbar takes one.
        let layout = Layout::new(Rect::new(0, 0, 20, 4), 30, 3);
        assert!(layout.vertical_bar && layout.horizontal_bar);
        assert_eq!(layout.body_height, 2);
    }

    #[test]
    fn a_vertical_scrollbar_can_bring_a_horizontal_one() {
        // 19 columns fit in 20, until the vertical scrollbar takes two.
        let layout = Layout::new(Rect::new(0, 0, 20, 4), 19, 10);
        assert!(layout.vertical_bar && layout.horizontal_bar);
        assert_eq!((layout.viewport_width, layout.body_height), (18, 2));
    }

    #[test]
    fn a_horizontal_scrollbar_in_one_row_hides_the_header() {
        // A compact section's one interior row, with columns wider than it: Textual draws only the scrollbar.
        let layout = Layout::new(Rect::new(1, 26, 38, 1), 60, 0);
        assert!(layout.horizontal_bar);
        assert_eq!(layout.table, Rect::new(1, 26, 38, 1));
        assert_eq!(layout.header().height, 0);
        assert_eq!(
            layout.horizontal_bar_rect(),
            Rect::new(1, 26, layout.viewport_width_u16(), 1)
        );
        // With room for it, the header is above the scrollbar.
        let layout = Layout::new(Rect::new(1, 26, 38, 2), 60, 0);
        assert_eq!(layout.header(), Rect::new(1, 26, 38, 1));
        assert_eq!(layout.horizontal_bar_rect(), Rect::new(1, 27, 38, 1));
    }

    fn columns(shrinking: Option<usize>) -> Vec<Column> {
        ["#", "Title", "Updated"]
            .into_iter()
            .enumerate()
            .map(|(i, header)| {
                let column = Column::new(header);
                if shrinking == Some(i) {
                    column.shrinking()
                } else {
                    column
                }
            })
            .collect()
    }

    #[test]
    fn fitting_leaves_widths_that_fit_and_tables_without_a_shrinking_column() {
        let widths = [4, 30, 12];
        let fits = fit_widths(&columns(Some(1)), &widths, Rect::new(0, 0, 46, 10), 3);
        assert!(matches!(fits, Cow::Borrowed(_)));
        let rigid = fit_widths(&columns(None), &widths, Rect::new(0, 0, 20, 10), 3);
        assert_eq!(&*rigid, widths);
    }

    #[test]
    fn the_shrinking_column_gives_up_the_overflow() {
        let widths = [4, 30, 12];
        let fitted = fit_widths(&columns(Some(1)), &widths, Rect::new(0, 0, 36, 10), 3);
        assert_eq!(&*fitted, [4, 20, 12]);
        assert!(!Layout::new(Rect::new(0, 0, 36, 10), fitted.iter().sum(), 3).horizontal_bar);
    }

    #[test]
    fn a_vertical_scrollbar_takes_its_columns_first() {
        // 12 rows and a header in 10: the scrollbar's two columns come off the title too.
        let fitted = fit_widths(&columns(Some(1)), &[4, 30, 12], Rect::new(0, 0, 36, 10), 12);
        assert_eq!(&*fitted, [4, 18, 12]);
        let layout = Layout::new(Rect::new(0, 0, 36, 10), fitted.iter().sum(), 12);
        assert!(layout.vertical_bar && !layout.horizontal_bar);
    }

    #[test]
    fn the_shrinking_column_stops_at_its_header() {
        // "Title" and its padding: 7. The rest still overflows, and scrolls.
        let fitted = fit_widths(&columns(Some(1)), &[4, 30, 12], Rect::new(0, 0, 10, 10), 3);
        assert_eq!(&*fitted, [4, 7, 12]);
    }

    #[test]
    fn several_shrinking_columns_give_from_the_widest_first() {
        let columns: Vec<Column> = ["Repo", "#", "Title", "Updated"]
            .into_iter()
            .map(|header| match header {
                "Repo" | "Title" => Column::new(header).shrinking(),
                _ => Column::new(header),
            })
            .collect();
        let widths = [26, 6, 20, 12];
        // 6 over: all from the repo, the widest, down to the title.
        let fitted = fit_widths(&columns, &widths, Rect::new(0, 0, 58, 10), 3);
        assert_eq!(&*fitted, [20, 6, 20, 12]);
        // 16 over: the repo down to the title's 20, then both alike, the title first on a tie.
        let fitted = fit_widths(&columns, &widths, Rect::new(0, 0, 48, 10), 3);
        assert_eq!(&*fitted, [15, 6, 15, 12]);
        // Past both headers the rest overflows.
        let fitted = fit_widths(&columns, &widths, Rect::new(0, 0, 20, 10), 3);
        assert_eq!(&*fitted, [6, 6, 7, 12]);
    }

    #[test]
    fn a_cut_cell_ends_with_an_ellipsis_inside_its_padding() {
        let cells = [Line::from("12"), Line::from("A long title"), Line::from("2026-10-28")];
        let line = padded(cells.iter(), &[4, 8, 12], Style::new(), None);
        assert_eq!(line.to_string(), " 12  A lon…  2026-10-28 ");
        // A wide grapheme that straddles the cut leaves its cell blank before the ellipsis.
        let wide = [Line::from("日本語")];
        assert_eq!(padded(wide.iter(), &[6], Style::new(), None).to_string(), " 日 … ");
    }

    #[test]
    fn the_python_repos_pane() {
        // Python's 160x48 repos pane: 62x15 inside the border, 86 columns and 50 rows of content.
        let layout = Layout::new(Rect::new(1, 4, 62, 15), 86, 50);
        assert_eq!((layout.viewport_width, layout.body_height), (60, 13));
        assert_eq!(layout.table.height, 15);
    }
}
