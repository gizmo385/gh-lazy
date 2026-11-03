//! The Search modal (`search.open`, and the palette's "Search issues and pull requests"): a query in GitHub's search
//! syntax, a scope, and the hits of `GET /search/issues` in a lazily paged table, up to GitHub's 1,000-result cap.
//! Choosing a hit closes the modal and opens it in Details, selecting its repo first (the main screen's
//! [`crate::screens::main::OpenInRepo`], shared with notifications). New behavior with no Python counterpart (decision
//! `search-issues-and-prs`).
//!
//! What outlives one open is a [`SearchSession`]: the text, the scope, the results with their cursor and paging, and
//! the status line. The main screen keeps it, the dialog takes it on open and hands it back when it closes, so going
//! back to the results after opening one is moving a struct, not searching again.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::github::{
    HitKind, SEARCH_MAX_REPOS, SEARCH_PER_PAGE, SearchError, SearchHit, SearchPage, SearchQuery, SearchScope,
};
use lazy_github_core::models::{IssueState, RepoName};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use tokio::time::Instant;

use super::frame::on_screen;
use super::{Dialog, DialogAction, DialogPane, FieldKind, FocusMove, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, Owner, RenderCx};
use crate::components::select::{self, Select, SelectOutcome};
use crate::components::table::{
    Cell, CellValue, Column, Handled, Page, SEARCHABLE_BINDINGS, SearchableTable, TableEvent, TableFocus, TableItem,
};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::options::AppOptions;

/// The heading, centered as the Notifications modal's.
const HEADING: &str = "Search issues and pull requests";

/// The rows above the scope: `MarkdownH1`'s two rows of margin above the heading and one below, as Notifications.
const HEADING_ROWS: u16 = 4;
const HEADING_MARGIN_TOP: u16 = 2;

/// The heading's side padding, inside which it's centered.
const HEADING_PADDING: u16 = 2;

/// The label left of the scope select, and the columns it takes with its gap.
const SCOPE_LABEL: &str = "Scope:";
const SCOPE_LABEL_WIDTH: u16 = 8;

/// The input's placeholder.
const PLACEHOLDER: &str = "is:pr is:open author:@me label:bug";

/// The dimmed line under the input, cut at the width.
const HINT: &str = "is:pr is:issue is:open is:merged author: assignee: label: review: involves: updated:>YYYY-MM-DD \
                    sort:updated-desc · AND OR ( ) · enter to search";

/// The status line before anything was searched.
const PROMPT: &str = "Type a query and press enter";
const SEARCHING: &str = "Searching…";
const NO_HITS: &str = "No issues or pull requests match";
const SCOPE_FROM_QUERY: &str = "Scope from the query";
const CAPPED: &str = "GitHub returns the first 1,000";
const INCOMPLETE: &str = "GitHub timed out, results may be incomplete";
const MORE_FAILED: &str = "loading more failed, move down to retry";

/// What a failed search logs at error; the error goes to debug.
const SEARCH_FAILED: &str = "Search failed";

/// The modal screen's bindings: only `modal.close` of its own.
pub(crate) const SEARCH_BINDINGS: &[BindingId] = dialog_bindings![ModalClose];

/// Where the modal was opened from, which picks the text of the first open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchOrigin {
    /// `search.open` on the Pull Requests table.
    PullRequests,
    /// `search.open` on the Issues table.
    Issues,
    /// The command palette's "Search issues and pull requests".
    Palette,
}

impl SearchOrigin {
    /// The text an open with an empty query starts with, cursor at its end.
    fn prefill(self) -> &'static str {
        match self {
            SearchOrigin::PullRequests => "is:pr is:open ",
            SearchOrigin::Issues => "is:issue is:open ",
            SearchOrigin::Palette => "",
        }
    }
}

/// The scopes the select offers (decision `search-issues-and-prs`, D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeChoice {
    /// The selected repo: `repo:<owner>/<name>`.
    Repo,
    /// Every repo of the selected repo's owner: `user:<owner>`.
    Owner,
    /// The favorites and tracked repos, the first [`SEARCH_MAX_REPOS`].
    Tracked,
    /// All of GitHub.
    All,
}

/// The table's column set, fixed for the life of one result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Columns {
    /// The "Repo" column, for a scope that can span repos.
    repo: bool,
}

impl Columns {
    fn headers(self) -> Vec<Column> {
        let mut headers = Vec::with_capacity(7);
        if self.repo {
            headers.push("Repo");
        }
        headers.extend(["#", "Type", "Status", "Author", "Title", "Updated"]);
        headers.into_iter().map(Column::new).collect()
    }
}

impl TableItem for SearchHit {
    // Hits from two repos can share a number.
    type Key = (RepoName, u64);
    type Ctx = Columns;

