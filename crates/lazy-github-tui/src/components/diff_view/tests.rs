//! The viewer on its own: its focus ring, the filter, the body's keys, the shared scroll, the mouse, the pending
//! comments under their lines and the review form, against PR 5's diff from the `pr-details` fixtures.

use std::path::Path;

use lazy_github_core::diff;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;

use super::*;
use crate::keymap::{KeySpec, Keymap};
use crate::test_util::buffer_to_string;
use lazy_github_core::github::ReviewEvent;

/// PR 5's diff: README.md (one hunk of 6 lines), assets/app.css (added, 3), build.rs (4), docs/old.md (deleted) and
/// src/viewer.rs (two hunks, of 8 and 9 lines). The body has 40 rows.
fn diff_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/GET.diff.json");
    let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

fn view() -> DiffView {
    DiffView::new(diff::parse(&diff_text()).unwrap())
}

const README: usize = 0;
const CSS: usize = 1;
const OLD: usize = 3;
const VIEWER: usize = 4;

fn at(file: usize, hunk: usize, line: usize) -> DiffLineRef {
    DiffLineRef { file, hunk, line }
}

/// Where the body's cursor is.
fn cursor(view: &DiffView) -> Option<Cursor> {
    view.body().cursor()
}

/// The cursor on a line, as [`cursor`] gives it.
#[allow(clippy::unnecessary_wraps)]
fn on_line(file: usize, hunk: usize, line: usize) -> Option<Cursor> {
    Some(Cursor::Line(at(file, hunk, line)))
}

/// Presses `keys` on the view as the app would: the filter first, then the chain's links in order.
fn press(view: &mut DiffView, keys: &[&str]) {
    let keymap = Keymap::defaults();
    for key in keys {
        let KeySpec::Chord(chord) = KeySpec::parse(key) else {
            panic!("{key} isn't a key");
        };
        if view
            .handle_key(&chord, &keymap, "", tokio::time::Instant::now())
            .is_some()
        {
            continue;
        }
        let mut chain = view.chain();
        ChainLink::filter_behind_inputs(&mut chain);
        let handled = chain.iter().any(|link| {
            keymap
                .link_matches(link, &chord)
                .into_iter()
                .any(|id| view.on_binding(link.owner, id).is_some())
        });
        assert!(handled, "{key} did nothing at {:?}", view.stop());
    }
}

fn draw_buffer(view: &mut DiffView, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), &Theme::default(), true))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn draw(view: &mut DiffView, width: u16, height: u16) -> String {
    buffer_to_string(&draw_buffer(view, width, height))
}

/// Every stop `tab` walks from the first, until focus would leave the viewer.
fn ring(view: &mut DiffView) -> Vec<DiffStop> {
    view.focus(DiffStop::Filter);
    let mut stops = vec![view.stop()];
    while view.focus_step(true) {
        stops.push(view.stop());
    }
    stops
}

/// The screen row `text` is first drawn on.
fn row_of(screen: &str, text: &str) -> u16 {
    let row = screen
        .lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("{text:?} isn't on screen:\n{screen}"));
    u16::try_from(row).unwrap()
}

// The ring.

#[test]
fn the_ring_is_the_filter_the_body_then_the_form() {
    use DiffStop::{Body, Filter, ReviewStatus, ReviewSubmit, ReviewSummary};
    let mut view = view();
    assert_eq!(ring(&mut view), [Filter, Body]);
    let mut view = with_two_comments();
    view.show_review_form(false);
    assert_eq!(
        ring(&mut view),
        [Filter, Body, ReviewStatus, ReviewSummary, ReviewSubmit]
    );
    // The pending comments are in the body: they're no stops of their own.
    // From before, focus lands on the body; from after, on the last stop. Past either end it leaves.
    view.focus_first();
    assert_eq!(view.stop(), Body);
    assert!(view.focus_step(false));
    assert_eq!(view.stop(), Filter);
    assert!(!view.focus_step(false));
    view.focus_last();
    assert_eq!(view.stop(), ReviewSubmit);
    assert!(!view.focus_step(true));
}

