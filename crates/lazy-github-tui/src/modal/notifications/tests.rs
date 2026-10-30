//! The Notifications modal: its cells, the first page, each tab's paging, `R`, `ctrl+r`, `r` / `u`, focus, closing,
//! the saved rows of `notifications.json`, and its drawing, against the `notifications-modal` parity fixtures.

use std::collections::HashSet;
use std::path::PathBuf;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use ratatui::style::Color;
use rstest::rstest;

use super::*;
use crate::components::footer::footer_items;
use crate::effect::Effect;
use crate::test_util::{LogCapture, TestApp};
use crate::toast::Severity;

/// A `notifications-modal` fixture's notifications.
fn fixture(file: &str) -> Vec<Notification> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/notifications-modal/notifications")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    serde_json::from_value(fixture["response"]["body"].clone()).unwrap()
}

/// The six threads of `all=true` page 1: 9100001, 9100002, 9100004 and 9100006 unread, 9100003 and 9100005 read.
fn all_six() -> Vec<Notification> {
    fixture("GET.all-true_page-1_per_page-50.json")
}

/// The four unread ones, as `all=false` page 1 lists them.
fn unread_four() -> Vec<Notification> {
    fixture("GET.all-false_page-1_per_page-50.json")
}

fn query(all: bool, page: u32) -> NotificationQuery {
    NotificationQuery { all, page }
}

/// GitHub as the parity fixtures answer before anything is marked: both first pages, empty second pages, and every
/// mark succeeding.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_notification_page(query(true, 1), Ok(all_six()))
        .with_notification_page(query(false, 1), Ok(unread_four()))
        .with_notification_read(9_100_001, Ok(()))
        .with_notification_read(9_100_002, Ok(()))
        .with_all_notifications_read(Ok(()))
}

fn failure() -> GhError {
    GhError::NotInstalled
}

/// The `ListNotifications` calls made so far, as `(all, page)`.
fn list_calls(gh: &FakeGithub) -> Vec<(bool, u32)> {
    gh.calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::ListNotifications(query) => Some((query.all, query.page)),
            _ => None,
        })
        .collect()
}

/// A dialog on its own, its requests answered by a [`FakeGithub`] in [`Self::settle`]. Its `notifications.json`
/// read is answered only by [`Self::read_saved`].
struct Harness {
    app: TestApp,
    gh: FakeGithub,
    dialog: NotificationsDialog,
    /// How many of the app's effects came before the dialog's.
    seen: usize,
    /// The requests already answered.
    answered: HashSet<RequestId>,
}

impl Harness {
    /// The dialog as it opens: loading, its first page asked for.
    fn new(gh: FakeGithub) -> Self {
        Self::with_config(gh, Config::default())
    }

    fn with_config(gh: FakeGithub, config: Config) -> Self {
        let mut app = TestApp::new(config);
        let seen = app.effects().len();
        let dialog = app.with_cx(NotificationsDialog::new);
        Self {
            app,
            gh,
            dialog,
            seen,
            answered: HashSet::new(),
        }
    }

    /// Opened on `saved` (read before the live page answers), with nothing else answered.
    fn saved(gh: FakeGithub, saved: &[Notification]) -> Self {
        let mut harness = Self::new(gh);
        harness.read_saved(Some(&cache::to_json(saved)));
        harness
    }

    /// Answers the dialog's `notifications.json` read with `text` (`None`: no file).
    fn read_saved(&mut self, text: Option<&str>) {
        let id = self.app.effects()[self.seen..]
            .iter()
            .find_map(|effect| match effect {
                Effect::ReadCache { id, path } if *path == saved_path() => Some(*id),
                _ => None,
            })
            .expect("the dialog reads notifications.json");
        let Self { app, dialog, .. } = self;
        let result = Ok(text.map(str::to_owned));
        let unclaimed = app.with_cx(|cx| dialog.on_cache_read(id, result, cx));
        assert!(unclaimed.is_none(), "the dialog takes its read");
    }

