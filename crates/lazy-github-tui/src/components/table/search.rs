//! The `/` search box above the table and the filter it applies: Python's `ToggleableSearchInput` and
//! `SearchableDataTable.handle_submitted_search`.
//!
//! The box starts hidden. `common.table.search` shows and focuses it; `enter` filters the rows once, by a
//! case-insensitive substring match over Python's `str()` of each row tuple ([`super::py_repr`]), and moves focus to
//! the table. When the box loses focus with only whitespace in it, it hides again.

use ratatui::layout::Rect;

use super::model::Cell;
use super::py_repr::py_repr_row;
use crate::components::text_input::{self, TextInput};

/// The box's placeholder.
const PLACEHOLDER: &str = "Search...";

/// Rows the shown box takes from the top of the table's area: its own 3 and `margin-bottom: 1`.
pub(crate) const ROWS: u16 = text_input::HEIGHT + 1;

/// Which part of the table component has focus, for the owner's focus model (#75). The component only knows this
/// while the owner gives it focus at all; [`super::SearchableTable::render`] reads `cx.focused` for that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableFocus {
    /// The table itself.
    #[default]
    Table,
    /// The search box.
    Search,
}

/// The search box: its input, and whether it's shown (Textual's `display`, which also gates `can_focus`).
#[derive(Debug, Clone)]
pub(crate) struct SearchBox {
    pub(crate) input: TextInput,
    pub(crate) visible: bool,
}

impl SearchBox {
    pub(crate) fn new() -> Self {
        Self {
            input: TextInput::new(PLACEHOLDER),
            visible: false,
        }
    }

    /// `ToggleableSearchInput._on_blur`: hides the box when what's left in it is blank.
    pub(crate) fn blur(&mut self) {
        if py_strip(self.input.value()).is_empty() {
            self.visible = false;
        }
    }

    /// The query a submit filters by: the text, stripped and lowercased as Python's `.strip().lower()` does.
    pub(crate) fn query(&self) -> String {
        py_strip(self.input.value()).to_lowercase()
    }

    /// Splits the component's `area` into the box (while shown) and the area the table lays itself out in: the rows
    /// left below the box.
    ///
    /// Python's `DataTable` keeps `max-height: 100%` of the whole area below the box, so the container clips up to 4
    /// of its rows and its horizontal scrollbar, and the cursor can move onto the hidden rows (Py bug B40). The
    /// table here gets only what the box leaves, so every row the cursor reaches is drawn
    /// (decision `search-box-table-height`).
    pub(crate) fn split(&self, area: Rect) -> (Option<Rect>, Rect) {
        if !self.visible {
            return (None, area);
        }
        let taken = area.height.min(ROWS);
        let search = Rect {
            height: taken.min(text_input::HEIGHT),
            ..area
        };
        let table = Rect {
            y: area.y.saturating_add(taken),
            height: area.height - taken,
            ..area
        };
        (Some(search), table)
    }
}

/// Whether a row with `cells` passes the filter `query` (already stripped and lowercased): an empty query passes
/// everything; otherwise it must be a substring of the lowercased row representation.
pub(crate) fn matches(cells: &[Cell], query: &str) -> bool {
    query.is_empty()
        || py_repr_row(cells.iter().map(|cell| &cell.value))
            .to_lowercase()
            .contains(query)
}

/// Python's `str.strip()`: `str.isspace` also counts the separators U+001C to U+001F, which Rust's `White_Space`
/// doesn't.
fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

#[cfg(test)]
mod tests;