    fn key(&self) -> (RepoName, u64) {
        (self.repo.clone(), self.number)
    }

    fn cells(&self, columns: &Columns) -> Vec<Cell> {
        let mut cells = Vec::with_capacity(7);
        if columns.repo {
            cells.push(Cell::text(self.repo.to_string()));
        }
        // An issue number fits an `i64` by far; the digits drawn are the number's own either way.
        let number = i64::try_from(self.number).unwrap_or(i64::MAX);
        cells.push(Cell::new(CellValue::Int(number), self.number.to_string()));
        cells.push(Cell::text(kind_label(self.kind)));
        cells.push(Cell::text(status_label(self)));
        cells.push(Cell::text(self.author.clone()));
        cells.push(Cell::text(self.title.clone()));
        cells.push(Cell::text(self.updated_at.strftime("%Y-%m-%d").to_string()));
        cells
    }
}

fn kind_label(kind: HitKind) -> &'static str {
    match kind {
        HitKind::Issue => "Issue",
        HitKind::PullRequest { .. } => "PR",
    }
}

/// `open`, `closed`, `merged` or `draft`.
fn status_label(hit: &SearchHit) -> &'static str {
    match (hit.kind, hit.state) {
        (HitKind::PullRequest { merged: true, .. }, _) => "merged",
        (HitKind::PullRequest { draft: true, .. }, IssueState::Open) => "draft",
        (_, IssueState::Open) => "open",
        (_, IssueState::Closed) => "closed",
    }
}

/// `n` with commas between the thousands: `885,203`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What the status line says, apart from the scope's suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    /// Nothing searched yet.
    Prompt,
    /// Page 1 is out.
    Searching,
    /// The results: GitHub's count, whether it gave up early, and whether the last later page failed.
    Results {
        total: u64,
        incomplete: bool,
        more_failed: bool,
    },
    /// The search found nothing.
    NoHits,
    /// Why the search failed, in the error color.
    Error(String),
}

/// The status line's text and whether it's an error.
fn status_text(status: &Status) -> (String, bool) {
    match status {
        Status::Prompt => (PROMPT.to_owned(), false),
        Status::Searching => (SEARCHING.to_owned(), false),
        Status::NoHits => (NO_HITS.to_owned(), false),
        Status::Results {
            total,
            incomplete,
            more_failed,
        } => {
            let noun = if *total == 1 { "result" } else { "results" };
            let mut line = format!("{} {noun}", thousands(*total));
            if *total > u64::from(lazy_github_core::github::SEARCH_RESULT_CAP) {
                line.push_str(" · ");
                line.push_str(CAPPED);
            }
            if *incomplete {
                line.push_str(" · ");
                line.push_str(INCOMPLETE);
            }
            if *more_failed {
                line.push_str(" · ");
                line.push_str(MORE_FAILED);
            }
            (line, false)
        }
        Status::Error(message) => (message.clone(), true),
    }
}

/// Where focus was when the modal last closed, for the next open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Remembered {
    #[default]
    Input,
    Table,
}

/// The Search modal's state that outlives one open: the main screen keeps it for the session, nothing across
/// restarts.
pub struct SearchSession {
    input: TextInput,
    scope: Option<ScopeChoice>,
    table: SearchableTable<SearchHit>,
    columns: Columns,
    status: Status,
    /// The query the shown results came from, which later pages ask with.
    query: Option<SearchQuery>,
    /// A search was sent this session, so a scope change searches again.
    submitted: bool,
    focus: Remembered,
}

impl SearchSession {
    /// A session before its first open: no text, no results.
    pub fn new(options: AppOptions) -> Self {
        let columns = Columns::default();
        Self {
            input: TextInput::new(PLACEHOLDER),
            scope: None,
            table: SearchableTable::new(columns.headers(), None).with_options(options),
            columns,
            status: Status::Prompt,
            query: None,
            submitted: false,
            focus: Remembered::Input,
        }
    }

    /// The query text, as typed.
    pub fn text(&self) -> &str {
        self.input.value()
    }

    /// The hits shown, top to bottom, by repo and number.
    pub fn hit_keys(&self) -> Vec<(String, u64)> {
        self.table
            .row_keys()
            .map(|(repo, number)| (repo.to_string(), *number))
            .collect()
    }

    /// The results table's cursor row.
    pub fn cursor(&self) -> usize {
        self.table.cursor()
    }
}

impl std::fmt::Debug for SearchSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchSession")
            .field("text", &self.input.value())
            .field("scope", &self.scope)
            .field("columns", &self.columns)
            .field("status", &self.status)
            .field("rows", &self.table.row_count())
            .field("cursor", &self.table.cursor())
            .finish_non_exhaustive()
    }
}

