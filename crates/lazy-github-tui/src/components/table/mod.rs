//! The searchable table every list view uses (repos, PRs, issues, workflows, notifications): Python's
//! `SearchableDataTable` around a `DataTable` with the `common.table.*` vim keys, in row-cursor mode.
//!
//! The table is pure state: keys, clicks and inserts change it synchronously and it never emits effects. What a key
//! means for the owning pane comes back as a [`TableEvent`]; the pane turns it into its own action. This module has
//! the rows, sorting, cursor, scrolling, rendering and the loading overlay; `search` has the `/` search box and its
//! filter (#101), and [`lazy`] asks for more pages (#102).

mod lazy;
mod model;
mod py_repr;
mod render;
mod search;

use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use indexmap::IndexMap;
use ratatui::Frame;
use ratatui::layout::{Position as CellPosition, Rect};
use tokio::time::Instant;

pub use lazy::{LOAD_MORE_BUFFER, LazyLoad, PageLoaded};
pub use model::{Body, CannotScroll, Cell, CellValue, Column, Horizontal, Position, SortSpec, TableItem};
pub use py_repr::{is_printable, py_repr_row, py_repr_str};
pub use search::TableFocus;

use self::model::{RowEntry, Rows, sort_rows};
use self::render::{Layout, Styles};
use self::search::SearchBox;
use crate::component::RenderCx;
use crate::components::loading;
use crate::components::text_input::InputOutcome;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::options::AppOptions;

/// The bindings the table answers while it has focus, in Textual's merged order, which the help panel lists them in:
/// the keys `DataTable` inherits from `ScrollView`, then the keys `DataTable` adds, then `_VimLikeDataTable`'s
/// `common.table.*`. Textual merges bindings key by key, so a key a subclass rebinds keeps its first place.
///
/// `DataTable`'s `enter` is not here: `_VimLikeDataTable`'s `common.table.select` (`enter,space`) replaces it, at
/// `enter`'s place. Rebinding `common.table.select` takes `enter` away too, as in Python.
pub const TABLE_BINDINGS: &[BindingId] = &[
    BindingId::TextualTableCursorUp,
    BindingId::TextualTableCursorDown,
    BindingId::TextualTableCursorLeft,
    BindingId::TextualTableCursorRight,
    BindingId::TextualTableScrollHome,
    BindingId::TextualTableScrollEnd,
    BindingId::TextualTablePageUp,
    BindingId::TextualTablePageDown,
    BindingId::TextualTablePageLeft,
    BindingId::TextualTablePageRight,
    BindingId::TableSelect,
    BindingId::TextualTableScrollTop,
    BindingId::TextualTableScrollBottom,
    BindingId::TableCursorDown,
    BindingId::TablePageDown,
    BindingId::TableCursorUp,
    BindingId::TablePageUp,
    BindingId::TableScrollRight,
    BindingId::TablePageRight,
    BindingId::TableScrollLeft,
    BindingId::TablePageLeft,
    BindingId::TableScrollTop,
    BindingId::TableScrollBottom,
];

/// The bindings of the component around the table, Python's `SearchableDataTable.BINDINGS`. They apply while focus
/// is anywhere inside it, after the table's own (or the search box's).
pub const SEARCHABLE_BINDINGS: &[BindingId] = &[BindingId::TableSearch];

/// Rows the mouse wheel scrolls per notch (Textual's `App.scroll_sensitivity_y`).
const WHEEL_ROWS: isize = 2;
/// Columns shift + wheel scrolls per notch (Textual's `App.scroll_sensitivity_x`).
const WHEEL_COLUMNS: isize = 4;

/// What the table tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableEvent<K> {
    /// `common.table.select`, or a second click on the cell under the cursor: the row with this key was chosen.
    Selected(K),
    /// A lazy table's cursor got near the end of the loaded items: fetch `page` with `per_page` items, and hand the
    /// result to [`SearchableTable::page_loaded`] with `generation`.
    LoadMore {
        /// The table's [`SearchableTable::generation`] when it asked.
        generation: u64,
        /// The page to fetch, from 1.
        page: u32,
        /// Items per page.
        per_page: u32,
    },
}

