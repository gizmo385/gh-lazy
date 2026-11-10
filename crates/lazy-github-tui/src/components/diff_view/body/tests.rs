//! The body on its own, against PR 5's diff from the `pr-details` fixtures: its rows, the cursor's moves, scrolling,
//! panning and drawing.

use std::path::Path;

use lazy_github_core::diff::{self, DiffLine, Hunk};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::test_util::{buffer_to_string, buffer_to_styled_string};

/// PR 5's diff: README.md (one hunk of 6 lines), assets/app.css (added, 3), build.rs (4), docs/old.md (deleted) and
/// src/viewer.rs (two hunks, of 8 and 9 lines).
fn diff_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/GET.diff.json");
    let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

fn body() -> DiffBody {
    DiffBody::new(diff::parse(&diff_text()).unwrap())
}

const README: usize = 0;
const CSS: usize = 1;
const BUILD: usize = 2;
const OLD: usize = 3;
const VIEWER: usize = 4;

fn at(file: usize, hunk: usize, line: usize) -> DiffLineRef {
    DiffLineRef { file, hunk, line }
}

/// The cursor on a line, as [`DiffBody::cursor`] gives it.
#[allow(clippy::unnecessary_wraps)]
fn line(file: usize, hunk: usize, line: usize) -> Option<Cursor> {
    Some(Cursor::Line(at(file, hunk, line)))
}

fn rows(body: &DiffBody) -> Vec<Row> {
    (0..body.rows()).map(|r| body.row(r).unwrap()).collect()
}

/// A file's rows as the body lists them: its header, then each hunk's separator and lines.
fn file_rows(f: usize, hunks: &[usize]) -> Vec<Row> {
    let mut out = vec![Row::FileHeader(f)];
    for (h, &lines) in hunks.iter().enumerate() {
        out.push(Row::HunkSeparator(f, h));
        out.extend((0..lines).map(|l| Row::Line(at(f, h, l))));
    }
    out
}

fn press(body: &mut DiffBody, id: BindingId, times: usize) {
    for _ in 0..times {
        assert!(body.handle(id), "{id:?} isn't the body's");
    }
}

fn down(body: &mut DiffBody, times: usize) {
    press(body, BindingId::DiffCursorDown, times);
}

fn up(body: &mut DiffBody, times: usize) {
    press(body, BindingId::DiffCursorUp, times);
}

fn draw(body: &mut DiffBody, width: u16, height: u16, focused: bool) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            body.render(frame.buffer_mut(), area, &Theme::default(), focused);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn rows_are_each_file_s_header_then_its_hunks_separators_and_lines() {
    let mut body = body();
    let mut expected = file_rows(README, &[6]);
    expected.extend(file_rows(CSS, &[3]));
    expected.extend(file_rows(BUILD, &[4]));
    // A deleted file starts folded.
    expected.push(Row::FileHeader(OLD));
    expected.extend(file_rows(VIEWER, &[8, 9]));
    assert_eq!(rows(&body), expected);
    assert_eq!(body.row(expected.len()), None);
    assert!(body.is_folded(OLD));

    // Unfolded, it shows `file was removed` in place of its hunk.
    body.set_folded(OLD, false);
    let rows = rows(&body);
    let old = rows.iter().position(|row| *row == Row::FileHeader(OLD)).unwrap();
    assert_eq!(rows[old + 1], Row::Removed(OLD));
    assert_eq!(rows[old + 2], Row::FileHeader(VIEWER));
}

#[test]
fn a_file_without_hunks_shows_only_its_header() {
    let text = "diff --git a/old.txt b/new.txt\nsimilarity index 100%\nrename from old.txt\nrename to new.txt\n\
                diff --git a/logo.png b/logo.png\nindex 1111111..2222222 100644\nBinary files a/logo.png and \
                b/logo.png differ\n";
    let body = DiffBody::new(diff::parse(text).unwrap());
    assert_eq!(rows(&body), [Row::FileHeader(0), Row::FileHeader(1)]);
    let mut body = body;
    let screen = buffer_to_string(&draw(&mut body, 40, 4, true));
    let lines: Vec<&str> = screen.lines().map(str::trim_end).collect();
    // `buffer_to_string` shows the cell a wide emoji covers as a space.
    assert_eq!(
        lines,
        [
            "▼ 📄  new.txt",
            "▼ 📄  logo.png",
            "",
            " new.txt                        0 hunks"
        ]
    );
}

#[test]
fn j_and_k_cross_hunks_and_files_and_stop_at_both_ends() {
    let mut body = body();
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    up(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    // The separator is skipped.
    down(&mut body, 1);
    assert_eq!(body.cursor(), line(README, 0, 0));
    up(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    // From a file's last line to the next file's header, and back.
    down(&mut body, 6);
    assert_eq!(body.cursor(), line(README, 0, 5));
    down(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(CSS)));
    up(&mut body, 1);
    assert_eq!(body.cursor(), line(README, 0, 5));
    // Across the folded deleted file, then from one hunk's last line to the next hunk's first.
    down(&mut body, 1 + 4 + 5);
    assert_eq!(body.cursor(), Some(Cursor::File(OLD)));
    down(&mut body, 1 + 8);
    assert_eq!(body.cursor(), line(VIEWER, 0, 7));
    down(&mut body, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 0));
    up(&mut body, 1);
    assert_eq!(body.cursor(), line(VIEWER, 0, 7));
    down(&mut body, 100);
    assert_eq!(body.cursor(), line(VIEWER, 1, 8));
    assert_eq!(body.cursor_row(), Some(body.rows() - 1));
}

