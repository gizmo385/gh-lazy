//! GFM tables, drawn as Textual's `MarkdownTable` draws them: a grid of `auto` columns with `grid-gutter: 1 1` and
//! `keyline: thin $foreground 20%`, cells padded one column each side, and the columns expanded to fill the width or
//! shrunk (wrapping their cells) to fit it, with `GridLayout`'s and `_resolve.resolve`'s arithmetic.

use std::iter;

use ratatui::style::Style;
use ratatui::text::Span;

use super::Inline;
use super::layout::{Painter, Row, TableLine};
use super::parse::is_space;
use crate::theme::{Paint, ThemeColor};

/// A cell's `padding: 0 1`, both sides.
const CELL_PADDING: usize = 2;

/// `white 30%`, the table's background on light themes.
const WHITE_30: ThemeColor = ThemeColor::Rgba {
    r: 255,
    g: 255,
    b: 255,
    a: 0.3,
};

/// How a table's parts are styled.
struct Styles {
    /// The table's own background, if it has one, under everything else.
    bg: Style,
    line: Style,
    header: Style,
    body: Style,
}

impl Painter<'_> {
    /// A table `width` cells wide: keylines around and between all cells, the header row first.
    pub(super) fn table(
        &self,
        head: &[Vec<Inline>],
        body: &[Vec<Vec<Inline>>],
        width: usize,
        under: Paint,
    ) -> Vec<Row> {
        let p = self.palette();
        // `MarkdownTable:light { background: white 30% }`; on dark themes it's drawn on what's under it.
        let background = (!self.theme.dark).then(|| WHITE_30.over(under));
        let under = background.unwrap_or(under);
        let bg = background.map_or_else(Style::new, |b| Style::new().bg(self.theme.paint(b)));
        let styles = Styles {
            bg,
            line: bg.patch(self.fg(p.foreground.multiply_alpha(0.2), under)),
            header: bg.patch(self.fg(p.primary, under)),
            body: bg.patch(self.text_style(under)),
        };

        let table_rows = || iter::once(head).chain(body.iter().map(Vec::as_slice));
        let natural: Vec<usize> = (0..head.len())
            .map(|c| {
                table_rows()
                    .map(|row| content_width(&row[c]) + CELL_PADDING)
                    .max()
                    .unwrap_or(CELL_PADDING)
            })
            .collect();
        // `GridLayout`'s `column_minimums`: at least 1, and each cell's longest word plus its padding.
        let minimums: Vec<usize> = (0..head.len())
            .map(|c| {
                table_rows()
                    .map(|row| minimal_width(&row[c]) + CELL_PADDING)
                    .fold(1, usize::max)
            })
            .collect();
        let widths = column_widths(&natural, &minimums, width);

        let mut rows = vec![keyline(&widths, ['┌', '┬', '┐'], styles.line)];
        for (index, cells) in table_rows().enumerate() {
            if index > 0 {
                rows.push(keyline(&widths, ['├', '┼', '┤'], styles.line));
            }
            rows.extend(self.table_row(cells, &widths, index == 0, under, &styles));
        }
        rows.push(keyline(&widths, ['└', '┴', '┘'], styles.line));
        rows
    }

    /// One table row's lines, as tall as its tallest cell. Every cell stretches to that height; header text sits at
    /// the bottom (`content-align: left bottom`), body text at the top.
    fn table_row(
        &self,
        cells: &[Vec<Inline>],
        widths: &[usize],
        header: bool,
        under: Paint,
        styles: &Styles,
    ) -> Vec<Row> {
        let base = if header { styles.header } else { styles.body };
        let laid_out: Vec<Vec<Row>> = cells
            .iter()
            .zip(widths)
            .map(|(cell, &width)| {
                let inner = width.saturating_sub(CELL_PADDING);
                if inner == 0 {
                    return Vec::new();
                }
                let mut rows = self.inlines(cell, inner, base, under);
                for span in rows.iter_mut().flat_map(|row| &mut row.spans) {
                    span.style = styles.bg.patch(span.style);
                }
                rows
            })
            .collect();
        let height = laid_out.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let mut lines: Vec<Row> = (0..height)
            .map(|y| Row {
                spans: vec![Span::styled("│", styles.line)],
                links: Vec::new(),
                table: Some(TableLine {
                    first: y == 0,
                    cells: Vec::new(),
                }),
            })
            .collect();
        // The header's fill takes its color, as Textual styles the whole header cell; a body cell's is plain.
        let fill = if header { styles.header } else { styles.bg };
        for (cell, (cell_rows, &width)) in laid_out.into_iter().zip(widths).enumerate() {
            let top = if header { height - cell_rows.len() } else { 0 };
            let mut cell_rows = cell_rows.into_iter();
            for (y, line) in lines.iter_mut().enumerate() {
                let content = if y >= top { cell_rows.next() } else { None };
                let before = line.links.len();
                push_cell(line, content, width, fill, styles.bg);
                let added = line.links.len() - before;
                if let Some(table) = &mut line.table {
                    table.cells.extend(iter::repeat_n(cell, added));
                }
                line.spans.push(Span::styled("│", styles.line));
            }
        }
        lines
    }
}