    /// What the dialog wrote to `notifications.json`, each write in turn.
    fn writes(&self) -> Vec<Vec<Notification>> {
        self.app.effects()[self.seen..]
            .iter()
            .filter_map(|effect| match effect {
                Effect::WriteCache { path, text } if *path == saved_path() => {
                    Some(cache::try_parse(path, text, CACHE_TYPE).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    /// Answers the pending requests `wanted` picks, once each, leaving the rest (and what the answers ask) pending.
    async fn answer(&mut self, wanted: impl Fn(&GithubRequest) -> bool) {
        for (id, request) in self.pending() {
            if wanted(&request) {
                self.answered.insert(id);
                let response = request.run(&self.gh).await;
                self.respond(id, response);
            }
        }
    }

    /// Opened and loaded, with every page it asked for on the way answered.
    async fn loaded(gh: FakeGithub) -> Self {
        let mut harness = Self::new(gh);
        harness.settle().await;
        harness
    }

    /// The requests the dialog made that aren't answered yet.
    fn pending(&self) -> Vec<(RequestId, GithubRequest)> {
        self.app.effects()[self.seen..]
            .iter()
            .filter_map(|effect| match effect {
                Effect::Github { id, request } if !self.answered.contains(id) => Some((*id, request.clone())),
                _ => None,
            })
            .collect()
    }

    /// Answers every request the dialog made, and the ones those answers made, until none is left.
    async fn settle(&mut self) {
        while !self.pending().is_empty() {
            self.answer(|_| true).await;
        }
    }

    fn respond(&mut self, id: RequestId, response: GithubResponse) {
        let Self { app, dialog, .. } = self;
        let unclaimed = app.with_cx(|cx| dialog.on_response(id, response, cx));
        assert!(unclaimed.is_none(), "the dialog takes the answers to its requests");
    }

    /// Applies what a binding or a key handed back, as `App` dispatches the dialog's own actions.
    fn apply(&mut self, action: Option<Action>) -> Option<Action> {
        match action {
            Some(Action::Dialog(DialogAction::Notifications(action))) => {
                let Self { app, dialog, .. } = self;
                app.with_cx(|cx| dialog.update(action, cx));
                Some(Action::Consumed)
            }
            other => other,
        }
    }

    /// Binding `id` matched at a link of the dialog's own (the screen or a pane).
    fn binding(&mut self, id: BindingId) -> Option<Action> {
        let Self { app, dialog, .. } = self;
        let action = app.with_cx(|cx| dialog.on_binding(id, cx));
        self.apply(action)
    }

    /// Binding `id` matched at the focused field's link.
    fn field_binding(&mut self, field: FieldKind, id: BindingId) -> Option<Action> {
        let Self { app, dialog, .. } = self;
        let action = app.with_cx(|cx| dialog.on_field_binding(field, id, cx));
        self.apply(action)
    }

    /// A key the search box takes.
    fn type_key(&mut self, key: &str) {
        let Self { app, dialog, .. } = self;
        let action = app.with_cx(|cx| dialog.handle_key(crate::test_util::key_event(key), cx));
        self.apply(action);
    }

    /// Searches the active tab for `query`.
    fn search(&mut self, query: &str) {
        self.binding(BindingId::TableSearch);
        for c in query.chars() {
            self.type_key(&c.to_string());
        }
        self.type_key("enter");
    }

    fn ids(&self, tab: NotificationsTab) -> Vec<u64> {
        self.dialog.thread_ids(tab)
    }

    fn item(&self, tab: NotificationsTab, thread: u64) -> Option<&Notification> {
        self.dialog.table(tab).get(&thread).map(|row| &row.0)
    }

    fn toasts(&self) -> Vec<(Severity, String)> {
        self.app
            .app()
            .toasts()
            .iter()
            .map(|toast| (toast.severity, toast.message.to_string()))
            .collect()
    }
}

/// The ids the parity fixtures sort into each tab, newest first.
const UNREAD_SORTED: [u64; 4] = [9_100_001, 9_100_002, 9_100_004, 9_100_006];
const READ_SORTED: [u64; 2] = [9_100_003, 9_100_005];

// Cells.

/// A copy of the first fixture thread with these fields.
fn thread(id: u64, updated_at: &str, unread: bool) -> Notification {
    let mut notification = all_six().remove(0);
    notification.id = id;
    notification.updated_at = updated_at.parse().unwrap();
    notification.unread = unread;
    notification
}

fn displayed(notification: Notification) -> Vec<String> {
    NotificationRow(notification)
        .cells(&())
        .into_iter()
        .map(|cell| cell.display.to_string())
        .collect()
}

#[test]
fn the_cells_are_pythons_row_tuple() {
    let mut notification = thread(9_100_002, "2026-10-24T17:15:30-05:00", true);
    notification.subject.title = "  Typo in the install docs \n".into();
    notification.subject.kind = "Issue".into();
    notification.reason = "review_requested".into();
    // The time in UTC, without a zone; the title stripped; the reason title-cased; the id's digits.
    assert_eq!(
        displayed(notification.clone()),
        [
            "2026-10-24 22:15:30",
            "Issue",
            "Typo in the install docs",
            "Review Requested",
            "9100002"
        ]
    );
    let cells = NotificationRow(notification).cells(&());
    assert_eq!(
        cells[0].value,
        CellValue::DateTime(jiff::civil::date(2026, 10, 24).at(22, 15, 30, 0))
    );
    assert_eq!(cells[4].value, CellValue::Int(9_100_002));
}

#[test]
fn a_title_is_literal_text() {
    let mut notification = thread(1, "2026-10-24T23:00:00Z", true);
    notification.subject.title = "[bold]not markup[/]".into();
    assert_eq!(displayed(notification)[2], "[bold]not markup[/]");
}

#[test]
fn microseconds_show_as_python_prints_them() {
    let at = jiff::civil::date(2026, 10, 24).at(23, 0, 0, 1_500);
    assert_eq!(py_datetime_str(at), "2026-10-24 23:00:00.000001");
    assert_eq!(
        py_datetime_str(jiff::civil::date(2026, 1, 2).at(3, 4, 5, 0)),
        "2026-01-02 03:04:05"
    );
}

#[rstest]
#[case("review requested", "Review Requested")]
#[case("ci activity", "Ci Activity")]
#[case("security alert", "Security Alert")]
#[case("approval requested", "Approval Requested")]
#[case("state change", "State Change")]
#[case("mention", "Mention")]
#[case("", "")]
#[case("MIXED cASE", "Mixed Case")]
#[case("they're 2nd", "They'Re 2Nd")]
#[case("été ça", "Été Ça")]
#[case("straße ßa", "Straße Ssa")]
#[case("ǆungla", "ǅungla")]
fn py_title_matches_python(#[case] text: &str, #[case] titled: &str) {
    assert_eq!(py_title(text), titled);
}

// The first page.

#[tokio::test]
async fn the_first_page_splits_by_unread_and_opens_on_unread() {
    let mut harness = Harness::new(github());
    assert_eq!(
        harness
            .pending()
            .iter()
            .map(|(_, request)| request.clone())
            .collect::<Vec<_>>(),
        [GithubRequest::ListNotifications(query(true, 1))]
    );
    assert!(harness.dialog.tables.iter().all(SearchableTable::is_loading));
    assert_eq!(harness.dialog.stop(), Stop::Container);

    harness.settle().await;
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    assert!(!harness.dialog.tables.iter().any(SearchableTable::is_loading));
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
    assert_eq!(harness.dialog.stop(), Stop::Table);
    assert_eq!(harness.dialog.active_table().cursor(), 0);
}

#[tokio::test]
async fn the_loading_overlay_animates_until_the_first_page() {
    let options = crate::options::AppOptions {
        animations: true,
        ..crate::options::AppOptions::default()
    };
    let mut app = TestApp::loading_with_options(Config::default(), FakeGithub::new(), options);
    let mut harness = Harness {
        seen: app.effects().len(),
        dialog: app.with_cx(NotificationsDialog::new),
        app,
        gh: FakeGithub::new(),
        answered: HashSet::new(),
    };
    let now = Instant::now();
    assert!(harness.dialog.next_deadline(now).is_some());
    harness.settle().await;
    assert_eq!(harness.dialog.next_deadline(now), None);
}

#[tokio::test]
async fn with_nothing_unread_it_opens_on_read() {
    let read: Vec<Notification> = all_six().into_iter().filter(|n| !n.unread).collect();
    let harness = Harness::loaded(FakeGithub::new().with_notification_page(query(true, 1), Ok(read))).await;
    assert_eq!(harness.ids(NotificationsTab::Unread), Vec::<u64>::new());
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    assert_eq!(harness.dialog.stop(), Stop::Table);
}

#[tokio::test]
async fn an_empty_first_page_opens_on_read() {
    let harness = Harness::loaded(FakeGithub::new()).await;
    assert!(harness.ids(NotificationsTab::Unread).is_empty());
    assert!(harness.ids(NotificationsTab::Read).is_empty());
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    // Nothing to page from: no row ever asked.
    assert_eq!(list_calls(&harness.gh), [(true, 1)]);
}

#[tokio::test]
async fn a_failed_first_page_logs_and_opens_empty_on_read() {
    let logs = LogCapture::start();
    let gh = FakeGithub::new().with_notification_page(query(true, 1), Err(failure()));
    let harness = Harness::loaded(gh).await;
    assert!(harness.ids(NotificationsTab::Unread).is_empty());
    assert!(harness.ids(NotificationsTab::Read).is_empty());
    assert!(!harness.dialog.tables.iter().any(SearchableTable::is_loading));
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    assert!(
        logs.text().contains(&format!("ERROR {FETCH_FAILED}\n")),
        "{}",
        logs.text()
    );
    assert_eq!(harness.toasts(), []);
}

#[tokio::test]
async fn rows_sort_newest_first_with_ties_in_arrival_order() {
    let page = vec![
        thread(1, "2026-10-24T10:00:00Z", true),
        thread(2, "2026-10-24T12:00:00Z", true),
        thread(3, "2026-10-24T10:00:00Z", true),
        thread(4, "2026-10-24T11:00:00Z", true),
        thread(5, "2026-10-24T10:00:00Z", true),
    ];
    let harness = Harness::loaded(FakeGithub::new().with_notification_page(query(true, 1), Ok(page))).await;
    assert_eq!(harness.ids(NotificationsTab::Unread), [2, 4, 1, 3, 5]);
}

// Paging.

#[tokio::test]
async fn each_tab_pages_from_page_one_again() {
    let mut harness = Harness::loaded(github()).await;
    // The first page, then each short tab's page 1: Unread's first, as its rows go in first.
    assert_eq!(list_calls(&harness.gh), [(true, 1), (false, 1), (true, 1)]);
    // Unread's page 1 is the four it has; the next cursor move asks for page 2.
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh)[3..], [(false, 2)]);
    // Page 2 was empty, so Unread's paging is over.
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh).len(), 4);
    // Read's page 1 (all six) kept only the read two.
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
}

#[tokio::test]
async fn read_paging_ends_on_an_empty_api_page_not_an_empty_filtered_one() {
    let gh = github().with_notification_page(query(true, 2), Ok(unread_four()));
    let mut harness = Harness::loaded(gh).await;
    harness.binding(BindingId::NotificationsViewRead);
    // Page 1 had none of Read's own beyond the two, but wasn't empty: the next move asks for page 2.
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh)[3..], [(true, 2)]);
    // Page 2 had only unread threads: none join Read, and paging goes on.
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorUp);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh)[4..], [(true, 3)]);
    // Page 3 is empty: paging stops.
    harness.field_binding(FieldKind::Table, BindingId::TableCursorUp);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh).len(), 5);
}