#[test]
fn j_stops_on_a_deleted_file_s_removed_row() {
    let mut body = body();
    body.set_folded(OLD, false);
    while body.cursor() != Some(Cursor::File(OLD)) {
        down(&mut body, 1);
    }
    down(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::Removed(OLD)));
    assert_eq!(body.cursor_line(), None);
    down(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
}

#[test]
fn big_j_goes_to_the_next_hunk_s_first_line() {
    let mut body = body();
    // From a header, that file's first hunk.
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(README, 0, 0));
    // From a line, the next hunk, here the next file's.
    down(&mut body, 2);
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(CSS, 0, 0));
    // The deleted file has no hunk to stop on.
    press(&mut body, BindingId::DiffNextHunk, 2);
    assert_eq!(body.cursor(), line(VIEWER, 0, 0));
    down(&mut body, 3);
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 0));
    // At the last hunk, nothing.
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 0));
}

#[test]
fn big_j_from_a_removed_row_goes_past_its_file() {
    let mut body = body();
    body.set_folded(OLD, false);
    body.cursor = Some(Cursor::Removed(OLD));
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 0, 0));
}

#[test]
fn big_k_goes_to_this_hunk_s_first_line_then_the_previous_hunk() {
    let mut body = body();
    // At the first hunk's first line, nothing.
    press(&mut body, BindingId::DiffNextHunk, 1);
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(README, 0, 0));
    // Below a hunk's first line, its first line.
    down(&mut body, 3);
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(README, 0, 0));

    body.cursor = line(VIEWER, 1, 0);
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 0, 0));
    // From the first line of a file's first hunk, the previous file's last hunk, past the deleted file.
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(BUILD, 0, 0));
    // From a header, the last hunk before its file.
    body.cursor = Some(Cursor::File(VIEWER));
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(BUILD, 0, 0));
    body.cursor = Some(Cursor::File(README));
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
}

#[test]
fn hunk_jumps_unfold_a_folded_file() {
    let mut body = body();
    body.set_folded(VIEWER, true);
    body.cursor = line(BUILD, 0, 1);
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 0, 0));
    assert!(!body.is_folded(VIEWER));

    body.set_folded(README, true);
    body.cursor = Some(Cursor::File(CSS));
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(README, 0, 0));
    assert!(!body.is_folded(README));

    // From a folded file's header, its first hunk.
    body.set_folded(BUILD, true);
    body.cursor = Some(Cursor::File(BUILD));
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(BUILD, 0, 0));
}

