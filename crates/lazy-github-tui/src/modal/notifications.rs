//! The Notifications modal (`notifications.open`): the "Notifications" heading over two tabs, "•Unread" and "✓Read",
//! each a lazily paged table of notification threads. On Unread, `R` marks the highlighted thread read and `ctrl+r`
//! marks them all read; Enter on a row in either tab chooses it, which closes the modal with it (marking it read
//! first when `notifications.mark_notification_as_read_when_selected` says so). Python's `NotificationsModal` around
//! a `NotificationsContainer`.
//!
//! One `all=true` first page fills both tabs, split by `unread`. Each tab then pages on its own from page 1 again, as
//! Python's `current_batch` starts at 0 although the first page is already shown: Unread asks for `all=false`
//! pages, Read for `all=true` pages filtered to the read threads.
//!
//! Nothing is cached: Python writes `notifications.json` and never reads it (Py bug B29).

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::NotificationQuery;
use lazy_github_core::models::Notification;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use std::collections::HashMap;
use tokio::time::Instant;

use super::{Dialog, DialogAction, DialogPane, FieldKind, FocusMove, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, Owner, RenderCx};
use crate::components::table::{
    Cell, CellValue, Column, Handled, Page, SEARCHABLE_BINDINGS, SearchableTable, SortSpec, TableEvent, TableFocus,
    TableItem, py_strip,
};
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::effect::{GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::Accent;
use crate::toast::Toast;

/// Threads per page: Python's `NOTIFICATIONS_PAGE_COUNT`, which `gh` sends as `per_page`.
const PER_PAGE: u32 = 50;

/// The heading: `Markdown("# Notifications")`.
const HEADING: &str = "Notifications";

/// The rows above the tab strip: `MarkdownH1`'s two rows of margin above the heading and one below.
const HEADING_ROWS: u16 = 4;
const HEADING_MARGIN_TOP: u16 = 2;

/// The `Markdown` widget's `padding: 0 2`, inside which the heading is centered.
const HEADING_PADDING: u16 = 2;

/// What every failed list logs: Python's `fetch_notifications` `lg.exception`.
const FETCH_FAILED: &str = "Failed to retrieve notifications from the Github API";

/// What a failed `R` logs and toasts (Py bug B81's recommendation).
const MARK_FAILED: &str = "Failed to mark notification as read";

/// What a failed `ctrl+r` logs and toasts (Py bug B82's recommendation).
const MARK_ALL_FAILED: &str = "Failed to mark all notifications as read";

/// Python's `notify` after marking everything read.
const MARKED_ALL: &str = "Marked all notifications as read";

/// The modal screen's bindings: only `modal.close` of its own.
const NOTIFICATIONS_BINDINGS: &[BindingId] = dialog_bindings![ModalClose];

/// `UnreadNotificationTabPane.BINDINGS`, while the Unread table or its search box has focus.
const UNREAD_BINDINGS: &[BindingId] = &[BindingId::NotificationsMarkRead, BindingId::NotificationsMarkAllAsRead];

/// `NotificationsContainer.BINDINGS`, wherever focus is in the modal.
const CONTAINER_BINDINGS: &[BindingId] = &[BindingId::NotificationsViewRead, BindingId::NotificationsViewUnread];

/// The two tabs, in strip order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationsTab {
    /// "•Unread": the unread threads.
    Unread,
    /// "✓Read": the read ones.
    Read,
}

impl NotificationsTab {
    fn index(self) -> usize {
        match self {
            NotificationsTab::Unread => 0,
            NotificationsTab::Read => 1,
        }
    }

    fn from_index(index: usize) -> Self {
        if index == 0 {
            NotificationsTab::Unread
        } else {
            NotificationsTab::Read
        }
    }

    /// The `all` value its later pages ask with.
    fn lists_all(self) -> bool {
        self == NotificationsTab::Read
    }
}