#[tokio::test]
async fn a_failed_page_logs_and_ends_that_tabs_paging() {
    let logs = LogCapture::start();
    let gh = github().with_notification_page(query(false, 1), Err(failure()));
    let mut harness = Harness::loaded(gh).await;
    assert!(
        logs.text().contains(&format!("ERROR {FETCH_FAILED}\n")),
        "{}",
        logs.text()
    );
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh), [(true, 1), (false, 1), (true, 1)]);
}

// `R`.

#[tokio::test]
async fn mark_read_moves_the_thread_once_github_answers() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsMarkRead);
    // Nothing moves until the PATCH answers.
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    harness.settle().await;
    assert!(
        harness
            .gh
            .calls()
            .contains(&GithubCall::MarkNotificationRead(9_100_001))
    );
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002, 9_100_004, 9_100_006]);
    assert_eq!(harness.ids(NotificationsTab::Read), [9_100_001, 9_100_003, 9_100_005]);
    // It keeps `unread` as fetched, and the cursor stays at its index.
    assert!(harness.item(NotificationsTab::Read, 9_100_001).unwrap().unread);
    assert_eq!(harness.dialog.table(NotificationsTab::Unread).cursor(), 0);
    assert_eq!(harness.toasts(), []);
}

#[tokio::test]
async fn mark_read_clamps_the_cursor_on_the_last_row() {
    let mut harness = Harness::loaded(github().with_notification_read(9_100_006, Ok(()))).await;
    harness.field_binding(FieldKind::Table, BindingId::TableScrollBottom);
    harness.settle().await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    assert_eq!(harness.dialog.table(NotificationsTab::Unread).cursor(), 2);
}

#[tokio::test]
async fn a_thread_marked_read_never_comes_back_to_unread() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    // Py bug B84: Python's search brings it back from the items it kept.
    harness.search("");
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002, 9_100_004, 9_100_006]);
    assert!(harness.item(NotificationsTab::Unread, 9_100_001).is_none());
}

#[tokio::test]
async fn mark_read_with_no_row_does_nothing() {
    let read: Vec<Notification> = all_six().into_iter().filter(|n| !n.unread).collect();
    let mut harness = Harness::loaded(FakeGithub::new().with_notification_page(query(true, 1), Ok(read))).await;
    harness.binding(BindingId::NotificationsViewUnread);
    assert!(matches!(
        harness.binding(BindingId::NotificationsMarkRead),
        Some(Action::Consumed)
    ));
    assert_eq!(harness.pending(), []);
}

#[tokio::test]
async fn a_second_mark_read_in_flight_is_ignored() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.binding(BindingId::NotificationsMarkRead);
    assert_eq!(
        harness
            .pending()
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        [GithubRequest::MarkNotificationRead(9_100_001)]
    );
}

#[tokio::test]
async fn a_failed_mark_read_keeps_the_thread_and_says_so() {
    let logs = LogCapture::start();
    let mut harness = Harness::loaded(github().with_notification_read(9_100_001, Err(failure()))).await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    assert_eq!(harness.toasts(), [(Severity::Error, MARK_FAILED.to_owned())]);
    assert!(
        logs.text().contains(&format!("ERROR {MARK_FAILED}\n")),
        "{}",
        logs.text()
    );
    // It can be tried again.
    harness.binding(BindingId::NotificationsMarkRead);
    assert_eq!(harness.pending().len(), 1);
}

// `ctrl+r`.

#[tokio::test]
async fn mark_all_read_moves_every_unread_thread_and_shows_read() {
    let mut harness = Harness::loaded(github()).await;
    // A search hides all but one; the hidden ones move too.
    harness.search("typo");
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002]);
    harness.binding(BindingId::NotificationsMarkAllAsRead);
    assert_eq!(
        harness.ids(NotificationsTab::Unread),
        [9_100_002],
        "nothing moves before the answer"
    );
    harness.settle().await;
    assert_eq!(harness.toasts(), [(Severity::Information, MARKED_ALL.to_owned())]);
    assert!(harness.ids(NotificationsTab::Unread).is_empty());
    assert_eq!(harness.dialog.table(NotificationsTab::Unread).items().len(), 0);
    assert_eq!(
        harness.ids(NotificationsTab::Read),
        [9_100_001, 9_100_003, 9_100_002, 9_100_004, 9_100_005, 9_100_006]
    );
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    assert_eq!(harness.dialog.stop(), Stop::Table);
    // Unread never pages again.
    assert!(harness.dialog.table(NotificationsTab::Unread).lazy_load().is_none());
}

#[tokio::test]
async fn a_second_mark_all_in_flight_is_ignored() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsMarkAllAsRead);
    harness.binding(BindingId::NotificationsMarkAllAsRead);
    assert_eq!(
        harness
            .pending()
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        [GithubRequest::MarkAllNotificationsRead]
    );
}

#[tokio::test]
async fn a_failed_mark_all_changes_nothing_and_says_so() {
    let logs = LogCapture::start();
    let mut harness = Harness::loaded(github().with_all_notifications_read(Err(failure()))).await;
    harness.binding(BindingId::NotificationsMarkAllAsRead);
    harness.settle().await;
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
    assert_eq!(harness.toasts(), [(Severity::Error, MARK_ALL_FAILED.to_owned())]);
    assert!(
        logs.text().contains(&format!("ERROR {MARK_ALL_FAILED}\n")),
        "{}",
        logs.text()
    );
}

// Tabs and focus.

#[tokio::test]
async fn view_read_and_unread_switch_and_focus_the_table() {
    let mut harness = Harness::loaded(github()).await;
    harness.dialog.move_focus(FocusMove::Next);
    assert_eq!(harness.dialog.stop(), Stop::Container);
    harness.binding(BindingId::NotificationsViewRead);
    assert_eq!(
        (harness.dialog.active_tab(), harness.dialog.stop()),
        (NotificationsTab::Read, Stop::Table)
    );
    harness.dialog.move_focus(FocusMove::Previous);
    assert_eq!(harness.dialog.stop(), Stop::Tabs);
    // The active tab's key just focuses its table.
    harness.binding(BindingId::NotificationsViewRead);
    assert_eq!(
        (harness.dialog.active_tab(), harness.dialog.stop()),
        (NotificationsTab::Read, Stop::Table)
    );
    harness.binding(BindingId::NotificationsViewUnread);
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
}