#[test]
fn page_keys_move_by_the_visible_height_less_one() {
    let mut body = body();
    // 10 rows of body and the position row.
    body.resize(100, 11);
    press(&mut body, BindingId::TextualScrollPageDown, 1);
    // Row 9 is CSS's separator, so the cursor goes on to its first line.
    assert_eq!(body.row(9), Some(Row::HunkSeparator(CSS, 0)));
    assert_eq!(body.cursor(), line(CSS, 0, 0));
    assert_eq!(body.cursor_row(), Some(10));
    press(&mut body, BindingId::TextualScrollPageDown, 1);
    assert_eq!(body.cursor_row(), Some(19));
    press(&mut body, BindingId::TextualScrollPageUp, 1);
    assert_eq!(body.cursor_row(), Some(10));
    // Row 1 is README's separator: going up, the cursor stops on the header above it.
    press(&mut body, BindingId::TextualScrollPageUp, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    press(&mut body, BindingId::TextualScrollPageDown, 10);
    assert_eq!(body.cursor_row(), Some(body.rows() - 1));
}

#[test]
fn home_and_end_go_to_the_first_and_last_rows() {
    let mut body = body();
    body.resize(100, 11);
    press(&mut body, BindingId::TextualScrollEnd, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 8));
    assert_eq!(body.scroll(), body.rows() - 10);
    press(&mut body, BindingId::TextualScrollHome, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    assert_eq!(body.scroll(), 0);
}

#[test]
fn panning_moves_eight_cells_until_the_widest_line_s_end_shows() {
    let mut body = body();
    // 60 cells less the scrollbar and the gutter: 48 cells of code. The widest line, in src/viewer.rs, is 66.
    body.resize(60, 30);
    let right = |body: &mut DiffBody| {
        press(body, BindingId::TextualScrollRight, 1);
        body.pan()
    };
    assert_eq!(right(&mut body), 8);
    assert_eq!(right(&mut body), 16);
    assert_eq!(right(&mut body), 18);
    assert_eq!(right(&mut body), 18);
    press(&mut body, BindingId::TextualScrollLeft, 1);
    assert_eq!(body.pan(), 10);
    press(&mut body, BindingId::TextualScrollLeft, 2);
    assert_eq!(body.pan(), 0);

    // Hiding the widest line pulls the pan back by the next draw: README's 50-cell line, the widest left, fits the
    // 50 cells the code gets once the rows fit without a scrollbar.
    press(&mut body, BindingId::TextualScrollRight, 3);
    body.set_folded(VIEWER, true);
    body.resize(60, 30);
    assert_eq!(body.pan(), 0);

    // When every line fits, there's nothing to pan.
    let mut body = self::body();
    body.resize(160, 48);
    press(&mut body, BindingId::TextualScrollRight, 1);
    assert_eq!(body.pan(), 0);
}

#[test]
fn the_cursor_keeps_its_line_when_files_before_it_fold_or_are_filtered_out() {
    let mut body = body();
    body.cursor = line(VIEWER, 1, 2);
    let row = body.cursor_row().unwrap();
    body.set_folded(README, true);
    assert_eq!(body.cursor(), line(VIEWER, 1, 2));
    assert_eq!(body.cursor_row(), Some(row - 7));
    body.set_filter("VIEWER");
    assert_eq!(body.cursor(), line(VIEWER, 1, 2));
    assert_eq!(body.cursor_row(), Some(1 + 9 + 1 + 2));
    body.set_filter("");
    assert_eq!(body.cursor(), line(VIEWER, 1, 2));

    // Its own file folded, the cursor goes to the file's header.
    body.set_folded(VIEWER, true);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
}

#[test]
fn the_cursor_goes_to_the_first_shown_header_when_its_file_is_filtered_out() {
    let mut body = body();
    body.cursor = line(VIEWER, 0, 1);
    body.set_filter(".css");
    assert_eq!(body.cursor(), Some(Cursor::File(CSS)));
    assert!(!body.is_shown(README));

    // Nothing shown: no rows, no cursor, no position, and the keys do nothing.
    body.set_filter("nothing matches this");
    assert_eq!(body.rows(), 0);
    assert_eq!(body.cursor(), None);
    assert_eq!(body.position(), None);
    for id in DIFF_BODY_BINDINGS {
        body.handle(*id);
    }
    assert_eq!(body.cursor(), None);
    let buf = draw(&mut body, 30, 4, true);
    assert_eq!(buffer_to_string(&buf).trim(), "");

    body.set_filter("");
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
}

#[test]
fn an_empty_diff_has_no_rows_and_no_cursor() {
    let mut body = DiffBody::new(Diff::default());
    assert_eq!((body.rows(), body.cursor()), (0, None));
    press(&mut body, BindingId::DiffNextHunk, 1);
    press(&mut body, BindingId::TextualScrollEnd, 1);
    assert_eq!(body.cursor(), None);
}

#[test]
fn add_comment_and_filter_files_are_the_diff_tab_s() {
    let mut body = body();
    assert!(!body.handle(BindingId::DiffAddComment));
    assert!(!body.handle(BindingId::DiffFilterFiles));
    assert!(!body.handle(BindingId::TextualFocusNext));
}

#[test]
fn down_and_up_move_the_cursor_as_j_and_k_do_and_the_view_follows() {
    let mut body = body();
    body.resize(100, 11);
    press(&mut body, BindingId::TextualScrollDown, 7);
    // As `j` seven times: README.md's hunk lines 1 to 6, then row 8, assets/app.css's header, two rows past the
    // margin, so the view scrolled two rows.
    assert_eq!(body.cursor(), Some(Cursor::File(CSS)));
    assert_eq!(body.scroll(), 2);
    press(&mut body, BindingId::TextualScrollUp, 1);
    assert_eq!(body.cursor(), line(README, 0, 5));
    press(&mut body, BindingId::TextualScrollUp, 10);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    assert_eq!(body.scroll(), 0);
}

fn next_file(body: &mut DiffBody, times: usize) {
    press(body, BindingId::DiffNextFile, times);
}

fn previous_file(body: &mut DiffBody, times: usize) {
    press(body, BindingId::DiffPreviousFile, times);
}

#[test]
fn next_file_goes_to_each_shown_file_s_header_and_stops_at_the_last() {
    let mut body = body();
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(CSS)));
    // From a line, the next file's header.
    down(&mut body, 2);
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    // The folded deleted file's header too, without unfolding it.
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(OLD)));
    assert!(body.is_folded(OLD));
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    // At the last file, nothing; nor from its last line.
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    press(&mut body, BindingId::DiffLastLine, 1);
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 8));
}

#[test]
fn previous_file_goes_to_this_file_s_header_then_the_previous_file_s() {
    let mut body = body();
    body.cursor = line(VIEWER, 1, 3);
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(OLD)));
    // From a `file was removed` row, its own header.
    body.set_folded(OLD, false);
    body.cursor = Some(Cursor::Removed(OLD));
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(OLD)));
    previous_file(&mut body, 3);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    // At the first file's header, nothing.
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
}