/// Whether the table took an input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Handled<E> {
    /// Not the table's: the owner passes it on, as Textual bubbles it to the parent.
    Ignored,
    /// Taken, with nothing to report.
    Consumed,
    /// Taken, and the owner should act on this.
    Event(E),
}

impl<E> Handled<E> {
    /// Whether the table took the input.
    pub fn is_handled(&self) -> bool {
        !matches!(self, Handled::Ignored)
    }
}

/// What a table binding does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableAction {
    Select,
    CursorDown,
    CursorUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    ScrollRight,
    ScrollLeft,
    PageRight,
    PageLeft,
    Home,
    End,
}

impl TableAction {
    fn from_binding(id: BindingId) -> Option<Self> {
        use BindingId as B;
        Some(match id {
            // Both run `select_cursor`. The chain never routes `textual.table.select_cursor` (see [`TABLE_BINDINGS`]),
            // but it means the same.
            B::TableSelect | B::TextualTableSelect => Self::Select,
            B::TableCursorDown | B::TextualTableCursorDown => Self::CursorDown,
            B::TableCursorUp | B::TextualTableCursorUp => Self::CursorUp,
            B::TablePageDown | B::TextualTablePageDown => Self::PageDown,
            B::TablePageUp | B::TextualTablePageUp => Self::PageUp,
            B::TableScrollTop | B::TextualTableScrollTop => Self::Top,
            B::TableScrollBottom | B::TextualTableScrollBottom => Self::Bottom,
            // In row-cursor mode, `DataTable`'s cursor right/left scroll instead.
            B::TableScrollRight | B::TextualTableCursorRight => Self::ScrollRight,
            B::TableScrollLeft | B::TextualTableCursorLeft => Self::ScrollLeft,
            B::TablePageRight | B::TextualTablePageRight => Self::PageRight,
            B::TablePageLeft | B::TextualTablePageLeft => Self::PageLeft,
            B::TextualTableScrollHome => Self::Home,
            B::TextualTableScrollEnd => Self::End,
            _ => return None,
        })
    }
}

/// A table of `T` items: sorted rows under a header, a row cursor, and two-way scrolling.
pub struct SearchableTable<T: TableItem> {
    columns: Vec<Column>,
    sort: SortSpec,
    /// Every item, in insertion order: Python's `self.items` dict, which the M2 cache writes in this order.
    items: IndexMap<T::Key, T>,
    /// The shown rows, sorted.
    rows: Rows<T::Key>,
    /// The cells of the items the last submitted search hid, so the next search can match them again.
    hidden: HashMap<T::Key, Vec<Cell>>,
    search: SearchBox,
    focus: TableFocus,
    next_seq: u64,
    pos: Position,
    /// The column of the cursor coordinate. Row mode never shows it, but clicking the cursor row selects only when
    /// the click hits the same cell as the coordinate.
    cursor_column: usize,
    scroll_x: usize,
    /// The row under the mouse, while the hover highlight is shown. Keys hide it until the mouse moves again.
    hover: Option<usize>,
    loading: bool,
    /// The last frame drew the table without its loading overlay ([`Self::render_content`]), so the overlay isn't
    /// there to take the mouse.
    overlay_hidden: bool,
    /// When the loading overlay was first drawn, which sets the animation's phase.
    loading_since: Option<Instant>,
    animations: bool,
    generation: u64,
    /// Paging, once the owner turns lazy loading on.
    lazy: Option<LazyLoad>,
    /// The area of the last render, which paging and scrolling measure against.
    area: Rect,
    /// How the last render laid the component out.
    drawn: Drawn,
}

/// How a render laid the table component out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawn {
    /// The search box, while it's shown, above the table. `box_shown` says the box was drawn: a frame that newly lays
    /// it out shortens the table, so it scrolls the cursor's row back into view (decision `search-box-table-height`).
    /// The loading overlay counts as not drawing it.
    WithSearch { box_shown: bool },
    /// The table alone ([`SearchableTable::render_table`]): a maximized `DataTable`, with the box outside it.
    TableAlone,
}

