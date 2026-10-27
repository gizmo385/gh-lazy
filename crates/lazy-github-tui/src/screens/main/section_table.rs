//! The one view of a section's table the main screen works through, whatever its items: every section's
//! `SearchableTable` goes through the same keys, mouse, focus, drawing and maximize paths as the repos table.

use crossterm::event::MouseEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use tokio::time::Instant;

use crate::component::RenderCx;
use crate::components::table::{Handled, SearchableTable, TableEvent, TableFocus, TableItem};
use crate::keymap::{BindingId, KeyChord, Keymap};

/// What a section's table tells the screen, without its key type. On [`SectionEvent::Selected`] the owner reads
/// its own table's `selected()`, which is the row the event was for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SectionEvent {
    /// The row under the cursor was chosen.
    Selected,
    /// A lazy table wants `page`: [`TableEvent::LoadMore`].
    LoadMore {
        /// The table's generation when it asked.
        generation: u64,
        /// The page to fetch, from 1.
        page: u32,
        /// Items per page.
        per_page: u32,
    },
    /// The search box copied or cut this text: [`TableEvent::Copied`].
    Copied(String),
}

impl<K> From<TableEvent<K>> for SectionEvent {
    fn from(event: TableEvent<K>) -> Self {
        match event {
            TableEvent::Selected(_) => SectionEvent::Selected,
            TableEvent::LoadMore {
                generation,
                page,
                per_page,
            } => SectionEvent::LoadMore {
                generation,
                page,
                per_page,
            },
            TableEvent::Copied(text) => SectionEvent::Copied(text),
        }
    }
}

fn erase<K>(handled: Handled<TableEvent<K>>) -> Handled<SectionEvent> {
    match handled {
        Handled::Ignored => Handled::Ignored,
        Handled::Consumed => Handled::Consumed,
        Handled::Event(event) => Handled::Event(event.into()),
    }
}

/// The methods of [`SearchableTable`] the main screen calls, object-safe so `MainScreen::table(id)` can hand out any
/// section's table. See [`SearchableTable`] for what each does.
pub(super) trait SectionTable {
    fn on_binding(&mut self, id: BindingId) -> Handled<SectionEvent>;
    fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str) -> Handled<SectionEvent>;
    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Handled<SectionEvent>;
    fn handle_paste(&mut self, text: &str) -> Handled<SectionEvent>;
    fn search_captures_mouse(&self) -> bool;
    fn search_captured_mouse(&mut self, event: MouseEvent) -> bool;
    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool;
    fn focus(&self) -> TableFocus;
    fn focus_search(&mut self) -> bool;
    fn focus_table(&mut self);
    fn open_search(&mut self);
    fn search_visible(&self) -> bool;
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>);
    fn render_table(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>);
    fn render_search(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>);
    fn table_rect(&self, area: Rect) -> Rect;
    fn bare_table_rect(&self, area: Rect) -> Rect;
    fn drawn_table_rect(&self, area: Rect) -> Rect;
    fn next_deadline(&self, now: Instant) -> Option<Instant>;
    fn overlay_shown(&self) -> bool;
}

impl<T: TableItem> SectionTable for SearchableTable<T> {
    fn on_binding(&mut self, id: BindingId) -> Handled<SectionEvent> {
        erase(SearchableTable::on_binding(self, id))
    }

    fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str) -> Handled<SectionEvent> {
        erase(SearchableTable::handle_key(self, key, keymap, clipboard))
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Handled<SectionEvent> {
        erase(SearchableTable::handle_mouse(self, event, area))
    }

    fn handle_paste(&mut self, text: &str) -> Handled<SectionEvent> {
        erase(SearchableTable::handle_paste(self, text))
    }

    fn search_captures_mouse(&self) -> bool {
        SearchableTable::search_captures_mouse(self)
    }

    fn search_captured_mouse(&mut self, event: MouseEvent) -> bool {
        SearchableTable::search_captured_mouse(self, event)
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        SearchableTable::consumes_key(self, key, keymap)
    }

    fn focus(&self) -> TableFocus {
        SearchableTable::focus(self)
    }

    fn focus_search(&mut self) -> bool {
        SearchableTable::focus_search(self)
    }

    fn focus_table(&mut self) {
        SearchableTable::focus_table(self);
    }

    fn open_search(&mut self) {
        SearchableTable::open_search(self);
    }

    fn search_visible(&self) -> bool {
        SearchableTable::search_visible(self)
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        SearchableTable::render(self, frame, area, cx);
    }

    fn render_table(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        SearchableTable::render_table(self, frame, area, cx);
    }

    fn render_search(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        SearchableTable::render_search(self, frame, area, cx);
    }

    fn table_rect(&self, area: Rect) -> Rect {
        SearchableTable::table_rect(self, area)
    }

    fn bare_table_rect(&self, area: Rect) -> Rect {
        SearchableTable::bare_table_rect(self, area)
    }

    fn drawn_table_rect(&self, area: Rect) -> Rect {
        SearchableTable::drawn_table_rect(self, area)
    }

    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        SearchableTable::next_deadline(self, now)
    }

    fn overlay_shown(&self) -> bool {
        SearchableTable::overlay_shown(self)
    }
}

/// The earliest animation frame any of `tables` asks for.
pub(super) fn earliest_deadline<'a>(
    tables: impl IntoIterator<Item = &'a dyn SectionTable>,
    now: Instant,
) -> Option<Instant> {
    tables.into_iter().filter_map(|table| table.next_deadline(now)).min()
}