/// The dialog's own actions: the parts that make requests or close it. Focus and tab changes happen where the key or
/// click is taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationsAction {
    /// `modal.close`: done, with nothing chosen.
    Close,
    /// A tab's table asked for `page`, in its `generation`.
    LoadMore {
        /// Whose table asked.
        tab: NotificationsTab,
        /// The table's generation when it asked.
        generation: u64,
        /// The page, from 1.
        page: u32,
    },
    /// `notifications.mark_read` on the Unread thread with this id.
    MarkRead(u64),
    /// `notifications.mark_all_as_read`.
    MarkAllRead,
    /// `common.table.select` (or a click on the cursor's row) on this thread in `tab`: choose it.
    Select {
        /// Whose table it was chosen in.
        tab: NotificationsTab,
        /// The thread's id.
        thread: u64,
    },
}

/// What has focus: Textual's focus chain of `NotificationsContainer`. The active tab's table counts its search box,
/// whose focus the table keeps itself ([`SearchableTable::focus`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The `ScrollableContainer` around the tabs: focused while the first page loads.
    Container,
    /// The tab strip.
    Tabs,
    /// The active tab's table, or its search box.
    Table,
}

/// One stop of the `tab` / `shift+tab` ring, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Container,
    Tabs,
    Search,
    Table,
}

/// One row: a notification thread as it was fetched. Python's `notification_to_row`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NotificationRow(Notification);

impl TableItem for NotificationRow {
    type Key = u64;
    type Ctx = ();

    fn key(&self) -> u64 {
        self.0.id
    }

    fn cells(&self, (): &()) -> Vec<Cell> {
        let notification = &self.0;
        let updated = TimeZone::UTC.to_datetime(notification.updated_at);
        // GitHub's thread ids fit an `i64` by far; the digits drawn are the id's own either way.
        let id = i64::try_from(notification.id).unwrap_or(i64::MAX);
        vec![
            Cell::new(CellValue::DateTime(updated), py_datetime_str(updated)),
            Cell::text(notification.subject.kind.clone()),
            Cell::text(py_strip(&notification.subject.title)),
            Cell::text(py_title(&notification.reason.replace('_', " "))),
            Cell::new(CellValue::Int(id), notification.id.to_string()),
        ]
    }
}

/// Python's `str(datetime)` of a naive datetime: `2026-10-24 23:00:00`, with `.ffffff` when there are microseconds.
fn py_datetime_str(at: DateTime) -> String {
    let seconds = at.strftime("%Y-%m-%d %H:%M:%S").to_string();
    match at.subsec_nanosecond() / 1000 {
        0 => seconds,
        micros => format!("{seconds}.{micros:06}"),
    }
}

/// Python's `str.title()`: a cased character right after an uncased one (or at the start) is title-cased, every
/// other cased character lowercased. `review requested` → `Review Requested`, `ci activity` → `Ci Activity`.
///
/// Title case is the uppercase mapping with any further characters lowercased (`ß` → `Ss`), and the Latin digraphs
/// that have their own title-case letter (`ǆ` → `ǅ`) map to it.
fn py_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_cased = false;
    for c in text.chars() {
        if previous_cased {
            out.extend(c.to_lowercase());
        } else if let Some(title) = digraph_title(c) {
            out.push(title);
        } else {
            let mut upper = c.to_uppercase();
            out.extend(upper.next());
            out.extend(upper.flat_map(char::to_lowercase));
        }
        previous_cased = is_cased(c);
    }
    out
}

/// The title-case letter of a Latin digraph (`Ǆ`, `ǅ` and `ǆ` all title-case to `ǅ`).
fn digraph_title(c: char) -> Option<char> {
    Some(match c {
        '\u{01C4}'..='\u{01C6}' => '\u{01C5}',
        '\u{01C7}'..='\u{01C9}' => '\u{01C8}',
        '\u{01CA}'..='\u{01CC}' => '\u{01CB}',
        '\u{01F1}'..='\u{01F3}' => '\u{01F2}',
        _ => return None,
    })
}