#[tokio::test]
async fn tab_cycles_the_container_the_strip_and_the_active_table() {
    let mut harness = Harness::loaded(github()).await;
    let mut ring = Vec::new();
    for _ in 0..4 {
        harness.dialog.move_focus(FocusMove::Next);
        ring.push(harness.dialog.stop());
    }
    assert_eq!(ring, [Stop::Container, Stop::Tabs, Stop::Table, Stop::Container]);
    harness.dialog.move_focus(FocusMove::Previous);
    assert_eq!(harness.dialog.stop(), Stop::Table);
    harness.dialog.move_focus(FocusMove::Previous);
    assert_eq!(harness.dialog.stop(), Stop::Tabs);

    // An open search box is a stop before its table.
    harness.binding(BindingId::NotificationsViewUnread);
    harness.binding(BindingId::TableSearch);
    harness.type_key("x");
    assert_eq!(harness.dialog.stop(), Stop::Search);
    let mut ring = Vec::new();
    for _ in 0..4 {
        harness.dialog.move_focus(FocusMove::Next);
        ring.push(harness.dialog.stop());
    }
    assert_eq!(ring, [Stop::Table, Stop::Container, Stop::Tabs, Stop::Search]);
}

#[tokio::test]
async fn left_and_right_on_the_strip_switch_tabs_and_wrap() {
    let mut harness = Harness::loaded(github()).await;
    harness.dialog.focus_stop(Stop::Tabs);
    let mut tabs = Vec::new();
    for id in [
        BindingId::TextualTabsNext,
        BindingId::TextualTabsNext,
        BindingId::TextualTabsPrevious,
    ] {
        assert!(matches!(
            harness.field_binding(FieldKind::Tabs, id),
            Some(Action::Consumed)
        ));
        tabs.push(harness.dialog.active_tab());
    }
    assert_eq!(
        tabs,
        [NotificationsTab::Read, NotificationsTab::Unread, NotificationsTab::Read]
    );
    assert_eq!(harness.dialog.stop(), Stop::Tabs);
    assert_eq!(harness.dialog.tabs.active(), 1);
}

#[tokio::test]
async fn the_chain_has_the_unread_pane_only_on_unread() {
    let mut harness = Harness::loaded(github()).await;
    let owners =
        |harness: &Harness| -> Vec<Owner> { harness.dialog.field_chain().iter().map(|link| link.owner).collect() };
    let kind = ModalKind::Notifications;
    assert_eq!(
        owners(&harness),
        [
            Owner::DialogField(kind, FieldKind::Table),
            Owner::DialogPane(kind, DialogPane::SearchableTable),
            Owner::DialogPane(kind, DialogPane::UnreadNotifications),
            Owner::DialogPane(kind, DialogPane::NotificationsContainer),
        ]
    );
    harness.binding(BindingId::NotificationsViewRead);
    assert_eq!(
        owners(&harness),
        [
            Owner::DialogField(kind, FieldKind::Table),
            Owner::DialogPane(kind, DialogPane::SearchableTable),
            Owner::DialogPane(kind, DialogPane::NotificationsContainer),
        ]
    );
    harness.dialog.focus_stop(Stop::Tabs);
    assert_eq!(
        owners(&harness),
        [
            Owner::DialogField(kind, FieldKind::Tabs),
            Owner::DialogPane(kind, DialogPane::NotificationsContainer),
        ]
    );
}

// Enter.

impl Harness {
    /// Like [`Self::loaded`], with `notifications.mark_notification_as_read_when_selected` set to `mark`.
    async fn loaded_marking(gh: FakeGithub, mark: bool) -> Self {
        let mut config = Config::default();
        config.notifications.mark_notification_as_read_when_selected = mark;
        let mut harness = Self::with_config(gh, config);
        harness.settle().await;
        harness
    }

    /// `common.table.select` on the focused table's cursor row.
    fn select(&mut self) -> Option<Action> {
        self.field_binding(FieldKind::Table, BindingId::TableSelect)
    }

    /// The thread the dialog closed with, if it closed.
    #[allow(clippy::option_option)]
    fn chosen(&mut self) -> Option<Option<u64>> {
        self.dialog
            .take_output()
            .map(|chosen| chosen.map(|notification| notification.id))
    }

    /// The marks asked for since the last settle (a cursor move may ask for a page too).
    fn requests(&self) -> Vec<GithubRequest> {
        self.pending()
            .into_iter()
            .map(|(_, request)| request)
            .filter(|request| !matches!(request, GithubRequest::ListNotifications(_)))
            .collect()
    }
}

#[tokio::test]
async fn enter_on_an_unread_thread_marks_it_read_before_closing() {
    let mut harness = Harness::loaded(github()).await;
    assert!(matches!(harness.select(), Some(Action::Consumed)));
    assert_eq!(harness.requests(), [GithubRequest::MarkNotificationRead(9_100_001)]);
    // Open until GitHub answers; another select meanwhile is ignored.
    assert_eq!(harness.chosen(), None);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.select();
    assert_eq!(harness.requests(), [GithubRequest::MarkNotificationRead(9_100_001)]);
    harness.settle().await;
    assert_eq!(harness.chosen(), Some(Some(9_100_001)));
    assert_eq!(harness.toasts(), []);
}

#[tokio::test]
async fn with_the_setting_off_enter_closes_at_once() {
    let mut harness = Harness::loaded_marking(github(), false).await;
    harness.select();
    assert_eq!(harness.chosen(), Some(Some(9_100_001)));
    assert_eq!(harness.requests(), []);
}

#[tokio::test]
async fn enter_on_a_read_thread_closes_at_once() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsViewRead);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.select();
    assert_eq!(harness.chosen(), Some(Some(9_100_005)));
    assert_eq!(harness.requests(), []);
}

#[tokio::test]
async fn a_thread_r_moved_to_read_is_marked_again() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    harness.binding(BindingId::NotificationsViewRead);
    assert_eq!(harness.ids(NotificationsTab::Read)[0], 9_100_001);
    harness.select();
    // Python never flips `unread`, so it's marked once more.
    assert_eq!(harness.requests(), [GithubRequest::MarkNotificationRead(9_100_001)]);
    harness.settle().await;
    assert_eq!(harness.chosen(), Some(Some(9_100_001)));
}