impl<T: TableItem> SearchableTable<T> {
    /// An empty table with `columns`, sorted by `sort`, animating its loading overlay.
    pub fn new(columns: Vec<Column>, sort: SortSpec) -> Self {
        Self {
            columns,
            sort,
            items: IndexMap::new(),
            rows: Rows::new(),
            hidden: HashMap::new(),
            search: SearchBox::new(),
            focus: TableFocus::Table,
            next_seq: 0,
            pos: Position::default(),
            cursor_column: 0,
            scroll_x: 0,
            hover: None,
            loading: false,
            overlay_hidden: false,
            loading_since: None,
            animations: true,
            generation: 0,
            lazy: None,
            area: Rect::default(),
            drawn: Drawn::WithSearch { box_shown: false },
        }
    }

    /// The table with the startup options applied: whether the loading overlay animates.
    #[must_use]
    pub fn with_options(mut self, options: AppOptions) -> Self {
        self.animations = options.animations;
        self
    }

    // Contents.

    /// Adds `item`, or replaces the item with its key, then re-sorts.
    ///
    /// A replaced item keeps its place in [`Self::items`], but its row is removed and appended again, so it goes to
    /// the end of its tie group. Removing it first clamps the cursor, as Textual's `remove_row` does: replacing the
    /// last row while the cursor is on it moves the cursor up one. Otherwise the cursor stays at its index, so the
    /// item under it can change.
    ///
    /// The first row of an empty table highlights row 0, as Textual's `add_row` does, which runs a lazy table's
    /// check: the result is its [`TableEvent::LoadMore`].
    #[must_use = "a lazy table's first rows may ask for the next page"]
    pub fn insert(&mut self, item: T, cx: &T::Ctx) -> Option<TableEvent<T::Key>> {
        self.insert_many([item], cx)
    }

    /// [`Self::insert`] for each item in turn.
    #[must_use = "a lazy table's first rows may ask for the next page"]
    pub fn insert_many(&mut self, items: impl IntoIterator<Item = T>, cx: &T::Ctx) -> Option<TableEvent<T::Key>> {
        let was_empty = self.rows.is_empty();
        for item in items {
            self.insert_unsorted(item, cx);
        }
        // Sorting is a total order on (value, insertion), so sorting once gives what sorting after each insert does.
        sort_rows(self.rows.edit(), self.sort);
        if was_empty && !self.rows.is_empty() {
            self.check_load_more(self.pos.cursor)
        } else {
            None
        }
    }

    fn insert_unsorted(&mut self, item: T, cx: &T::Ctx) {
        let key = item.key();
        if self.items.contains_key(&key) {
            self.remove_row(&key);
            // Python appends every added row, even one a search hid.
            self.hidden.remove(&key);
        }
        let cells = item.cells(cx);
        // `IndexMap::insert` keeps an existing key's position.
        self.items.insert(key.clone(), item);
        self.rows.edit().push(RowEntry {
            key,
            cells,
            seq: self.next_seq,
        });
        self.next_seq += 1;
    }

    /// Removes the item with `key`, clamping the cursor to the rows left.
    pub fn remove(&mut self, key: &T::Key) -> Option<T> {
        let item = self.items.shift_remove(key)?;
        self.remove_row(key);
        self.hidden.remove(key);
        Some(item)
    }

    fn remove_row(&mut self, key: &T::Key) {
        if let Some(index) = self.rows.iter().position(|row| &row.key == key) {
            self.rows.edit().remove(index);
            // Only a cursor past the end moves, and only then is the body height (and so the column widths) needed.
            if self.pos.cursor >= self.rows.len() {
                self.pos = self.body().clamped(self.pos);
            }
        }
    }

    /// Removes everything: the cursor and both scroll offsets go back to 0, [`Self::generation`] moves on, and lazy
    /// loading turns off, as Python's `clear_rows` drops the load function. The search box and its text stay, as
    /// `clear_rows` leaves the input alone.
    pub fn clear(&mut self) {
        self.lazy = None;
        self.items.clear();
        self.rows.edit().clear();
        self.hidden.clear();
        self.pos = Position::default();
        self.cursor_column = 0;
        self.scroll_x = 0;
        self.hover = None;
        self.generation += 1;
    }

    /// [`Self::clear`], then [`Self::insert_many`]: Python's `clear_rows` followed by `add_items`.
    pub fn replace_all(&mut self, items: impl IntoIterator<Item = T>, cx: &T::Ctx) {
        self.clear();
        // Lazy loading is off after a clear, so the first rows run no check.
        let _ = self.insert_many(items, cx);
    }