/// Python's cased characters: lowercase, uppercase or title-case letters.
fn is_cased(c: char) -> bool {
    c.is_lowercase() || c.is_uppercase() || digraph_title(c) == Some(c)
}

/// A tab's later page in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PageRequest {
    id: RequestId,
    /// The table's generation when it asked.
    generation: u64,
    page: u32,
}

/// Where the last draw put the strip and the active table.
#[derive(Debug, Clone, Copy, Default)]
struct Areas {
    tabs: Rect,
    table: Rect,
}

/// The Notifications modal. Its output is the notification chosen there, or `None` when it closed without one.
pub struct NotificationsDialog {
    tabs: TabStrip,
    active: NotificationsTab,
    /// The two tables, by [`NotificationsTab::index`].
    tables: [SearchableTable<NotificationRow>; 2],
    focus: Focus,
    /// The first page, until it answers.
    first_page: Option<RequestId>,
    /// Each tab's later page in flight, by [`NotificationsTab::index`].
    pages: [Option<PageRequest>; 2],
    /// The `R` requests in flight, with the thread each marks.
    marking: HashMap<RequestId, u64>,
    /// The `ctrl+r` request in flight.
    marking_all: Option<RequestId>,
    /// The chosen notification, while its mark-on-select request is out. The modal closes with it once GitHub
    /// answers, either way.
    choosing: Option<(RequestId, Box<Notification>)>,
    areas: Areas,
    // `Some(None)` is a close with nothing chosen, as `take_output` hands it on.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<Notification>>>,
}

impl std::fmt::Debug for NotificationsDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationsDialog")
            .field("active", &self.active)
            .field("focus", &self.focus)
            .field("unread", &self.tables[0].row_count())
            .field("read", &self.tables[1].row_count())
            .finish_non_exhaustive()
    }
}

impl NotificationsDialog {
    /// The modal, loading: both tables under the loading overlay, focus on the container, and the first page asked
    /// for at once, as Python's `on_mount` does.
    pub fn new(cx: &mut Cx<'_>) -> Self {
        let options = cx.options();
        let table = || {
            let columns = ["Updated At", "Subject", "Title", "Reason", "Thread ID"]
                .map(Column::new)
                .into();
            let mut table = SearchableTable::new(columns, SortSpec::descending(0)).with_options(options);
            table.set_loading(true);
            table
        };
        let first_page = cx.request(GithubRequest::ListNotifications(NotificationQuery {
            all: true,
            page: 1,
        }));
        Self {
            tabs: TabStrip::accented([("•Unread", Accent::Red), ("✓Read", Accent::GreenYellow)]),
            active: NotificationsTab::Unread,
            tables: [table(), table()],
            focus: Focus::Container,
            first_page: Some(first_page),
            pages: [None, None],
            marking: HashMap::new(),
            marking_all: None,
            choosing: None,
            areas: Areas::default(),
            output: None,
        }
    }

    fn action(action: NotificationsAction) -> Action {
        Action::Dialog(DialogAction::Notifications(action))
    }

    fn table(&self, tab: NotificationsTab) -> &SearchableTable<NotificationRow> {
        &self.tables[tab.index()]
    }

    fn table_mut(&mut self, tab: NotificationsTab) -> &mut SearchableTable<NotificationRow> {
        &mut self.tables[tab.index()]
    }

    fn active_table(&self) -> &SearchableTable<NotificationRow> {
        self.table(self.active)
    }

    fn active_table_mut(&mut self) -> &mut SearchableTable<NotificationRow> {
        self.table_mut(self.active)
    }

    /// The active tab.
    pub fn active_tab(&self) -> NotificationsTab {
        self.active
    }

    /// The thread ids a tab shows, top to bottom.
    pub fn thread_ids(&self, tab: NotificationsTab) -> Vec<u64> {
        self.table(tab).row_keys().copied().collect()
    }

