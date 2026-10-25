//! Lazy loading: Python's `LazilyLoadedDataTable`. Once its owner turns it on, the table asks for the next page
//! whenever the cursor gets within [`LOAD_MORE_BUFFER`] rows of the end of the loaded items.
//!
//! The table never fetches anything itself. A check that fires comes back as [`TableEvent::LoadMore`]; the owner
//! turns it into a `gh` request tagged with the table's generation and hands the result to
//! [`SearchableTable::page_loaded`].

use lazy_github_core::gh::GhError;

use super::{SearchableTable, TableEvent, TableItem};

/// How close to the end of the loaded items the cursor gets before the next page is asked for: Python's
/// `load_more_data_buffer`.
pub const LOAD_MORE_BUFFER: usize = 5;

/// A lazy table's paging state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyLoad {
    /// Items per page.
    batch_size: u32,
    /// The last page loaded.
    current_batch: u32,
    /// False once a page came back empty.
    can_load_more: bool,
    /// The page asked for and not yet loaded.
    in_flight: Option<u32>,
    /// The highest cursor index seen while a page was in flight, re-checked once when it lands. Python queues each
    /// highlight behind its fetch lock and re-checks every one after the fetch; only the highest can still fire.
    pending_cursor: Option<usize>,
}

impl LazyLoad {
    fn new(batch_size: u32, current_batch: u32) -> Self {
        Self {
            batch_size,
            current_batch,
            can_load_more: true,
            in_flight: None,
            pending_cursor: None,
        }
    }

    /// Items per page.
    pub fn batch_size(&self) -> u32 {
        self.batch_size
    }

    /// The last page loaded.
    pub fn current_batch(&self) -> u32 {
        self.current_batch
    }

    /// False once a page came back empty.
    pub fn can_load_more(&self) -> bool {
        self.can_load_more
    }

    /// The page asked for and not yet loaded.
    pub fn in_flight(&self) -> Option<u32> {
        self.in_flight
    }
}

/// The page to ask for, if any: the one after the last loaded, once `remaining = items - cursor` is at most
/// [`LOAD_MORE_BUFFER`]. `items` counts every loaded item, hidden by a search or not, as Python's `len(self.items)`
/// does. Whether a page is already in flight is the caller's concern.
pub(crate) fn should_load(items: usize, cursor: usize, lazy: &LazyLoad) -> Option<u32> {
    let remaining = items.saturating_sub(cursor);
    (lazy.can_load_more && remaining <= LOAD_MORE_BUFFER).then_some(lazy.current_batch + 1)
}

/// What [`SearchableTable::page_loaded`] did with a page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct PageLoaded<K> {
    /// The page was inserted (an empty page too), so the item set may have changed: the owner writes its cache, as
    /// Python's `add_items` does after every page. False for a failed, stale or unexpected page.
    pub applied: bool,
    /// The next [`TableEvent::LoadMore`], when the re-check after the page fires at once.
    pub event: Option<TableEvent<K>>,
}

impl<K> PageLoaded<K> {
    fn dropped() -> Self {
        Self {
            applied: false,
            event: None,
        }
    }
}

impl<T: TableItem> SearchableTable<T> {
    /// Turns lazy loading on with `batch_size` items per page, `current_batch` pages already loaded.
    ///
    /// Call it before inserting the first page: Python sets the load function in the same handler that adds the
    /// page, and the highlight the first row posts is only handled after both, so a short first page asks for page
    /// 2 straight away.
    pub fn enable_lazy_load(&mut self, batch_size: u32, current_batch: u32) {
        self.lazy = Some(LazyLoad::new(batch_size, current_batch));
    }

    /// The paging state, while lazy loading is on.
    pub fn lazy_load(&self) -> Option<&LazyLoad> {
        self.lazy.as_ref()
    }

    /// The check Python runs on every `RowHighlighted`, for the cursor at `cursor`.
    pub(super) fn check_load_more(&mut self, cursor: usize) -> Option<TableEvent<T::Key>> {
        let items = self.items.len();
        let lazy = self.lazy.as_mut()?;
        if lazy.in_flight.is_some() {
            lazy.pending_cursor = Some(lazy.pending_cursor.map_or(cursor, |seen| seen.max(cursor)));
            return None;
        }
        let page = should_load(items, cursor, lazy)?;
        lazy.in_flight = Some(page);
        Some(TableEvent::LoadMore {
            generation: self.generation,
            page,
            per_page: lazy.batch_size,
        })
    }

    /// A page the owner fetched for a [`TableEvent::LoadMore`] arrived.
    ///
    /// A page from an older [`Self::generation`] (the table was cleared since), or one that isn't the page in
    /// flight, is dropped. A failure keeps the paging state, so the next cursor move asks again. Otherwise the items
    /// are inserted as [`Self::insert_many`] does, and the cursor seen while the page was in flight is checked again.
    pub fn page_loaded(
        &mut self,
        generation: u64,
        page: u32,
        result: Result<Vec<T>, GhError>,
        cx: &T::Ctx,
    ) -> PageLoaded<T::Key> {
        if generation != self.generation {
            tracing::debug!(
                generation,
                page,
                current = self.generation,
                "dropping a page for a cleared table"
            );
            return PageLoaded::dropped();
        }
        let Some(lazy) = self.lazy.as_mut() else {
            tracing::debug!(page, "dropping a page for a table that isn't lazy");
            return PageLoaded::dropped();
        };
        if lazy.in_flight != Some(page) {
            tracing::warn!(page, in_flight = ?lazy.in_flight, "dropping a page that wasn't asked for");
            return PageLoaded::dropped();
        }
        lazy.in_flight = None;
        let pending = lazy.pending_cursor.take();
        let items = match result {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(page, %error, "loading the next page failed");
                return PageLoaded::dropped();
            }
        };
        lazy.current_batch += 1;
        if items.is_empty() {
            lazy.can_load_more = false;
        }
        let first_rows = self.insert_many(items, cx);
        // A page that filled an empty table already ran the check for cursor 0; otherwise re-check what was seen.
        let event = first_rows.or_else(|| pending.and_then(|cursor| self.check_load_more(cursor)));
        PageLoaded { applied: true, event }
    }
}

#[cfg(test)]
mod tests;