#[tokio::test]
async fn a_failed_mark_on_select_opens_anyway_and_only_logs() {
    let logs = LogCapture::start();
    let mut harness = Harness::loaded(github().with_notification_read(9_100_001, Err(failure()))).await;
    harness.select();
    harness.settle().await;
    assert_eq!(harness.chosen(), Some(Some(9_100_001)));
    assert_eq!(harness.toasts(), []);
    assert!(
        logs.text().contains(&format!("ERROR {MARK_FAILED}\n")),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn closing_while_the_mark_is_out_chooses_nothing() {
    let mut harness = Harness::loaded(github()).await;
    harness.select();
    harness.binding(BindingId::ModalClose);
    assert_eq!(harness.chosen(), Some(None));
}

#[tokio::test]
async fn enter_on_an_empty_table_does_nothing() {
    let read: Vec<Notification> = all_six().into_iter().filter(|n| !n.unread).collect();
    let mut harness = Harness::loaded(FakeGithub::new().with_notification_page(query(true, 1), Ok(read))).await;
    harness.binding(BindingId::NotificationsViewUnread);
    harness.select();
    assert_eq!(harness.chosen(), None);
    assert_eq!(harness.requests(), []);
}

// Through the app.

fn enabled() -> Config {
    let mut config = Config::default();
    config.notifications.enabled = true;
    config
}

/// The main screen with notifications on and the startup count answered.
async fn main_screen() -> TestApp {
    let mut app = TestApp::with_github(enabled(), github().with_unread_count(Ok(4)));
    app.settle().await;
    app
}

/// The modal opened and loaded.
async fn opened() -> TestApp {
    let mut app = main_screen().await;
    app.press(&["ctrl+n"]);
    app.settle().await;
    app
}

/// Rows `rows` of the modal's interior at 160x48 (columns 17 to 142), with their styles.
fn styled_rows(buffer: &ratatui::buffer::Buffer, rows: std::ops::Range<u16>) -> String {
    let area = Rect::new(17, rows.start, 126, rows.end - rows.start);
    let mut crop = ratatui::buffer::Buffer::empty(Rect::new(0, 0, area.width, area.height));
    for (dy, y) in rows.enumerate() {
        for dx in 0..area.width {
            crop[(dx, u16::try_from(dy).unwrap())] = buffer[(area.x + dx, y)].clone();
        }
    }
    crate::test_util::buffer_to_styled_string(&crop)
}

fn footer(app: &mut TestApp) -> Vec<String> {
    footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

const UNREAD_FOOTER: [&str; 6] = [
    "/ Search",
    "R Mark as Read",
    "^r Mark all as read",
    "r View Read Notifications",
    "u View Unread Notifications",
    "q Close",
];
const READ_FOOTER: [&str; 4] = [
    "/ Search",
    "r View Read Notifications",
    "u View Unread Notifications",
    "q Close",
];
const CONTAINER_FOOTER: [&str; 3] = ["r View Read Notifications", "u View Unread Notifications", "q Close"];

#[tokio::test]
async fn ctrl_n_opens_it_loading() {
    let mut app = main_screen().await;
    app.press(&["ctrl+n"]);
    assert_eq!(app.app().modals(), [ModalKind::Notifications]);
    assert_eq!(footer(&mut app), CONTAINER_FOOTER);
    insta::assert_snapshot!("loading", app.screen(160, 48));
}

#[tokio::test]
async fn unread_opens_with_its_rows_and_the_table_focused() {
    let mut app = opened().await;
    assert_eq!(footer(&mut app), UNREAD_FOOTER);
    insta::assert_snapshot!("unread", app.screen(160, 48));
}

#[tokio::test]
async fn the_read_tab() {
    let mut app = opened().await;
    app.press(&["r"]);
    assert_eq!(footer(&mut app), READ_FOOTER);
    insta::assert_snapshot!("read", app.screen(160, 48));
}

#[tokio::test]
async fn an_empty_unread_tab_shows_its_header() {
    let mut app = opened().await;
    app.press(&["ctrl+r"]);
    app.settle().await;
    app.press(&["u"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Marked all notifications as read"));
    insta::assert_snapshot!("unread_empty", screen);
    // `R` there does nothing.
    let before = app.effects().len();
    app.press(&["R"]);
    assert_eq!(app.effects()[before..], []);
}

#[tokio::test]
async fn a_table_wider_than_the_modal_scrolls_sideways() {
    let mut app = opened().await;
    let screen = app.screen(100, 30);
    insta::assert_snapshot!("unread_narrow", screen);
    app.press(&["l", "l"]);
    let scrolled = app.screen(100, 30);
    assert_ne!(scrolled, screen);
}

#[tokio::test]
async fn the_tab_strip_focused() {
    let mut app = opened().await;
    app.press(&["shift+tab"]);
    assert_eq!(footer(&mut app), CONTAINER_FOOTER);
    insta::assert_snapshot!("tabs_focused", app.screen(160, 48));
    // The active label in the block cursor, the bar brighter, and the table's cursor row blurred.
    insta::assert_snapshot!("tabs_focused_styled", styled_rows(&app.render(160, 48), 10..14));
    app.press(&["right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("v1.0.0") && !screen.contains("Typo in the install docs"));
}

#[tokio::test]
async fn the_search_box_open() {
    let mut app = opened().await;
    // The box types `r` and `u` rather than switching tabs.
    app.press(&["/", "t", "y", "p", "o", "r", "u"]);
    assert!(app.screen(160, 48).contains("typoru"));
    app.press(&["backspace", "backspace"]);
    insta::assert_snapshot!("search_open", app.screen(160, 48));
    app.press(&["enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Typo in the install docs") && !screen.contains("Add the notifications modal"));
}

#[tokio::test]
async fn the_tab_labels_keep_their_colors() {
    let mut app = opened().await;
    let buffer = app.render(160, 48);
    // " •Unread  ✓Read" from the interior's left edge at column 17, row 10.
    assert_eq!(buffer[(18, 10)].symbol(), "•");
    assert_eq!(buffer[(18, 10)].fg, Color::Rgb(0xff, 0x00, 0x00));
    assert_eq!(buffer[(27, 10)].symbol(), "✓");
    assert_eq!(buffer[(27, 10)].fg, Color::Rgb(0xad, 0xff, 0x2f));
}

#[tokio::test]
async fn a_click_on_a_tab_label_focuses_the_strip_and_shows_the_tab() {
    let mut app = opened().await;
    app.render(160, 48);
    app.click(28, 10);
    assert_eq!(footer(&mut app), CONTAINER_FOOTER);
    let screen = app.screen(160, 48);
    assert!(screen.contains("v1.0.0") && !screen.contains("Typo in the install docs"));
    // A click on a row focuses the table again (one off the cursor's row, which would choose it).
    app.click(30, 14);
    assert_eq!(footer(&mut app), READ_FOOTER);
}

#[tokio::test]
async fn closing_refreshes_the_unread_count() {
    let mut app = opened().await;
    let counts = |app: &TestApp| {
        app.gh()
            .calls()
            .iter()
            .filter(|call| **call == GithubCall::UnreadNotificationCount)
            .count()
    };
    let before = counts(&app);
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    app.settle().await;
    assert_eq!(counts(&app), before + 1);
}

#[tokio::test]
async fn answers_after_closing_are_dropped() {
    let mut app = main_screen().await;
    app.press(&["ctrl+n", "q"]);
    assert!(app.app().modals().is_empty());
    // The first page answers with the modal gone.
    app.settle().await;
    assert!(list_calls(app.gh()).contains(&(true, 1)));
    assert!(app.app().modals().is_empty());
    assert!(!app.screen(160, 48).contains("Updated At"));
}

// The saved rows (`notifications.json`).

/// `notifications.json` under the default config's cache directory.
fn saved_path() -> PathBuf {
    cache::cache_path(
        &Config::default().cache.cache_directory,
        CacheScope::App,
        cache::NOTIFICATIONS,
    )
}

/// A copy of the first fixture thread (9100001) with this id, title, time and flag.
fn saved_thread(id: u64, title: &str, updated_at: &str, unread: bool) -> Notification {
    let mut notification = thread(id, updated_at, unread);
    notification.subject.title = title.into();
    notification
}

/// What the parity seed `notifications-cache` holds: an unread and a read thread only it has, and 9100001 saved as
/// read (live, it's unread).
fn saved_three() -> Vec<Notification> {
    vec![
        saved_thread(9_100_901, "Saved unread thread", "2026-10-25T01:00:00Z", true),
        saved_thread(9_100_902, "Saved read thread", "2026-10-25T00:30:00Z", false),
        saved_thread(9_100_001, "Add the notifications modal", "2026-10-24T23:00:00Z", false),
    ]
}

fn saved_read_only() -> Vec<Notification> {
    saved_three().into_iter().filter(|n| !n.unread).collect()
}

/// The threads of `page` with the given ids flipped to read, as the file holds them after those marks.
fn marked(mut page: Vec<Notification>, ids: &[u64]) -> Vec<Notification> {
    for notification in &mut page {
        if ids.contains(&notification.id) {
            notification.unread = false;
        }
    }
    page
}

fn is_first_page(request: &GithubRequest) -> bool {
    *request == GithubRequest::ListNotifications(query(true, 1))
}

fn is_mark(request: &GithubRequest) -> bool {
    matches!(
        request,
        GithubRequest::MarkNotificationRead(_) | GithubRequest::MarkAllNotificationsRead
    )
}

#[tokio::test]
async fn it_reads_the_file_before_asking_for_the_first_page() {
    let harness = Harness::new(github());
    let effects: Vec<&Effect> = harness.app.effects()[harness.seen..].iter().collect();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::ReadCache { path, .. }, Effect::Github { request, .. }]
                if *path == saved_path() && is_first_page(request)
        ),
        "{effects:?}"
    );
}

#[rstest]
#[case::no_file(None)]
#[case::empty(Some("[]"))]
#[tokio::test]
async fn with_nothing_saved_it_opens_loading(#[case] text: Option<&str>) {
    let mut harness = Harness::new(github());
    harness.read_saved(text);
    assert_eq!(harness.dialog.shown, Shown::Loading);
    assert!(harness.dialog.tables.iter().all(SearchableTable::is_loading));
    assert_eq!(harness.dialog.stop(), Stop::Container);
    harness.settle().await;
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    assert_eq!(harness.dialog.shown, Shown::Live);
}

#[tokio::test]
async fn a_corrupt_file_warns_and_opens_loading() {
    let logs = LogCapture::start();
    let mut harness = Harness::new(github());
    harness.read_saved(Some("[{"));
    assert_eq!(harness.dialog.shown, Shown::Loading);
    assert!(harness.dialog.tables.iter().all(SearchableTable::is_loading));
    assert!(
        logs.text().contains("WARN Failed to parse cache file '"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn a_bad_thread_warns_with_the_notification_schema_and_opens_loading() {
    let logs = LogCapture::start();
    let mut harness = Harness::new(github());
    let mut saved: Vec<serde_json::Value> = serde_json::from_str(&cache::to_json(saved_three())).unwrap();
    saved[1]["unread"] = serde_json::json!("no");
    harness.read_saved(Some(&serde_json::Value::Array(saved).to_string()));
    assert_eq!(harness.dialog.shown, Shown::Loading);
    assert!(
        logs.text()
            .contains("doesn't match the expected schema for Notification"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn saved_rows_show_at_once_split_by_unread() {
    let harness = Harness::saved(github(), &saved_three());
    assert_eq!(harness.dialog.shown, Shown::Saved { failed: false });
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_901]);
    assert_eq!(harness.ids(NotificationsTab::Read), [9_100_902, 9_100_001]);
    assert!(!harness.dialog.tables.iter().any(SearchableTable::is_loading));
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
    assert_eq!(harness.dialog.stop(), Stop::Table);
    // Only the live first page is out: the saved rows don't page.
    assert!(harness.dialog.tables.iter().all(|table| table.lazy_load().is_none()));
    assert_eq!(
        harness
            .pending()
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        [GithubRequest::ListNotifications(query(true, 1))]
    );
}

#[tokio::test]
async fn all_saved_read_opens_on_read() {
    let harness = Harness::saved(github(), &saved_read_only());
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    assert_eq!(harness.dialog.stop(), Stop::Table);
}

#[tokio::test]
async fn a_tab_picked_under_the_overlay_stays_on_the_saved_rows() {
    let mut harness = Harness::new(github());
    harness.binding(BindingId::NotificationsViewRead);
    harness.read_saved(Some(&cache::to_json(saved_three())));
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
}

#[tokio::test]
async fn a_live_page_before_the_read_drops_it() {
    let mut harness = Harness::new(github());
    harness.answer(is_first_page).await;
    harness.read_saved(Some(&cache::to_json(saved_three())));
    assert_eq!(harness.dialog.shown, Shown::Live);
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
}

// The live page over the saved rows.

#[tokio::test]
async fn the_live_page_replaces_the_saved_rows() {
    let mut harness = Harness::saved(github(), &saved_three());
    harness.settle().await;
    assert_eq!(harness.dialog.shown, Shown::Live);
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
    assert_eq!(harness.dialog.stop(), Stop::Table);
    assert_eq!(harness.dialog.active_table().cursor(), 0);
    // Each short tab asks for its page 1, as after a live first page.
    assert_eq!(list_calls(&harness.gh), [(true, 1), (false, 1), (true, 1)]);
}

#[tokio::test]
async fn the_cursor_stays_on_its_thread() {
    let mut harness = Harness::saved(github(), &all_six());
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    assert_eq!(
        harness.dialog.active_table().selected().map(|row| row.0.id),
        Some(9_100_004)
    );
    // Live, 9100001 was marked read elsewhere, so 9100004 is one row higher.
    let live = marked(all_six(), &[9_100_001]);
    let gh = github().with_notification_page(query(true, 1), Ok(live));
    harness.gh = gh;
    harness.answer(is_first_page).await;
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002, 9_100_004, 9_100_006]);
    assert_eq!(
        harness.dialog.active_table().selected().map(|row| row.0.id),
        Some(9_100_004)
    );
    assert_eq!(harness.dialog.active_table().cursor(), 1);
}

#[tokio::test]
async fn a_moved_cursor_whose_thread_left_the_tab_goes_to_the_top() {
    let mut harness = Harness::saved(github(), &saved_three());
    harness.binding(BindingId::NotificationsViewRead);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    assert_eq!(
        harness.dialog.active_table().selected().map(|row| row.0.id),
        Some(9_100_001)
    );
    harness.settle().await;
    // Live, 9100001 is unread, so it's not in Read.
    assert_eq!(harness.ids(NotificationsTab::Read), READ_SORTED);
    assert_eq!(harness.dialog.active_table().cursor(), 0);
}

#[tokio::test]
async fn the_tab_follows_the_live_page_unless_picked() {
    // Saved all read, so Read; live has unread threads, so Unread.
    let mut harness = Harness::saved(github(), &saved_read_only());
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    harness.settle().await;
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
}

#[derive(Debug, Clone, Copy)]
enum Pick {
    ViewRead,
    StripNext,
}

#[rstest]
#[case::view_read(Pick::ViewRead)]
#[case::strip(Pick::StripNext)]
#[tokio::test]
async fn a_picked_tab_stays_when_the_live_page_lands(#[case] pick: Pick) {
    let mut harness = Harness::saved(github(), &saved_three());
    match pick {
        Pick::ViewRead => {
            harness.binding(BindingId::NotificationsViewRead);
        }
        Pick::StripNext => {
            harness.dialog.focus_stop(Stop::Tabs);
            harness.field_binding(FieldKind::Tabs, BindingId::TextualTabsNext);
        }
    }
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    let stop = harness.dialog.stop();
    harness.settle().await;
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    assert_eq!(harness.dialog.stop(), stop, "focus stays");
}

#[tokio::test]
async fn view_unread_picked_stays_over_a_live_page_with_nothing_unread() {
    let read: Vec<Notification> = all_six().into_iter().filter(|n| !n.unread).collect();
    let gh = FakeGithub::new().with_notification_page(query(true, 1), Ok(read));
    let mut harness = Harness::saved(gh, &saved_three());
    harness.binding(BindingId::NotificationsViewUnread);
    harness.settle().await;
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Unread);
    assert!(harness.ids(NotificationsTab::Unread).is_empty());
}

#[tokio::test]
async fn a_focused_search_box_keeps_its_text_and_the_live_rows_all_show() {
    let mut harness = Harness::saved(github(), &all_six());
    harness.binding(BindingId::TableSearch);
    for c in "typo".chars() {
        harness.type_key(&c.to_string());
    }
    harness.settle().await;
    assert_eq!(harness.dialog.stop(), Stop::Search);
    assert_eq!(harness.dialog.active_table().search_text(), "typo");
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
}

#[tokio::test]
async fn a_submitted_search_keeps_its_text_but_the_live_rows_all_show() {
    let mut harness = Harness::saved(github(), &all_six());
    harness.search("typo");
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002]);
    harness.settle().await;
    assert_eq!(harness.dialog.active_table().search_text(), "typo");
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    // The submit put the cursor on the top row, where it stays.
    assert_eq!(harness.dialog.active_table().cursor(), 0);
}

#[tokio::test]
async fn a_cursor_left_on_the_top_row_stays_there() {
    // Saved, 9100002 is Unread's top row; live, 9100001 is above it.
    let mut harness = Harness::saved(github(), &marked(all_six(), &[9_100_001]));
    assert_eq!(
        harness.dialog.active_table().selected().map(|row| row.0.id),
        Some(9_100_002)
    );
    harness.settle().await;
    assert_eq!(harness.dialog.active_table().cursor(), 0);
    assert_eq!(
        harness.dialog.active_table().selected().map(|row| row.0.id),
        Some(9_100_001)
    );
}

#[tokio::test]
async fn r_on_a_saved_row_carries_over_to_the_live_page() {
    let mut harness = Harness::saved(github(), &all_six());
    harness.binding(BindingId::NotificationsMarkRead);
    harness.answer(is_mark).await;
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002, 9_100_004, 9_100_006]);
    harness.settle().await;
    // Live, GitHub still lists 9100001 as unread; the mark puts it in Read anyway, `unread` as fetched.
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002, 9_100_004, 9_100_006]);
    assert_eq!(harness.ids(NotificationsTab::Read), [9_100_001, 9_100_003, 9_100_005]);
    assert!(harness.item(NotificationsTab::Read, 9_100_001).unwrap().unread);
    // The file has it read.
    assert_eq!(harness.writes().last().unwrap(), &marked(all_six(), &[9_100_001]));
}

