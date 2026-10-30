use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use indexmap::IndexMap;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tokio::time::Instant;

use super::*;
use crate::keymap::KeySpec;
use crate::test_util::buffer_to_string;
use crate::theme::Theme;

/// A test row: a favorite marker, an owner, a long title and a number.
#[derive(Debug, Clone, PartialEq)]
struct Item {
    id: i64,
    owner: &'static str,
    title: String,
    favorite: bool,
}

/// What the cells need: the favorite marker's color, standing in for an adapter's context.
struct Cx {
    star: Color,
}

impl TableItem for Item {
    type Key = i64;
    type Ctx = Cx;

    fn key(&self) -> i64 {
        self.id
    }

    fn cells(&self, cx: &Cx) -> Vec<Cell> {
        let favorite = if self.favorite {
            Cell::new(
                CellValue::Str("[green]★[/]".into()),
                Span::styled("★", Style::new().fg(cx.star)),
            )
        } else {
            Cell::new(CellValue::Str("☆".into()), "☆")
        };
        vec![
            favorite,
            Cell::text(self.owner),
            Cell::text(self.title.clone()),
            Cell::int(self.id),
        ]
    }
}

/// Long enough to take more than two 38-column pages.
const LONG_TITLE: &str = "A deliberately long title that overflows the forty-column table, twice over at the least";

const CX: Cx = Cx {
    star: Color::Rgb(0x98, 0xe0, 0x24),
};

fn item(id: i64, favorite: bool) -> Item {
    let owner = if id % 2 == 0 { "parity-bot" } else { "octo" };
    let title = if id == 3 {
        LONG_TITLE.to_owned()
    } else {
        format!("Item {id}")
    };
    Item {
        id,
        owner,
        title,
        favorite,
    }
}

fn columns() -> Vec<Column> {
    vec![
        Column::new(Span::styled("★", Style::new().fg(CX.star))),
        Column::new("Owner"),
        Column::new("Title"),
        Column::new("#"),
    ]
}

/// 12 rows sorted by the favorite column, ids 1 to 12 with 4 and 9 favorited, as in a repos table.
fn twelve() -> SearchableTable<Item> {
    let mut table = SearchableTable::new(columns(), SortSpec::ascending(0));
    let _ = table.insert_many((1..=12).map(|id| item(id, id == 4 || id == 9)), &CX);
    table
}

fn by_id() -> SearchableTable<Item> {
    SearchableTable::new(columns(), SortSpec::ascending(3))
}