    /// Recomputes every row's cells, for example after a favorite toggle, then re-sorts. Rows keep their insertion
    /// order for ties.
    pub fn refresh_rows(&mut self, cx: &T::Ctx) {
        for row in self.rows.edit() {
            if let Some(item) = self.items.get(&row.key) {
                row.cells = item.cells(cx);
            }
        }
        for (key, cells) in &mut self.hidden {
            if let Some(item) = self.items.get(key) {
                *cells = item.cells(cx);
            }
        }
        sort_rows(self.rows.edit(), self.sort);
    }

    /// Every item, in insertion order.
    pub fn items(&self) -> impl ExactSizeIterator<Item = &T> {
        self.items.values()
    }

    /// The item with `key`.
    pub fn get(&self, key: &T::Key) -> Option<&T> {
        self.items.get(key)
    }

    /// The keys of the shown rows, top to bottom.
    pub fn row_keys(&self) -> impl ExactSizeIterator<Item = &T::Key> {
        self.rows.iter().map(|row| &row.key)
    }

    /// How many rows are shown.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The item under the cursor.
    pub fn selected(&self) -> Option<&T> {
        self.rows.get(self.pos.cursor).and_then(|row| self.items.get(&row.key))
    }

    /// The cursor's row index.
    pub fn cursor(&self) -> usize {
        self.pos.cursor
    }

    /// The scroll offsets: the first body row shown, and the first column.
    pub fn scroll(&self) -> (usize, usize) {
        (self.pos.scroll_y, self.scroll_x)
    }

    /// Bumped by every [`Self::clear`], so a page that arrives for an older generation is dropped.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    // Loading.

