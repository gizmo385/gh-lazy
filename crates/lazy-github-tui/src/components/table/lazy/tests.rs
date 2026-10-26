use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tokio::time::Instant;

use super::*;
use crate::component::RenderCx;
use crate::components::table::{Cell, Column, Handled, SortSpec};
use crate::keymap::{KeySpec, Keymap};
use crate::theme::Theme;

/// A test pull request: a number and a title, sorted by number descending as Python's PR table is.
#[derive(Debug, Clone, PartialEq)]
struct Pr {
    number: i64,
}

impl TableItem for Pr {
    type Key = i64;
    type Ctx = ();

    fn key(&self) -> i64 {
        self.number
    }

    fn cells(&self, (): &()) -> Vec<Cell> {
        vec![
            Cell::int(self.number),
            Cell::text(format!("Pull request {}", self.number)),
        ]
    }
}

fn prs(numbers: impl IntoIterator<Item = i64>) -> Vec<Pr> {
    numbers.into_iter().map(|number| Pr { number }).collect()
}

fn table() -> SearchableTable<Pr> {
    SearchableTable::new(vec![Column::new("#"), Column::new("Title")], SortSpec::descending(0))
}

/// A lazy table with a full first page of 30, as the PR table has after its first fetch.
fn thirty() -> SearchableTable<Pr> {
    let mut table = table();
    table.enable_lazy_load(30, 1);
    assert_eq!(table.insert_many(prs(1..=30), &()), None);
    table
}

fn press(table: &mut SearchableTable<Pr>, key: &str) -> Handled<TableEvent<i64>> {
    let KeySpec::Chord(chord) = KeySpec::parse(key) else {
        panic!("{key:?} isn't a key");
    };
    table.handle_key(&chord, &Keymap::defaults())
}

/// Presses `j` until the cursor is at `cursor`, asserting that no move on the way asks for a page.
fn down_to(table: &mut SearchableTable<Pr>, cursor: usize) {
    while table.cursor() < cursor {
        assert_eq!(press(table, "j"), Handled::Consumed, "at cursor {}", table.cursor());
    }
}

fn load_more(generation: u64, page: u32, per_page: u32) -> Handled<TableEvent<i64>> {
    Handled::Event(TableEvent::LoadMore {
        generation,
        page,
        per_page,
    })
}

fn http_error() -> GhError {
    GhError::Http {
        status: 502,
        message: "Bad Gateway".into(),
        body: Vec::new(),
    }
}

fn numbers(table: &SearchableTable<Pr>) -> Vec<i64> {
    table.row_keys().copied().collect()
}

#[test]
fn should_load_counts_from_the_cursor_to_the_end_of_the_items() {
    let lazy = LazyLoad::new(30, 1);
    assert_eq!(should_load(30, 24, &lazy), None, "remaining 6");
    assert_eq!(should_load(30, 25, &lazy), Some(2), "remaining 5");
    assert_eq!(should_load(30, 29, &lazy), Some(2), "remaining 1");
    let done = LazyLoad {
        can_load_more: false,
        ..lazy
    };
    assert_eq!(should_load(30, 29, &done), None);
}

#[test]
fn a_hidden_item_still_counts_toward_remaining() {
    // `items` is every loaded item. With 10 items of which a search shows 5, the cursor on the last shown row (4)
    // leaves 6 remaining, not 1, as Python's `len(self.items) - cursor_row` does.
    let lazy = LazyLoad::new(30, 1);
    assert_eq!(should_load(10, 4, &lazy), None);
    assert_eq!(should_load(10, 5, &lazy), Some(2));
}

#[test]
fn no_request_while_disabled() {
    let mut table = table();
    assert_eq!(table.insert_many(prs(1..=3), &()), None);
    assert_eq!(press(&mut table, "G"), Handled::Consumed);
    assert_eq!(table.lazy_load(), None);
}

#[test]
fn moving_within_five_rows_of_the_end_requests_the_next_page() {
    let mut table = thirty();
    down_to(&mut table, 24);
    assert_eq!(press(&mut table, "j"), load_more(0, 2, 30));
    assert_eq!(table.cursor(), 25);
    assert_eq!(table.lazy_load().and_then(LazyLoad::in_flight), Some(2));
}

