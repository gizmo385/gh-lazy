//! A repo-scoped, lazily paged list section: the table of the Pull Requests, Issues and Workflows sections. Python
//! spreads this over `LazilyLoadedDataTable` and the `LazyGithubContainer` around it.
//!
//! The section owns the later pages. Its first page belongs to its owner, which requests it (PRs and issues share
//! theirs) and hands the result to [`ListSection::first_page`].

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::models::Repository;

use crate::component::Cx;
use crate::components::table::{Column, Page, SearchableTable, SortSpec, TableEvent, TableItem};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::options::AppOptions;

/// Items per page, and so the size of a first page: Python's `batch_size=30`.
pub(super) const PER_PAGE: u32 = 30;

/// What fills one kind of list section: its columns, its items and how it asks for a page of them.
pub(super) trait ListSource {
    /// The rows' items.
    type Item: TableItem<Ctx = ()>;

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
    compact: bool,
}

impl<S: ListSource> std::fmt::Debug for ListSection<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListSection")
            .field("rows", &self.table.row_count())
            .field("loading", &self.table.is_loading())
            .field("repo", &self.repo.as_ref().map(|repo| &repo.full_name))
            .field("page", &self.page)
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
    pub(super) fn repo(&self) -> Option<&Repository> {
        self.repo.as_ref()
    }

    /// A repo was selected: the part of #170's `load_repo` contract every list pane shares. The page in flight is
    /// cancelled, the rows clear (cursor and scroll back to 0, paging off) and the loading overlay shows. `compact`
    /// stays as it was until the first page lands, as in Python.
    ///
    /// The per-repo cache restore goes here (a later M2 issue).
    pub(super) fn begin(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        if let Some(page) = self.page.take() {
            cx.effect(Effect::Cancel(page.id));
        }
        self.table.clear();
        self.table.set_loading(true);
        self.repo = Some(repo.clone());
    }

    /// The first page arrived. Paging turns on first, as `lazy.rs` asks, so a short page asks for page 2 at once, as
    /// Python's first `RowHighlighted` does. A failure counts as an empty page; the owner logs it.
    ///
    /// The first page's [`Page::last`] isn't read: Python only ends paging on a later page, and a first page with no
    /// rows has no cursor to move.
    pub(super) fn first_page(&mut self, result: Result<Page<S::Item>, GhError>, cx: &mut Cx<'_>) {
        self.table.enable_lazy_load(PER_PAGE, 1);
        let items = result.map(|page| page.items).unwrap_or_default();
        let event = self.table.insert_many(items, &());
        self.follow(event.as_ref(), cx);
        self.table.set_loading(false);
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

#[cfg(test)]
mod tests;