    /// Shows or hides the loading overlay over the whole area. Keys keep working underneath it, as in Python.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        self.overlay_hidden = false;
        if !loading {
            self.loading_since = None;
        }
    }

    /// Whether the loading overlay is shown.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Whether the last frame drew the loading overlay, which takes mouse events inside the area.
    pub fn overlay_shown(&self) -> bool {
        self.loading && !self.overlay_hidden
    }

    /// When the next animation frame is due, while the animated overlay is shown.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        (self.loading && self.animations).then_some(now + loading::FRAME_INTERVAL)
    }

    // Input.

    /// The bindings the table declares, for the binding chain, the footer and the help panel.
    pub fn bindings(&self) -> &'static [BindingId] {
        TABLE_BINDINGS
    }

    /// A key. While the search box has focus it goes to the box. Otherwise it's matched against [`TABLE_BINDINGS`]
    /// through `keymap`, each binding it fires tried in turn until one applies, and then against
    /// [`SEARCHABLE_BINDINGS`]. A key that none takes is [`Handled::Ignored`].
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap) -> Handled<TableEvent<T::Key>> {
        if self.focus == TableFocus::Search {
            return self.search_key(key, keymap);
        }
        for id in keymap.matches_in(TABLE_BINDINGS, key) {
            let handled = self.on_binding(id);
            if handled.is_handled() {
                return handled;
            }
        }
        if keymap.match_in(SEARCHABLE_BINDINGS, key).is_some() {
            self.open_search();
            return Handled::Consumed;
        }
        Handled::Ignored
    }

    /// One of [`TABLE_BINDINGS`] fired. Horizontal scrolling by one column is [`Handled::Ignored`] when the columns
    /// fit, so the key falls through to the parent (Textual's `SkipAction`).
    pub fn on_binding(&mut self, id: BindingId) -> Handled<TableEvent<T::Key>> {
        let Some(action) = TableAction::from_binding(id) else {
            return Handled::Ignored;
        };
        let (body, horizontal) = self.extents();
        let handled = match action {
            TableAction::Select => {
                return match self.rows.get(self.pos.cursor) {
                    Some(row) => {
                        let key = row.key.clone();
                        self.hover = None;
                        Handled::Event(TableEvent::Selected(key))
                    }
                    None => Handled::Consumed,
                };
            }
            TableAction::CursorDown => self.move_to(body.cursor_down(self.pos)),
            TableAction::CursorUp => self.move_to(body.cursor_up(self.pos)),
            TableAction::PageDown => self.move_to(body.page_down(self.pos)),
            TableAction::PageUp => self.move_to(body.page_up(self.pos)),
            TableAction::Top => self.move_to(body.top(self.pos)),
            TableAction::Bottom => self.move_to(body.bottom(self.pos)),
            TableAction::ScrollRight => self.scroll_x_to(horizontal.scroll_right(self.scroll_x)),
            TableAction::ScrollLeft => self.scroll_x_to(horizontal.scroll_left(self.scroll_x)),
            TableAction::PageRight => self.scroll_x_to(Ok(horizontal.page_right(self.scroll_x))),
            TableAction::PageLeft => self.scroll_x_to(Ok(horizontal.page_left(self.scroll_x))),
            TableAction::Home => self.scroll_x_to(Ok(0)),
            TableAction::End => self.scroll_x_to(Ok(horizontal.max_scroll())),
        };
        if handled.is_handled() {
            self.hover = None;
        }
        handled
    }

    fn move_to(&mut self, pos: Position) -> Handled<TableEvent<T::Key>> {
        let moved = pos.cursor != self.pos.cursor;
        self.pos = pos;
        self.after_cursor_move(moved)
    }

    /// Textual posts `RowHighlighted` only when the cursor index changes; a lazy table checks for more then.
    fn after_cursor_move(&mut self, moved: bool) -> Handled<TableEvent<T::Key>> {
        match moved.then(|| self.check_load_more(self.pos.cursor)).flatten() {
            Some(event) => Handled::Event(event),
            None => Handled::Consumed,
        }
    }

    fn scroll_x_to(&mut self, x: Result<usize, CannotScroll>) -> Handled<TableEvent<T::Key>> {
        match x {
            Ok(x) => {
                self.scroll_x = x;
                Handled::Consumed
            }
            Err(CannotScroll) => Handled::Ignored,
        }
    }

    /// A mouse event. `area` is where the table was last drawn; a move outside it hides the hover highlight, so an
    /// owner can pass every move on. While loading, the overlay swallows events inside the area, as Textual's
    /// `LoadingIndicator` does. A left press on the search box focuses it, and one on the table moves focus off the
    /// box; the owner reads [`Self::focus`] afterwards.
    pub fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Handled<TableEvent<T::Key>> {
        let at = CellPosition::new(event.column, event.row);
        if !area.contains(at) {
            if matches!(event.kind, MouseEventKind::Moved) {
                self.hover = None;
            }
            return Handled::Ignored;
        }
        if self.overlay_shown() {
            self.hover = None;
            return Handled::Consumed;
        }
        // A press focuses what it lands on, as Textual focuses the widget under a mouse down.
        let press = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
        let (search, area) = self.laid_out_split(area);
        if let Some(search) = search.filter(|search| search.contains(at)) {
            self.hover = None;
            if press {
                // Textual focuses the box first, which selects everything, and then the press places the cursor.
                self.focus_box();
                self.search.input.press(search, at.x);
                return Handled::Consumed;
            }
            return Handled::Ignored;
        }
        let layout = self.layout_in(area);
        // Below the rows is the unfocusable container, which leaves focus where it is, as Textual does.
        if press && layout.table.contains(at) {
            self.focus_table();
        }
        let row = self.row_at(layout, at);
        match event.kind {
            MouseEventKind::Moved => {
                self.hover = row;
                if row.is_some() {
                    Handled::Consumed
                } else {
                    Handled::Ignored
                }
            }
            MouseEventKind::Up(MouseButton::Left) => match row {
                Some(row) => self.click(layout, row, at.x),
                None => Handled::Ignored,
            },
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let sign = if matches!(event.kind, MouseEventKind::ScrollDown) {
                    1
                } else {
                    -1
                };
                if event.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) {
                    self.wheel_x(layout, sign * WHEEL_COLUMNS)
                } else {
                    self.wheel_y(layout, sign * WHEEL_ROWS)
                }
            }
            MouseEventKind::ScrollRight => self.wheel_x(layout, WHEEL_COLUMNS),
            MouseEventKind::ScrollLeft => self.wheel_x(layout, -WHEEL_COLUMNS),
            _ => Handled::Ignored,
        }
    }

    /// The row index under `at`, including the blank part of the line past the last column. The header, the
    /// scrollbars and the space below the table are no row.
    fn row_at(&self, layout: Layout, at: CellPosition) -> Option<usize> {
        let in_body = (0..layout.body_height).find(|&i| layout.body_row(i).contains(at))?;
        let index = self.pos.scroll_y + in_body;
        (index < self.rows.len()).then_some(index)
    }

    /// Textual's `_on_click`: the cursor moves to the clicked cell, and a click on the cell the cursor coordinate
    /// already holds selects the row.
    fn click(&mut self, layout: Layout, row: usize, x: u16) -> Handled<TableEvent<T::Key>> {
        let column = self.column_at(usize::from(x - layout.table.x) + self.scroll_x);
        let same_cell = row == self.pos.cursor && column == self.cursor_column;
        let before = self.pos.cursor;
        self.cursor_column = column;
        self.pos = Body {
            rows: self.rows.len(),
            height: layout.body_height,
        }
        .clamped(Position {
            cursor: row,
            ..self.pos
        });
        self.hover = Some(row);
        match self.rows.get(row) {
            Some(entry) if same_cell => Handled::Event(TableEvent::Selected(entry.key.clone())),
            _ => self.after_cursor_move(self.pos.cursor != before),
        }
    }

    /// The column at content offset `x`; past the last column it's column 0, as Textual's out-of-bounds cells say.
    fn column_at(&self, x: usize) -> usize {
        let mut right = 0;
        for (i, width) in self.widths().iter().enumerate() {
            right += width;
            if x < right {
                return i;
            }
        }
        0
    }

    /// The wheel scrolls the view, not the cursor. It bubbles when there's no scrollbar or the view is already at
    /// that end (Textual's `_scroll_down_for_pointer` returning false).
    fn wheel_y(&mut self, layout: Layout, delta: isize) -> Handled<TableEvent<T::Key>> {
        if !layout.vertical_bar {
            return Handled::Ignored;
        }
        let body = Body {
            rows: self.rows.len(),
            height: layout.body_height,
        };
        let scrolled = body.scroll_by(body.clamp_scroll(self.pos), delta);
        if scrolled == self.pos {
            return Handled::Ignored;
        }
        self.pos = scrolled;
        Handled::Consumed
    }

    fn wheel_x(&mut self, layout: Layout, delta: isize) -> Handled<TableEvent<T::Key>> {
        if !layout.horizontal_bar {
            return Handled::Ignored;
        }
        let horizontal = Horizontal {
            content: self.widths().iter().sum(),
            viewport: layout.viewport_width,
        };
        let x = horizontal.scroll_by(self.scroll_x, delta);
        if x == self.scroll_x {
            return Handled::Ignored;
        }
        self.scroll_x = x;
        Handled::Consumed
    }

    // Searching.

    /// Which part of the component has focus.
    pub fn focus(&self) -> TableFocus {
        self.focus
    }

    /// Whether the search box is shown. Only then is it a focus stop, before the table (Textual's `can_focus`).
    pub fn search_visible(&self) -> bool {
        self.search.visible
    }

    /// The search box's text.
    pub fn search_text(&self) -> &str {
        self.search.input.value()
    }

    /// `common.table.search`: shows the search box and focuses it, keeping any text already in it.
    ///
    /// Focusing the box selects all of its text, as Textual's `Input` does on focus, so typing replaces the last query.
    pub fn open_search(&mut self) {
        self.search.visible = true;
        self.focus_box();
        self.hover = None;
    }

    /// Focuses the search box, if it's shown (Shift+Tab from the table). Returns whether it took focus.
    pub fn focus_search(&mut self) -> bool {
        if self.search.visible {
            self.focus_box();
        }
        self.focus == TableFocus::Search
    }

    /// Focus moves onto the box, which selects all of its text. A box that already has focus keeps its selection, as
    /// Textual only selects on a focus change.
    fn focus_box(&mut self) {
        if self.focus != TableFocus::Search {
            self.focus = TableFocus::Search;
            self.search.input.select_all();
        }
    }

    /// Focus moves off the search box: to the table, or out of the component. The owner calls this on every focus
    /// change away from the box. The box blurs, and hides when it's blank.
    pub fn focus_table(&mut self) {
        if self.focus == TableFocus::Search {
            self.search.blur();
        }
        self.focus = TableFocus::Table;
    }

    /// Whether the focused part takes `key` before the binding chain: the search box claims printable keys and its
    /// editing keys, so `q` types a q instead of quitting. The table claims nothing.
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.focus == TableFocus::Search && self.search.input.consumes_key(key, keymap)
    }

    /// A bracketed paste: its first line goes into the search box while the box has focus.
    pub fn handle_paste(&mut self, text: &str) -> Handled<TableEvent<T::Key>> {
        if self.focus != TableFocus::Search {
            return Handled::Ignored;
        }
        self.search.input.handle_paste(text);
        Handled::Consumed
    }

    fn search_key(&mut self, key: &KeyChord, keymap: &Keymap) -> Handled<TableEvent<T::Key>> {
        match self.search.input.handle_key(key, keymap) {
            InputOutcome::Changed => Handled::Consumed,
            InputOutcome::Submitted => match self.apply_search() {
                Some(event) => Handled::Event(event),
                None => Handled::Consumed,
            },
            // A search key the box doesn't type, once rebound, still applies inside the component: it focuses the
            // box, which already has focus.
            InputOutcome::Ignored if keymap.match_in(SEARCHABLE_BINDINGS, key).is_some() => Handled::Consumed,
            InputOutcome::Ignored => Handled::Ignored,
        }
    }

    /// `handle_submitted_search`: every item is put back in [`Self::items`] order and those that don't match are
    /// hidden, so ties go back to `items` order. As Textual's `clear` does, the cursor and the scroll go back to 0.
    /// Then focus moves to the table.
    ///
    /// When any row is left, the first one is highlighted, as the first `add_row` into Textual's cleared table does,
    /// and a lazy table runs its load check for row 0. That check counts every item, the hidden ones too, as Python's
    /// `len(self.items)` does. A search that matches nothing adds no row and runs no check.
    fn apply_search(&mut self) -> Option<TableEvent<T::Key>> {
        let query = self.search.query();
        let mut cells: HashMap<T::Key, Vec<Cell>> = self
            .rows
            .edit()
            .drain(..)
            .map(|row| (row.key, row.cells))
            .chain(self.hidden.drain())
            .collect();
        for key in self.items.keys() {
            let Some(cells) = cells.remove(key) else {
                continue;
            };
            if search::matches(&cells, &query) {
                self.rows.edit().push(RowEntry {
                    key: key.clone(),
                    cells,
                    seq: self.next_seq,
                });
                self.next_seq += 1;
            } else {
                self.hidden.insert(key.clone(), cells);
            }
        }
        sort_rows(self.rows.edit(), self.sort);
        self.pos = Position::default();
        self.cursor_column = 0;
        self.scroll_x = 0;
        self.hover = None;
        self.focus_table();
        if self.rows.is_empty() {
            None
        } else {
            self.check_load_more(0)
        }
    }

    /// Where the search box is drawn in `area`, the component's area, while it's shown.
    pub fn search_area(&self, area: Rect) -> Option<Rect> {
        self.search.split(area).0
    }

    // Measuring.

    fn widths(&self) -> &[usize] {
        self.rows.widths(&self.columns)
    }

    fn layout_in(&self, area: Rect) -> Layout {
        Layout::new(area, self.widths().iter().sum(), self.rows.len())
    }

    /// Where [`Self::render`] puts the table itself in `area`: below the search box while it's shown, as tall as the
    /// header, the rows and any horizontal scrollbar, capped at what's left of `area` (Textual's `height: auto`).
    /// An owner hit-tests clicks against it and sizes a maximized table by it.
    pub fn table_rect(&self, area: Rect) -> Rect {
        let (_, table) = self.search.split(area);
        self.layout_in(table).table.intersection(area)
    }

    /// Where [`Self::render_table`] puts the table in `area`: the table alone, with no room for the search box.
    pub fn bare_table_rect(&self, area: Rect) -> Rect {
        self.layout_in(area).table
    }

    /// Where the last frame drew the table itself in `area`, the area it was given: below the search box while the
    /// box was laid out, or all of it when the table was drawn alone. It takes clicks.
    pub fn drawn_table_rect(&self, area: Rect) -> Rect {
        self.layout_in(self.laid_out_split(area).1).table.intersection(area)
    }

    /// [`SearchBox::split`], as the last render laid the box out.
    fn laid_out_split(&self, area: Rect) -> (Option<Rect>, Rect) {
        match self.drawn {
            Drawn::WithSearch { .. } => self.search.split(area),
            Drawn::TableAlone => (None, area),
        }
    }

    /// Where the table went in the last rendered area: below the search box while it's shown.
    fn table_area(&self) -> Rect {
        self.laid_out_split(self.area).1
    }

    /// The vertical and horizontal extents, measured against the last rendered area.
    fn extents(&self) -> (Body, Horizontal) {
        let content = self.widths().iter().sum();
        let layout = Layout::new(self.table_area(), content, self.rows.len());
        let body = Body {
            rows: self.rows.len(),
            height: layout.body_height,
        };
        let horizontal = Horizontal {
            content,
            viewport: layout.viewport_width,
        };
        (body, horizontal)
    }

    /// The vertical extent, measured against the last rendered area.
    fn body(&self) -> Body {
        self.extents().0
    }

    // Drawing.

    /// Draws the search box, while it's shown, and the table below it (it's only as tall as its rows), or the loading
    /// overlay over all of `area`. `cx.focused` says the component has focus, and [`Self::focus`] which part gets
    /// the focused styles. `cx.now` picks the animation frame.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        if self.loading {
            self.area = area;
            self.drawn = Drawn::WithSearch { box_shown: false };
            self.overlay_hidden = false;
            let since = *self.loading_since.get_or_insert(cx.now);
            let elapsed = cx.now.checked_duration_since(since).unwrap_or(Duration::ZERO);
            loading::render(frame.buffer_mut(), area, cx.theme, self.animations, elapsed);
            return;
        }
        self.render_content(frame, area, cx);
    }

    /// [`Self::render`] without the loading overlay, even while loading: the search box and the table. Python's
    /// overlay belongs to the container around the `DataTable`, so a table maximized during a load shows without it.
    pub fn render_content(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let (search, table) = self.search.split(area);
        let reveal = search.is_some() && self.drawn != (Drawn::WithSearch { box_shown: true });
        self.area = area;
        self.drawn = Drawn::WithSearch {
            box_shown: search.is_some(),
        };
        self.overlay_hidden = self.loading;
        if let Some(search) = search {
            self.draw_search(frame, search, cx);
        }
        self.draw_table(frame, table, reveal, cx);
    }

    /// The table alone in all of `area`, without the search box or the loading overlay: the maximized `DataTable`,
    /// which leaves the box outside it even while the box has focus. Typing into the box still filters this table.
    pub fn render_table(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        self.area = area;
        self.drawn = Drawn::TableAlone;
        self.overlay_hidden = self.loading;
        self.draw_table(frame, area, false, cx);
    }

    /// The search box alone in `area`, as a maximized `Input` draws it. Clicks on it don't reach the table.
    pub fn render_search(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        self.draw_search(frame, area, cx);
    }

    fn draw_search(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let focused = cx.focused && self.focus == TableFocus::Search;
        self.search
            .input
            .render(frame.buffer_mut(), area, cx.theme, cx.theme.background(), focused);
    }

    /// The table laid out in `area`. With `reveal`, the view first scrolls the cursor's row into view.
    fn draw_table(&mut self, frame: &mut Frame<'_>, area: Rect, reveal: bool, cx: &RenderCx<'_>) {
        let widths = self.rows.widths(&self.columns);
        let layout = Layout::new(area, widths.iter().sum(), self.rows.len());
        // Rows may have gone since the last frame; the view never scrolls past the end.
        let body = Body {
            rows: self.rows.len(),
            height: layout.body_height,
        };
        self.pos = if reveal {
            body.reveal_cursor(self.pos)
        } else {
            body.clamp_scroll(self.pos)
        };
        self.scroll_x = self.scroll_x.min(
            Horizontal {
                content: widths.iter().sum(),
                viewport: layout.viewport_width,
            }
            .max_scroll(),
        );
        let styles = Styles::new(cx.theme, cx.focused && self.focus == TableFocus::Table);
        render::draw(
            frame.buffer_mut(),
            &render::Frame {
                columns: &self.columns,
                rows: &self.rows,
                widths,
                layout,
                scroll_x: self.scroll_x,
                scroll_y: self.pos.scroll_y,
                cursor: self.pos.cursor,
                hover: self.hover,
            },
            &styles,
        );
    }
}

#[cfg(test)]
mod tests;