#[test]
fn a_move_that_leaves_the_cursor_in_place_runs_no_check() {
    let mut table = table();
    table.enable_lazy_load(30, 1);
    assert_eq!(
        table.insert_many(prs(1..=3), &()),
        Some(TableEvent::LoadMore {
            generation: 0,
            page: 2,
            per_page: 30
        })
    );
    let page = table.page_loaded(0, 2, Ok(prs(4..=6).into()), &());
    assert!(page.applied);
    // Page 2 landed with nothing seen meanwhile, so nothing re-checks until the cursor moves.
    assert_eq!(page.event, None);
    // `k` at the top doesn't move the cursor, so Textual posts no highlight.
    assert_eq!(press(&mut table, "k"), Handled::Consumed);
    assert_eq!(press(&mut table, "j"), load_more(0, 3, 30));
}

#[test]
fn a_short_first_page_requests_page_two_as_soon_as_it_is_inserted() {
    let mut table = table();
    table.enable_lazy_load(30, 1);
    assert_eq!(
        table.insert_many(prs(1..=3), &()),
        Some(TableEvent::LoadMore {
            generation: 0,
            page: 2,
            per_page: 30
        })
    );
}

#[test]
fn rows_added_to_a_non_empty_table_run_no_check() {
    // Cached rows came first (lazy loading off), then the fetched first page: Textual posts no highlight.
    let mut table = table();
    assert_eq!(table.insert_many(prs(1..=3), &()), None);
    table.enable_lazy_load(30, 1);
    assert_eq!(table.insert_many(prs(4..=6), &()), None);
}

#[test]
fn a_single_insert_into_an_empty_table_runs_the_check() {
    let mut table = table();
    table.enable_lazy_load(30, 1);
    assert_eq!(
        table.insert(Pr { number: 1 }, &()),
        Some(TableEvent::LoadMore {
            generation: 0,
            page: 2,
            per_page: 30
        })
    );
}

#[test]
fn an_empty_page_stops_further_requests() {
    let mut table = thirty();
    assert_eq!(press(&mut table, "G"), load_more(0, 2, 30));
    let page = table.page_loaded(0, 2, Ok(Vec::new().into()), &());
    assert!(
        page.applied,
        "the owner still writes its cache, as Python's add_items does"
    );
    assert_eq!(page.event, None);
    let lazy = table.lazy_load().unwrap();
    assert_eq!((lazy.current_batch(), lazy.can_load_more()), (2, false));
    assert_eq!(press(&mut table, "k"), Handled::Consumed);
    assert_eq!(press(&mut table, "j"), Handled::Consumed);
}

#[test]
fn a_page_with_no_items_that_is_not_the_last_keeps_paging_on() {
    // A PR table's page 2 held only issues: nothing to add, but the API has more (Py bug B44).
    let mut table = thirty();
    assert_eq!(press(&mut table, "G"), load_more(0, 2, 30));
    let page = table.page_loaded(
        0,
        2,
        Ok(Page {
            items: Vec::new(),
            last: false,
        }),
        &(),
    );
    assert!(page.applied);
    assert_eq!(page.event, None);
    let lazy = table.lazy_load().unwrap();
    assert_eq!(
        (lazy.current_batch(), lazy.can_load_more(), lazy.in_flight()),
        (2, true, None)
    );
    // No eager chaining: the next cursor move asks for page 3.
    assert_eq!(press(&mut table, "k"), load_more(0, 3, 30));
}

#[test]
fn a_last_page_with_items_ends_paging() {
    let mut table = thirty();
    assert_eq!(press(&mut table, "G"), load_more(0, 2, 30));
    let page = table.page_loaded(
        0,
        2,
        Ok(Page {
            items: prs(31..=32),
            last: true,
        }),
        &(),
    );
    assert!(page.applied);
    assert_eq!(table.items().len(), 32);
    assert!(!table.lazy_load().unwrap().can_load_more());
    assert_eq!(press(&mut table, "G"), Handled::Consumed);
}

#[test]
fn a_page_in_flight_suppresses_duplicates_and_the_pending_cursor_is_rechecked_once() {
    let mut table = thirty();
    down_to(&mut table, 24);
    assert_eq!(press(&mut table, "j"), load_more(0, 2, 30));
    // Two more moves while page 2 is in flight ask for nothing.
    assert_eq!(press(&mut table, "j"), Handled::Consumed);
    assert_eq!(press(&mut table, "j"), Handled::Consumed);
    assert_eq!(press(&mut table, "k"), Handled::Consumed);
    // A short page 2: 32 items, and the highest cursor seen (27) leaves 5, so page 3 is asked for at once.
    let page = table.page_loaded(0, 2, Ok(prs(31..=32).into()), &());
    assert!(page.applied);
    assert_eq!(
        page.event,
        Some(TableEvent::LoadMore {
            generation: 0,
            page: 3,
            per_page: 30
        })
    );
    // The re-check ran once: page 3 landing has nothing left to re-check.
    let page = table.page_loaded(0, 3, Ok(prs(33..=40).into()), &());
    assert!(page.applied);
    assert_eq!(page.event, None);
    assert_eq!(table.lazy_load().unwrap().current_batch(), 3);
}

