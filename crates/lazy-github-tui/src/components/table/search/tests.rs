use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use tokio::time::Instant;

use super::super::*;
use crate::component::RenderCx;
use crate::keymap::KeySpec;
use crate::test_util::buffer_to_string;
use crate::theme::Theme;

/// A repo-like row: `(favorite, owner, name)`, as Python's `repo_to_row` builds it.
#[derive(Debug, Clone, PartialEq)]
struct Repo {
    id: i64,
    owner: &'static str,
    name: &'static str,
    favorite: bool,
}

impl TableItem for Repo {
    type Key = i64;
    type Ctx = ();

    fn key(&self) -> i64 {
        self.id
    }

    fn cells(&self, (): &()) -> Vec<Cell> {
        let favorite = if self.favorite {
            Cell::new(CellValue::Str("[green]★[/]".into()), "★")
        } else {
            Cell::text("☆")
        };
        vec![favorite, Cell::text(self.owner), Cell::text(self.name)]
    }
}

const REPOS: [(&str, &str, bool); 8] = [
    ("parity-bot", "lazy-github", false),
    ("octo", "gh-cli", false),
    ("parity-bot", "dotfiles", true),
    ("octo", "LazyVim", false),
    ("parity-org", "gh-dash", false),
    ("octo", "spoon-knife", false),
    ("parity-org", "lazygit", true),
    ("parity-bot", "notes", false),
];

fn repo(id: usize) -> Repo {
    let (owner, name, favorite) = REPOS[id - 1];
    Repo {
        id: i64::try_from(id).unwrap(),
        owner,
        name,
        favorite,
    }
}

/// The eight repos, sorted favorites first (`[green]★[/]` sorts before `☆`), then insertion order.
fn table() -> SearchableTable<Repo> {
    let mut table = SearchableTable::new(
        vec![Column::new("★"), Column::new("Owner"), Column::new("Name")],
        SortSpec::ascending(0),
    );
    assert_eq!(table.insert_many((1..=REPOS.len()).map(repo), &()), None);
    table
}

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn press(table: &mut SearchableTable<Repo>, key: &str) -> Handled<TableEvent<i64>> {
    table.handle_key(&chord(key), &Keymap::defaults())
}

/// Types `text` one key per character, as a terminal sends it.
fn type_text(table: &mut SearchableTable<Repo>, text: &str) {
    for c in text.chars() {
        let key = KeyChord::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE);
        assert!(table.consumes_key(&key, &Keymap::defaults()), "{c:?}");
        assert_eq!(table.handle_key(&key, &Keymap::defaults()), Handled::Consumed);
    }
}

/// `/`, the query, `enter`.
fn search(table: &mut SearchableTable<Repo>, query: &str) {
    assert_eq!(press(table, "slash"), Handled::Consumed);
    for _ in 0..table.search_text().chars().count() {
        let _ = press(table, "backspace");
    }
    type_text(table, query);
    assert_eq!(press(table, "enter"), Handled::Consumed);
}

fn ids(table: &SearchableTable<Repo>) -> Vec<i64> {
    table.row_keys().copied().collect()
}