    /// The ring's stop with focus now.
    fn stop(&self) -> Stop {
        match self.focus {
            Focus::Container => Stop::Container,
            Focus::Tabs => Stop::Tabs,
            Focus::Table if self.active_table().focus() == TableFocus::Search => Stop::Search,
            Focus::Table => Stop::Table,
        }
    }

    /// Moves focus to `stop`. Leaving the search box blurs it, which hides it when it's blank.
    fn focus_stop(&mut self, stop: Stop) {
        let table = self.active_table_mut();
        if stop == Stop::Search {
            table.focus_search();
        } else {
            table.focus_table();
        }
        self.focus = match stop {
            Stop::Container => Focus::Container,
            Stop::Tabs => Focus::Tabs,
            Stop::Search | Stop::Table => Focus::Table,
        };
    }

    /// Makes `tab` the active one. Focus stays where it is; the old tab's search box blurs.
    fn switch_to(&mut self, tab: NotificationsTab) {
        if tab == self.active {
            return;
        }
        self.active_table_mut().focus_table();
        self.active = tab;
        self.tabs.select(tab.index());
    }

    /// `notifications.view_read` / `view_unread`: Python's `action_view_read` and `action_view_unread`, which
    /// activate the tab and focus its `DataTable`.
    fn view(&mut self, tab: NotificationsTab) {
        self.switch_to(tab);
        self.focus_stop(Stop::Table);
    }

    /// What a table's answer to an input means.
    fn table_outcome(tab: NotificationsTab, handled: Handled<TableEvent<u64>>) -> Option<Action> {
        match handled {
            Handled::Ignored => None,
            Handled::Consumed => Some(Action::Consumed),
            Handled::Event(TableEvent::Selected(thread)) => {
                Some(Self::action(NotificationsAction::Select { tab, thread }))
            }
            Handled::Event(TableEvent::LoadMore { generation, page, .. }) => {
                Some(Self::action(NotificationsAction::LoadMore { tab, generation, page }))
            }
            Handled::Event(TableEvent::Copied(text)) => Some(Action::Copy(text)),
        }
    }

    /// Requests `page` for `tab`, as its table asked in `generation`. An ask from before the table was last cleared
    /// is dropped.
    fn request_page(&mut self, tab: NotificationsTab, generation: u64, page: u32, cx: &mut Cx<'_>) {
        if generation != self.table(tab).generation() {
            tracing::debug!(?tab, generation, page, "dropping a page request for a cleared table");
            return;
        }
        let query = NotificationQuery {
            all: tab.lists_all(),
            page,
        };
        let id = cx.request(GithubRequest::ListNotifications(query));
        self.pages[tab.index()] = Some(PageRequest { id, generation, page });
    }

    /// Requests the page a table event asked for, if it asked.
    fn follow(&mut self, tab: NotificationsTab, event: Option<&TableEvent<u64>>, cx: &mut Cx<'_>) {
        if let Some(&TableEvent::LoadMore { generation, page, .. }) = event {
            self.request_page(tab, generation, page, cx);
        }
    }

    /// Python's `load_notifications`: the first page goes into the tabs by `unread`, the overlays clear, and the
    /// modal shows Unread when anything is unread, else Read, with that tab's table focused. A failure is logged and
    /// leaves both tabs empty, as Python's fetcher returns `[]`.
    fn first_page_loaded(&mut self, result: Result<Vec<Notification>, GhError>, cx: &mut Cx<'_>) {
        let notifications = result.unwrap_or_else(|error| {
            tracing::error!("{FETCH_FAILED}");
            tracing::debug!("Listing notifications failed: {error}");
            Vec::new()
        });
        let (unread, read): (Vec<_>, Vec<_>) = notifications
            .into_iter()
            .map(NotificationRow)
            .partition(|row| row.0.unread);
        let any_unread = !unread.is_empty();
        // Paging is on before the rows go in, so a short tab asks for its page 1 at once, as Python's first
        // `RowHighlighted` does with `current_batch` still 0. Unread asks first, as Python adds its rows first.
        for (tab, rows) in [(NotificationsTab::Unread, unread), (NotificationsTab::Read, read)] {
            let table = self.table_mut(tab);
            table.set_loading(false);
            table.enable_lazy_load(PER_PAGE, 0);
            let event = table.insert_many(rows, &());
            self.follow(tab, event.as_ref(), cx);
        }
        self.view(if any_unread {
            NotificationsTab::Unread
        } else {
            NotificationsTab::Read
        });
    }