/// What the modal closes with: the session to keep, and the hit chosen, if any.
#[derive(Debug)]
pub struct SearchOutcome {
    /// The session, for the next open.
    pub session: SearchSession,
    /// The hit to open, when one was chosen.
    pub hit: Option<Box<SearchHit>>,
}

/// The dialog's own actions: the parts that make requests or close it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchAction {
    /// `modal.close`: done, keeping the session.
    Close,
    /// `enter` in the input: search page 1.
    Submit,
    /// Another scope was picked: search again if a search was sent and there's text.
    Rescope,
    /// The table asked for `page` in its `generation`.
    LoadMore {
        /// The table's generation when it asked.
        generation: u64,
        /// The page, from 1.
        page: u32,
    },
    /// `common.table.select` (or a click on the cursor's row) on this hit: open it.
    Open((RepoName, u64)),
}

/// What has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Scope,
    /// The table, or its search box, whose focus the table keeps itself.
    Table,
}

/// One stop of the `tab` / `shift+tab` ring, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Input,
    Scope,
    Search,
    Table,
}

/// The scopes on offer, from the selected repo and the config when the modal opened.
#[derive(Debug, Clone, Default)]
struct Offered {
    /// The selected repo's `owner/name` and owner.
    repo: Option<(String, String)>,
    /// `repositories.favorites` then `repositories.additional_repos_to_track`, de-duplicated, every one of them.
    tracked: Vec<String>,
}

impl Offered {
    fn new(cx: &Cx<'_>) -> Self {
        let repo = cx
            .current_repo()
            .map(|repo| (repo.full_name.clone(), repo.owner.login.clone()));
        let settings = &cx.config().repositories;
        let mut tracked: Vec<String> = Vec::new();
        for name in settings.favorites.iter().chain(&settings.additional_repos_to_track) {
            if !tracked.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
                tracked.push(name.clone());
            }
        }
        Self { repo, tracked }
    }

    /// The select's options, in order.
    fn options(&self) -> Vec<(String, ScopeChoice)> {
        let mut options = Vec::with_capacity(4);
        if let Some((full_name, owner)) = &self.repo {
            options.push((format!("This repo ({full_name})"), ScopeChoice::Repo));
            options.push((format!("This owner ({owner})"), ScopeChoice::Owner));
        }
        if !self.tracked.is_empty() {
            options.push(("Favorites and tracked repos".to_owned(), ScopeChoice::Tracked));
        }
        options.push(("All of GitHub".to_owned(), ScopeChoice::All));
        options
    }

    /// `choice` if it's on offer, else the default: "This repo", or "All of GitHub" without a repo.
    fn pick(&self, choice: Option<ScopeChoice>) -> ScopeChoice {
        let offered = |choice: ScopeChoice| match choice {
            ScopeChoice::Repo | ScopeChoice::Owner => self.repo.is_some(),
            ScopeChoice::Tracked => !self.tracked.is_empty(),
            ScopeChoice::All => true,
        };
        match choice.filter(|&choice| offered(choice)) {
            Some(choice) => choice,
            None if self.repo.is_some() => ScopeChoice::Repo,
            None => ScopeChoice::All,
        }
    }

    /// The search scope `choice` stands for.
    fn scope(&self, choice: ScopeChoice) -> SearchScope {
        match (choice, &self.repo) {
            (ScopeChoice::Repo, Some((full_name, _))) => SearchScope::Repo(full_name.clone()),
            (ScopeChoice::Owner, Some((_, owner))) => SearchScope::Owner(owner.clone()),
            (ScopeChoice::Tracked, _) => {
                SearchScope::Repos(self.tracked.iter().take(SEARCH_MAX_REPOS).cloned().collect())
            }
            (ScopeChoice::All | ScopeChoice::Repo | ScopeChoice::Owner, _) => SearchScope::All,
        }
    }
}

/// Page 1 in flight.
#[derive(Debug)]
struct FirstPage {
    id: RequestId,
    query: SearchQuery,
    columns: Columns,
    /// The status before, put back if the request is cancelled.
    before: Status,
}

/// A later page in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PageRequest {
    id: RequestId,
    generation: u64,
    page: u32,
}

/// Where each part goes in the interior, top to bottom.
#[derive(Debug, Clone, Copy, Default)]
struct Areas {
    heading: Rect,
    scope_label: Rect,
    scope: Rect,
    input: Rect,
    hint: Rect,
    status: Rect,
    table: Rect,
}

