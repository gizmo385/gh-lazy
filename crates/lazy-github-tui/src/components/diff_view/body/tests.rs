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
    assert!(!body.handle(BindingId::TextualScrollUp));
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