    /// A tab's later page arrived. Read keeps only the read threads, and its paging ends on an empty API page, not
    /// an empty filtered one (Py bug B44's recommendation). A failure is logged and ends that tab's paging, as
    /// Python's fetcher returns `[]`.
    fn page_loaded(
        &mut self,
        tab: NotificationsTab,
        request: PageRequest,
        result: Result<Vec<Notification>, GhError>,
        cx: &mut Cx<'_>,
    ) {
        let page = match result {
            Ok(notifications) => {
                let last = notifications.is_empty();
                let items = notifications
                    .into_iter()
                    .filter(|notification| tab == NotificationsTab::Unread || !notification.unread)
                    .map(NotificationRow)
                    .collect();
                Page { items, last }
            }
            Err(error) => {
                tracing::error!("{FETCH_FAILED}");
                tracing::debug!(?tab, page = request.page, "Listing notifications failed: {error}");
                Page {
                    items: Vec::new(),
                    last: true,
                }
            }
        };
        let loaded = self
            .table_mut(tab)
            .page_loaded(request.generation, request.page, Ok(page), &());
        self.follow(tab, loaded.event.as_ref(), cx);
    }

    /// `R`'s answer. On success the thread leaves Unread for good (Py bug B84: its item goes too, so no later search
    /// brings it back) and joins Read, still `unread` as fetched. On failure it stays and the error is shown (Py bug
    /// B81's recommendation).
    fn marked_read(&mut self, thread: u64, result: Result<(), GhError>, cx: &mut Cx<'_>) {
        if let Err(error) = result {
            tracing::error!("{MARK_FAILED}");
            tracing::debug!(thread, "Marking the notification read failed: {error}");
            cx.notify(Toast::error(MARK_FAILED));
            return;
        }
        // Gone already when a mark all answered first.
        let Some(row) = self.table_mut(NotificationsTab::Unread).remove(&thread) else {
            return;
        };
        let event = self.table_mut(NotificationsTab::Read).insert(row, &());
        self.follow(NotificationsTab::Read, event.as_ref(), cx);
    }

    /// `ctrl+r`'s answer. On success every Unread thread, those a search hides included, joins Read; Unread empties
    /// and stops paging; Read becomes active with its table focused. On failure nothing changes and the error is
    /// shown (Py bug B82's recommendation).
    fn marked_all_read(&mut self, result: Result<(), GhError>, cx: &mut Cx<'_>) {
        if let Err(error) = result {
            tracing::error!("{MARK_ALL_FAILED}");
            tracing::debug!("Marking every notification read failed: {error}");
            cx.notify(Toast::error(MARK_ALL_FAILED));
            return;
        }
        cx.notify(Toast::info(MARKED_ALL));
        let unread = self.table_mut(NotificationsTab::Unread);
        let rows: Vec<NotificationRow> = unread.items().cloned().collect();
        // Python's `clear_rows`, which drops the load function: Unread never pages again, and a page in flight is
        // dropped by its generation.
        unread.clear();
        self.pages[NotificationsTab::Unread.index()] = None;
        let event = self.table_mut(NotificationsTab::Read).insert_many(rows, &());
        self.follow(NotificationsTab::Read, event.as_ref(), cx);
        self.view(NotificationsTab::Read);
    }