impl Areas {
    fn new(area: Rect, select_width: u16) -> Self {
        let mut y = area.y;
        let mut next = |height: u16| {
            let rect = Rect { y, height, ..area }.intersection(area);
            y = y.saturating_add(height);
            rect
        };
        let heading_block = next(HEADING_ROWS);
        let heading = Rect {
            x: area.x + HEADING_PADDING.min(area.width),
            y: heading_block.y.saturating_add(HEADING_MARGIN_TOP),
            width: area.width.saturating_sub(2 * HEADING_PADDING),
            height: 1,
        }
        .intersection(area);
        let scope_row = next(select::HEIGHT);
        let scope_label = Rect {
            x: scope_row.x.saturating_add(1),
            y: scope_row.y.saturating_add(1),
            width: SCOPE_LABEL_WIDTH.min(scope_row.width),
            height: 1,
        }
        .intersection(scope_row);
        let left = SCOPE_LABEL_WIDTH.min(scope_row.width);
        let scope = Rect {
            x: scope_row.x + left,
            width: select_width.min(scope_row.width - left),
            ..scope_row
        };
        let input = next(text_input::HEIGHT);
        let hint = next(1);
        let status = next(1);
        let table = Rect {
            y,
            height: area.bottom().saturating_sub(y),
            ..area
        }
        .intersection(area);
        Self {
            heading,
            scope_label,
            scope,
            input,
            hint,
            status,
            table,
        }
    }
}

/// The Search modal. Its output is the session to keep and the hit chosen, if any.
pub struct SearchDialog {
    session: SearchSession,
    offered: Offered,
    select: Select<ScopeChoice>,
    focus: Focus,
    first: Option<FirstPage>,
    page: Option<PageRequest>,
    areas: Areas,
    options: AppOptions,
    output: Option<SearchOutcome>,
}

impl std::fmt::Debug for SearchDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchDialog")
            .field("session", &self.session)
            .field("focus", &self.focus)
            .field("first", &self.first)
            .field("page", &self.page)
            .finish_non_exhaustive()
    }
}

impl SearchDialog {
    /// The modal over `session`, opened from `origin`: an empty query takes the origin's text, the scope is the
    /// session's if it's still on offer, and focus goes to the table when the session left it there with results,
    /// else the input. Nothing is searched until `enter`.
    pub fn new(mut session: SearchSession, origin: SearchOrigin, cx: &Cx<'_>) -> Self {
        let offered = Offered::new(cx);
        if session.input.value().is_empty() {
            session.input.set_value(origin.prefill());
        }
        let choice = offered.pick(session.scope);
        session.scope = Some(choice);
        let select = Select::new(select::DEFAULT_PROMPT, offered.options(), Some(&choice)).without_blank();
        let focus = if session.focus == Remembered::Table && session.table.row_count() > 0 {
            Focus::Table
        } else {
            Focus::Input
        };
        let mut dialog = Self {
            session,
            offered,
            select,
            focus,
            first: None,
            page: None,
            areas: Areas::default(),
            options: cx.options(),
            output: None,
        };
        dialog.text_changed();
        dialog
    }

    fn action(action: SearchAction) -> Action {
        Action::Dialog(DialogAction::Search(action))
    }

    /// The query text, as typed.
    pub fn text(&self) -> &str {
        self.session.input.value()
    }

    /// The scope picked.
    pub fn scope(&self) -> Option<ScopeChoice> {
        self.select.value().copied()
    }

    /// The scope select's labels, in order.
    pub fn scope_labels(&self) -> Vec<String> {
        self.select.labels().map(str::to_owned).collect()
    }

    /// Whether the scope select is disabled, by a scope in the query.
    pub fn scope_disabled(&self) -> bool {
        self.select.is_disabled()
    }

    /// The status line, as drawn.
    pub fn status_line(&self) -> String {
        self.status().0
    }

    /// The hits shown, top to bottom, by repo and number.
    pub fn hit_keys(&self) -> Vec<(String, u64)> {
        self.session.hit_keys()
    }

    /// Whether the results table (or its search box) has focus.
    pub fn table_focused(&self) -> bool {
        self.focus == Focus::Table
    }

    /// The status line: the status, then the scope's suffix.
    fn status(&self) -> (String, bool) {
        let (mut line, error) = status_text(&self.session.status);
        if let Some(suffix) = self.scope_suffix() {
            line.push_str(" · ");
            line.push_str(&suffix);
        }
        (line, error)
    }

    /// "Scope from the query" while the text scopes itself; "first 20 of <n> repos" while the favorites and tracked
    /// repos are picked and some are cut.
    fn scope_suffix(&self) -> Option<String> {
        if self.select.is_disabled() {
            return Some(SCOPE_FROM_QUERY.to_owned());
        }
        let count = self.offered.tracked.len();
        (self.scope() == Some(ScopeChoice::Tracked) && count > SEARCH_MAX_REPOS)
            .then(|| format!("first {SEARCH_MAX_REPOS} of {count} repos"))
    }

