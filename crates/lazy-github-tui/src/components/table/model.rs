//! The table's data model: items and their cells, the sorted rows, and the cursor and scroll arithmetic.
//!
//! Nothing here renders. The arithmetic is plain functions over counts and offsets, so it's tested without a
//! terminal.

use std::fmt::Debug;
use std::hash::Hash;

use ratatui::text::Line;

/// Something a table shows as one row. Each table is generic over its item type.
pub trait TableItem {
    /// What identifies an item. Inserting an item whose key is already in the table replaces it.
    type Key: Clone + Eq + Hash + Debug;
    /// What the cells need besides the item, such as the favorites list.
    type Ctx;

    /// The item's key.
    fn key(&self) -> Self::Key;

    /// One cell per column, in column order.
    fn cells(&self, cx: &Self::Ctx) -> Vec<Cell>;
}

/// One cell: the value Python put in the row tuple, which sorting (and later search) uses, and what's drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// What sorting compares.
    pub value: CellValue,
    /// What's drawn, already styled.
    pub display: Line<'static>,
}

impl Cell {
    /// A cell with `value`, drawn as `display`. Control characters in `display` are drawn as U+FFFD, so they can't
    /// throw the row's columns out of line.
    pub fn new(value: CellValue, display: impl Into<Line<'static>>) -> Self {
        Self {
            value,
            display: crate::display::sanitize_line(display.into()),
        }
    }

    /// A string cell drawn as its own unstyled text.
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self::new(CellValue::Str(text.clone()), text)
    }

    /// An integer cell drawn as its decimal digits.
    pub fn int(n: i64) -> Self {
        Self::new(CellValue::Int(n), n.to_string())
    }
}

/// A cell's sort value: an `int`, a `str` or a naive `datetime`, as in Python's row tuples.
///
/// A column always holds one variant, so the derived order only ever compares like with like. `Str` compares by
/// code point, as Python does.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CellValue {
    /// A Python `int`.
    Int(i64),
    /// A Python `str`, raw: including markup such as `[green]★[/]`.
    Str(String),
    /// A naive Python `datetime`.
    DateTime(jiff::civil::DateTime),
}

/// A column: its header, already styled.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// The header text.
    pub header: Line<'static>,
}

impl Column {
    /// A column headed `header`.
    pub fn new(header: impl Into<Line<'static>>) -> Self {
        Self { header: header.into() }
    }
}

/// Which column the rows are sorted by, and in which direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SortSpec {
    /// The column's index.
    pub column: usize,
    /// Descending when true.
    pub reverse: bool,
}

impl SortSpec {
    /// Ascending by `column`.
    pub fn ascending(column: usize) -> Self {
        Self { column, reverse: false }
    }

    /// Descending by `column`.
    pub fn descending(column: usize) -> Self {
        Self { column, reverse: true }
    }
}

/// One shown row: its item's key, its computed cells, and when it was inserted.
#[derive(Debug, Clone)]
pub(crate) struct RowEntry<K> {
    pub(crate) key: K,
    pub(crate) cells: Vec<Cell>,
    /// Assigned from a table-wide counter on every insertion into the rows, so ties sort in insertion order.
    pub(crate) seq: u64,
}

/// Sorts `rows` by `sort`: the sort column's value, reversed for a descending sort, then insertion order.
///
/// Python sorts the rows with a stable `sorted(..., reverse=...)` over their insertion order, which keeps ties in
/// insertion order in both directions. Comparing `seq` ascending after the (possibly reversed) value does the same.
pub(crate) fn sort_rows<K>(rows: &mut [RowEntry<K>], sort: SortSpec) {
    rows.sort_by(|a, b| {
        let by_value = match (a.cells.get(sort.column), b.cells.get(sort.column)) {
            (Some(a), Some(b)) => a.value.cmp(&b.value),
            // A row without the sort column (an adapter bug) sorts first, consistently.
            (a, b) => a.is_some().cmp(&b.is_some()),
        };
        let by_value = if sort.reverse { by_value.reverse() } else { by_value };
        by_value.then(a.seq.cmp(&b.seq))
    });
}