#[test]
fn file_jumps_cross_a_folded_file_without_unfolding_it() {
    let mut body = body();
    body.set_folded(BUILD, true);
    body.cursor = line(CSS, 0, 1);
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(OLD)));
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    assert!(body.is_folded(BUILD));
}

#[test]
fn file_jumps_skip_the_files_the_filter_hides() {
    let mut body = body();
    // build.rs and src/viewer.rs.
    body.set_filter(".rs");
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    next_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
}

#[test]
fn file_jumps_scroll_the_header_into_view() {
    let mut body = body();
    body.resize(100, 11);
    next_file(&mut body, 4);
    assert_eq!(body.cursor_row(), Some(20));
    assert_eq!(body.scroll(), 20 + 3 + 1 - 10);
    previous_file(&mut body, 4);
    assert_eq!(body.scroll(), 0);
}

#[test]
fn enter_folds_and_unfolds_the_header_s_file_and_does_nothing_elsewhere() {
    let mut body = body();
    press(&mut body, BindingId::DiffToggle, 1);
    assert!(body.is_folded(README));
    assert_eq!(body.row(1), Some(Row::FileHeader(CSS)));
    // `j` steps over the folded file's header without unfolding it.
    down(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(CSS)));
    up(&mut body, 1);
    press(&mut body, BindingId::DiffToggle, 1);
    assert!(!body.is_folded(README));
    assert_eq!(body.row(1), Some(Row::HunkSeparator(README, 0)));
    // On a line, `enter` is the body's but does nothing.
    down(&mut body, 2);
    press(&mut body, BindingId::DiffToggle, 1);
    assert_eq!(body.cursor(), line(README, 0, 1));
    assert!(!body.is_folded(README));
    // A deleted file unfolds to its `file was removed` row.
    body.cursor = Some(Cursor::File(OLD));
    press(&mut body, BindingId::DiffToggle, 1);
    assert_eq!(body.row(body.cursor_row().unwrap() + 1), Some(Row::Removed(OLD)));
}

#[test]
fn z_folds_every_file_then_unfolds_them_all_the_cursor_on_its_file_s_header() {
    let mut body = body();
    body.resize(100, 11);
    body.cursor = line(VIEWER, 1, 4);
    assert!(!body.all_folded());
    press(&mut body, BindingId::DiffFoldAll, 1);
    // The file list: one header per file.
    assert!(body.all_folded());
    assert_eq!(rows(&body), (0..5).map(Row::FileHeader).collect::<Vec<_>>());
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    assert_eq!(body.scroll(), 0);
    // `k` and `[` move between the headers, and `enter` opens one.
    up(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(OLD)));
    previous_file(&mut body, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    press(&mut body, BindingId::DiffToggle, 1);
    assert!(!body.is_folded(BUILD));
    assert!(!body.all_folded());
    // With one file unfolded, `z` folds everything again.
    down(&mut body, 2);
    press(&mut body, BindingId::DiffFoldAll, 1);
    assert!(body.all_folded());
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
    // Then unfolds every file, the deleted one included.
    press(&mut body, BindingId::DiffFoldAll, 1);
    assert!((0..5).all(|f| !body.is_folded(f)));
    assert!(rows(&body).contains(&Row::Removed(OLD)));
    assert_eq!(body.cursor(), Some(Cursor::File(BUILD)));
}

#[test]
fn z_folds_only_the_shown_files() {
    let mut body = body();
    body.set_filter("viewer");
    press(&mut body, BindingId::DiffFoldAll, 1);
    assert!(body.is_folded(VIEWER));
    assert!(!body.is_folded(README));
    // Every shown file is folded, so the next `z` unfolds.
    assert!(body.all_folded());
}

#[test]
fn big_j_unfolds_a_folded_file_it_lands_in_from_the_file_list() {
    let mut body = body();
    press(&mut body, BindingId::DiffFoldAll, 1);
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(README, 0, 0));
    assert!(!body.is_folded(README));
    assert!(body.is_folded(CSS));
}

#[test]
fn g_and_big_g_go_to_the_first_and_last_rows() {
    let mut body = body();
    body.resize(100, 11);
    press(&mut body, BindingId::DiffLastLine, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 8));
    assert_eq!(body.scroll(), body.rows() - 10);
    press(&mut body, BindingId::DiffFirstLine, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    assert_eq!(body.scroll(), 0);
}

#[test]
fn the_sticky_header_is_the_cursor_s_file_once_its_header_is_above_the_rows_shown() {
    let mut body = body();
    // src/viewer.rs's header is row 20; its first hunk's lines are rows 22 to 29.
    body.cursor = line(VIEWER, 0, 5);
    assert_eq!(body.sticky_file(21..40), Some(VIEWER));
    // The real header shows: no sticky header.
    assert_eq!(body.sticky_file(20..40), None);
    assert_eq!(body.sticky_file(10..30), None);
    // Nine rows are enough, eight aren't.
    assert_eq!(body.sticky_file(21..30), Some(VIEWER));
    assert_eq!(body.sticky_file(21..29), None);
    // It follows the cursor's file.
    body.cursor = line(BUILD, 0, 2);
    assert_eq!(body.sticky_file(16..40), Some(BUILD));
}