fn draw<T: TableItem>(table: &mut SearchableTable<T>, focused: bool, width: u16, height: u16) -> Buffer {
    let theme = Theme::default();
    let keymap = Keymap::defaults();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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

/// The 40x8 snapshot area.
fn snap(table: &mut SearchableTable<Item>, focused: bool) -> Buffer {
    draw(table, focused, 40, 8)
}

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn press<T: TableItem>(table: &mut SearchableTable<T>, keymap: &Keymap, key: &str) -> Handled<TableEvent<T::Key>> {
    table.handle_key(&chord(key), keymap, "")
}

fn ids(table: &SearchableTable<Item>) -> Vec<i64> {
    table.row_keys().copied().collect()
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 40,
    height: 8,
};

// Sorting and inserting.

#[test]
fn favorites_sort_first_and_ties_keep_insertion_order() {
    let table = twelve();
    assert_eq!(ids(&table), [4, 9, 1, 2, 3, 5, 6, 7, 8, 10, 11, 12]);
}

#[test]
fn a_descending_sort_keeps_ties_in_insertion_order() {
    let mut table = SearchableTable::new(columns(), SortSpec::descending(0));
    let _ = table.insert_many((1..=5).map(|id| item(id, id == 2)), &CX);
    // `☆` is above `[green]★[/]`, so the favorite goes last; the other four stay in insertion order.
    assert_eq!(ids(&table), [1, 3, 4, 5, 2]);
}

#[test]
fn a_reinserted_key_goes_to_the_end_of_its_tie_group_and_keeps_its_item_slot() {
    let mut table = twelve();
    let mut changed = item(1, false);
    changed.title = "Renamed".into();
    let _ = table.insert(changed, &CX);
    assert_eq!(ids(&table), [4, 9, 2, 3, 5, 6, 7, 8, 10, 11, 12, 1]);
    let items: Vec<i64> = table.items().map(|i| i.id).collect();
    assert_eq!(items, (1..=12).collect::<Vec<_>>());
    assert_eq!(table.get(&1).map(|i| i.title.as_str()), Some("Renamed"));
    assert_eq!(table.row_count(), 12);

    // Descending, the same: the end of its tie group.
    let mut table = SearchableTable::new(columns(), SortSpec::descending(0));
    let _ = table.insert_many((1..=4).map(|id| item(id, false)), &CX);
    let _ = table.insert(item(2, false), &CX);
    assert_eq!(ids(&table), [1, 3, 4, 2]);
}

/// The favorites, by id: what [`Repo`]'s star comes from, as the repos table's comes from the config.
struct Favs(Vec<i64>);

/// A row whose star follows [`Favs`], so [`SearchableTable::refresh_rows`] can change it.
#[derive(Debug)]
struct Repo(i64);

impl TableItem for Repo {
    type Key = i64;
    type Ctx = Favs;
    fn key(&self) -> i64 {
        self.0
    }
    fn cells(&self, cx: &Favs) -> Vec<Cell> {
        let star = if cx.0.contains(&self.0) { "[green]★[/]" } else { "☆" };
        vec![Cell::text(star), Cell::int(self.0)]
    }
}

fn starred(n: i64) -> SearchableTable<Repo> {
    let mut table = SearchableTable::new(vec![Column::new("★"), Column::new("#")], SortSpec::ascending(0));
    let _ = table.insert_many((1..=n).map(Repo), &Favs(vec![]));
    table
}

#[test]
fn refresh_rows_recomputes_cells_and_resorts() {
    let mut table = starred(4);
    table.refresh_rows(&Favs(vec![3]));
    assert_eq!(table.row_keys().copied().collect::<Vec<_>>(), [3, 1, 2, 4]);
    table.refresh_rows(&Favs(vec![]));
    assert_eq!(table.row_keys().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);
}

#[test]
fn refresh_rows_keeps_the_cursor_index_and_the_scroll() {
    let keymap = Keymap::defaults();
    let mut table = starred(12);
    // Seven body rows: the cursor on row 9 scrolls the body down three.
    draw(&mut table, true, 40, 8);
    for _ in 0..9 {
        let _ = press(&mut table, &keymap, "j");
    }
    let before = (table.cursor(), table.scroll());
    assert_eq!(before, (9, (3, 0)));
    assert_eq!(table.selected().map(|repo| repo.0), Some(10));
    // Favoriting the row under the cursor moves it to the top; the cursor stays on row 9, now the 9th id.
    table.refresh_rows(&Favs(vec![10]));
    assert_eq!((table.cursor(), table.scroll()), before);
    assert_eq!(table.selected().map(|repo| repo.0), Some(9));
    draw(&mut table, true, 40, 8);
    assert_eq!((table.cursor(), table.scroll()), before);
}

/// Textual's `Column.content_width` only grows as rows are added: removing, clearing and searching keep it.
#[test]
fn widths_are_a_high_water_mark_over_every_added_row() {
    let title = |table: &SearchableTable<Item>| table.widths()[2];
    let mut table = SearchableTable::new(columns(), SortSpec::ascending(3));
    let empty = table.widths().to_vec();
    assert_eq!(empty, render::column_widths(&table.columns, &table.rows));

    let _ = table.insert(item(1, false), &CX);
    let short = title(&table);
    assert!(short > empty[2]);
    let _ = table.insert_many([item(2, false), item(3, false)], &CX);
    let long = table.widths().to_vec();
    assert_eq!(long[2], LONG_TITLE.len() + 2);

    assert!(table.remove(&3).is_some());
    assert_eq!(table.widths(), long, "removing a row keeps its width");
    table.clear();
    assert_eq!(table.widths(), long, "clearing keeps the columns' widths");
    table.replace_all([item(1, false)], &CX);
    assert_eq!(title(&table), long[2]);
    assert_eq!(render::column_widths(&table.columns, &table.rows)[2], short);
}

#[test]
fn refresh_rows_measures_the_new_cells() {
    struct Titles(&'static str);
    #[derive(Debug)]
    struct Row(i64);
    impl TableItem for Row {
        type Key = i64;
        type Ctx = Titles;
        fn key(&self) -> i64 {
            self.0
        }
        fn cells(&self, cx: &Titles) -> Vec<Cell> {
            vec![Cell::int(self.0), Cell::text(cx.0)]
        }
    }
    let mut rows = SearchableTable::new(vec![Column::new("#"), Column::new("T")], SortSpec::ascending(0));
    let _ = rows.insert(Row(1), &Titles("a"));
    assert_eq!(rows.widths()[1], 3);
    rows.refresh_rows(&Titles("a longer title"));
    assert_eq!(rows.widths()[1], "a longer title".len() + 2);
}

// The cursor.

#[test]
fn the_cursor_stays_at_its_index_through_inserts_and_sorts() {
    let keymap = Keymap::defaults();
    let mut table = by_id();
    let _ = table.insert_many([5, 7, 9].map(|id| item(id, false)), &CX);
    snap(&mut table, true);
    let _ = press(&mut table, &keymap, "j");
    assert_eq!((table.cursor(), table.selected().map(|i| i.id)), (1, Some(7)));
    // A row sorted in above the cursor shifts the items, not the cursor.
    let _ = table.insert(item(1, false), &CX);
    assert_eq!((table.cursor(), table.selected().map(|i| i.id)), (1, Some(5)));
    let _ = table.insert(item(8, false), &CX);
    assert_eq!(table.cursor(), 1);
}

#[test]
fn removals_clamp_the_cursor() {
    let keymap = Keymap::defaults();
    let mut table = by_id();
    let _ = table.insert_many((1..=4).map(|id| item(id, false)), &CX);
    snap(&mut table, true);
    let _ = press(&mut table, &keymap, "G");
    assert_eq!(table.cursor(), 3);
    assert_eq!(table.remove(&4).map(|i| i.id), Some(4));
    assert_eq!(table.cursor(), 2);
    assert_eq!(table.remove(&1).map(|i| i.id), Some(1));
    assert_eq!(table.cursor(), 1);
    assert_eq!(table.selected().map(|i| i.id), Some(3));
    assert_eq!(table.remove(&99), None);
    table.remove(&2);
    table.remove(&3);
    assert_eq!((table.cursor(), table.selected()), (0, None));
}

#[test]
fn replacing_the_last_row_under_the_cursor_moves_the_cursor_up() {
    // Textual's `remove_row` clamps the cursor before the new row is appended.
    let keymap = Keymap::defaults();
    let mut table = by_id();
    let _ = table.insert_many((1..=3).map(|id| item(id, false)), &CX);
    snap(&mut table, true);
    let _ = press(&mut table, &keymap, "G");
    let _ = table.insert(item(3, false), &CX);
    assert_eq!(table.cursor(), 1);
}

#[test]
fn clear_resets_the_cursor_and_scroll_and_bumps_the_generation() {
    let keymap = Keymap::defaults();
    let mut table = twelve();
    snap(&mut table, true);
    let _ = press(&mut table, &keymap, "G");
    let _ = press(&mut table, &keymap, "$");
    assert_ne!(table.scroll(), (0, 0));
    let generation = table.generation();
    table.clear();
    assert_eq!((table.cursor(), table.scroll(), table.row_count()), (0, (0, 0), 0));
    assert_eq!(table.items().len(), 0);
    assert_eq!(table.generation(), generation + 1);
    table.replace_all([item(2, false)], &CX);
    assert_eq!(ids(&table), [2]);
    assert_eq!(table.generation(), generation + 2);
}

// Keys.

#[test]
fn every_binding_resolves_through_the_keymap() {
    let keymap = Keymap::defaults();
    let mut table = twelve();
    snap(&mut table, true);
    // 40x8 with both scrollbars: 6 rows fit below the header, 38 columns beside the vertical bar.
    for (key, cursor) in [
        ("j", 1),
        ("down", 2),
        ("k", 1),
        ("up", 0),
        ("J", 6),
        ("K", 0),
        ("pagedown", 6),
        ("pageup", 0),
        ("G", 11),
        ("g", 0),
        ("ctrl+end", 11),
        ("ctrl+home", 0),
    ] {
        assert_eq!(press(&mut table, &keymap, key), Handled::Consumed, "{key}");
        assert_eq!(table.cursor(), cursor, "{key}");
    }
    let content = 3 + 12 + (LONG_TITLE.len() + 2) + 4;
    let max_x = content - 38;
    // More than one page and less than two, so the second page clamps.
    assert!(38 < max_x && max_x < 2 * 38);
    for (key, x) in [
        ("l", 1),
        ("right", 2),
        ("h", 1),
        ("left", 0),
        ("$", 38),
        ("ctrl+pagedown", max_x),
        ("^", max_x - 38),
        ("ctrl+pageup", 0),
        ("end", max_x),
        ("home", 0),
    ] {
        assert_eq!(press(&mut table, &keymap, key), Handled::Consumed, "{key}");
        assert_eq!(table.scroll().1, x, "{key}");
    }
    assert_eq!(
        press(&mut table, &keymap, "enter"),
        Handled::Event(TableEvent::Selected(4))
    );
    assert_eq!(
        press(&mut table, &keymap, "space"),
        Handled::Event(TableEvent::Selected(4))
    );
}

#[test]
fn shift_h_and_l_are_not_table_keys() {
    // Py bug B9: Python's later `^`/`$` definitions replace `H`/`L`.
    let keymap = Keymap::defaults();
    let mut table = twelve();
    snap(&mut table, true);
    // `/` opens the search box (`search::tests`).
    for key in ["H", "L", "x", "q", "1", "ctrl+f"] {
        assert_eq!(press(&mut table, &keymap, key), Handled::Ignored, "{key}");
    }
    assert_eq!((table.cursor(), table.scroll()), (0, (0, 0)));
}

#[test]
fn an_override_replaces_the_vim_key_but_not_textuals() {
    let overrides: IndexMap<String, String> = [("common.table.cursor_down".to_owned(), "n".to_owned())].into();
    let keymap = Keymap::new(&overrides);
    let mut table = twelve();
    snap(&mut table, true);
    assert_eq!(press(&mut table, &keymap, "j"), Handled::Ignored);
    assert_eq!(table.cursor(), 0);
    assert_eq!(press(&mut table, &keymap, "n"), Handled::Consumed);
    assert_eq!(press(&mut table, &keymap, "down"), Handled::Consumed);
    assert_eq!(table.cursor(), 2);
}

#[test]
fn rebinding_select_takes_enter_with_it() {
    // `common.table.select` replaced `DataTable`'s own `enter` binding, so rebinding it leaves `enter` unbound.
    // Python does the same: the merged table has no other binding on `enter`, so Rust's rule that an override
    // strips only its own keys (decisions/override-strips-only-its-own-keys.md, #119) changes nothing here.
    let overrides: IndexMap<String, String> = [("common.table.select".to_owned(), "o".to_owned())].into();
    let keymap = Keymap::new(&overrides);
    let mut table = twelve();
    assert_eq!(press(&mut table, &keymap, "enter"), Handled::Ignored);
    assert_eq!(press(&mut table, &keymap, "space"), Handled::Ignored);
    assert_eq!(press(&mut table, &keymap, "o"), Handled::Event(TableEvent::Selected(4)));
}

#[test]
fn select_on_an_empty_table_reports_nothing() {
    let keymap = Keymap::defaults();
    let mut table = by_id();
    assert_eq!(press(&mut table, &keymap, "enter"), Handled::Consumed);
    assert_eq!(table.selected(), None);
}

#[test]
fn one_column_scrolls_fall_through_when_the_columns_fit() {
    let keymap = Keymap::defaults();
    let mut table = by_id();
    let _ = table.insert_many([1, 2].map(|id| item(id, false)), &CX);
    snap(&mut table, true);
    // Textual's `scroll_left`/`scroll_right` raise `SkipAction` without a horizontal scrollbar.
    for key in ["l", "h", "right", "left"] {
        assert_eq!(press(&mut table, &keymap, key), Handled::Ignored, "{key}");
    }
    // Paging, home and end do nothing but still take the key.
    for key in ["$", "^", "home", "end", "ctrl+pagedown"] {
        assert_eq!(press(&mut table, &keymap, key), Handled::Consumed, "{key}");
    }
    assert_eq!(table.scroll(), (0, 0));
}

#[test]
fn keys_work_while_loading() {
    let keymap = Keymap::defaults();
    let mut table = twelve();
    snap(&mut table, true);
    table.set_loading(true);
    assert_eq!(press(&mut table, &keymap, "j"), Handled::Consumed);
    assert_eq!(table.cursor(), 1);
    assert_eq!(
        press(&mut table, &keymap, "enter"),
        Handled::Event(TableEvent::Selected(9))
    );
}

// The mouse.

#[test]
fn clicking_a_row_moves_the_cursor_and_clicking_its_cell_again_selects() {
    let mut table = twelve();
    snap(&mut table, true);
    let up = MouseEventKind::Up(MouseButton::Left);
    // Row 3 of the body (screen row 3), in the Owner column.
    assert_eq!(table.handle_mouse(mouse(up, 6, 3), AREA), Handled::Consumed);
    assert_eq!(table.cursor(), 2);
    // Another column of the same row only moves the coordinate.
    assert_eq!(table.handle_mouse(mouse(up, 20, 3), AREA), Handled::Consumed);
    assert_eq!(
        table.handle_mouse(mouse(up, 25, 3), AREA),
        Handled::Event(TableEvent::Selected(1))
    );
    // The header, the scrollbars and the space outside do nothing.
    assert_eq!(table.handle_mouse(mouse(up, 6, 0), AREA), Handled::Ignored);
    assert_eq!(table.handle_mouse(mouse(up, 39, 3), AREA), Handled::Ignored);
    assert_eq!(table.handle_mouse(mouse(up, 6, 7), AREA), Handled::Ignored);
    assert_eq!(table.handle_mouse(mouse(up, 50, 3), AREA), Handled::Ignored);
    assert_eq!(table.cursor(), 2);
}

#[test]
fn the_first_click_on_cell_zero_zero_selects_at_once() {
    // The cursor coordinate starts at (0, 0), so the first click there is a click on the cursor's cell.
    let mut table = twelve();
    snap(&mut table, true);
    let up = MouseEventKind::Up(MouseButton::Left);
    assert_eq!(
        table.handle_mouse(mouse(up, 1, 1), AREA),
        Handled::Event(TableEvent::Selected(4))
    );
}

#[test]
fn the_wheel_scrolls_without_moving_the_cursor() {
    let mut table = twelve();
    snap(&mut table, true);
    assert_eq!(
        table.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 3), AREA),
        Handled::Consumed
    );
    assert_eq!((table.cursor(), table.scroll()), (0, (2, 0)));
    for _ in 0..3 {
        let _ = table.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 3), AREA);
    }
    // At the end the wheel bubbles, as Textual's does when nothing scrolled.
    assert_eq!(table.scroll(), (6, 0));
    assert_eq!(
        table.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 3), AREA),
        Handled::Ignored
    );
    let mut shifted = mouse(MouseEventKind::ScrollDown, 5, 3);
    shifted.modifiers = KeyModifiers::SHIFT;
    assert_eq!(table.handle_mouse(shifted, AREA), Handled::Consumed);
    assert_eq!(table.scroll(), (6, 4));
    assert_eq!(
        table.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 3), AREA),
        Handled::Consumed
    );
    assert_eq!((table.cursor(), table.scroll()), (0, (4, 4)));

    // A table that fits doesn't take the wheel.
    let mut short = by_id();
    let _ = short.insert(item(1, false), &CX);
    snap(&mut short, true);
    assert_eq!(
        short.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 1), AREA),
        Handled::Ignored
    );
}