    /// The text changed: a scope in it disables the select.
    fn text_changed(&mut self) {
        let scoped = SearchQuery::new(self.session.input.value(), SearchScope::All).scope_from_text();
        self.select.set_disabled(scoped);
        if scoped && self.focus == Focus::Scope {
            self.focus = Focus::Input;
        }
    }

    /// The ring's stop with focus now.
    fn stop(&self) -> Stop {
        match self.focus {
            Focus::Input => Stop::Input,
            Focus::Scope => Stop::Scope,
            Focus::Table if self.session.table.focus() == TableFocus::Search => Stop::Search,
            Focus::Table => Stop::Table,
        }
    }

    /// Moves focus to `stop`. The select's list closes when it loses focus; the table's search box blurs.
    fn focus_stop(&mut self, stop: Stop) {
        if stop != Stop::Scope {
            self.select.close();
        }
        let table = &mut self.session.table;
        if stop == Stop::Search {
            table.focus_search();
        } else {
            table.focus_table();
        }
        self.focus = match stop {
            Stop::Input => Focus::Input,
            Stop::Scope => Focus::Scope,
            Stop::Search | Stop::Table => Focus::Table,
        };
    }

    /// What a table's answer to an input means.
    fn table_outcome(handled: Handled<TableEvent<(RepoName, u64)>>) -> Option<Action> {
        match handled {
            Handled::Ignored => None,
            Handled::Consumed => Some(Action::Consumed),
            Handled::Event(TableEvent::Selected(key)) => Some(Self::action(SearchAction::Open(key))),
            Handled::Event(TableEvent::LoadMore { generation, page, .. }) => {
                Some(Self::action(SearchAction::LoadMore { generation, page }))
            }
            Handled::Event(TableEvent::Copied(text)) => Some(Action::Copy(text)),
        }
    }

    /// What the select's answer to an input means: a new pick searches again (if it should).
    fn select_outcome(outcome: SelectOutcome) -> Option<Action> {
        match outcome {
            SelectOutcome::Ignored => None,
            SelectOutcome::Changed => Some(Self::action(SearchAction::Rescope)),
            SelectOutcome::Opened | SelectOutcome::Closed | SelectOutcome::Consumed => Some(Action::Consumed),
        }
    }