/// The shown rows and each column's width. Reads go through `Deref`; every change goes through a method here, so
/// the widths can't miss one.
///
/// The widths are Textual's `Column.content_width` plus the padding: a high-water mark over the header and every row
/// ever added. Removing rows, clearing the table or filtering it with a search never narrows a column, so after a
/// repo switch the tables keep the previous repo's widths until wider rows arrive, as Python's do. Only
/// [`Rows::refresh_cells`], Textual's `update_cell`, measures a changed column again over the rows shown.
#[derive(Debug)]
pub(crate) struct Rows<K> {
    entries: Vec<RowEntry<K>>,
    widths: Vec<usize>,
}

impl<K> Rows<K> {
    /// No rows: each column as wide as its header.
    pub(crate) fn new(columns: &[Column]) -> Self {
        Self {
            entries: Vec::new(),
            widths: super::render::column_widths::<K>(columns, &[]),
        }
    }

    /// Appends `entry`, widening any column its cells don't fit.
    pub(crate) fn push(&mut self, entry: RowEntry<K>) {
        for (width, cell) in self.widths.iter_mut().zip(&entry.cells) {
            *width = (*width).max(super::render::cell_width(cell));
        }
        self.entries.push(entry);
    }

    /// Removes the row at `index`. The widths stay.
    pub(crate) fn remove(&mut self, index: usize) -> RowEntry<K> {
        self.entries.remove(index)
    }

    /// Removes every row. The widths stay, as Textual's `clear()` keeps its columns.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Takes every row out, for putting some back with [`Rows::push`]. The widths stay.
    pub(crate) fn drain(&mut self) -> std::vec::Drain<'_, RowEntry<K>> {
        self.entries.drain(..)
    }

    /// Sorts the rows by `sort`.
    pub(crate) fn sort(&mut self, sort: SortSpec) {
        sort_rows(&mut self.entries, sort);
    }

    /// Lets `update` change each row's cells, then measures every column whose cells changed again over the rows
    /// shown and its header, which can narrow it: Textual's `update_cell`. Other columns keep their widths.
    pub(crate) fn refresh_cells(&mut self, columns: &[Column], mut update: impl FnMut(&mut RowEntry<K>)) {
        let mut changed = vec![false; self.widths.len()];
        for row in &mut self.entries {
            let before = row.cells.clone();
            update(row);
            for (i, flag) in changed.iter_mut().enumerate() {
                *flag |= before.get(i) != row.cells.get(i);
            }
        }
        if changed.contains(&true) {
            let measured = super::render::column_widths(columns, &self.entries);
            for ((width, fresh), changed) in self.widths.iter_mut().zip(measured).zip(changed) {
                if changed {
                    *width = fresh;
                }
            }
        }
    }

    /// Each column's width, padding included.
    pub(crate) fn widths(&self) -> &[usize] {
        &self.widths
    }
}

impl<K> std::ops::Deref for Rows<K> {
    type Target = [RowEntry<K>];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

/// The cursor and the vertical scroll offset, in rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// The cursor's row index.
    pub cursor: usize,
    /// The index of the first row shown below the header.
    pub scroll_y: usize,
}

/// The vertical extent of the body: how many rows there are and how many fit below the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body {
    /// Rows in the table.
    pub rows: usize,
    /// Rows that fit below the header (Textual's `h`: the content height minus the header).
    pub height: usize,
}

impl Body {
    /// The largest scroll offset: the last row at the bottom.
    pub fn max_scroll(self) -> usize {
        self.rows.saturating_sub(self.height)
    }

    /// The last row's index, or 0 for an empty table.
    fn last(self) -> usize {
        self.rows.saturating_sub(1)
    }

    /// `cursor` clamped to the rows, as Textual's `_clamp_cursor_coordinate` does.
    pub fn clamp(self, cursor: usize) -> usize {
        cursor.min(self.last())
    }

    /// `pos` with the scroll offset clamped to the rows.
    pub fn clamp_scroll(self, pos: Position) -> Position {
        Position {
            scroll_y: pos.scroll_y.min(self.max_scroll()),
            ..pos
        }
    }