#[test]
fn without_a_body_the_filter_is_the_first_stop() {
    let mut view = DiffView::new(diff::parse("").unwrap());
    view.focus_first();
    assert_eq!(view.stop(), DiffStop::Filter);
    assert_eq!(ring(&mut view), [DiffStop::Filter]);
    view.show_review_form(true);
    view.focus_first();
    assert_eq!(view.stop(), DiffStop::Filter);
    assert_eq!(
        ring(&mut view),
        [DiffStop::Filter, DiffStop::ReviewSummary, DiffStop::ReviewSubmit]
    );
}

// The body's keys.

#[test]
fn j_and_k_move_the_cursor_across_hunks_and_files() {
    let mut view = view();
    view.focus_first();
    draw(&mut view, 100, 30);
    assert_eq!(cursor(&view), Some(Cursor::File(README)));
    // The hunk separator can't hold the cursor: one `j` from the header is the hunk's first line.
    press(&mut view, &["j"]);
    assert_eq!(cursor(&view), on_line(README, 0, 0));
    // Past README.md's last line onto assets/app.css's header.
    press(&mut view, &["j", "j", "j", "j", "j", "j"]);
    assert_eq!(cursor(&view), Some(Cursor::File(CSS)));
    press(&mut view, &["k"]);
    assert_eq!(cursor(&view), on_line(README, 0, 5));
    assert!(draw(&mut view, 100, 30).contains("Line: 6/6"));
}

#[test]
fn big_j_and_k_jump_hunks_from_the_body_and_from_the_form() {
    let mut view = view();
    view.show_review_form(false);
    view.focus_first();
    press(&mut view, &["J", "J", "J", "J", "J"]);
    assert_eq!(cursor(&view), on_line(VIEWER, 1, 0));
    press(&mut view, &["K"]);
    assert_eq!(cursor(&view), on_line(VIEWER, 0, 0));
    // From the form, the body takes focus and its cursor jumps.
    view.focus(DiffStop::ReviewSubmit);
    press(&mut view, &["J"]);
    assert_eq!(view.stop(), DiffStop::Body);
    assert_eq!(cursor(&view), on_line(VIEWER, 1, 0));
}

#[test]
fn the_cursor_s_row_stays_in_view() {
    let mut view = view();
    view.focus_first();
    draw(&mut view, 100, 20);
    press(&mut view, &["end"]);
    let screen = draw(&mut view, 100, 20);
    assert!(view.offset() > 0);
    assert!(screen.contains("►   28  │     }"), "{screen}");
    press(&mut view, &["home"]);
    draw(&mut view, 100, 20);
    assert_eq!(view.offset(), 0);
}

#[test]
fn the_position_row_stays_at_the_bottom_while_the_body_fills_the_view() {
    let mut view = view();
    view.focus_first();
    let screen = draw(&mut view, 100, 20);
    let last = screen.lines().last().unwrap();
    assert!(
        last.starts_with(" README.md") && last.trim_end().ends_with("1 hunk"),
        "{last}"
    );
    press(&mut view, &["j"]);
    let screen = draw(&mut view, 100, 20);
    assert!(screen.lines().last().unwrap().contains("Line: 1/6"), "{screen}");
}

// The filter.

#[test]
fn j_and_k_type_into_the_filter() {
    let mut view = view();
    press(&mut view, &["/", "J", "K"]);
    assert_eq!(view.stop(), DiffStop::Filter);
    assert_eq!(view.filter_text(), "JK");
    assert!((0..5).all(|f| !view.is_file_shown(f)));
    // The body has no rows, so it isn't a stop.
    assert_eq!(ring(&mut view), [DiffStop::Filter]);
}

#[test]
fn typing_filters_by_path_ignoring_case_and_moves_the_cursor_off_a_hidden_file() {
    let mut view = view();
    view.focus_first();
    press(&mut view, &["j", "/"]);
    assert_eq!(cursor(&view), on_line(README, 0, 0));
    press(&mut view, &["V", "i", "E", "w"]);
    let shown: Vec<bool> = (0..5).map(|f| view.is_file_shown(f)).collect();
    assert_eq!(shown, [false, false, false, false, true]);
    assert_eq!(cursor(&view), Some(Cursor::File(VIEWER)));
    // The full path counts, not just the name.
    press(&mut view, &["ctrl+u", "d", "o", "c", "s", "/"]);
    let shown: Vec<bool> = (0..5).map(|f| view.is_file_shown(f)).collect();
    assert_eq!(shown, [false, false, false, true, false]);
    // Empty text shows them all.
    press(&mut view, &["ctrl+u"]);
    assert!((0..5).all(|f| view.is_file_shown(f)));
}