#[test]
fn a_pending_cursor_far_from_the_end_asks_for_nothing() {
    let mut table = thirty();
    down_to(&mut table, 24);
    assert_eq!(press(&mut table, "j"), load_more(0, 2, 30));
    assert_eq!(press(&mut table, "j"), Handled::Consumed);
    let page = table.page_loaded(0, 2, Ok(prs(31..=60).into()), &());
    assert!(page.applied);
    assert_eq!(page.event, None);
    assert_eq!(table.items().len(), 60);
}

#[test]
fn an_error_keeps_the_state_and_the_next_move_retries() {
    let mut table = thirty();
    down_to(&mut table, 24);
    assert_eq!(press(&mut table, "j"), load_more(0, 2, 30));
    assert_eq!(press(&mut table, "j"), Handled::Consumed);
    let page = table.page_loaded(0, 2, Err(http_error()), &());
    assert_eq!(
        page,
        PageLoaded {
            applied: false,
            event: None
        }
    );
    let lazy = table.lazy_load().unwrap();
    assert_eq!(
        (lazy.current_batch(), lazy.can_load_more(), lazy.in_flight()),
        (1, true, None)
    );
    assert_eq!(table.items().len(), 30);
    assert_eq!(press(&mut table, "j"), load_more(0, 2, 30));
}

#[test]
fn clear_drops_a_late_page_by_generation() {
    let mut table = thirty();
    assert_eq!(press(&mut table, "G"), load_more(0, 2, 30));
    assert_eq!(table.lazy_load().and_then(LazyLoad::in_flight), Some(2));
    // The user picks another repo: the table is cleared and gets that repo's first page.
    table.clear();
    assert_eq!(table.lazy_load(), None);
    table.enable_lazy_load(30, 1);
    assert_eq!(table.insert_many(prs(101..=130), &()), None);
    // The first repo's page 2 arrives late.
    let page = table.page_loaded(0, 2, Ok(prs(31..=60).into()), &());
    assert!(!page.applied);
    assert_eq!(table.items().len(), 30);
    assert!(table.get(&31).is_none());
    // The new generation's state is untouched.
    let lazy = table.lazy_load().unwrap();
    assert_eq!((lazy.current_batch(), lazy.in_flight()), (1, None));
}

#[test]
fn a_page_nobody_asked_for_is_dropped() {
    let mut table = thirty();
    let page = table.page_loaded(0, 2, Ok(prs(31..=60).into()), &());
    assert!(!page.applied);
    assert_eq!(table.items().len(), 30);
}

#[test]
fn clicking_near_the_end_requests_the_next_page() {
    let mut table = table();
    table.enable_lazy_load(30, 1);
    assert_eq!(table.insert_many(prs(1..=8), &()), None);
    let area = Rect::new(0, 0, 40, 12);
    draw(&mut table, area);
    let up = |row| MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 2,
        row,
        modifiers: KeyModifiers::NONE,
    };
    // Screen row 2 is cursor 1: 7 remaining.
    assert_eq!(table.handle_mouse(up(2), area), Handled::Consumed);
    // Screen row 4 is cursor 3: 5 remaining.
    assert_eq!(table.handle_mouse(up(4), area), load_more(0, 2, 30));
    assert_eq!(table.cursor(), 3);
}

fn draw(table: &mut SearchableTable<Pr>, area: Rect) -> Buffer {
    let theme = Theme::default();
    let keymap = Keymap::defaults();
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let cx = RenderCx {
                theme: &theme,
                keymap: &keymap,
                now: Instant::now(),
                focused: true,
                focus_within: true,
            };
            table.render(frame, frame.area(), &cx);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn lazy_after_page_2() {
    let mut table = table();
    table.enable_lazy_load(4, 1);
    // Page 1 holds the even numbers and page 2 the odd ones, so the rows only come out 8..1 if both pages sort
    // together.
    assert_eq!(
        table.insert_many(prs([8, 6, 4, 2]), &()),
        Some(TableEvent::LoadMore {
            generation: 0,
            page: 2,
            per_page: 4
        })
    );
    let page = table.page_loaded(0, 2, Ok(prs([7, 5, 3, 1]).into()), &());
    assert!(page.applied);
    assert_eq!(numbers(&table), [8, 7, 6, 5, 4, 3, 2, 1]);
    insta::assert_debug_snapshot!(draw(&mut table, Rect::new(0, 0, 30, 10)));
}