fn draw(table: &mut SearchableTable<Repo>, focused: bool) -> Buffer {
    let theme = Theme::default();
    let keymap = Keymap::defaults();
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| {
            let cx = RenderCx {
                theme: &theme,
                keymap: &keymap,
                now: Instant::now(),
                focused,
                focus_within: focused,
            };
            table.render(frame, frame.area(), &cx);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

// Opening, typing and hiding.

#[test]
fn slash_shows_and_focuses_the_box() {
    let mut table = table();
    assert!(!table.search_visible());
    assert_eq!(table.focus(), TableFocus::Table);
    assert!(!table.consumes_key(&chord("q"), &Keymap::defaults()));
    assert_eq!(press(&mut table, "slash"), Handled::Consumed);
    assert!(table.search_visible());
    assert_eq!(table.focus(), TableFocus::Search);
}

#[test]
fn a_rebound_search_key_opens_the_box() {
    let overrides = [("common.table.search".to_owned(), "question_mark".to_owned())]
        .into_iter()
        .collect();
    let keymap = Keymap::new(&overrides);
    let mut table = table();
    assert_eq!(table.handle_key(&chord("slash"), &keymap), Handled::Ignored);
    assert_eq!(table.handle_key(&chord("question_mark"), &keymap), Handled::Consumed);
    assert_eq!(table.focus(), TableFocus::Search);
    // Inside the box a printable search key types, and one that isn't printable stays the component's.
    assert_eq!(table.handle_key(&chord("question_mark"), &keymap), Handled::Consumed);
    assert_eq!(table.search_text(), "?");
    let overrides = [("common.table.search".to_owned(), "f2".to_owned())]
        .into_iter()
        .collect();
    let keymap = Keymap::new(&overrides);
    assert_eq!(table.handle_key(&chord("f2"), &keymap), Handled::Consumed);
    assert_eq!(table.focus(), TableFocus::Search);
}

#[test]
fn the_box_takes_printable_keys_and_lets_focus_keys_through() {
    let mut table = table();
    let _ = press(&mut table, "slash");
    type_text(&mut table, "qj/ G");
    assert_eq!(table.search_text(), "qj/ G");
    // The table didn't move.
    assert_eq!(table.cursor(), 0);
    for key in ["tab", "shift+tab", "escape"] {
        assert!(!table.consumes_key(&chord(key), &Keymap::defaults()));
        assert_eq!(press(&mut table, key), Handled::Ignored, "{key}");
    }
    assert_eq!(table.focus(), TableFocus::Search);
}

#[test]
fn paste_goes_into_the_focused_box_only() {
    let mut table = table();
    assert_eq!(table.handle_paste("lazy"), Handled::Ignored);
    let _ = press(&mut table, "slash");
    assert_eq!(table.handle_paste("lazy\ngit"), Handled::Consumed);
    assert_eq!(table.search_text(), "lazy");
}

#[test]
fn blurring_a_blank_box_hides_it_and_one_with_text_stays() {
    let mut table = table();
    let _ = press(&mut table, "slash");
    type_text(&mut table, "  ");
    table.focus_table();
    assert!(!table.search_visible());
    assert!(!table.focus_search(), "a hidden box isn't a focus stop");
    assert_eq!(table.focus(), TableFocus::Table);

    // Reopening selects the blank text, so typing replaces it.
    let _ = press(&mut table, "slash");
    type_text(&mut table, "lazy");
    table.focus_table();
    assert!(table.search_visible());
    // Typed but not submitted: nothing is filtered.
    assert_eq!(table.row_count(), 8);
    assert!(table.focus_search());
    assert_eq!(table.search_text(), "lazy", "refocusing keeps the text");
}

#[test]
fn reopening_selects_the_text_so_typing_replaces_it() {
    let mut table = table();
    search(&mut table, "lazy");
    assert_eq!(press(&mut table, "slash"), Handled::Consumed);
    assert_eq!(table.search_text(), "lazy", "reopening keeps the text");
    type_text(&mut table, "gh-");
    assert_eq!(table.search_text(), "gh-");
    assert_eq!(press(&mut table, "enter"), Handled::Consumed);
    assert_eq!(ids(&table), [2, 5]);

    // Shift+Tab back into the box selects it too, and a delete key removes the whole selection.
    assert!(table.focus_search());
    assert_eq!(press(&mut table, "ctrl+w"), Handled::Consumed);
    assert_eq!(table.search_text(), "");

    // `left` collapses the selection to its start, so typing goes in front.
    table.focus_table();
    let _ = press(&mut table, "slash");
    type_text(&mut table, "dash");
    table.focus_table();
    assert!(table.focus_search());
    let _ = press(&mut table, "left");
    type_text(&mut table, "gh-");
    assert_eq!(table.search_text(), "gh-dash");
}

#[test]
fn a_box_that_has_focus_keeps_its_selection_state() {
    let mut table = table();
    let _ = press(&mut table, "slash");
    type_text(&mut table, "lazy");
    // Focusing the box again while it has focus selects nothing new.
    assert!(table.focus_search());
    type_text(&mut table, "git");
    assert_eq!(table.search_text(), "lazygit");
}

// Filtering.

#[test]
fn a_submit_filters_case_insensitively_and_focuses_the_table() {
    let mut table = table();
    search(&mut table, "LAZY");
    // `LazyVim`, `lazygit` and `lazy-github`, favorites first.
    assert_eq!(ids(&table), [7, 1, 4]);
    assert_eq!(table.focus(), TableFocus::Table);
    assert!(table.search_visible());
    assert_eq!(table.search_text(), "LAZY");
}

#[test]
fn the_query_is_trimmed() {
    let mut table = table();
    search(&mut table, "  gh-  ");
    assert_eq!(ids(&table), [2, 5]);
}

#[test]
fn an_empty_query_restores_every_row_and_hides_the_box() {
    let mut table = table();
    search(&mut table, "lazy");
    search(&mut table, "");
    assert_eq!(ids(&table), [3, 7, 1, 2, 4, 5, 6, 8]);
    assert!(!table.search_visible());
}

#[test]
fn the_match_runs_over_the_whole_row_repr() {
    let mut table = table();
    // `('☆', 'octo', 'gh-cli')`: the end of one cell and the start of the next.
    search(&mut table, "', 'gh");
    assert_eq!(ids(&table), [2, 5]);
    // The favorite cell's raw markup.
    search(&mut table, "green");
    assert_eq!(ids(&table), [3, 7]);
    // Every row has a quote and a parenthesis.
    search(&mut table, "(");
    assert_eq!(table.row_count(), 8);
}

#[test]
fn a_submit_resets_the_cursor_and_the_scroll() {
    let mut table = table();
    let _ = draw(&mut table, true);
    for _ in 0..5 {
        let _ = press(&mut table, "j");
    }
    assert_eq!(table.cursor(), 5);
    search(&mut table, "o");
    assert_eq!(table.cursor(), 0);
    assert_eq!(table.scroll(), (0, 0));
}

#[test]
fn a_submit_puts_ties_back_in_items_order() {
    let mut table = table();
    // Re-adding 2 sends its row to the end of its tie group, but `items` keeps it second.
    assert_eq!(table.insert(repo(2), &()), None);
    assert_eq!(ids(&table), [3, 7, 1, 4, 5, 6, 8, 2]);
    search(&mut table, "");
    assert_eq!(ids(&table), [3, 7, 1, 2, 4, 5, 6, 8]);
}

#[test]
fn rows_inserted_after_a_filter_show_even_when_they_dont_match() {
    let mut table = table();
    search(&mut table, "lazy");
    let mut extra = repo(8);
    extra.id = 9;
    assert_eq!(table.insert(extra, &()), None);
    assert_eq!(ids(&table), [7, 1, 4, 9]);
    // A hidden key that's re-added shows again.
    assert_eq!(table.insert(repo(6), &()), None);
    assert_eq!(ids(&table), [7, 1, 4, 9, 6]);
    // And the next search sees every item again.
    search(&mut table, "notes");
    assert_eq!(ids(&table), [8, 9]);
}

#[test]
fn removing_a_hidden_item_forgets_it() {
    let mut table = table();
    search(&mut table, "lazy");
    assert_eq!(table.remove(&2).map(|r| r.name), Some("gh-cli"));
    search(&mut table, "");
    assert_eq!(ids(&table), [3, 7, 1, 4, 5, 6, 8]);
}

#[test]
fn clear_empties_the_items_but_keeps_the_box() {
    let mut table = table();
    search(&mut table, "lazy");
    table.clear();
    assert_eq!(table.row_count(), 0);
    assert!(table.search_visible());
    assert_eq!(table.search_text(), "lazy");
    assert_eq!(table.insert_many((1..=3).map(repo), &()), None);
    assert_eq!(ids(&table), [3, 1, 2], "the old filter doesn't apply to new items");
}

#[test]
fn clicks_move_focus_between_the_box_and_the_table() {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let area = Rect::new(0, 0, 40, 12);
    let click = |row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let mut table = table();
    search(&mut table, "lazy");
    assert_eq!(table.search_area(area), Some(Rect::new(0, 0, 40, 3)));
    // Focusing selects everything, then the press puts the cursor under it: between `la` and `zy`.
    assert_eq!(table.handle_mouse(click(1), area), Handled::Consumed);
    assert_eq!(table.focus(), TableFocus::Search);
    type_text(&mut table, "-");
    assert_eq!(table.search_text(), "la-zy");
    // The table's first row is below the box, its margin and the header.
    let _ = table.handle_mouse(click(5), area);
    assert_eq!(table.focus(), TableFocus::Table);
}

// Lazy loading (#102).

/// `/`, the query, `enter`, returning what `enter` did.
fn submit(table: &mut SearchableTable<Repo>, query: &str) -> Handled<TableEvent<i64>> {
    let _ = press(table, "slash");
    let _ = press(table, "backspace");
    type_text(table, query);
    press(table, "enter")
}

#[test]
fn a_submit_runs_the_lazy_check_for_the_first_row() {
    // Four items: row 0 is within 5 of the end, so a lazy table asks for the next page.
    let mut table = SearchableTable::new(
        vec![Column::new("★"), Column::new("Owner"), Column::new("Name")],
        SortSpec::ascending(0),
    );
    assert_eq!(table.insert_many((1..=4).map(repo), &()), None);
    table.enable_lazy_load(30, 1);
    let generation = table.generation();
    assert_eq!(
        submit(&mut table, "lazy"),
        Handled::Event(TableEvent::LoadMore {
            generation,
            page: 2,
            per_page: 30,
        })
    );
    assert_eq!(ids(&table), [1, 4]);
    assert_eq!(table.lazy_load().and_then(LazyLoad::in_flight), Some(2));
}

#[test]
fn a_submit_that_matches_nothing_runs_no_lazy_check() {
    let mut table = SearchableTable::new(
        vec![Column::new("★"), Column::new("Owner"), Column::new("Name")],
        SortSpec::ascending(0),
    );
    assert_eq!(table.insert_many((1..=4).map(repo), &()), None);
    table.enable_lazy_load(30, 1);
    assert_eq!(submit(&mut table, "nothing matches this"), Handled::Consumed);
    assert_eq!(table.row_count(), 0);
    assert_eq!(table.lazy_load().and_then(LazyLoad::in_flight), None);
}

#[test]
fn the_lazy_check_after_a_submit_counts_hidden_items() {
    // Three rows are left, but all eight items count, as Python's `len(self.items)` does: 8 - 0 > 5.
    let mut table = table();
    table.enable_lazy_load(30, 1);
    assert_eq!(submit(&mut table, "lazy"), Handled::Consumed);
    assert_eq!(ids(&table), [7, 1, 4]);
    assert_eq!(table.lazy_load().and_then(LazyLoad::in_flight), None);
}

// Snapshots, 40x12 in textual-dark.

#[test]
fn search_open_empty() {
    let mut table = table();
    let _ = press(&mut table, "slash");
    let buffer = draw(&mut table, true);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn search_typed() {
    let mut table = table();
    let _ = press(&mut table, "slash");
    type_text(&mut table, "lazy");
    let buffer = draw(&mut table, true);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn search_reopened_selected() {
    let mut table = table();
    search(&mut table, "lazy");
    let _ = press(&mut table, "slash");
    let buffer = draw(&mut table, true);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn search_submitted_filtered() {
    let mut table = table();
    search(&mut table, "lazy");
    let buffer = draw(&mut table, true);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn search_closed_after_empty_submit() {
    let mut table = table();
    search(&mut table, "lazy");
    search(&mut table, "");
    let buffer = draw(&mut table, true);
    // The table is back at the top of the area.
    assert!(buffer_to_string(&buffer).starts_with(" ★  Owner"));
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn a_search_measures_the_rows_it_leaves() {
    let mut table = table();
    assert_eq!(table.widths()[2], "lazy-github".len() + 2);
    let _ = submit(&mut table, "gh-");
    assert_eq!(table.widths(), render::column_widths(&table.columns, &table.rows));
    assert_eq!(table.widths()[2], "gh-dash".len() + 2);
}