#[test]
fn enter_leaves_the_filter_for_the_first_shown_file_s_header() {
    let mut view = view();
    view.focus_first();
    press(&mut view, &["J", "/", "v", "i", "e", "w", "enter"]);
    assert_eq!(view.stop(), DiffStop::Body);
    assert_eq!(cursor(&view), Some(Cursor::File(VIEWER)));
    assert_eq!(view.filter_text(), "view");
    // With nothing shown, focus stays.
    press(&mut view, &["/", "z", "z", "enter"]);
    assert_eq!(view.stop(), DiffStop::Filter);
}

#[test]
fn escape_clears_the_filter_and_focuses_the_body_with_the_cursor_kept() {
    let mut view = view();
    view.focus_first();
    press(&mut view, &["J", "J", "j", "/", "c", "s", "s", "escape"]);
    assert_eq!(view.stop(), DiffStop::Body);
    assert_eq!(view.filter_text(), "");
    assert!((0..5).all(|f| view.is_file_shown(f)));
    assert_eq!(cursor(&view), on_line(CSS, 0, 1));
}

#[test]
fn a_paste_filters() {
    let mut view = view();
    view.focus(DiffStop::Filter);
    assert!(view.handle_paste("old\nignored"));
    assert_eq!(view.filter_text(), "old");
    assert!(view.is_file_shown(OLD) && !view.is_file_shown(README));
    view.focus(DiffStop::Body);
    assert!(!view.handle_paste("x"));
}

#[test]
fn an_invalid_diff_shows_the_filter_and_the_reason() {
    let mut view = DiffView::invalid("no hunks");
    let screen = draw(&mut view, 80, 10);
    assert!(
        screen.contains("Filter files (press '/' to focus, ESC to clear)"),
        "{screen}"
    );
    assert!(
        screen.contains("error parsing diff - please view on github: no hunks"),
        "{screen}"
    );
    view.focus_first();
    assert_eq!(view.stop(), DiffStop::Filter);
}

#[test]
fn slash_from_anywhere_focuses_the_filter() {
    let mut view = with_two_comments();
    view.show_review_form(false);
    for stop in [DiffStop::Body, DiffStop::ReviewStatus, DiffStop::ReviewSubmit] {
        view.focus(stop);
        press(&mut view, &["/"]);
        assert_eq!(view.stop(), DiffStop::Filter, "from {stop:?}");
    }
}

// The footer's view of the body.

#[test]
fn add_comment_is_hidden_off_a_diff_line() {
    let mut view = view();
    view.focus_first();
    let state = |view: &DiffView| view.binding_state(Owner::DiffBody, BindingId::DiffAddComment);
    assert_eq!(state(&view), BindingState::Hidden);
    press(&mut view, &["J"]);
    assert_eq!(state(&view), BindingState::Enabled);
    // The deleted file's `file was removed` row isn't a line either.
    view.focus(DiffStop::Body);
    assert_eq!(
        view.binding_state(Owner::DiffBody, BindingId::DiffNextHunk),
        BindingState::Enabled
    );
}

// The mouse.