#[test]
fn hover_follows_the_mouse_and_keys_hide_it() {
    let keymap = Keymap::defaults();
    let mut table = twelve();
    snap(&mut table, true);
    assert_eq!(
        table.handle_mouse(mouse(MouseEventKind::Moved, 5, 3), AREA),
        Handled::Consumed
    );
    assert_eq!(table.hover, Some(2));
    let _ = press(&mut table, &keymap, "j");
    assert_eq!(table.hover, None);
    let _ = table.handle_mouse(mouse(MouseEventKind::Moved, 5, 3), AREA);
    let _ = table.handle_mouse(mouse(MouseEventKind::Moved, 50, 3), AREA);
    assert_eq!(table.hover, None);
}

#[test]
fn the_loading_overlay_swallows_the_mouse() {
    let mut table = twelve();
    snap(&mut table, true);
    table.set_loading(true);
    let up = MouseEventKind::Up(MouseButton::Left);
    assert_eq!(table.handle_mouse(mouse(up, 6, 3), AREA), Handled::Consumed);
    assert_eq!(table.cursor(), 0);
    let now = Instant::now();
    assert_eq!(table.next_deadline(now), Some(now + loading::FRAME_INTERVAL));
    let mut still = twelve().with_options(AppOptions {
        animations: false,
        ..AppOptions::default()
    });
    still.set_loading(true);
    assert_eq!(still.next_deadline(now), None);
    table.set_loading(false);
    assert_eq!(table.next_deadline(now), None);
}