    /// Python's `handle_notification_selected`: a thread unread as fetched is marked read first when the setting
    /// says so, and the modal stays open until GitHub answers; any other closes with it at once. A thread `R` moved
    /// to Read still counts as unread here, as Python never flips the flag. While a mark is out, another choice is
    /// ignored.
    fn choose(&mut self, tab: NotificationsTab, thread: u64, cx: &mut Cx<'_>) {
        if self.choosing.is_some() || self.output.is_some() {
            return;
        }
        let Some(row) = self.table(tab).get(&thread) else {
            return;
        };
        let notification = Box::new(row.0.clone());
        if notification.unread && cx.config().notifications.mark_notification_as_read_when_selected {
            let id = cx.request(GithubRequest::MarkNotificationRead(thread));
            self.choosing = Some((id, notification));
        } else {
            self.output = Some(Some(notification));
        }
    }

    /// The mark-on-select answered: the modal closes with the notification. A failure is only logged (Py bug B81's
    /// recommendation for Enter): the notification opens anyway.
    fn chosen_marked(&mut self, notification: Box<Notification>, result: Result<(), GhError>) {
        if let Err(error) = result {
            tracing::error!("{MARK_FAILED}");
            tracing::debug!(
                thread = notification.id,
                "Marking the chosen notification read failed: {error}"
            );
        }
        self.output = Some(Some(notification));
    }

    /// The tab whose page `id` is.
    fn page_for(&self, id: RequestId) -> Option<(NotificationsTab, PageRequest)> {
        [NotificationsTab::Unread, NotificationsTab::Read]
            .into_iter()
            .find_map(|tab| Some((tab, self.pages[tab.index()].filter(|page| page.id == id)?)))
    }
}

impl Component for NotificationsDialog {
    type Action = NotificationsAction;