fn up(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

#[test]
fn a_click_on_a_row_focuses_the_body_and_moves_the_cursor() {
    let mut view = view();
    view.focus(DiffStop::Filter);
    let screen = draw(&mut view, 100, 30);
    let row = row_of(&screen, "color: yellow;");
    view.click(Position::new(20, row));
    assert_eq!(view.stop(), DiffStop::Body);
    assert_eq!(cursor(&view), on_line(CSS, 0, 1));
    // A hunk separator gives the cursor to the hunk's first line.
    let row = row_of(&screen, "Lines 1-4 (+1)");
    view.click(Position::new(4, row));
    assert_eq!(cursor(&view), on_line(2, 0, 0));
    // A header takes it; past its title, the file stays unfolded.
    let row = row_of(&screen, "README.md (+3 -1)");
    view.click(Position::new(60, row));
    assert_eq!(cursor(&view), Some(Cursor::File(README)));
    assert!(!view.body().is_folded(README));
    // The click didn't scroll.
    draw(&mut view, 100, 30);
    assert_eq!(view.offset(), 0);
    // The filter takes focus.
    view.click(Position::new(10, 1));
    assert_eq!(view.stop(), DiffStop::Filter);
}

#[test]
fn the_wheel_scrolls_and_keeps_the_cursor_on_a_row_in_view() {
    let mut view = view();
    view.focus_first();
    draw(&mut view, 100, 20);
    assert_eq!(cursor(&view), Some(Cursor::File(README)));
    view.wheel(Wheel::Down);
    // Two rows a notch, as the app's other scrolls.
    assert_eq!(view.offset(), 2);
    // The header and the separator scrolled out of view: the cursor is on the first row still in it that README.md's
    // sticky header doesn't cover, its second line.
    assert_eq!(cursor(&view), on_line(README, 0, 1));
    let screen = draw(&mut view, 100, 20);
    assert_eq!(row_of(&screen, "README.md (+3 -1)"), 3, "{screen}");
    assert_eq!(row_of(&screen, "►    4  │"), 4, "{screen}");
    // Next `j` continues from there.
    press(&mut view, &["j"]);
    assert_eq!(cursor(&view), on_line(README, 0, 2));
    // Scrolled back up, a cursor still in view stays where it is.
    view.wheel(Wheel::Up);
    assert_eq!(view.offset(), 0);
    assert_eq!(cursor(&view), on_line(README, 0, 2));
}

#[test]
fn a_click_on_a_header_s_title_folds_and_unfolds_its_file() {
    let mut view = view();
    let screen = draw(&mut view, 100, 30);
    let row = row_of(&screen, "README.md (+3 -1)");
    view.click(Position::new(4, row));
    assert_eq!(view.stop(), DiffStop::Body);
    assert_eq!(cursor(&view), Some(Cursor::File(README)));
    assert!(view.body().is_folded(README));
    let screen = draw(&mut view, 100, 30);
    assert!(screen.contains("▶ 📄  README.md (+3 -1)"), "{screen}");
    view.click(Position::new(4, row));
    assert!(!view.body().is_folded(README));
}

#[test]
fn the_sticky_header_shows_the_cursor_s_file_and_a_click_on_it_goes_to_the_header() {
    let mut view = view();
    view.focus_first();
    // 100x20: 16 rows of body under the filter. `G` scrolls src/viewer.rs's header (row 20) above the top (row 24).
    draw(&mut view, 100, 20);
    press(&mut view, &["G"]);
    let screen = draw(&mut view, 100, 20);
    assert_eq!(view.offset(), 24);
    let top = screen.lines().nth(3).unwrap();
    assert!(top.starts_with("▼ 📄  src/viewer.rs (+5 -2)"), "{screen}");
    // The click goes to the real header, which scrolls into view, and the sticky header goes.
    view.click(Position::new(40, 3));
    assert_eq!(cursor(&view), Some(Cursor::File(VIEWER)));
    let screen = draw(&mut view, 100, 20);
    assert_eq!(view.offset(), 20 - 3);
    assert_eq!(row_of(&screen, "src/viewer.rs (+5 -2)"), 3 + 3, "{screen}");
}

#[test]
fn the_wheel_keeps_the_cursor_off_the_row_under_the_sticky_header() {
    let mut view = view();
    view.focus_first();
    draw(&mut view, 100, 20);
    // On src/viewer.rs's first line (row 22), then the view scrolls past it: the top row it lands on is under the
    // sticky header, so the cursor goes one row lower.
    press(&mut view, &["J", "J", "J", "J"]);
    assert_eq!(cursor(&view), on_line(VIEWER, 0, 0));
    draw(&mut view, 100, 20);
    for _ in 0..8 {
        view.wheel(Wheel::Down);
        draw(&mut view, 100, 20);
    }
    let top = view.offset();
    assert!(top > 22, "{top}");
    assert_eq!(view.body().cursor_row(), Some(top + 1));
    assert_eq!(view.body().sticky_file(top..top + 16), Some(VIEWER));
}

#[test]
fn fold_shows_on_a_header_and_fold_files_reads_unfold_files_once_everything_is_folded() {
    let mut view = view();
    view.focus_first();
    let state = |view: &DiffView, id| view.binding_state(Owner::DiffBody, id);
    assert_eq!(state(&view, BindingId::DiffToggle), BindingState::Enabled);
    assert_eq!(state(&view, BindingId::DiffFoldAll), BindingState::Enabled);
    press(&mut view, &["j"]);
    assert_eq!(state(&view, BindingId::DiffToggle), BindingState::Hidden);
    press(&mut view, &["z"]);
    assert_eq!(cursor(&view), Some(Cursor::File(README)));
    assert_eq!(
        state(&view, BindingId::DiffFoldAll),
        BindingState::Relabeled("Unfold Files")
    );
    press(&mut view, &["enter"]);
    assert_eq!(state(&view, BindingId::DiffFoldAll), BindingState::Enabled);
}

#[test]
fn a_click_on_a_pending_comment_moves_the_cursor_there() {
    let mut view = with_two_comments();
    view.focus(DiffStop::Filter);
    let screen = draw(&mut view, 100, 40);
    let row = row_of(&screen, "Mention the arrow keys too.");
    view.click(Position::new(30, row));
    assert_eq!(view.stop(), DiffStop::Body);
    assert_eq!(cursor(&view), on_pending(0, 1));
    // There's nothing to release on: the comment stays.
    assert_eq!(view.mouse(up(30, row)), ViewMouse::Ignored);
    assert_eq!(view.pending().len(), 2);
}

// Pending comments (#272).

/// A pending comment on line `line` of README.md's hunk.
fn comment(view: &DiffView, line: usize, body: &str) -> PendingComment {
    let file = view.body().file(README).unwrap();
    PendingComment {
        body: body.to_owned(),
        ..PendingComment::on(&file.path, &file.hunks[0], at(README, 0, line)).unwrap()
    }
}

/// The view with comments on README.md's lines 4 and 1, in that order.
fn with_two_comments() -> DiffView {
    let mut view = view();
    let first = comment(&view, 3, "Mention the arrow keys too.");
    let second = comment(&view, 0, "A heading for\nthe usage.");
    view.add_comment(first);
    view.add_comment(second);
    view
}

fn bodies(view: &DiffView) -> Vec<&str> {
    view.pending().iter().map(|c| c.body.as_str()).collect()
}

#[test]
fn the_comment_target_is_the_body_s_cursor_line() {
    let mut view = view();
    view.focus_first();
    // On a header, nothing.
    assert_eq!(view.comment_target(), None);
    press(&mut view, &["J", "j", "j", "j"]);
    let target = view.comment_target().unwrap();
    assert_eq!(
        (
            target.path.as_str(),
            target.line,
            target.line_number,
            target.position,
            target.line_text.as_str()
        ),
        (
            "README.md",
            at(README, 0, 3),
            5,
            4,
            "+Run `viewer FILE`, then press `j` and `k` to move."
        )
    );
    assert_eq!(target.body, "");
    // Only with the body focused.
    view.focus(DiffStop::Filter);
    assert_eq!(view.comment_target(), None);
}

/// The cursor on row `row` of pending comment `index`, in README.md.
#[allow(clippy::unnecessary_wraps)]
fn on_pending(index: usize, row: usize) -> Option<Cursor> {
    Some(Cursor::Pending {
        file: README,
        index,
        row,
    })
}

/// Whether `key` is taken by the view's chain (not before it).
fn chain_takes(view: &mut DiffView, key: &str) -> bool {
    let keymap = Keymap::defaults();
    let KeySpec::Chord(chord) = KeySpec::parse(key) else {
        panic!("{key} isn't a key");
    };
    let chain = view.chain();
    chain.iter().any(|link| {
        keymap
            .link_matches(link, &chord)
            .into_iter()
            .any(|id| view.on_binding(link.owner, id).is_some())
    })
}

#[test]
fn pending_comments_show_under_their_lines_in_the_order_added() {
    let mut view = with_two_comments();
    let third = comment(&view, 3, "And `q`.");
    view.add_comment(third);
    let screen = draw(&mut view, 100, 40);
    let pending = |index, row| Row::Pending {
        file: README,
        index,
        row,
    };
    let line = |l| Row::Line(at(README, 0, l));
    assert_eq!(
        (0..15).map(|r| view.body().row(r).unwrap()).collect::<Vec<_>>(),
        [
            Row::FileHeader(README),
            Row::HunkSeparator(README, 0),
            line(0),
            pending(1, 0),
            pending(1, 1),
            pending(1, 2),
            line(1),
            line(2),
            line(3),
            pending(0, 0),
            pending(0, 1),
            pending(2, 0),
            pending(2, 1),
            line(4),
            line(5),
        ]
    );
    // The title, then the body as typed (its line break kept), behind the bar; the line's flag cell is marked.
    let title = usize::from(row_of(&screen, "▎ Pending comment"));
    assert!(screen.lines().nth(title - 1).unwrap().contains('●'), "{screen}");
    assert!(screen.contains("          ▎ A heading for"), "{screen}");
    assert!(screen.contains("          ▎ the usage."), "{screen}");
    assert!(screen.contains("          ▎ And `q`."), "{screen}");
    // README.md's header counts them.
    assert!(screen.contains("README.md (+3 -1) · 3 comments"), "{screen}");
}

#[test]
fn x_on_each_row_of_a_comment_removes_it_and_the_cursor_goes_to_its_line() {
    for row in 0..3 {
        let mut view = with_two_comments();
        view.focus_first();
        draw(&mut view, 100, 40);
        // From the header: line 1 (the separator is skipped), then the comment's rows.
        press(&mut view, &["j"]);
        for _ in 0..=row {
            press(&mut view, &["j"]);
        }
        assert_eq!(cursor(&view), on_pending(1, row));
        assert_eq!(
            view.binding_state(Owner::DiffBody, BindingId::DiffRemoveComment),
            BindingState::Enabled
        );
        press(&mut view, &["x"]);
        assert_eq!(bodies(&view), ["Mention the arrow keys too."], "row {row}");
        assert_eq!(cursor(&view), on_line(README, 0, 0));
        assert!(!draw(&mut view, 100, 40).contains("A heading for"));
    }
}

#[test]
fn remove_comment_is_hidden_off_a_pending_comment_and_x_does_nothing_there() {
    let mut view = with_two_comments();
    view.focus_first();
    draw(&mut view, 100, 40);
    let state = |view: &DiffView| view.binding_state(Owner::DiffBody, BindingId::DiffRemoveComment);
    // A header, a line with comments, a line without.
    for keys in [&[][..], &["j"], &["j", "j", "j", "j"]] {
        press(&mut view, keys);
        assert_eq!(state(&view), BindingState::Hidden, "{:?}", cursor(&view));
        assert!(!chain_takes(&mut view, "x"), "{:?}", cursor(&view));
        assert_eq!(view.pending().len(), 2);
    }
    // `c` is hidden on a pending comment, as on a thread.
    press(&mut view, &["k"]);
    assert_eq!(cursor(&view), on_pending(1, 2));
    assert_eq!(
        view.binding_state(Owner::DiffBody, BindingId::DiffAddComment),
        BindingState::Hidden
    );
}

#[test]
fn removing_a_comment_before_the_cursor_s_keeps_the_cursor_on_its_comment() {
    let mut view = with_two_comments();
    let third = comment(&view, 5, "third");
    view.add_comment(third);
    view.focus_first();
    draw(&mut view, 100, 40);
    // On the third comment added, then the first one added goes: the cursor stays on the third, now second.
    press(&mut view, &["n", "n", "n"]);
    assert_eq!(cursor(&view), on_pending(2, 0));
    view.with_body(|body| body.remove_pending(0));
    assert_eq!(cursor(&view), on_pending(1, 0));
    assert_eq!(bodies(&view), ["A heading for\nthe usage.", "third"]);
    assert!(draw(&mut view, 100, 40).contains("Comment 2/2"));
    // One after it going leaves it be.
    press(&mut view, &["N"]);
    assert_eq!(cursor(&view), on_pending(0, 0));
    view.with_body(|body| body.remove_pending(1));
    assert_eq!(cursor(&view), on_pending(0, 0));
}

#[test]
fn n_and_big_n_stop_on_pending_comments_and_show_without_threads() {
    let state = |view: &DiffView| view.binding_state(Owner::DiffBody, BindingId::DiffNextComment);
    let mut view = view();
    view.focus_first();
    assert_eq!(state(&view), BindingState::Hidden);
    let mut view = with_two_comments();
    view.focus_first();
    draw(&mut view, 100, 40);
    assert_eq!(state(&view), BindingState::Enabled);
    press(&mut view, &["n"]);
    assert_eq!(cursor(&view), on_pending(1, 0));
    assert!(draw(&mut view, 100, 40).contains("Comment 1/2"));
    press(&mut view, &["n"]);
    assert_eq!(cursor(&view), on_pending(0, 0));
    assert!(draw(&mut view, 100, 40).contains("Comment 2/2"));
    // The last: `n` stays.
    press(&mut view, &["n"]);
    assert_eq!(cursor(&view), on_pending(0, 0));
    // From the comment's second row, `N` goes to its title, then the one before.
    press(&mut view, &["j", "N"]);
    assert_eq!(cursor(&view), on_pending(0, 0));
    press(&mut view, &["N"]);
    assert_eq!(cursor(&view), on_pending(1, 0));
    // `J` from a comment goes on from its line; `K` back to its hunk's first line.
    press(&mut view, &["J"]);
    assert_eq!(cursor(&view), on_line(CSS, 0, 0));
    press(&mut view, &["N", "K"]);
    assert_eq!(cursor(&view), on_line(README, 0, 0));
    // `enter` isn't shown on one, and does nothing.
    press(&mut view, &["n"]);
    assert_eq!(
        view.binding_state(Owner::DiffBody, BindingId::DiffToggle),
        BindingState::Hidden
    );
    press(&mut view, &["enter"]);
    assert_eq!(cursor(&view), on_pending(1, 0));
}

#[test]
fn a_comment_on_a_filtered_out_file_is_hidden_but_still_sent_and_counted() {
    let mut view = with_two_comments();
    view.show_review_form(false);
    press(&mut view, &["/", "v", "i", "e", "w"]);
    let screen = draw(&mut view, 100, 60);
    assert!(!screen.contains("Pending comment"), "{screen}");
    assert!(
        screen.contains("2 pending comments will be sent with the review"),
        "{screen}"
    );
    assert_eq!(view.body().comment_count(), 0);
    assert_eq!(view.review_request("abc").unwrap().unwrap().comments.len(), 2);
    // Folding hides them with their file too.
    press(&mut view, &["escape", "home", "enter"]);
    assert!(view.body().is_folded(README));
    assert!(!draw(&mut view, 100, 60).contains("Pending comment"));
}

#[test]
fn the_note_above_the_form_counts_the_pending_comments() {
    let mut view = view();
    view.show_review_form(false);
    assert!(!draw(&mut view, 100, 80).contains("will be sent with the review"));
    let first = comment(&view, 3, "x");
    view.add_comment(first);
    let screen = draw(&mut view, 100, 80);
    // After the body's position row, then a blank row, then the form.
    let note = row_of(&screen, "1 pending comment will be sent with the review");
    assert!(row_of(&screen, "1 hunk") < note, "{screen}");
    assert_eq!(row_of(&screen, "Review Status:"), note + 2, "{screen}");
}

#[test]
fn a_stop_after_the_body_scrolls_into_view_and_the_body_s_cursor_back() {
    let mut view = with_two_comments();
    view.show_review_form(false);
    view.focus_first();
    draw(&mut view, 80, 30);
    assert_eq!(view.offset(), 0);
    view.focus(DiffStop::ReviewSubmit);
    let screen = draw(&mut view, 80, 30);
    assert!(screen.contains("Submit Review"), "{screen}");
    assert!(view.offset() > 0);
    // Back on the body, its cursor (on README.md's header) shows again.
    view.focus(DiffStop::Body);
    let screen = draw(&mut view, 80, 30);
    assert!(screen.contains("README.md (+3 -1)"), "{screen}");
    assert_eq!(view.offset(), 0);
}

// The review form (#273).

#[test]
fn the_author_s_form_has_no_status() {
    use DiffStop::{Body, ReviewSubmit, ReviewSummary};
    let mut view = with_two_comments();
    view.show_review_form(true);
    let stops = ring(&mut view);
    assert_eq!(stops[stops.len() - 3..], [Body, ReviewSummary, ReviewSubmit]);
    assert_eq!(view.review_button(), Some("Add Comments"));
}

#[test]
fn an_invalid_diff_shows_no_form() {
    let mut view = DiffView::invalid("bad");
    view.show_review_form(false);
    assert!(!view.has_review_form());
}

#[test]
fn the_review_sends_each_comment_s_diff_position_in_order() {
    let mut view = with_two_comments();
    view.show_review_form(false);
    let review = view.review_request("abc").unwrap().unwrap();
    assert_eq!(review.event, ReviewEvent::Comment);
    assert_eq!((review.commit_id.as_str(), review.body.as_str()), ("abc", ""));
    let comments: Vec<(&str, u32)> = review.comments.iter().map(|c| (c.body.as_str(), c.position)).collect();
    assert_eq!(
        comments,
        [("Mention the arrow keys too.", 4), ("A heading for\nthe usage.", 1)]
    );
}

#[test]
fn the_author_always_comments_and_the_blank_status_is_missing() {
    let mut view = view();
    view.show_review_form(true);
    assert_eq!(view.review_request("abc").unwrap().unwrap().event, ReviewEvent::Comment);
    let mut view = self::view();
    view.show_review_form(false);
    view.focus(DiffStop::ReviewStatus);
    press(&mut view, &["enter", "home", "enter"]);
    assert_eq!(view.review_status(), Some(None));
    assert_eq!(view.review_request("abc"), Some(Err(MissingStatus)));
}

#[test]
fn typing_goes_into_the_summary_and_tab_closes_an_open_list() {
    let mut view = view();
    view.show_review_form(false);
    view.focus(DiffStop::ReviewSummary);
    press(&mut view, &["o", "k", "J"]);
    assert_eq!(view.review_summary(), Some("okJ"));
    view.focus(DiffStop::ReviewStatus);
    press(&mut view, &["enter"]);
    assert!(view.status_open());
    assert!(view.focus_step(true));
    assert!(!view.status_open());
}

#[test]
fn the_status_list_opens_over_the_view() {
    let mut view = view();
    view.show_review_form(false);
    view.focus(DiffStop::ReviewStatus);
    press(&mut view, &["enter"]);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal
        .draw(|frame| {
            view.render(frame, frame.area(), &Theme::default(), true);
            view.render_popups(frame, &Theme::default());
        })
        .unwrap();
    let screen = buffer_to_string(terminal.backend().buffer());
    for option in ["Approved", "Changes Requested", "Commented"] {
        assert!(screen.contains(option), "{option}: {screen}");
    }
}

// The window's rows.

#[test]
fn the_window_follows_the_body_with_its_position_row_then_the_tail() {
    let window = Window {
        body: 40,
        tail: 10,
        height: 20,
    };
    assert_eq!(window.content(), 51);
    // The body fills the view: its position row takes the last row.
    assert_eq!(
        window.split(0),
        Split {
            body: 0..19,
            position: Some(19),
            tail: 0..0,
            tail_top: 20
        }
    );
    // The body's end in view: its rows, the position row, then the tail.
    assert_eq!(
        window.split(35),
        Split {
            body: 35..40,
            position: Some(5),
            tail: 0..10,
            tail_top: 6
        }
    );
    // Past the position row, the tail alone.
    assert_eq!(
        window.split(45),
        Split {
            body: 45..45,
            position: None,
            tail: 4..10,
            tail_top: 0
        }
    );
    // No body: the tail from the top.
    let window = Window {
        body: 0,
        tail: 10,
        height: 4,
    };
    assert_eq!(window.content(), 10);
    assert_eq!(window.split(3).tail, 3..7);
}