// Rendering.

#[test]
fn table_initial_focused() {
    let mut table = twelve();
    let buffer = snap(&mut table, true);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn table_initial_blurred() {
    let mut table = twelve();
    let buffer = snap(&mut table, false);
    insta::assert_debug_snapshot!(buffer);
}

#[test]
#[allow(non_snake_case)]
fn table_after_J() {
    let mut table = twelve();
    snap(&mut table, true);
    let _ = press(&mut table, &Keymap::defaults(), "J");
    insta::assert_debug_snapshot!(snap(&mut table, true));
}

#[test]
#[allow(non_snake_case)]
fn table_after_G() {
    let mut table = twelve();
    snap(&mut table, true);
    let _ = press(&mut table, &Keymap::defaults(), "G");
    insta::assert_debug_snapshot!(snap(&mut table, true));
}

#[test]
fn table_scrolled_right() {
    let mut table = twelve();
    snap(&mut table, true);
    let _ = press(&mut table, &Keymap::defaults(), "$");
    insta::assert_debug_snapshot!(snap(&mut table, true));
}

#[test]
fn table_empty() {
    let mut table = by_id();
    insta::assert_debug_snapshot!(snap(&mut table, true));
}

#[test]
fn table_loading_static() {
    let mut table = twelve().with_options(AppOptions {
        animations: false,
        ..AppOptions::default()
    });
    table.set_loading(true);
    insta::assert_debug_snapshot!(snap(&mut table, true));
}

#[test]
fn table_hover() {
    let mut table = twelve();
    snap(&mut table, true);
    let _ = table.handle_mouse(mouse(MouseEventKind::Moved, 5, 3), AREA);
    insta::assert_debug_snapshot!(snap(&mut table, true));
}

#[test]
fn colors_match_the_python_repos_table() {
    // Measured from Python's 160x48 repos table under textual-dark, focused.
    let mut table = twelve();
    let buffer = snap(&mut table, true);
    let style = |x: u16, y: u16| {
        let cell = &buffer[(x, y)];
        (cell.fg, cell.bg, cell.modifier.contains(Modifier::BOLD))
    };
    let rgb = |r, g, b| Color::Rgb(r, g, b);
    // The header: bold `$foreground` on `$panel` tinted, the star keeping its own color, across the whole row.
    assert_eq!(style(0, 0), (rgb(0xe0, 0xe0, 0xe0), rgb(0x2d, 0x37, 0x40), true));
    assert_eq!(style(1, 0), (CX.star, rgb(0x2d, 0x37, 0x40), true));
    // The cursor row: `$block-cursor-*`, bold, with the cursor's foreground over the star's.
    assert_eq!(style(1, 1), (rgb(0xdd, 0xed, 0xf9), rgb(0x01, 0x78, 0xd4), true));
    // Other rows: `$surface` tinted.
    assert_eq!(style(5, 2), (rgb(0xe0, 0xe0, 0xe0), rgb(0x27, 0x27, 0x27), false));
    // The scrollbar track and the corner.
    assert_eq!(buffer[(38, 7)].bg, rgb(0, 0, 0));
    assert_eq!(buffer[(39, 7)].fg, Color::Reset);

    let blurred = snap(&mut table, false);
    let cell = &blurred[(1, 1)];
    // `$block-cursor-blurred-background` (#0178d4 at 30%) over `$surface`, untinted.
    assert_eq!(
        (cell.bg, cell.modifier.contains(Modifier::BOLD)),
        (rgb(0x15, 0x38, 0x54), false)
    );
    assert_eq!(blurred[(5, 2)].bg, rgb(0x1e, 0x1e, 0x1e));
}

#[test]
fn a_short_table_is_only_as_tall_as_its_rows() {
    let mut table = by_id();
    let _ = table.insert_many([1, 2].map(|id| item(id, false)), &CX);
    let buffer = draw(&mut table, true, 30, 6);
    assert_eq!(
        buffer_to_string(&buffer),
        " ★  Owner       Title   #\n ☆  octo        Item 1  1\n ☆  parity-bot  Item 2  2\n\n\n\n"
    );
    // The cursor and header extend across the row; below the table the parent's background shows.
    assert_eq!(buffer[(29, 0)].bg, Color::Rgb(0x2d, 0x37, 0x40));
    assert_eq!(buffer[(29, 1)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    assert_eq!(buffer[(29, 2)].bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(buffer[(0, 3)].bg, Color::Reset);
}

#[test]
fn an_empty_table_is_the_header_only() {
    let mut table = by_id();
    let buffer = draw(&mut table, true, 30, 4);
    assert_eq!(buffer_to_string(&buffer), " ★  Owner  Title  #\n\n\n\n");
}

#[test]
fn rows_removed_since_the_last_frame_do_not_leave_the_view_past_the_end() {
    let keymap = Keymap::defaults();
    let mut table = twelve();
    snap(&mut table, true);
    let _ = press(&mut table, &keymap, "G");
    assert_eq!(table.scroll().0, 6);
    for id in 5..=12 {
        table.remove(&id);
    }
    snap(&mut table, true);
    assert_eq!(table.scroll().0, 0);
}

#[test]
fn tiny_areas_do_not_panic() {
    let keymap = Keymap::defaults();
    for (w, h) in [(0, 0), (1, 1), (2, 1), (3, 2), (40, 1), (1, 40)] {
        let mut table = twelve();
        draw(&mut table, true, w, h);
        for key in ["j", "J", "G", "$", "^", "l", "K"] {
            let _ = press(&mut table, &keymap, key);
        }
        draw(&mut table, false, w, h);
        let mut empty = by_id();
        draw(&mut empty, true, w, h);
    }
}

#[test]
fn styled_cells_keep_their_spans() {
    let line = Line::from(vec![Span::raw("a"), Span::styled("b", Style::new().fg(Color::Red))]);
    let cell = Cell::new(CellValue::Str("ab".into()), line.clone());
    assert_eq!(cell.display, line);
    assert_eq!(Cell::int(7).value, CellValue::Int(7));
}