#[test]
fn the_sticky_header_is_drawn_over_the_top_row() {
    let mut body = body();
    // 15 rows of body: the last row's at the bottom, src/viewer.rs's header five rows above the top, and under it
    // `pub struct Viewer {`, longer than the header.
    press(&mut body, BindingId::DiffLastLine, 1);
    let screen = buffer_to_string(&draw(&mut body, 100, 16, true));
    assert_eq!(body.scroll(), 25);
    let top = screen.lines().next().unwrap();
    let header = "▼ 📄  src/viewer.rs (+5 -2)";
    assert!(top.starts_with(header), "{screen}");
    // Nothing of the row under it shows past the header, up to the scrollbar.
    let tail: String = top[header.len()..].chars().take(70).collect();
    assert_eq!(tail.trim(), "", "{screen}");
    // At eight rows of body, it's left out.
    let screen = buffer_to_string(&draw(&mut body, 100, 9, true));
    assert!(!screen.contains("src/viewer.rs (+5 -2)"), "{screen}");
}

#[test]
fn a_click_on_a_header_s_title_folds_its_file_and_one_past_it_doesn_t() {
    let mut body = body();
    body.cursor = line(VIEWER, 0, 0);
    // `▼ 📄 README.md (+3 -1)` is 22 cells wide.
    assert!(body.click_row(0, 30));
    assert_eq!(body.cursor(), Some(Cursor::File(README)));
    assert!(!body.is_folded(README));
    assert!(body.click_row(0, 21));
    assert!(body.is_folded(README));
    assert!(body.click_row(0, 0));
    assert!(!body.is_folded(README));
    // A line's click doesn't fold.
    assert!(body.click_row(2, 0));
    assert_eq!(body.cursor(), line(README, 0, 0));
    assert!(!body.is_folded(README));
    assert!(!body.click_row(body.rows(), 0));
}

#[test]
fn each_new_binding_is_rebindable_through_the_overrides() {
    let cases = [
        (BindingId::DiffNextFile, "diff.next_file", "]"),
        (BindingId::DiffPreviousFile, "diff.previous_file", "["),
        (BindingId::DiffToggle, "diff.toggle", "enter"),
        (BindingId::DiffFoldAll, "diff.fold_all", "z"),
        (BindingId::DiffFirstLine, "diff.first_line", "g"),
        (BindingId::DiffLastLine, "diff.last_line", "G"),
    ];
    let chord = |key: &str| match crate::keymap::KeySpec::parse(key) {
        crate::keymap::KeySpec::Chord(chord) => chord,
        crate::keymap::KeySpec::Unknown(key) => panic!("{key} isn't a key"),
    };
    let defaults = crate::keymap::Keymap::defaults();
    for (id, name, default) in cases {
        assert_eq!(id.config_id(), Some(name));
        assert_eq!(id.def().default_keys, default, "{name}");
        assert_eq!(
            defaults.match_in(DIFF_BODY_BINDINGS, &chord(default)),
            Some(id),
            "{name}"
        );
        let overrides = [(name.to_owned(), "f5".to_owned())].into_iter().collect();
        let keymap = crate::keymap::Keymap::new(&overrides);
        assert_eq!(keymap.match_in(DIFF_BODY_BINDINGS, &chord("f5")), Some(id), "{name}");
        assert_ne!(keymap.match_in(DIFF_BODY_BINDINGS, &chord(default)), Some(id), "{name}");
    }
}

#[test]
fn the_view_keeps_the_cursor_three_rows_from_its_edges() {
    let mut body = body();
    body.resize(100, 11);
    // Rows 0 and 2..=7 hold the cursor going down from the top; row 1 is a separator.
    down(&mut body, 5);
    assert_eq!((body.cursor_row(), body.scroll()), (Some(6), 0));
    down(&mut body, 1);
    assert_eq!((body.cursor_row(), body.scroll()), (Some(7), 1));
    down(&mut body, 1);
    // Row 8 is CSS's header.
    assert_eq!((body.cursor_row(), body.scroll()), (Some(8), 2));
    // Going back up, the view holds still until the cursor is three rows from the top.
    up(&mut body, 3);
    assert_eq!((body.cursor_row(), body.scroll()), (Some(5), 2));
    up(&mut body, 1);
    assert_eq!((body.cursor_row(), body.scroll()), (Some(4), 1));
    // Near the end, the view stops at the last row rather than keep the margin.
    press(&mut body, BindingId::TextualScrollEnd, 1);
    up(&mut body, 1);
    assert_eq!(body.scroll(), body.rows() - 10);
}

#[test]
fn the_position_row_says_where_the_cursor_is() {
    let mut body = body();
    let position = |body: &DiffBody| {
        let p = body.position().unwrap();
        (p.path, p.detail)
    };
    assert_eq!(position(&body), ("README.md".into(), "1 hunk".into()));
    down(&mut body, 4);
    assert_eq!(position(&body), ("README.md".into(), "Line: 4/6".into()));
    body.cursor = Some(Cursor::File(VIEWER));
    assert_eq!(position(&body), ("src/viewer.rs".into(), "2 hunks".into()));
}