    /// `pos` scrolled, with the least movement, so the cursor's row is in view, and clamped to the rows. The cursor
    /// stays where it is.
    pub fn reveal_cursor(self, pos: Position) -> Position {
        Position {
            scroll_y: self.scroll_to(pos.cursor, pos.scroll_y),
            ..pos
        }
    }

    /// The scroll offset that brings `cursor` into view with the least movement (Textual's `scroll_to_region`).
    fn scroll_to(self, cursor: usize, scroll_y: usize) -> usize {
        let scroll_y = if cursor < scroll_y {
            cursor
        } else if self.height > 0 && cursor >= scroll_y + self.height {
            cursor + 1 - self.height
        } else {
            scroll_y
        };
        scroll_y.min(self.max_scroll())
    }

    /// Moves the cursor to `cursor`, clamped. As Textual's `cursor_coordinate` watcher does, the view only scrolls
    /// the cursor into view when the cursor actually moved.
    fn move_to(self, pos: Position, cursor: usize) -> Position {
        let cursor = self.clamp(cursor);
        let pos = self.clamp_scroll(pos);
        if cursor == pos.cursor {
            return pos;
        }
        Position {
            cursor,
            scroll_y: self.scroll_to(cursor, pos.scroll_y),
        }
    }

    /// `pos` with the cursor clamped to the rows, scrolled into view if that moved it: what Textual's re-assigning
    /// `cursor_coordinate` after removing a row does.
    pub fn clamped(self, pos: Position) -> Position {
        self.move_to(pos, pos.cursor)
    }

    /// `common.table.cursor_down`: one row down, clamped to the last row.
    pub fn cursor_down(self, pos: Position) -> Position {
        self.move_to(pos, pos.cursor + 1)
    }

    /// `common.table.cursor_up`: one row up, clamped to the first.
    pub fn cursor_up(self, pos: Position) -> Position {
        self.move_to(pos, pos.cursor.saturating_sub(1))
    }

    /// `common.table.page_down`: Textual's `action_page_down`. It counts rows from the cursor until their heights
    /// exceed `h`, so with one-line rows the cursor moves `h` rows, clamped to the last; the view scrolls down `h`.
    pub fn page_down(self, pos: Position) -> Position {
        let pos = Position {
            scroll_y: (pos.scroll_y + self.height).min(self.max_scroll()),
            ..pos
        };
        self.move_to(pos, pos.cursor + self.height)
    }

    /// `common.table.page_up`: the mirror of [`Self::page_down`].
    pub fn page_up(self, pos: Position) -> Position {
        let pos = Position {
            scroll_y: pos.scroll_y.saturating_sub(self.height),
            ..pos
        };
        self.move_to(pos, pos.cursor.saturating_sub(self.height))
    }

    /// `common.table.scroll_top`: the cursor to the first row.
    pub fn top(self, pos: Position) -> Position {
        self.move_to(pos, 0)
    }

    /// `common.table.scroll_bottom`: the cursor to the last row.
    pub fn bottom(self, pos: Position) -> Position {
        self.move_to(pos, self.last())
    }

    /// The view scrolled by `delta` rows (the mouse wheel), clamped. The cursor stays where it is.
    pub fn scroll_by(self, pos: Position, delta: isize) -> Position {
        let scroll_y = pos.scroll_y.saturating_add_signed(delta).min(self.max_scroll());
        Position { scroll_y, ..pos }
    }
}

/// The horizontal extent: the table's full width and the width of the viewport showing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Horizontal {
    /// The width of every column together.
    pub content: usize,
    /// The columns the rows are drawn in.
    pub viewport: usize,
}

/// A horizontal scroll that can't happen because the content fits, so the key falls through to the parent: Textual's
/// `action_scroll_left`/`right` raise `SkipAction` when there's no horizontal scrollbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CannotScroll;

impl Horizontal {
    /// The largest horizontal offset.
    pub fn max_scroll(self) -> usize {
        self.content.saturating_sub(self.viewport)
    }

    /// Whether there's anything to scroll, which is when Textual shows a horizontal scrollbar.
    pub fn overflows(self) -> bool {
        self.content > self.viewport
    }