    /// Searches page 1 with the text and scope, replacing any search in flight. With nothing to search for, nothing
    /// is sent and the status line says so.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        let choice = self.offered.pick(self.scope());
        let query = SearchQuery::new(self.session.input.value(), self.offered.scope(choice));
        if query.q().is_err() {
            self.session.status = Status::Error(SearchError::Empty.to_string());
            return;
        }
        let columns = Columns {
            repo: query.scope_from_text() || choice != ScopeChoice::Repo,
        };
        let before = self.cancel_first(cx).unwrap_or_else(|| self.session.status.clone());
        self.cancel_page(cx);
        let id = cx.request(GithubRequest::SearchIssues(query.clone()));
        self.first = Some(FirstPage {
            id,
            query,
            columns,
            before,
        });
        self.session.submitted = true;
        self.session.status = Status::Searching;
        self.session.table.set_loading(true);
    }

    /// Cancels page 1 in flight, if any: the overlay goes and the status it replaced, which it hands back.
    fn cancel_first(&mut self, cx: &mut Cx<'_>) -> Option<Status> {
        let first = self.first.take()?;
        cx.effect(Effect::Cancel(first.id));
        self.session.table.set_loading(false);
        Some(first.before)
    }

    /// Cancels the later page in flight, if any: the table asks for it again on the next cursor move.
    fn cancel_page(&mut self, cx: &mut Cx<'_>) {
        if let Some(page) = self.page.take() {
            cx.effect(Effect::Cancel(page.id));
            let _ = self.session.table.page_failed(page.generation, page.page);
        }
    }

    /// Page 1 answered.
    fn first_loaded(&mut self, first: FirstPage, result: Result<SearchPage, SearchError>, cx: &mut Cx<'_>) {
        self.session.table.set_loading(false);
        let page = match result {
            Ok(page) => page,
            Err(SearchError::PastLimit) => SearchPage {
                total_count: 0,
                incomplete_results: false,
                items: Vec::new(),
            },
            Err(error) => return self.first_failed(&error, cx),
        };
        let last = page.is_last(1, first.query.per_page);
        let SearchPage {
            total_count,
            incomplete_results,
            items,
        } = page;
        self.start_results(first.columns);
        let table = &mut self.session.table;
        table.enable_lazy_load(first.query.per_page, 0);
        let generation = table.generation();
        let _ = table.load_next_page();
        let loaded = table.page_loaded(generation, 1, Ok(Page { items, last }), &self.session.columns);
        self.session.query = Some(first.query);
        if self.session.table.row_count() == 0 {
            self.session.status = Status::NoHits;
        } else {
            self.session.status = Status::Results {
                total: total_count,
                incomplete: incomplete_results,
                more_failed: false,
            };
            self.focus_stop(Stop::Table);
        }
        if let Some(TableEvent::LoadMore { generation, page, .. }) = loaded.event {
            self.request_page(generation, page, cx);
        }
    }

    /// An empty table for a new result set, in `columns`: the same table cleared while the column set is the same,
    /// which keeps its column widths, else a new one.
    fn start_results(&mut self, columns: Columns) {
        if columns == self.session.columns {
            self.session.table.clear();
        } else {
            self.session.table = SearchableTable::new(columns.headers(), None).with_options(self.options);
            self.session.columns = columns;
        }
    }

    /// Page 1 failed. A query GitHub couldn't read clears the results; anything else leaves them.
    fn first_failed(&mut self, error: &SearchError, cx: &mut Cx<'_>) {
        let message = match error {
            SearchError::Invalid { .. } => {
                self.session.table.clear();
                self.session.query = None;
                error.to_string()
            }
            SearchError::Empty | SearchError::PastLimit => error.to_string(),
            SearchError::Gh(gh) => {
                if let Some(limit) = gh.rate_limit() {
                    let now = cx.local_now();
                    let at = limit
                        .reset(now.timestamp())
                        .to_zoned(now.time_zone().clone())
                        .strftime("%H:%M");
                    format!("Search rate limit reached. Try again at {at}")
                } else {
                    tracing::error!("{SEARCH_FAILED}");
                    tracing::debug!("Searching failed: {gh}");
                    format!("{SEARCH_FAILED}: {gh}")
                }
            }
        };
        self.session.status = Status::Error(message);
    }

    /// Asks for `page` of the shown results, as the table asked in `generation`. An ask from an older result set is
    /// dropped; one while page 1 of a new search is out is given back to the table, which asks again on the next move.
    fn request_page(&mut self, generation: u64, page: u32, cx: &mut Cx<'_>) {
        let table = &mut self.session.table;
        if generation != table.generation() {
            tracing::debug!(generation, page, "dropping a search page request for older results");
            return;
        }
        let (Some(query), None) = (&self.session.query, &self.first) else {
            let _ = table.page_failed(generation, page);
            return;
        };
        let id = cx.request(GithubRequest::SearchIssues(SearchQuery { page, ..query.clone() }));
        self.page = Some(PageRequest { id, generation, page });
    }

    /// A later page answered: its hits join the table. Past the cap is the end of the results; any other failure
    /// keeps the rows and the next cursor move asks again.
    fn page_loaded(&mut self, request: PageRequest, result: Result<SearchPage, SearchError>, cx: &mut Cx<'_>) {
        let per_page = self
            .session
            .query
            .as_ref()
            .map_or(SEARCH_PER_PAGE, |query| query.per_page);
        let page = match result {
            Ok(page) => Page {
                last: page.is_last(request.page, per_page),
                items: page.items,
            },
            Err(SearchError::PastLimit) => Page {
                items: Vec::new(),
                last: true,
            },
            Err(error) => {
                tracing::debug!(page = request.page, "Loading more search results failed: {error}");
                let _ = self.session.table.page_failed(request.generation, request.page);
                self.set_more_failed(true);
                return;
            }
        };
        self.set_more_failed(false);
        let loaded = self
            .session
            .table
            .page_loaded(request.generation, request.page, Ok(page), &self.session.columns);
        if let Some(TableEvent::LoadMore { generation, page, .. }) = loaded.event {
            self.request_page(generation, page, cx);
        }
    }

    fn set_more_failed(&mut self, failed: bool) {
        if let Status::Results { more_failed, .. } = &mut self.session.status {
            *more_failed = failed;
        }
    }

    /// Done: whatever is in flight is cancelled, and the session goes back with `hit`.
    fn close(&mut self, hit: Option<Box<SearchHit>>, cx: &mut Cx<'_>) {
        if self.output.is_some() {
            return;
        }
        if let Some(before) = self.cancel_first(cx) {
            self.session.status = before;
        }
        self.cancel_page(cx);
        self.select.close();
        self.session.scope = self.scope();
        self.session.focus = if self.focus == Focus::Table {
            Remembered::Table
        } else {
            Remembered::Input
        };
        let session = std::mem::replace(&mut self.session, SearchSession::new(self.options));
        self.output = Some(SearchOutcome { session, hit });
    }

    /// The select's width: its longest label, its border and padding, and the arrow.
    fn select_width(&self) -> u16 {
        let longest = self
            .select
            .labels()
            .map(|label| label.chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + 8).unwrap_or(u16::MAX)
    }
}