/// Appends one line of a cell `width` wide to `line`: its padding, its text if it has any here, and fill.
fn push_cell(line: &mut Row, content: Option<Row>, width: usize, fill: Style, bg: Style) {
    if width < CELL_PADDING {
        line.spans.push(Span::styled(" ".repeat(width), bg));
        return;
    }
    let start = line.width() + 1;
    line.spans.push(Span::styled(" ", bg));
    let mut used = 0;
    if let Some(content) = content {
        used = content.width();
        line.links.extend(
            content
                .links
                .into_iter()
                .map(|(columns, target)| (columns.start + start..columns.end + start, target)),
        );
        line.spans.extend(content.spans);
    }
    let inner = width - CELL_PADDING;
    if used < inner {
        line.spans.push(Span::styled(" ".repeat(inner - used), fill));
    }
    line.spans.push(Span::styled(" ", bg));
}

/// A horizontal keyline across columns `widths` wide, with `left`, `middle` and `right` at the joins.
fn keyline(widths: &[usize], [left, middle, right]: [char; 3], style: Style) -> Row {
    let mut text = String::from(left);
    for (index, &width) in widths.iter().enumerate() {
        if index > 0 {
            text.push(middle);
        }
        text.extend(iter::repeat_n('─', width));
    }
    text.push(right);
    Row {
        spans: vec![Span::styled(text, style)],
        links: Vec::new(),
        table: None,
    }
}

/// The cells' text as one string, a hard break as `\n`.
fn plain(cell: &[Inline]) -> String {
    cell.iter().map(|run| run.text.as_str()).collect()
}

/// The widest line of a cell's text: Textual's `get_content_width` for a `Static`.
fn content_width(cell: &[Inline]) -> usize {
    plain(cell)
        .split('\n')
        .map(|line| Span::raw(line).width())
        .max()
        .unwrap_or(0)
}

/// The widest word of a cell's text, or 0 when it's blank: `Content.get_minimal_width`.
fn minimal_width(cell: &[Inline]) -> usize {
    plain(cell)
        .split(is_space)
        .map(|word| Span::raw(word).width())
        .max()
        .unwrap_or(0)
}

/// The width of each column of a table drawn `width` wide, from the columns' natural and minimal widths (padding
/// included), as Textual's `GridLayout` resolves `auto` columns with `expand` and `shrink` on.
///
/// The space for columns is the width less the outer keylines and one gutter between columns. Columns too wide for it
/// give up space in order, each down to its minimum, until they fit (`resolve`'s `minimums` pass, whose
/// `max(width / used, 1)` is always 1). Then all of them are scaled to fill it exactly, which expands narrow tables
/// and shrinks any still too wide, and each column's edge is the floor of its fractional position.
pub(super) fn column_widths(natural: &[usize], minimums: &[usize], width: usize) -> Vec<usize> {
    let gutters = natural.len().saturating_sub(1);
    // Narrower than its keylines and gutters, a table gets columns of no width rather than negative ones.
    let total = width.saturating_sub(2 + gutters);
    let mut widths = natural.to_vec();
    let mut used: usize = widths.iter().sum();
    if used > total {
        for (column, &minimum) in widths.iter_mut().zip(minimums) {
            let excess = used - total;
            let updated = minimum.max(column.saturating_sub(excess));
            used = used - *column + updated;
            *column = updated;
            if used <= total {
                break;
            }
        }
    }
    if used == 0 {
        return vec![0; widths.len()];
    }
    // Edges at `floor(total * prefix / used)`, exactly, in integers.
    let mut prefix = 0;
    let mut edge = 0;
    widths
        .iter()
        .map(|&column| {
            prefix += column;
            let next = total * prefix / used;
            let width = next - edge;
            edge = next;
            width
        })
        .collect()
}