    /// `common.table.scroll_right`: one column right, clamped.
    pub fn scroll_right(self, x: usize) -> Result<usize, CannotScroll> {
        if self.overflows() {
            Ok((x + 1).min(self.max_scroll()))
        } else {
            Err(CannotScroll)
        }
    }

    /// `common.table.scroll_left`: one column left, clamped.
    pub fn scroll_left(self, x: usize) -> Result<usize, CannotScroll> {
        if self.overflows() {
            Ok(x.saturating_sub(1).min(self.max_scroll()))
        } else {
            Err(CannotScroll)
        }
    }

    /// `common.table.page_right`: one viewport right, clamped. A no-op, not a fall-through, when the content fits.
    pub fn page_right(self, x: usize) -> usize {
        (x + self.viewport).min(self.max_scroll())
    }

    /// `common.table.page_left`: one viewport left, clamped.
    pub fn page_left(self, x: usize) -> usize {
        x.saturating_sub(self.viewport).min(self.max_scroll())
    }

    /// The view scrolled by `delta` columns (shift + wheel), clamped.
    pub fn scroll_by(self, x: usize, delta: isize) -> usize {
        x.saturating_add_signed(delta).min(self.max_scroll())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(rows: usize, height: usize) -> Body {
        Body { rows, height }
    }

    fn pos(cursor: usize, scroll_y: usize) -> Position {
        Position { cursor, scroll_y }
    }

    fn row(key: &'static str, value: CellValue, seq: u64) -> RowEntry<&'static str> {
        RowEntry {
            key,
            cells: vec![Cell::new(value, key)],
            seq,
        }
    }

    fn keys(rows: &[RowEntry<&'static str>]) -> Vec<&'static str> {
        rows.iter().map(|r| r.key).collect()
    }

    #[test]
    fn ties_keep_insertion_order_in_both_directions() {
        let mut rows = vec![
            row("a", CellValue::Int(1), 0),
            row("b", CellValue::Int(2), 1),
            row("c", CellValue::Int(1), 2),
            row("d", CellValue::Int(2), 3),
        ];
        sort_rows(&mut rows, SortSpec::ascending(0));
        assert_eq!(keys(&rows), ["a", "c", "b", "d"]);
        sort_rows(&mut rows, SortSpec::descending(0));
        assert_eq!(keys(&rows), ["b", "d", "a", "c"]);
    }

    #[test]
    fn strings_sort_by_code_point_like_python() {
        // `[green]★[/]` (a favorite) sorts before `☆`, because `[` is below U+2606.
        let mut rows = vec![
            row("plain", CellValue::Str("☆".into()), 0),
            row("fav", CellValue::Str("[green]★[/]".into()), 1),
            row("Upper", CellValue::Str("Z".into()), 2),
            row("lower", CellValue::Str("a".into()), 3),
        ];
        sort_rows(&mut rows, SortSpec::ascending(0));
        assert_eq!(keys(&rows), ["Upper", "fav", "lower", "plain"]);
    }

    #[test]
    fn datetimes_sort_chronologically() {
        let at = |h| CellValue::DateTime(jiff::civil::date(2026, 10, 25).at(h, 0, 0, 0));
        let mut rows = vec![row("noon", at(12), 0), row("dawn", at(6), 1), row("dusk", at(18), 2)];
        sort_rows(&mut rows, SortSpec::descending(0));
        assert_eq!(keys(&rows), ["dusk", "noon", "dawn"]);
    }

    #[test]
    fn page_down_and_up_move_by_the_body_height() {
        let b = body(12, 5);
        // Top: the cursor moves 5 and the view follows by 5.
        assert_eq!(b.page_down(pos(0, 0)), pos(5, 5));
        // Middle.
        assert_eq!(b.page_down(pos(3, 1)), pos(8, 6));
        // Near the end: both clamp, the cursor to the last row and the view to the last page.
        assert_eq!(b.page_down(pos(8, 6)), pos(11, 7));
        assert_eq!(b.page_down(pos(11, 7)), pos(11, 7));

        assert_eq!(b.page_up(pos(11, 7)), pos(6, 2));
        assert_eq!(b.page_up(pos(6, 2)), pos(1, 0));
        assert_eq!(b.page_up(pos(1, 0)), pos(0, 0));
        assert_eq!(b.page_up(pos(0, 0)), pos(0, 0));
    }

    #[test]
    fn page_down_scrolls_the_view_even_when_the_cursor_is_already_at_the_end() {
        // The wheel can leave the view above the cursor; paging scrolls it before the cursor moves.
        let b = body(12, 5);
        assert_eq!(b.page_down(pos(11, 0)), pos(11, 5));
    }

    #[test]
    fn cursor_moves_clamp_and_scroll_the_cursor_into_view() {
        let b = body(12, 5);
        assert_eq!(b.cursor_down(pos(4, 0)), pos(5, 1));
        assert_eq!(b.cursor_down(pos(11, 7)), pos(11, 7));
        assert_eq!(b.cursor_up(pos(5, 5)), pos(4, 4));
        assert_eq!(b.cursor_up(pos(0, 0)), pos(0, 0));
        assert_eq!(b.top(pos(9, 7)), pos(0, 0));
        assert_eq!(b.bottom(pos(0, 0)), pos(11, 7));
    }

    #[test]
    fn a_cursor_that_does_not_move_does_not_scroll() {
        // Textual only scrolls the cursor into view when the coordinate changes.
        let b = body(12, 5);
        assert_eq!(b.top(pos(0, 4)), pos(0, 4));
        assert_eq!(b.cursor_up(pos(0, 4)), pos(0, 4));
        assert_eq!(b.bottom(pos(11, 0)), pos(11, 0));
    }

    #[test]
    fn an_empty_table_keeps_the_cursor_at_zero() {
        let b = body(0, 5);
        for f in [
            Body::cursor_down,
            Body::cursor_up,
            Body::page_down,
            Body::page_up,
            Body::top,
            Body::bottom,
        ] {
            assert_eq!(f(b, pos(0, 0)), pos(0, 0));
        }
        assert_eq!(b.clamp(3), 0);
    }

    #[test]
    fn a_table_that_fits_never_scrolls() {
        let b = body(3, 5);
        assert_eq!(b.page_down(pos(0, 0)), pos(2, 0));
        assert_eq!(b.scroll_by(pos(0, 0), 2), pos(0, 0));
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let b = body(12, 5);
        assert_eq!(b.scroll_by(pos(0, 0), 2), pos(0, 2));
        assert_eq!(b.scroll_by(pos(0, 6), 2), pos(0, 7));
        assert_eq!(b.scroll_by(pos(0, 1), -2), pos(0, 0));
    }

    #[test]
    fn a_zero_height_body_still_moves_the_cursor() {
        let b = body(4, 0);
        assert_eq!(b.cursor_down(pos(0, 0)), pos(1, 0));
        assert_eq!(b.page_down(pos(1, 0)), pos(1, 0));
    }

    #[test]
    fn horizontal_scroll_clamps() {
        let span = Horizontal {
            content: 50,
            viewport: 20,
        };
        assert_eq!(span.max_scroll(), 30);
        assert_eq!(span.scroll_right(0), Ok(1));
        assert_eq!(span.scroll_right(30), Ok(30));
        assert_eq!(span.scroll_left(0), Ok(0));
        assert_eq!(span.scroll_left(5), Ok(4));
        assert_eq!(span.page_right(0), 20);
        assert_eq!(span.page_right(20), 30);
        assert_eq!(span.page_left(30), 10);
        assert_eq!(span.page_left(10), 0);
        assert_eq!(span.scroll_by(29, 4), 30);
    }

    #[test]
    fn horizontal_scroll_is_a_no_op_when_the_content_fits() {
        let span = Horizontal {
            content: 20,
            viewport: 20,
        };
        assert_eq!(span.scroll_right(0), Err(CannotScroll));
        assert_eq!(span.scroll_left(0), Err(CannotScroll));
        assert_eq!(span.page_right(0), 0);
        assert_eq!(span.page_left(0), 0);
    }
}