impl Component for SearchDialog {
    type Action = SearchAction;

    fn bindings(&self) -> &'static [BindingId] {
        SEARCH_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalClose => Some(Self::action(SearchAction::Close)),
            BindingId::TableSearch => {
                self.session.table.open_search();
                self.focus = Focus::Table;
                Some(Action::Consumed)
            }
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match self.focus {
            Focus::Input => self.session.input.consumes_key(key, keymap),
            Focus::Scope => self.select.consumes_key(key, keymap),
            Focus::Table => self.session.table.consumes_key(key, keymap),
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        match self.stop() {
            Stop::Input => {
                let before = self.session.input.value().to_owned();
                let outcome = self.session.input.handle_key(&chord, cx.keymap(), cx.clipboard());
                if self.session.input.value() != before {
                    self.text_changed();
                }
                match outcome {
                    InputOutcome::Submitted => Some(Self::action(SearchAction::Submit)),
                    InputOutcome::Changed => Some(Action::Consumed),
                    InputOutcome::Copied(text) => Some(Action::Copy(text)),
                    InputOutcome::Ignored => None,
                }
            }
            Stop::Scope => Self::select_outcome(self.select.handle_key(&chord, cx.keymap(), cx.now())),
            // Only the search box takes keys outside the chain.
            Stop::Search => {
                let handled = self.session.table.handle_key(&chord, cx.keymap(), cx.clipboard());
                Some(Self::table_outcome(handled).unwrap_or(Action::Consumed))
            }
            Stop::Table => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        // A drag that started on the input or the table's search box, wherever the pointer is now.
        if self.session.input.captured_mouse(event) {
            return Some(Action::Consumed);
        }
        if self.session.table.search_captures_mouse() {
            self.session.table.search_captured_mouse(event);
            return Some(Action::Consumed);
        }
        let areas = self.areas;
        let at = Position::new(event.column, event.row);
        let press = event.kind == MouseEventKind::Down(MouseButton::Left);
        let on_box = areas.scope.contains(at);
        if self.select.is_open() {
            if self.select.overlay_contains(event.column, event.row) {
                return Self::select_outcome(self.select.handle_overlay_mouse(&event)).or(Some(Action::Consumed));
            }
            // A press elsewhere takes the focus from the list, which closes it; a press on the box toggles it.
            if press && !on_box {
                self.select.close();
            }
        }
        if !areas.table.contains(at) && event.kind == MouseEventKind::Moved {
            let _ = self.session.table.handle_mouse(event, areas.table);
        }
        if areas.table.contains(at) {
            let on_rows = self.session.table.drawn_table_rect(areas.table).contains(at);
            let handled = self.session.table.handle_mouse(event, areas.table);
            if press && (on_rows || self.session.table.focus() == TableFocus::Search) {
                self.select.close();
                self.focus = Focus::Table;
            }
            return Some(Self::table_outcome(handled).unwrap_or(Action::Consumed));
        }
        if !press {
            return Some(Action::Consumed);
        }
        if on_box && !self.select.is_disabled() {
            self.focus_stop(Stop::Scope);
            let _ = self.select.toggle();
        } else if areas.input.contains(at) {
            self.focus_stop(Stop::Input);
            self.session.input.press(areas.input, event.column);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: SearchAction, cx: &mut Cx<'_>) {
        match action {
            SearchAction::Close => self.close(None, cx),
            SearchAction::Submit => self.submit(cx),
            SearchAction::Rescope => {
                if self.session.submitted && !self.session.input.value().trim().is_empty() {
                    self.submit(cx);
                }
            }
            SearchAction::LoadMore { generation, page } => self.request_page(generation, page, cx),
            SearchAction::Open(key) => {
                if let Some(hit) = self.session.table.get(&key).cloned() {
                    self.close(Some(Box::new(hit)), cx);
                }
            }
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let r = theme.roles();
        let under = self.background(r).over(theme.background());
        let areas = Areas::new(area, self.select_width());
        self.areas = areas;
        let screen = frame.area();

        {
            let buf = frame.buffer_mut();
            // `MarkdownH1`: centered, in `$markdown-h1-color` and its text style.
            let h1 = r.markdown.h1;
            let heading_style = h1.style.apply(Style::new().fg(theme.color(h1.fg, under)));
            let heading = on_screen(areas.heading, screen);
            let width = usize::from(heading.width);
            buf.set_stringn(heading.x, heading.y, format!("{HEADING:^width$}"), width, heading_style);

            let bold = Style::new()
                .fg(theme.color(r.text.plain, under))
                .add_modifier(Modifier::BOLD);
            let label = on_screen(areas.scope_label, screen);
            buf.set_span(label.x, label.y, &Span::styled(SCOPE_LABEL, bold), label.width);

            let select_focused = self.focus == Focus::Scope && !self.select.is_open();
            self.select.render_box(buf, areas.scope, theme, under, select_focused);
            self.session
                .input
                .render(buf, areas.input, theme, under, self.focus == Focus::Input);

            let muted = Style::new().fg(theme.color(r.text.muted, under));
            let hint = on_screen(areas.hint, screen);
            buf.set_stringn(hint.x, hint.y, HINT, usize::from(hint.width), muted);

            let (line, error) = self.status();
            let style = if error {
                Style::new().fg(theme.color(r.text.error, under))
            } else {
                muted
            };
            let status = on_screen(areas.status, screen);
            buf.set_stringn(status.x, status.y, line, usize::from(status.width), style);
        }

        let focused = self.focus == Focus::Table;
        let table_cx = RenderCx {
            focused,
            focus_within: focused,
            ..*cx
        };
        self.session.table.render(frame, areas.table, &table_cx);
        // The open list last, over everything below it.
        self.select
            .render_overlay(frame.buffer_mut(), areas.scope, screen, theme, under);
    }
}

impl Dialog for SearchDialog {
    type Output = SearchOutcome;

    fn kind(&self) -> ModalKind {
        ModalKind::Search
    }

    fn own_action(&self, action: DialogAction) -> Option<SearchAction> {
        match action {
            DialogAction::Search(action) => Some(action),
            _ => None,
        }
    }

    /// The ring: the input, the scope select unless a scope in the query disables it, the table's search box while
    /// it's shown, and the table.
    fn move_focus(&mut self, direction: FocusMove) {
        let mut stops = vec![Stop::Input];
        if !self.select.is_disabled() {
            stops.push(Stop::Scope);
        }
        if self.session.table.search_visible() {
            stops.push(Stop::Search);
        }
        stops.push(Stop::Table);
        let len = stops.len();
        let current = stops.iter().position(|&stop| stop == self.stop()).unwrap_or(0);
        let next = match direction {
            FocusMove::Next => (current + 1) % len,
            FocusMove::Previous => (current + len - 1) % len,
        };
        self.focus_stop(stops[next]);
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        // 80% of the screen each way, as the Notifications modal.
        (available.width * 4 / 5, available.height * 4 / 5)
    }

    fn take_output(&mut self) -> Option<SearchOutcome> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let kind = self.kind();
        let pane = ChainLink::new(
            Owner::DialogPane(kind, DialogPane::SearchableTable),
            SEARCHABLE_BINDINGS,
        );
        match self.stop() {
            Stop::Input => vec![FieldKind::TextInput.link(kind)],
            Stop::Scope if self.select.is_open() => vec![FieldKind::SelectOverlay.link(kind)],
            Stop::Scope => vec![FieldKind::Select.link(kind)],
            Stop::Search => vec![FieldKind::TextInput.link(kind), pane],
            Stop::Table => vec![FieldKind::Table.link(kind), pane],
        }
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match field {
            FieldKind::Table => Self::table_outcome(self.session.table.on_binding(id)),
            FieldKind::Select | FieldKind::SelectOverlay => Self::select_outcome(self.select.on_binding(id)),
            // `handle_key` already gave the input and the search box every key they bind.
            FieldKind::TextInput => Some(Action::Consumed),
            FieldKind::TextEditor
            | FieldKind::Button
            | FieldKind::Switch
            | FieldKind::Scroll
            | FieldKind::SelectionList
            | FieldKind::Tabs
            | FieldKind::KeyList
            | FieldKind::KeyCapture
            | FieldKind::ListEditor
            | FieldKind::ListEditorInput => None,
        }
    }

    fn captures_mouse(&self) -> bool {
        self.session.input.is_selecting() || self.session.table.search_captures_mouse()
    }

    fn popup_contains(&self, column: u16, row: u16) -> bool {
        self.select.overlay_contains(column, row)
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        match self.focus {
            Focus::Input => {
                self.session.input.handle_paste(text);
                self.text_changed();
                Some(Action::Consumed)
            }
            Focus::Table => Self::table_outcome(self.session.table.handle_paste(text)),
            Focus::Scope => None,
        }
    }

    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.session.table.next_deadline(now)
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let GithubResponse::SearchIssues(result) = response else {
            return Some(response);
        };
        if let Some(first) = self.first.take_if(|first| first.id == id) {
            self.first_loaded(first, result, cx);
            return None;
        }
        if let Some(page) = self.page.take_if(|page| page.id == id) {
            self.page_loaded(page, result, cx);
            return None;
        }
        Some(GithubResponse::SearchIssues(result))
    }
}

#[cfg(test)]
mod tests;