#[tokio::test]
async fn ctrl_r_on_saved_rows_carries_over_and_unread_doesnt_page() {
    let mut harness = Harness::saved(github(), &all_six());
    harness.binding(BindingId::NotificationsMarkAllAsRead);
    harness.answer(is_mark).await;
    assert_eq!(harness.dialog.active_tab(), NotificationsTab::Read);
    harness.settle().await;
    assert!(harness.ids(NotificationsTab::Unread).is_empty());
    assert_eq!(harness.ids(NotificationsTab::Read).len(), 6);
    assert_eq!(
        harness.dialog.active_tab(),
        NotificationsTab::Read,
        "ctrl+r's switch counts as a pick"
    );
    assert!(harness.dialog.table(NotificationsTab::Unread).lazy_load().is_none());
    // Unread doesn't page, and Read's six rows are more than five from its cursor: nothing past the first page.
    assert_eq!(list_calls(&harness.gh), [(true, 1)]);
    let all = [9_100_001, 9_100_002, 9_100_003, 9_100_004, 9_100_005, 9_100_006];
    assert_eq!(harness.writes().last().unwrap(), &marked(all_six(), &all));
}

#[tokio::test]
async fn r_in_flight_across_the_swap_moves_the_live_row() {
    let mut harness = Harness::saved(github(), &all_six());
    harness.binding(BindingId::NotificationsMarkRead);
    harness.answer(is_first_page).await;
    assert_eq!(harness.ids(NotificationsTab::Unread), UNREAD_SORTED);
    harness.answer(is_mark).await;
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_002, 9_100_004, 9_100_006]);
    assert_eq!(harness.ids(NotificationsTab::Read)[0], 9_100_001);
}