#[test]
fn only_the_rows_on_screen_are_highlighted() {
    let mut body = body();
    draw(&mut body, 100, 12, true);
    assert!(body.files[README].code[0].is_some());
    assert!(body.files[VIEWER].code.iter().all(Option::is_none));
}

/// A deterministic run of pseudo-random numbers, so the generated diff is the same every run.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap() % n
    }
}

/// A diff of `files` files of 0 to 4 hunks of 0 to 9 lines each, some deleted.
fn generated(rng: &mut Lcg, files: usize) -> Diff {
    let files = (0..files)
        .map(|f| {
            let hunks = (0..rng.below(5))
                .map(|_| Hunk {
                    header: "@@ -1 +1 @@".into(),
                    range: None,
                    context: String::new(),
                    lines: (0..rng.below(10))
                        .map(|_| DiffLine {
                            kind: LineKind::Added,
                            text: "x".into(),
                            old: None,
                            new: Some(1),
                            position: 1,
                        })
                        .collect(),
                })
                .collect();
            ChangedFile {
                path: format!("file_{f}"),
                old_path: format!("file_{f}"),
                deleted: rng.below(5) == 0,
                hunks,
            }
        })
        .collect();
    Diff { files }
}

/// Every row, listed one by one from the files' state.
fn brute_force(body: &DiffBody) -> Vec<Row> {
    let mut out = Vec::new();
    for (f, state) in body.files.iter().enumerate().filter(|(_, state)| state.shown) {
        out.push(Row::FileHeader(f));
        if state.folded {
            continue;
        }
        if state.file.deleted {
            out.push(Row::Removed(f));
            continue;
        }
        let lines: Vec<usize> = state.file.hunks.iter().map(|hunk| hunk.lines.len()).collect();
        out.extend(file_rows(f, &lines).into_iter().skip(1));
    }
    out
}

#[test]
fn the_row_index_matches_a_brute_force_row_list() {
    let mut rng = Lcg(387);
    for _ in 0..50 {
        let files = 1 + rng.below(30);
        let mut body = DiffBody::new(generated(&mut rng, files));
        for _ in 0..5 {
            for state in &mut body.files {
                state.folded = rng.below(3) == 0;
                state.shown = rng.below(4) != 0;
            }
            body.reindex();
            let expected = brute_force(&body);
            assert_eq!(body.rows(), expected.len());
            for (r, row) in expected.iter().enumerate() {
                assert_eq!(body.row(r), Some(*row), "row {r}");
                assert_eq!(body.index.row_of(&body.files, *row), Some(r), "{row:?}");
            }
            assert_eq!(body.row(expected.len()), None);
            // `j` from the top visits every row that can hold the cursor, in order.
            let holding: Vec<Cursor> = expected.iter().filter_map(|row| row.cursor()).collect();
            body.cursor = holding.first().copied();
            let mut visited: Vec<Cursor> = body.cursor.into_iter().collect();
            for _ in 1..holding.len() {
                down(&mut body, 1);
                visited.extend(body.cursor);
            }
            assert_eq!(visited, holding);
        }
    }
}

#[test]
fn diff_body_top_160x48() {
    let mut body = body();
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&mut body, 160, 48, true)));
}

#[test]
fn diff_body_cursor_on_added_line_100x30() {
    let mut body = body();
    down(&mut body, 4);
    assert_eq!(body.cursor(), line(README, 0, 3));
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&mut body, 100, 30, true)));
}

#[test]
fn diff_body_unfocused() {
    let mut body = body();
    down(&mut body, 4);
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&mut body, 100, 30, false)));
}

#[test]
fn diff_body_panned() {
    let mut body = body();
    body.cursor = line(VIEWER, 1, 3);
    draw(&mut body, 60, 30, true);
    press(&mut body, BindingId::TextualScrollRight, 2);
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&mut body, 60, 30, true)));
}

#[test]
fn diff_body_deleted_unfolded() {
    let mut body = body();
    body.set_folded(OLD, false);
    body.cursor = Some(Cursor::Removed(OLD));
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&mut body, 100, 30, true)));
}

// Review threads (#390).

fn thread(id: u64, path: &str, position: Option<u64>, original: Option<u64>, replies: usize) -> DiffThread {
    use lazy_github_core::models::{IssueComment, Review, ReviewComment, ReviewState, User};
    let comment = |id: u64, reply_to: Option<u64>| ReviewComment {
        comment: IssueComment {
            id,
            body: format!("Comment {id}\nsecond line"),
            user: Some(User {
                login: "parity-bot".to_owned(),
                id: 1,
                name: None,
                html_url: "https://github.com/parity-bot".to_owned(),
            }),
            created_at: jiff::Timestamp::from_second(id.cast_signed()).unwrap(),
            updated_at: jiff::Timestamp::from_second(id.cast_signed()).unwrap(),
            author_association: String::new(),
        },
        pull_request_review_id: 1,
        path: path.to_owned(),
        position,
        original_position: original,
        in_reply_to_id: reply_to,
        diff_hunk: "@@ -1,2 +1,2 @@\n a\n-gone".to_owned(),
    };
    let mut comments = vec![comment(id, None)];
    comments.extend((1..=replies as u64).map(|r| comment(id + r, Some(id))));
    let review = Review {
        id: 1,
        user: comments[0].comment.user.clone().unwrap(),
        body: String::new(),
        state: ReviewState::Commented,
        submitted_at: None,
        comments,
    };
    lazy_github_core::review_anchor::diff_threads(&[review]).remove(0)
}

