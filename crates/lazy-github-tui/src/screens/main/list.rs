//! A repo-scoped, lazily paged list section: the table of the Pull Requests, Issues and Workflows sections. Python
//! spreads this over `LazilyLoadedDataTable` and the `LazyGithubContainer` around it.
//!
//! The section owns the later pages. Its first page belongs to its owner, which requests it (PRs and issues share
//! theirs) and hands the result to [`ListSection::first_page`].
//!
//! The section also keeps the repo's table cache (`<owner>_<repo>_<name>.json`, Python's `cache_name`): the cached
//! rows show under the loading overlay until the first page answers, and every row is written back after each page.

use std::path::PathBuf;

use lazy_github_core::cache::{self, CacheScope};
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::models::Repository;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::component::Cx;
use crate::components::table::{Column, Page, SearchableTable, SortSpec, TableEvent, TableItem};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::options::AppOptions;

/// Items per page, and so the size of a first page: Python's `batch_size=30`.
pub(super) const PER_PAGE: u32 = 30;

/// What fills one kind of list section: its columns, its items and how it asks for a page of them.
pub(super) trait ListSource {
    /// The rows' items.
    type Item: TableItem<Ctx = ()> + Serialize + DeserializeOwned;

    /// The name of the repo's cache file, `<owner>_<repo>_<CACHE_NAME>.json`: Python's `cache_name`.
    const CACHE_NAME: &'static str;

    /// The item's model name in the Python app, for the cache's schema warning.
    const TYPE_NAME: &'static str;

    /// The table's columns.
    fn columns() -> Vec<Column>;

    /// How the rows sort.
    fn sort() -> SortSpec;

    /// The request for one later page, built from the repo and the config at that moment.
    fn page_request(repo: &Repository, page: u32, per_page: u32, config: &Config) -> GithubRequest;

    /// This source's page out of a response, or the response handed back if it isn't one.
    fn page_items(response: GithubResponse) -> Result<Result<Page<Self::Item>, GhError>, GithubResponse>;

    /// What a failed later page does, after logging the section's line.
    fn page_failed(error: &GhError) -> PageFailure;
}

/// What a failed later page does to paging (Py bug B43).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PageFailure {
    /// Rows and paging state stay, and the next cursor move asks for the page again.
    Retry,
    /// The failure counts as an empty last page, which ends paging: Python's workflow fetchers return `[]` on an
    /// HTTP error.
    End,
}

/// The later page in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PageRequest {
    id: RequestId,
    /// The table's generation when it asked.
    generation: u64,
    page: u32,
}

/// One list section's state: the table, the repo it was loaded for, the later page in flight and whether it's
/// compact (` (empty)`).
pub(super) struct ListSection<S: ListSource> {
    table: SearchableTable<S::Item>,
    repo: Option<Repository>,
    page: Option<PageRequest>,
    /// The cache read in flight for the repo.
    cache: Option<RequestId>,
    compact: bool,
}

impl<S: ListSource> std::fmt::Debug for ListSection<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListSection")
            .field("rows", &self.table.row_count())
            .field("loading", &self.table.is_loading())
            .field("repo", &self.repo.as_ref().map(|repo| &repo.full_name))
            .field("page", &self.page)
            .field("cache", &self.cache)
            .field("compact", &self.compact)
            .finish()
    }
}

impl<S: ListSource> ListSection<S> {
    /// An empty table, before any repo is selected: its header row only, not compact.
    pub(super) fn new(options: AppOptions) -> Self {
        Self {
            table: SearchableTable::new(S::columns(), S::sort()).with_options(options),
            repo: None,
            page: None,
            cache: None,
            compact: false,
        }
    }

    /// The table.
    pub(super) fn table(&self) -> &SearchableTable<S::Item> {
        &self.table
    }

    /// The table, for the screen's input and drawing paths.
    pub(super) fn table_mut(&mut self) -> &mut SearchableTable<S::Item> {
        &mut self.table
    }