#[tokio::test]
async fn enter_on_a_saved_row_chooses_it_marking_it_first() {
    let mut harness = Harness::saved(github().with_notification_read(9_100_901, Ok(())), &saved_three());
    harness.select();
    assert_eq!(harness.requests(), [GithubRequest::MarkNotificationRead(9_100_901)]);
    harness.answer(is_mark).await;
    assert_eq!(harness.chosen(), Some(Some(9_100_901)));
    assert_eq!(
        harness.writes(),
        [marked(saved_three(), &[9_100_901])],
        "the mark is saved"
    );
}

#[tokio::test]
async fn with_the_setting_off_enter_on_a_saved_row_closes_at_once() {
    let mut config = Config::default();
    config.notifications.mark_notification_as_read_when_selected = false;
    let mut harness = Harness::with_config(github(), config);
    harness.read_saved(Some(&cache::to_json(saved_three())));
    harness.select();
    assert_eq!(harness.chosen(), Some(Some(9_100_901)));
    assert_eq!(harness.requests(), []);
}

#[tokio::test]
async fn a_failed_live_page_keeps_the_saved_rows() {
    let logs = LogCapture::start();
    let gh = FakeGithub::new().with_notification_page(query(true, 1), Err(failure()));
    let mut harness = Harness::saved(gh, &saved_three());
    harness.settle().await;
    assert_eq!(harness.dialog.shown, Shown::Saved { failed: true });
    assert_eq!(harness.ids(NotificationsTab::Unread), [9_100_901]);
    assert_eq!(harness.ids(NotificationsTab::Read), [9_100_902, 9_100_001]);
    assert!(
        logs.text().contains(&format!("ERROR {FETCH_FAILED}\n")),
        "{}",
        logs.text()
    );
    assert_eq!(harness.toasts(), []);
    assert_eq!(harness.writes(), Vec::<Vec<Notification>>::new());
    // Nothing pages from the saved rows.
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert_eq!(list_calls(&harness.gh), [(true, 1)]);
}

// Writes.

#[tokio::test]
async fn the_live_first_page_is_written_without_later_pages() {
    let gh = github().with_notification_page(
        query(true, 2),
        Ok(vec![thread(9_200_001, "2026-10-01T00:00:00Z", false)]),
    );
    let mut harness = Harness::loaded(gh).await;
    harness.binding(BindingId::NotificationsViewRead);
    harness.field_binding(FieldKind::Table, BindingId::TableCursorDown);
    harness.settle().await;
    assert!(harness.ids(NotificationsTab::Read).contains(&9_200_001));
    assert_eq!(harness.writes(), [all_six()]);
}

#[tokio::test]
async fn marks_rewrite_the_file() {
    let mut harness = Harness::loaded(github()).await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    assert_eq!(harness.writes().last().unwrap(), &marked(all_six(), &[9_100_001]));
    harness.binding(BindingId::NotificationsMarkAllAsRead);
    harness.settle().await;
    let all = [9_100_001, 9_100_002, 9_100_003, 9_100_004, 9_100_005, 9_100_006];
    assert_eq!(harness.writes().last().unwrap(), &marked(all_six(), &all));
    assert_eq!(harness.writes().len(), 3);
}