/// src/viewer.rs's threads: 100 under its 6th line (with a reply), 200 at a position past the file, 300 on the file.
fn threaded() -> DiffBody {
    let mut body = body();
    body.set_threads(
        vec![
            thread(100, "src/viewer.rs", Some(6), Some(6), 1),
            thread(200, "src/viewer.rs", Some(99), Some(3), 0),
            thread(300, "src/viewer.rs", None, None, 0),
            thread(400, "gone.rs", Some(1), Some(1), 0),
        ],
        "parity-bot/pr-details",
    );
    body
}

#[test]
fn threads_take_rows_under_their_lines_and_after_the_file() {
    let body = threaded();
    let viewer: Vec<Row> = rows(&body).into_iter().filter(|row| row.file() == VIEWER).collect();
    let th = |thread, row| Row::Thread {
        file: VIEWER,
        thread,
        row,
    };
    let mut expected = vec![Row::FileHeader(VIEWER), th(2, 0), Row::HunkSeparator(VIEWER, 0)];
    expected.extend((0..6).map(|l| Row::Line(at(VIEWER, 0, l))));
    expected.push(th(0, 0));
    expected.extend((6..8).map(|l| Row::Line(at(VIEWER, 0, l))));
    expected.push(Row::HunkSeparator(VIEWER, 1));
    expected.extend((0..9).map(|l| Row::Line(at(VIEWER, 1, l))));
    // A position past the file: outdated.
    expected.extend([Row::OutdatedSeparator(VIEWER), th(1, 0)]);
    assert_eq!(viewer, expected);
    // The thread on a file the diff doesn't have is left out.
    assert_eq!(body.thread_count(), 3);
    // Every row maps back to its number.
    for (number, row) in rows(&body).into_iter().enumerate() {
        assert_eq!(body.index.row_of(&body.files, row), Some(number), "{row:?}");
    }
}

#[test]
fn expanding_adds_the_comment_rows_and_a_reload_keeps_it_and_the_cursor() {
    let mut body = threaded();
    let _ = draw(&mut body, 100, 40, true);
    let before = body.rows();
    press(&mut body, BindingId::DiffNextComment, 1);
    assert_eq!(
        body.cursor(),
        Some(Cursor::Thread {
            file: VIEWER,
            thread: 2,
            row: 0
        })
    );
    press(&mut body, BindingId::DiffNextComment, 1);
    assert_eq!(
        body.cursor(),
        Some(Cursor::Thread {
            file: VIEWER,
            thread: 0,
            row: 0
        })
    );
    press(&mut body, BindingId::DiffToggle, 1);
    assert!(body.is_expanded(0));
    // Two comments, each an author line and its body (one paragraph: markdown joins the soft break).
    assert_eq!(body.rows(), before + 4);
    down(&mut body, 4);
    assert_eq!(
        body.cursor(),
        Some(Cursor::Thread {
            file: VIEWER,
            thread: 0,
            row: 4
        })
    );
    // The same threads again, in another order: thread 100 is now index 1, still expanded, the cursor on it.
    body.set_threads(
        vec![
            thread(300, "src/viewer.rs", None, None, 0),
            thread(100, "src/viewer.rs", Some(6), Some(6), 1),
        ],
        "parity-bot/pr-details",
    );
    assert!(body.is_expanded(1));
    assert_eq!(
        body.cursor(),
        Some(Cursor::Thread {
            file: VIEWER,
            thread: 1,
            row: 4
        })
    );
    // Without it, the cursor goes to the file's header.
    body.set_threads(Vec::new(), "parity-bot/pr-details");
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
    assert_eq!(body.rows(), rows(&body).len());
}

#[test]
fn a_narrower_body_wraps_expanded_threads_onto_more_rows() {
    let mut body = threaded();
    let _ = draw(&mut body, 100, 40, true);
    press(&mut body, BindingId::DiffNextComment, 2);
    press(&mut body, BindingId::DiffToggle, 1);
    let wide = body.rows();
    let _ = draw(&mut body, 24, 40, true);
    assert!(body.rows() > wide, "{} rows, {wide} wide", body.rows());
    for (number, row) in rows(&body).into_iter().enumerate() {
        assert_eq!(body.index.row_of(&body.files, row), Some(number), "{row:?}");
    }
}