    fn bindings(&self) -> &'static [BindingId] {
        NOTIFICATIONS_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalClose => Some(Self::action(NotificationsAction::Close)),
            BindingId::TableSearch => {
                self.active_table_mut().open_search();
                self.focus = Focus::Table;
                Some(Action::Consumed)
            }
            BindingId::NotificationsViewRead => {
                self.view(NotificationsTab::Read);
                Some(Action::Consumed)
            }
            BindingId::NotificationsViewUnread => {
                self.view(NotificationsTab::Unread);
                Some(Action::Consumed)
            }
            // Only the Unread pane's link has these. With no row there's nothing to mark (Py bug B80: Python
            // crashes), and a thread whose request is still out isn't asked for twice.
            BindingId::NotificationsMarkRead => {
                let thread = self.table(NotificationsTab::Unread).selected().map(|row| row.0.id);
                Some(match thread {
                    Some(thread) if !self.marking.values().any(|&marking| marking == thread) => {
                        Self::action(NotificationsAction::MarkRead(thread))
                    }
                    _ => Action::Consumed,
                })
            }
            BindingId::NotificationsMarkAllAsRead if self.marking_all.is_none() => {
                Some(Self::action(NotificationsAction::MarkAllRead))
            }
            BindingId::NotificationsMarkAllAsRead => Some(Action::Consumed),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.focus == Focus::Table && self.active_table().consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        // Only the search box takes keys outside the chain.
        if self.stop() != Stop::Search {
            return None;
        }
        let chord = KeyChord::from_event(&key)?;
        let tab = self.active;
        let handled = self.active_table_mut().handle_key(&chord, cx.keymap(), cx.clipboard());
        Some(Self::table_outcome(tab, handled).unwrap_or(Action::Consumed))
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        let tab = self.active;
        // A drag that started on the search box, wherever the pointer is now.
        if self.active_table().search_captures_mouse() {
            self.active_table_mut().search_captured_mouse(event);
            return Some(Action::Consumed);
        }
        let at = (event.column, event.row).into();
        let press = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
        let Areas { tabs, table } = self.areas;
        if !table.contains(at) && matches!(event.kind, MouseEventKind::Moved) {
            // Off the table: a move still clears its hover.
            let _ = self.active_table_mut().handle_mouse(event, table);
        }
        if tabs.contains(at) {
            if press {
                // As a click on a details tab: the strip takes focus, then the tab activates.
                self.focus_stop(Stop::Tabs);
                if let Some(index) = self.tabs.hit(event.column - tabs.x) {
                    self.switch_to(NotificationsTab::from_index(index));
                }
            }
            return Some(Action::Consumed);
        }
        if !table.contains(at) {
            return Some(Action::Consumed);
        }
        let on_rows = self.active_table().drawn_table_rect(table).contains(at);
        let handled = self.active_table_mut().handle_mouse(event, table);
        if press {
            let on_search = self.active_table().focus() == TableFocus::Search;
            if on_rows || on_search {
                self.focus = Focus::Table;
            } else if !self.active_table().overlay_shown() {
                // Below the rows is the scrolling container, which takes focus.
                self.focus_stop(Stop::Container);
            }
        }
        Some(Self::table_outcome(tab, handled).unwrap_or(Action::Consumed))
    }

    fn update(&mut self, action: NotificationsAction, cx: &mut Cx<'_>) {
        match action {
            NotificationsAction::Close => self.output = Some(None),
            NotificationsAction::LoadMore { tab, generation, page } => self.request_page(tab, generation, page, cx),
            NotificationsAction::MarkRead(thread) => {
                if !self.marking.values().any(|&marking| marking == thread) {
                    let id = cx.request(GithubRequest::MarkNotificationRead(thread));
                    self.marking.insert(id, thread);
                }
            }
            NotificationsAction::MarkAllRead => {
                if self.marking_all.is_none() {
                    self.marking_all = Some(cx.request(GithubRequest::MarkAllNotificationsRead));
                }
            }
            NotificationsAction::Select { tab, thread } => self.choose(tab, thread, cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let p = theme.palette;
        let under = self.background(p).over(theme.background());

        // `MarkdownH1`: centered, in `$markdown-h1-color` and its text style.
        let heading = Rect {
            x: area.x + HEADING_PADDING.min(area.width),
            y: area.y.saturating_add(HEADING_MARGIN_TOP),
            width: area.width.saturating_sub(2 * HEADING_PADDING),
            height: 1,
        }
        .intersection(area);
        let style = p
            .markdown_h1_text_style
            .apply(Style::new().fg(theme.color(p.markdown_h1_color, under)));
        let width = usize::from(heading.width);
        frame
            .buffer_mut()
            .set_stringn(heading.x, heading.y, format!("{HEADING:^width$}"), width, style);

        let tabs = Rect {
            y: area.y.saturating_add(HEADING_ROWS),
            height: TAB_STRIP_HEIGHT,
            ..area
        }
        .intersection(area);
        self.tabs
            .render_over(frame, tabs, theme, self.focus == Focus::Tabs, under);

        let top = area.y.saturating_add(HEADING_ROWS + TAB_STRIP_HEIGHT);
        let table = Rect {
            y: top,
            height: area.bottom().saturating_sub(top),
            ..area
        }
        .intersection(area);
        let focused = self.focus == Focus::Table;
        let table_cx = RenderCx {
            focused,
            focus_within: focused,
            ..*cx
        };
        self.active_table_mut().render(frame, table, &table_cx);
        self.areas = Areas { tabs, table };
    }
}

impl Dialog for NotificationsDialog {
    type Output = Option<Box<Notification>>;

    fn kind(&self) -> ModalKind {
        ModalKind::Notifications
    }

    fn own_action(&self, action: DialogAction) -> Option<NotificationsAction> {
        match action {
            DialogAction::Notifications(action) => Some(action),
            _ => None,
        }
    }

    /// The ring: the container, the strip, the search box while it's shown, and the active table. The inactive
    /// tab's table isn't a stop.
    fn move_focus(&mut self, direction: FocusMove) {
        let mut stops = vec![Stop::Container, Stop::Tabs];
        if self.active_table().search_visible() {
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
        // `max-width: 80%` of the screen and the modal's `height: 80%`, rounded down as Textual does.
        (available.width * 4 / 5, available.height * 4 / 5)
    }

    fn take_output(&mut self) -> Option<Option<Box<Notification>>> {
        self.output.take()
    }

    /// The focused stop's link, then, from a table or its search box, the `SearchableDataTable` around it, and on
    /// Unread its tab pane; then the container, wherever focus is.
    fn field_chain(&self) -> Vec<ChainLink> {
        let kind = self.kind();
        let pane = |pane: DialogPane, bindings| ChainLink::new(Owner::DialogPane(kind, pane), bindings);
        let mut chain = match self.stop() {
            Stop::Container => vec![FieldKind::Scroll.link(kind)],
            Stop::Tabs => vec![FieldKind::Tabs.link(kind)],
            stop @ (Stop::Search | Stop::Table) => {
                let field = if stop == Stop::Search {
                    FieldKind::TextInput
                } else {
                    FieldKind::Table
                };
                let mut links = vec![field.link(kind), pane(DialogPane::SearchableTable, SEARCHABLE_BINDINGS)];
                if self.active == NotificationsTab::Unread {
                    links.push(pane(DialogPane::UnreadNotifications, UNREAD_BINDINGS));
                }
                links
            }
        };
        chain.push(pane(DialogPane::NotificationsContainer, CONTAINER_BINDINGS));
        chain
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match field {
            FieldKind::Table => {
                let tab = self.active;
                let handled = self.active_table_mut().on_binding(id);
                Self::table_outcome(tab, handled)
            }
            // `left` / `right` on the strip switch tabs, wrapping; focus stays on the strip.
            FieldKind::Tabs => {
                let mut strip = self.tabs.clone();
                match id {
                    BindingId::TextualTabsNext => strip.select_next(),
                    BindingId::TextualTabsPrevious => strip.select_previous(),
                    _ => return None,
                };
                self.switch_to(NotificationsTab::from_index(strip.active()));
                Some(Action::Consumed)
            }
            // The container's content always fits, so its scroll keys have nothing to scroll. The search box got
            // every key it binds in `handle_key`.
            FieldKind::Scroll | FieldKind::TextInput => Some(Action::Consumed),
            FieldKind::TextEditor
            | FieldKind::Button
            | FieldKind::Switch
            | FieldKind::Select
            | FieldKind::SelectOverlay
            | FieldKind::SelectionList
            | FieldKind::KeyCapture => None,
        }
    }

    fn captures_mouse(&self) -> bool {
        self.active_table().search_captures_mouse()
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        if self.focus != Focus::Table {
            return None;
        }
        let tab = self.active;
        let handled = self.active_table_mut().handle_paste(text);
        Self::table_outcome(tab, handled)
    }

    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.active_table().next_deadline(now)
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.first_page == Some(id) {
            let GithubResponse::ListNotifications(result) = response else {
                return Some(response);
            };
            self.first_page = None;
            self.first_page_loaded(result, cx);
            return None;
        }
        if let Some((tab, request)) = self.page_for(id) {
            let GithubResponse::ListNotifications(result) = response else {
                return Some(response);
            };
            self.pages[tab.index()] = None;
            self.page_loaded(tab, request, result, cx);
            return None;
        }
        if let Some(&thread) = self.marking.get(&id) {
            let GithubResponse::MarkNotificationRead(result) = response else {
                return Some(response);
            };
            self.marking.remove(&id);
            self.marked_read(thread, result, cx);
            return None;
        }
        if self.choosing.as_ref().is_some_and(|(choosing, _)| *choosing == id) {
            let GithubResponse::MarkNotificationRead(result) = response else {
                return Some(response);
            };
            if let Some((_, notification)) = self.choosing.take() {
                self.chosen_marked(notification, result);
            }
            return None;
        }
        if self.marking_all == Some(id) {
            let GithubResponse::MarkAllNotificationsRead(result) = response else {
                return Some(response);
            };
            self.marking_all = None;
            self.marked_all_read(result, cx);
            return None;
        }
        Some(response)
    }
}

#[cfg(test)]
mod tests;