#[tokio::test]
async fn a_mark_on_select_rewrites_the_file() {
    let mut harness = Harness::loaded(github()).await;
    harness.select();
    harness.settle().await;
    assert_eq!(harness.chosen(), Some(Some(9_100_001)));
    assert_eq!(harness.writes().last().unwrap(), &marked(all_six(), &[9_100_001]));
}

#[tokio::test]
async fn failed_marks_and_failed_pages_write_nothing() {
    let gh = FakeGithub::new()
        .with_notification_page(query(true, 1), Err(failure()))
        .with_notification_read(9_100_001, Err(failure()));
    let harness = Harness::loaded(gh).await;
    assert_eq!(harness.writes(), Vec::<Vec<Notification>>::new());

    let mut harness = Harness::loaded(github().with_notification_read(9_100_001, Err(failure()))).await;
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    harness.select();
    harness.settle().await;
    assert_eq!(harness.writes(), [all_six()]);
}

#[tokio::test]
async fn r_on_a_later_page_row_writes_nothing() {
    let later = thread(9_200_001, "2026-10-01T00:00:00Z", true);
    let gh = github()
        .with_notification_page(query(false, 1), Ok(vec![later]))
        .with_notification_read(9_200_001, Ok(()));
    let mut harness = Harness::loaded(gh).await;
    harness.field_binding(FieldKind::Table, BindingId::TableScrollBottom);
    harness.settle().await;
    assert_eq!(
        harness.dialog.active_table().selected().map(|row| row.0.id),
        Some(9_200_001)
    );
    harness.binding(BindingId::NotificationsMarkRead);
    harness.settle().await;
    assert!(harness.ids(NotificationsTab::Read).contains(&9_200_001));
    assert_eq!(harness.writes(), [all_six()]);
}

// The saved rows through the app.

/// The main screen with `notifications.json` holding `saved`, the modal opened on it and the live page still out.
async fn opened_saved(gh: FakeGithub, saved: &[Notification]) -> TestApp {
    let mut app = TestApp::with_github(enabled(), gh.with_unread_count(Ok(4)))
        .with_cache_file(saved_path(), cache::to_json(saved));
    app.settle().await;
    app.press(&["ctrl+n"]);
    app.answer_cache_reads();
    app
}

#[tokio::test]
async fn the_saved_rows_with_the_line_under_the_heading() {
    let mut app = opened_saved(github(), &saved_three()).await;
    assert_eq!(footer(&mut app), UNREAD_FOOTER);
    let screen = app.screen(160, 48);
    assert!(screen.contains(SAVED_REFRESHING) && !screen.contains("Loading"));
    insta::assert_snapshot!("saved_unread", screen);
    insta::assert_snapshot!("saved_unread_narrow", app.screen(100, 30));
    // The line is muted.
    let buffer = app.render(160, 48);
    let (x, y) = (0..48)
        .flat_map(|y| (0..160).map(move |x| (x, y)))
        .find(|&(x, y)| buffer[(x, y)].symbol() == "S" && buffer[(x + 1, y)].symbol() == "h")
        .unwrap();
    assert_ne!(buffer[(x, y)].fg, buffer[(x, y - 1)].fg, "not the heading's color");

    app.settle().await;
    let screen = app.screen(160, 48);
    assert!(!screen.contains("Showing saved notifications"));
    assert!(screen.contains("Typo in the install docs") && !screen.contains("Saved unread thread"));
    // The file now holds the live first page.
    let written: Vec<Notification> =
        cache::try_parse(&saved_path(), app.cache_file(&saved_path()).unwrap(), CACHE_TYPE).unwrap();
    assert_eq!(written, all_six());
}

#[tokio::test]
async fn a_failed_refresh_says_so_under_the_heading() {
    let gh = github().with_notification_page(query(true, 1), Err(failure()));
    let mut app = opened_saved(gh, &saved_three()).await;
    app.settle().await;
    let screen = app.screen(160, 48);
    assert!(screen.contains(SAVED_REFRESH_FAILED) && screen.contains("Saved unread thread"));
    insta::assert_snapshot!("saved_refresh_failed", screen);
}

#[tokio::test]
async fn a_click_on_a_tab_counts_as_a_pick() {
    let mut app = opened_saved(github(), &saved_three()).await;
    app.render(160, 48);
    // "✓Read" on the strip.
    app.click(28, 10);
    app.settle().await;
    let screen = app.screen(160, 48);
    assert!(screen.contains("v1.0.0") && !screen.contains("Typo in the install docs"));
}

#[tokio::test]
async fn reopening_shows_what_the_last_open_saved() {
    let mut app = opened().await;
    app.press(&["escape"]);
    app.settle().await;
    app.press(&["ctrl+n"]);
    app.answer_cache_reads();
    let screen = app.screen(160, 48);
    assert!(screen.contains(SAVED_REFRESHING) && screen.contains("Add the notifications modal"));
}

// `gh` failing mid-session (#323): the pages and the marks.

mod gh_failures {
    use super::*;
    use crate::effect::Effect;
    use crate::test_util::assert_gh_failures_surface;

    fn listed() -> TestApp {
        TestApp::with_github(enabled(), github().with_unread_count(Ok(4)))
    }

    fn opened() -> TestApp {
        let mut app = listed();
        futures_util::FutureExt::now_or_never(app.settle()).unwrap();
        app.press(&["ctrl+n"]);
        app
    }

    fn sent(app: &TestApp, wanted: fn(&GithubRequest) -> bool) -> usize {
        app.effects()
            .iter()
            .filter(|effect| matches!(effect, Effect::Github { request, .. } if wanted(request)))
            .count()
    }

    #[test]
    fn the_first_page() {
        assert_gh_failures_surface(
            listed,
            |app| {
                app.press(&["ctrl+n"]);
            },
            |app| {
                if app.app().modals() == [ModalKind::Notifications] {
                    Ok(())
                } else {
                    Err(format!("{:?}", app.app().modals()))
                }
            },
        );
    }

    #[test]
    fn the_first_page_over_saved_rows_keeps_them() {
        assert_gh_failures_surface(
            || listed().with_cache_file(saved_path(), cache::to_json(saved_three())),
            |app| {
                app.press(&["ctrl+n"]);
            },
            |app| {
                let screen = app.screen(160, 48);
                if screen.contains(SAVED_REFRESH_FAILED) && screen.contains("Saved unread thread") {
                    Ok(())
                } else {
                    Err(format!("the saved rows went:\n{screen}"))
                }
            },
        );
    }

    #[test]
    fn marking_one_read() {
        let marks = |request: &GithubRequest| matches!(request, GithubRequest::MarkNotificationRead(_));
        assert_gh_failures_surface(
            opened,
            |app| {
                app.press(&["R"]);
            },
            |app| {
                let before = sent(app, marks);
                app.press(&["R"]);
                (sent(app, marks) > before)
                    .then_some(())
                    .ok_or_else(|| "R is stuck".into())
            },
        );
    }

    #[test]
    fn marking_all_read() {
        let marks = |request: &GithubRequest| matches!(request, GithubRequest::MarkAllNotificationsRead);
        assert_gh_failures_surface(
            opened,
            |app| {
                app.press(&["ctrl+r"]);
            },
            |app| {
                let before = sent(app, marks);
                app.press(&["ctrl+r"]);
                (sent(app, marks) > before)
                    .then_some(())
                    .ok_or_else(|| "ctrl+r is stuck".into())
            },
        );
    }
}