#[test]
fn hunk_keys_from_a_thread_go_by_its_place() {
    let mut body = threaded();
    press(&mut body, BindingId::DiffNextComment, 2);
    assert_eq!(
        body.cursor(),
        Some(Cursor::Thread {
            file: VIEWER,
            thread: 0,
            row: 0
        })
    );
    press(&mut body, BindingId::DiffNextHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 0));
    press(&mut body, BindingId::DiffNextComment, 1);
    press(&mut body, BindingId::DiffPreviousHunk, 1);
    assert_eq!(body.cursor(), line(VIEWER, 1, 0));
    press(&mut body, BindingId::DiffNextComment, 1);
    assert_eq!(
        body.cursor(),
        Some(Cursor::Thread {
            file: VIEWER,
            thread: 1,
            row: 0
        })
    );
    press(&mut body, BindingId::DiffPreviousFile, 1);
    assert_eq!(body.cursor(), Some(Cursor::File(VIEWER)));
}

// Replying to a thread (#385).

/// The root id and the quoted comment's id `r` would reply with.
fn reply(body: &DiffBody) -> Option<(u64, u64)> {
    body.reply_target()
        .map(|target| (target.root, target.quoted.comment.id))
}

#[allow(clippy::unnecessary_wraps)]
fn on_viewer_thread(thread: usize, row: usize) -> Option<Cursor> {
    Some(Cursor::Thread {
        file: VIEWER,
        thread,
        row,
    })
}

#[test]
fn the_reply_quotes_the_comment_under_the_cursor_and_goes_to_the_root() {
    let mut body = threaded();
    let _ = draw(&mut body, 100, 40, true);
    // Collapsed: the root.
    body.cursor = on_viewer_thread(0, 0);
    assert_eq!(reply(&body), Some((100, 100)));
    // Expanded: the summary and the root's author and body rows quote the root, the reply's rows the reply.
    press(&mut body, BindingId::DiffToggle, 1);
    for (row, quoted) in [(0, 100), (1, 100), (2, 100), (3, 101), (4, 101)] {
        body.cursor = on_viewer_thread(0, row);
        assert_eq!(reply(&body), Some((100, quoted)), "row {row}");
    }
    // An outdated thread, expanded: its summary and the line it was written on quote the root.
    body.cursor = on_viewer_thread(1, 0);
    press(&mut body, BindingId::DiffToggle, 1);
    assert!(body.threads[1].original.is_some());
    for row in 0..3 {
        body.cursor = on_viewer_thread(1, row);
        assert_eq!(reply(&body), Some((200, 200)), "row {row}");
    }
    // A comment on the whole file.
    body.cursor = on_viewer_thread(2, 0);
    assert_eq!(reply(&body), Some((300, 300)));
}

#[test]
fn the_reply_quotes_by_the_wrapped_rows_at_the_drawn_width() {
    let mut body = threaded();
    body.cursor = on_viewer_thread(0, 0);
    press(&mut body, BindingId::DiffToggle, 1);
    // Narrow, each body wraps: the reply's author line comes later.
    let _ = draw(&mut body, 24, 40, true);
    let width = body.thread_width();
    let rows = body.threads[0].rows(width);
    assert!(rows > 5, "{rows} rows");
    body.cursor = on_viewer_thread(0, rows - 1);
    assert_eq!(reply(&body), Some((100, 101)));
    body.cursor = on_viewer_thread(0, 3);
    assert_eq!(reply(&body), Some((100, 100)));
}

#[test]
fn nothing_to_reply_to_off_a_thread() {
    let mut body = threaded();
    // A line with a thread under it, a header, `file was removed` and a pending comment.
    body.cursor = line(VIEWER, 0, 5);
    assert_eq!(reply(&body), None);
    body.cursor = Some(Cursor::File(VIEWER));
    assert_eq!(reply(&body), None);
    body.set_folded(OLD, false);
    body.cursor = Some(Cursor::Removed(OLD));
    assert_eq!(reply(&body), None);
    let file = body.file(VIEWER).unwrap();
    let comment = PendingComment::on(&file.path, &file.hunks[0], at(VIEWER, 0, 5)).unwrap();
    body.add_pending(comment);
    body.cursor = Some(Cursor::Pending {
        file: VIEWER,
        index: 0,
        row: 0,
    });
    assert_eq!(reply(&body), None);
}

#[test]
fn expand_root_expands_the_thread_and_keeps_the_cursor() {
    let mut body = threaded();
    let _ = draw(&mut body, 100, 40, true);
    body.cursor = on_viewer_thread(0, 0);
    let before = body.rows();
    body.expand_root(100);
    assert!(body.is_expanded(0));
    assert_eq!(body.rows(), before + 4);
    assert_eq!(body.cursor(), on_viewer_thread(0, 0));
    // Already expanded: nothing changes.
    body.expand_root(100);
    assert!(body.is_expanded(0));
    assert_eq!(body.rows(), before + 4);
    // A thread not shown yet is expanded once it is.
    body.expand_root(500);
    body.set_threads(
        vec![thread(500, "src/viewer.rs", Some(6), Some(6), 0)],
        "parity-bot/pr-details",
    );
    assert!(body.is_expanded(0));
}