    /// Whether the section is drawn three rows high with ` (empty)` in its title, as of the last
    /// [`Self::refresh_compact`].
    pub(super) fn compact(&self) -> bool {
        self.compact
    }

    /// The repo the rows are for, once one was selected.
    #[cfg(test)]
    pub(super) fn repo(&self) -> Option<&Repository> {
        self.repo.as_ref()
    }

    /// A repo was selected: the part of #170's `load_repo` contract every list pane shares. The page in flight is
    /// cancelled, the rows clear (cursor and scroll back to 0, paging off) and the loading overlay shows. `compact`
    /// stays as it was until the first page lands, as in Python.
    ///
    /// Then the repo's cache file is read, for [`Self::on_cache_read`] to show under the overlay: Python's
    /// `load_cached_*` before the fetch.
    pub(super) fn begin(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        for id in self
            .page
            .take()
            .map(|page| page.id)
            .into_iter()
            .chain(self.cache.take())
        {
            cx.effect(Effect::Cancel(id));
        }
        self.table.clear();
        self.table.set_loading(true);
        self.cache = Some(cx.read_cache(cache_file::<S>(repo, cx.config())));
        self.repo = Some(repo.clone());
    }

    /// Takes the result of the section's cache read, and hands back any other.
    ///
    /// Python's `initialize_from_cache`: the cached rows go in under the overlay. Python restores before it fetches;
    /// here the two race, so a read that lands after the first page is dropped rather than mixing older rows into
    /// newer ones. A read for an earlier repo was cancelled by [`Self::begin`], and if it still arrives it's handed
    /// back.
    pub(super) fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        if self.cache.take_if(|pending| *pending == id).is_none() {
            return Some(result);
        }
        let Some(repo) = &self.repo else {
            return Some(result);
        };
        // Only the first page turns paging on after the clear in `begin`.
        if self.table.lazy_load().is_some() {
            tracing::debug!(
                name = S::CACHE_NAME,
                "Dropping the cached rows: the first page already arrived"
            );
            return None;
        }
        let items: Vec<S::Item> = cache::load(&cache_file::<S>(repo, cx.config()), result, S::TYPE_NAME);
        // Paging is off until the first page, so the first rows ask for nothing.
        let _ = self.table.insert_many(items, &());
        None
    }

    /// The first page arrived, `None` when it failed (the owner logs that).
    ///
    /// A page replaces any cached rows (Py bug B53: Python adds it to them); a failure keeps them. Paging then turns
    /// on, as `lazy.rs` asks before the rows go in, so a short page asks for page 2 at once, as Python's first
    /// `RowHighlighted` does. Paging counts the kept cached rows, as Python's `remaining` does. Either way every row
    /// is written to the cache, which after a failure rewrites what was read.
    ///
    /// The first page's [`Page::last`] isn't read: Python only ends paging on a later page, and a first page with no
    /// rows has no cursor to move.
    pub(super) fn first_page(&mut self, page: Option<Page<S::Item>>, cx: &mut Cx<'_>) {
        if let Some(page) = page {
            if self.table.row_count() > 0 {
                self.table.clear();
            }
            self.table.enable_lazy_load(PER_PAGE, 1);
            let event = self.table.insert_many(page.items, &());
            self.follow(event.as_ref(), cx);
        } else {
            self.table.enable_lazy_load(PER_PAGE, 1);
        }
        self.table.set_loading(false);
        self.save_cache(cx);
    }

    /// Python's `save_to_cache`: writes every item in the table, in insertion order, to the cache file of the repo
    /// the rows were loaded for.
    fn save_cache(&self, cx: &mut Cx<'_>) {
        let Some(repo) = &self.repo else {
            return;
        };
        let path = cache_file::<S>(repo, cx.config());
        cx.effect(Effect::WriteCache {
            path,
            text: cache::to_json(self.table.items()),
        });
    }

    /// Adds `item` unless a row already has its key, whose row then stays as it was: Python's `item_in_table` check
    /// before `add_item`, for a looked-up item. A new row is sorted into place and written to the cache, and the
    /// section stops being compact.
    pub(super) fn insert_if_missing(&mut self, item: S::Item, cx: &mut Cx<'_>) {
        if self.table.get(&item.key()).is_some() {
            return;
        }
        // The first row of an empty table runs the lazy check, as Textual's `add_row` does.
        let event = self.table.insert(item, &());
        self.follow(event.as_ref(), cx);
        self.save_cache(cx);
        self.refresh_compact();
    }

    /// Adds `item`, or replaces the row with its key: Python's `add_item`, for a created or edited item. The row is
    /// sorted into place, the cache is rewritten and the section stops being compact.
    pub(super) fn upsert(&mut self, item: S::Item, cx: &mut Cx<'_>) {
        // The first row of an empty table runs the lazy check, as Textual's `add_row` does.
        let event = self.table.insert(item, &());
        self.follow(event.as_ref(), cx);
        self.save_cache(cx);
        self.refresh_compact();
    }

    /// Compact exactly when the table has no items. The owner calls it where Python calls `set_compact`.
    pub(super) fn refresh_compact(&mut self) {
        self.compact = self.table.items().len() == 0;
    }

    /// Requests `page` for the repo the rows are for, as the table asked in `generation`
    /// ([`MainAction::LoadMore`](super::MainAction::LoadMore)). An ask from before the last [`Self::begin`] is
    /// dropped.
    pub(super) fn load_more(&mut self, generation: u64, page: u32, per_page: u32, cx: &mut Cx<'_>) {
        if generation != self.table.generation() {
            tracing::debug!(generation, page, "dropping a page request for a cleared table");
            return;
        }
        let Some(repo) = &self.repo else {
            tracing::debug!(page, "dropping a page request before any repo was selected");
            return;
        };
        let request = S::page_request(repo, page, per_page, cx.config());
        let id = cx.request(request);
        // The table asks for one page at a time, so nothing else is in flight; a stale one goes anyway.
        if let Some(old) = self.page.replace(PageRequest { id, generation, page }) {
            cx.effect(Effect::Cancel(old.id));
        }
    }

    /// Takes the response to the page in flight, and hands back any other. A page asked for before the last
    /// [`Self::begin`] was cancelled there, so if it still arrives it's handed back and dropped (Py bug B45).
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let Some(request) = self.page.filter(|page| page.id == id) else {
            return Some(response);
        };
        let result = match S::page_items(response) {
            Ok(result) => result,
            Err(other) => return Some(other),
        };
        self.page = None;
        let result = match result {
            Ok(page) => Ok(page),
            Err(error) => match S::page_failed(&error) {
                PageFailure::Retry => Err(error),
                PageFailure::End => Ok(Page {
                    items: Vec::new(),
                    last: true,
                }),
            },
        };
        let loaded = self.table.page_loaded(request.generation, request.page, result, &());
        if loaded.applied {
            self.save_cache(cx);
        }
        self.follow(loaded.event.as_ref(), cx);
        None
    }

    /// Requests the page a table event asked for, if it asked.
    fn follow(&mut self, event: Option<&TableEvent<<S::Item as TableItem>::Key>>, cx: &mut Cx<'_>) {
        if let Some(&TableEvent::LoadMore {
            generation,
            page,
            per_page,
        }) = event
        {
            self.load_more(generation, page, per_page, cx);
        }
    }
}

/// The repo's cache file for `S`, under the config's cache directory.
fn cache_file<S: ListSource>(repo: &Repository, config: &Config) -> PathBuf {
    cache::cache_path(
        &config.cache.cache_directory,
        CacheScope::Repo(&repo.full_name),
        S::CACHE_NAME,
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod cache_app_tests;
