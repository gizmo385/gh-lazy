//! The viewer on its own: its focus ring, the filter, `J` / `K` and the line cursor, against PR 5's diff from the
//! `pr-details` fixtures.

use std::path::Path;

use lazy_github_core::diff;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;

use super::*;
use crate::keymap::{KeySpec, Keymap};
use crate::test_util::{buffer_to_string, buffer_to_styled_string};
use lazy_github_core::github::ReviewEvent;

/// PR 5's diff: README.md (one hunk), assets/app.css (added), build.rs, docs/old.md (deleted) and src/viewer.rs (two
/// hunks).
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
const BUILD: usize = 2;
const OLD: usize = 3;
const VIEWER: usize = 4;

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

fn draw(view: &mut DiffView, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), &Theme::default(), true))
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

/// Every stop `tab` walks from the scroll area, until focus would leave the viewer.
fn ring(view: &mut DiffView) -> Vec<DiffStop> {
    view.focus_first();
    let mut stops = vec![view.stop()];
    while view.focus_step(true) {
        stops.push(view.stop());
    }
    stops
}

#[test]
fn files_are_open_but_deleted_ones_and_hunks_are_closed() {
    let view = view();
    let open: Vec<bool> = (0..5).map(|f| view.is_file_open(f)).collect();
    assert_eq!(open, [true, true, true, false, true]);
    assert!(!view.is_hunk_open(VIEWER, 0));
    assert!(!view.is_hunk_open(VIEWER, 1));
}

#[test]
fn the_ring_walks_the_shown_titles_and_open_panes() {
    use DiffStop::{FileTitle, Filter, HunkTitle, Pane, Scroll};
    let mut view = view();
    assert_eq!(
        ring(&mut view),
        [
            Scroll,
            Filter,
            FileTitle(README),
            HunkTitle(README, 0),
            FileTitle(CSS),
            HunkTitle(CSS, 0),
            FileTitle(BUILD),
            HunkTitle(BUILD, 0),
            FileTitle(OLD),
            FileTitle(VIEWER),
            HunkTitle(VIEWER, 0),
            HunkTitle(VIEWER, 1),
        ]
    );
    // An open hunk adds its pane; a closed file hides its hunks; a filtered-out file is skipped; an open deleted
    // file has nothing to focus.
    view.focus(HunkTitle(README, 0));
    press(&mut view, &["enter"]);
    view.focus(FileTitle(VIEWER));
    press(&mut view, &["enter"]);
    view.focus(FileTitle(OLD));
    press(&mut view, &["enter"]);
    assert_eq!(
        ring(&mut view),
        [
            Scroll,
            Filter,
            FileTitle(README),
            HunkTitle(README, 0),
            Pane(README, 0),
            FileTitle(CSS),
            HunkTitle(CSS, 0),
            FileTitle(BUILD),
            HunkTitle(BUILD, 0),
            FileTitle(OLD),
            FileTitle(VIEWER),
        ]
    );
    view.focus(Filter);
    press(&mut view, &["r", "s"]);
    assert_eq!(
        ring(&mut view),
        [Scroll, Filter, FileTitle(BUILD), HunkTitle(BUILD, 0), FileTitle(VIEWER)]
    );
    // Back from the end, then out past the start.
    view.focus_last();
    assert_eq!(view.stop(), FileTitle(VIEWER));
    assert!(view.focus_step(false));
    assert_eq!(view.stop(), HunkTitle(BUILD, 0));
    view.focus_first();
    assert!(!view.focus_step(false));
}

#[test]
fn j_and_k_move_the_current_line_and_stop_at_the_ends() {
    let mut view = view();
    press(&mut view, &["J"]);
    assert_eq!(view.stop(), DiffStop::Pane(README, 0));
    let cursor = |view: &DiffView| view.pane(README, 0).unwrap().cursor();
    press(&mut view, &["k"]);
    assert_eq!(cursor(&view), 0);
    press(&mut view, &["j", "j", "k"]);
    assert_eq!(cursor(&view), 1);
    assert!(draw(&mut view, 96, 40).contains("Line: 2/6"));
    press(&mut view, &["j", "j", "j", "j", "j", "j"]);
    assert_eq!(cursor(&view), 5);
    assert!(draw(&mut view, 96, 40).contains("Line: 6/6"));
}

#[test]
fn next_and_previous_hunk_open_what_they_land_on() {
    use DiffStop::{FileTitle, HunkTitle, Pane, Scroll};
    let mut view = view();
    // From the scroll area: the first hunk, or the last.
    press(&mut view, &["K"]);
    assert_eq!(view.stop(), Pane(VIEWER, 1));
    assert!(view.is_hunk_open(VIEWER, 1));
    view.focus(Scroll);
    press(&mut view, &["J"]);
    assert_eq!(view.stop(), Pane(README, 0));
    // From a pane, the next across files; the deleted file has none.
    press(&mut view, &["J", "J", "J"]);
    assert_eq!(view.stop(), Pane(VIEWER, 0));
    // A closed file opens.
    view.focus(FileTitle(VIEWER));
    press(&mut view, &["enter"]);
    assert!(!view.is_file_open(VIEWER));
    view.focus(HunkTitle(BUILD, 0));
    press(&mut view, &["J"]);
    assert_eq!(view.stop(), Pane(VIEWER, 0));
    assert!(view.is_file_open(VIEWER));
    // At the ends, nothing.
    press(&mut view, &["J", "J"]);
    assert_eq!(view.stop(), Pane(VIEWER, 1));
    view.focus(Pane(README, 0));
    press(&mut view, &["K"]);
    assert_eq!(view.stop(), Pane(README, 0));
    // A title outside any hunk counts as outside: `K` goes to the last.
    view.focus(FileTitle(CSS));
    press(&mut view, &["K"]);
    assert_eq!(view.stop(), Pane(VIEWER, 1));
}

/// The view laid out `width` wide, its lines with their styles and where everything landed.
fn laid_out(view: &mut DiffView, width: u16) -> (Vec<String>, String) {
    let theme = Theme::default();
    let lines = view
        .body
        .layout(width, &theme)
        .lines
        .iter()
        .map(|line| format!("{line:?}"))
        .collect();
    (lines, format!("{:?}", view.body.placements(width)))
}

/// What [`laid_out`] gives with every cached layout dropped first, the view's and each file's.
fn laid_out_afresh(view: &mut DiffView, width: u16) -> (Vec<String>, String) {
    view.body.layouts.entries.clear();
    for file in &mut view.body.files {
        file.layouts.clear();
    }
    laid_out(view, width)
}

#[test]
fn a_view_put_together_from_cached_files_is_the_one_laid_out_afresh() {
    // Each step changes one file, or focus, the filter or the comments, and is drawn (at two widths, as the view
    // overflows) before the comparison, so the next one starts from cached layouts (#325).
    let steps: &[&[&str]] = &[
        &["J"],
        &["j", "j"],
        &["J", "J"],
        &["k"],
        &["K"],
        &["shift+tab"],
        &["enter"],
        &["tab", "tab"],
        &["J", "j", "c"],
        &["/", "v"],
        &["escape"],
    ];
    let mut view = view();
    draw(&mut view, 100, 30);
    for keys in steps {
        for key in *keys {
            // Focus moves and comments come from outside the view: the app's focus ring and the comment modal.
            match *key {
                "tab" => {
                    view.focus_step(true);
                }
                "shift+tab" => {
                    view.focus_step(false);
                }
                "c" => {
                    let pending = comment(&view, 1, "a comment");
                    view.add_comment(pending);
                }
                key => press(&mut view, &[key]),
            }
        }
        draw(&mut view, 100, 30);
        for width in [100, 98] {
            let cached = laid_out(&mut view, width);
            let viewports: Vec<usize> = view
                .body
                .files
                .iter()
                .flat_map(|f| &f.hunks)
                .map(|h| h.pane.viewport())
                .collect();
            assert_eq!(cached, laid_out_afresh(&mut view, width), "after {keys:?} at {width}");
            let fresh: Vec<usize> = view
                .body
                .files
                .iter()
                .flat_map(|f| &f.hunks)
                .map(|h| h.pane.viewport())
                .collect();
            assert_eq!(viewports, fresh, "after {keys:?} at {width}");
        }
    }
}

#[test]
fn j_and_k_type_into_the_filter() {
    let mut view = view();
    press(&mut view, &["/", "J", "K"]);
    assert_eq!(view.stop(), DiffStop::Filter);
    assert_eq!(view.filter_text(), "JK");
    assert!((0..5).all(|f| !view.is_file_shown(f)));
}

#[test]
fn typing_filters_by_path_ignoring_case() {
    let mut view = view();
    press(&mut view, &["J", "/"]);
    assert_eq!(view.stop(), DiffStop::Filter);
    press(&mut view, &["V", "i", "E", "w"]);
    let shown: Vec<bool> = (0..5).map(|f| view.is_file_shown(f)).collect();
    assert_eq!(shown, [false, false, false, false, true]);
    // The full path counts, not just the name.
    press(&mut view, &["ctrl+u", "d", "o", "c", "s", "/"]);
    let shown: Vec<bool> = (0..5).map(|f| view.is_file_shown(f)).collect();
    assert_eq!(shown, [false, false, false, true, false]);
    // Empty text shows them all.
    press(&mut view, &["ctrl+u"]);
    assert!((0..5).all(|f| view.is_file_shown(f)));
}

#[test]
fn escape_clears_the_filter_and_leaves_it_for_the_scroll_area() {
    let mut view = view();
    press(&mut view, &["/", "v", "i", "e", "w", "escape"]);
    assert_eq!(view.stop(), DiffStop::Scroll);
    assert_eq!(view.filter_text(), "");
    assert!((0..5).all(|f| view.is_file_shown(f)));
}

#[test]
fn enter_leaves_the_filter_for_the_first_shown_hunk() {
    use DiffStop::{HunkTitle, Pane};
    let mut view = view();
    // Its title while the hunk is closed (Py bug B59)...
    press(&mut view, &["/", "v", "i", "e", "w", "enter"]);
    assert_eq!(view.stop(), HunkTitle(VIEWER, 0));
    assert_eq!(view.filter_text(), "view");
    // ...and its pane once it's open.
    press(&mut view, &["enter", "/", "enter"]);
    assert_eq!(view.stop(), Pane(VIEWER, 0));
}

#[test]
fn the_focused_stop_scrolls_into_view() {
    let mut view = view();
    draw(&mut view, 80, 20);
    assert_eq!(view.offset(), 0);
    press(&mut view, &["K"]);
    let screen = draw(&mut view, 80, 20);
    assert!(view.offset() > 0);
    assert!(screen.contains("Line: 1/9"), "{screen}");
    // Back at the filter, the view scrolls back up.
    press(&mut view, &["/"]);
    draw(&mut view, 80, 20);
    assert_eq!(view.offset(), 0);
}

#[test]
fn clicks_toggle_titles_and_focus_panes_and_the_filter() {
    let mut view = view();
    let screen = draw(&mut view, 96, 60);
    let at = |screen: &str, needle: &str| -> Position {
        let row = screen.lines().position(|line| line.contains(needle)).unwrap();
        let line = screen.lines().nth(row).unwrap();
        let column = line[..line.find(needle).unwrap()].chars().count();
        Position::new(u16::try_from(column).unwrap(), u16::try_from(row).unwrap())
    };
    view.click(at(&screen, "Lines 3-7"));
    assert_eq!(view.stop(), DiffStop::HunkTitle(README, 0));
    assert!(view.is_hunk_open(README, 0));
    let screen = draw(&mut view, 96, 60);
    view.click(at(&screen, "## Usage"));
    assert_eq!(view.stop(), DiffStop::Pane(README, 0));
    view.click(at(&screen, "README.md"));
    assert_eq!(view.stop(), DiffStop::FileTitle(README));
    assert!(!view.is_file_open(README));
    view.click(at(&screen, "Filter files"));
    assert_eq!(view.stop(), DiffStop::Filter);
    // Past a title's end, the scroll area.
    let mut beyond = at(&screen, "app.css");
    beyond.x += 40;
    view.click(beyond);
    assert_eq!(view.stop(), DiffStop::Scroll);
    assert!(view.is_file_open(CSS));
}

#[test]
fn an_invalid_diff_shows_the_filter_and_the_reason() {
    let mut view = DiffView::invalid("unexpected end of the file header of `diff --git a/x b/x`");
    let screen = draw(&mut view, 120, 8);
    assert!(
        screen.contains("Filter files (press '/' to focus, ESC to clear)"),
        "{screen}"
    );
    assert!(
        screen.contains(
            "error parsing diff - please view on github: unexpected end of the file header of `diff --git a/x b/x`"
        ),
        "{screen}"
    );
    assert_eq!(ring(&mut view), [DiffStop::Scroll, DiffStop::Filter]);
}

#[test]
fn the_viewer_keys_are_hidden_only_at_the_scroll_area() {
    let mut view = view();
    let id = BindingId::DiffNextHunk;
    assert_eq!(view.binding_state(Owner::DiffViewer, id), BindingState::Hidden);
    view.focus(DiffStop::Filter);
    assert_eq!(view.binding_state(Owner::DiffViewer, id), BindingState::Enabled);
    assert_eq!(view.binding_state(Owner::DiffFilter, id), BindingState::Enabled);
}

/// Hunk `h` of file `f`'s pane laid out alone, `width` cells wide, its current line `moves` lines down, with its
/// colors.
fn pane_screen(f: usize, h: usize, moves: usize, width: u16) -> String {
    let parsed = diff::parse(&diff_text()).unwrap();
    let file = &parsed.files[f];
    let hunk = &file.hunks[h];
    let mut pane = HunkPane::new(hunk);
    for _ in 0..moves {
        pane.cursor_down();
    }
    let theme = Theme::default();
    let layout = pane.layout(file, hunk, width, &theme, true, theme.background());
    let mut buf = Buffer::empty(Rect::new(0, 0, width, u16::try_from(layout.lines.len()).unwrap()));
    for (y, line) in layout.lines.iter().enumerate() {
        buf.set_line(0, u16::try_from(y).unwrap(), line, width);
    }
    buffer_to_styled_string(&buf)
}

#[test]
fn invalid_utf8_and_controls_in_a_diff_are_drawn_as_replacement_characters() {
    // A binary file's bytes, as `gh` prints them: the body is decoded lossily (`GhResponse::text`).
    let bytes = b"diff --git a/logo.bin b/logo.bin\n--- a/logo.bin\n+++ b/logo.bin\n@@ -1 +1 @@\n-\x89PNG\xff\n+\xfe\x1b[0m ok\n";
    let parsed = diff::parse(&String::from_utf8_lossy(bytes)).unwrap();
    let (file, hunk) = (&parsed.files[0], &parsed.files[0].hunks[0]);
    let theme = Theme::default();
    let layout = HunkPane::new(hunk).layout(file, hunk, 40, &theme, true, theme.background());
    let rows: Vec<String> = layout.lines.iter().map(ToString::to_string).collect();
    assert!(
        rows.iter().any(|row| row.contains("│ \u{fffd}PNG\u{fffd}")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("│ \u{fffd}\u{fffd}[0m ok")),
        "{rows:#?}"
    );
    // Every boxed row is the pane's width: each replacement character takes the one cell it's counted as.
    assert!(
        layout
            .lines
            .iter()
            .filter(|line| line.width() > 0)
            .all(|line| line.width() == 40),
        "{rows:#?}"
    );
}

#[test]
fn pr_diff_hunk_highlighted_rust() {
    insta::assert_snapshot!(pane_screen(VIEWER, 1, 0, 76));
}

#[test]
fn pr_diff_hunk_highlighted_markdown() {
    insta::assert_snapshot!(pane_screen(README, 0, 1, 64));
}

#[test]
fn pr_diff_hunk_current_line_highlighted() {
    // The current line is the added `let marker = …;`: its tokens on the brighter green.
    insta::assert_snapshot!(pane_screen(VIEWER, 1, 4, 76));
}

// Pending comments (#272).

/// A pending comment on line `line` of README.md's hunk.
fn comment(view: &DiffView, line: usize, body: &str) -> PendingComment {
    let file = &view.body.files[README].file;
    let at = DiffLineRef {
        file: README,
        hunk: 0,
        line,
    };
    PendingComment {
        body: body.to_owned(),
        ..PendingComment::on(&file.path, &file.hunks[0], at).unwrap()
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
fn the_comment_target_is_the_focused_pane_s_current_line() {
    let mut view = view();
    assert_eq!(view.comment_target(), None);
    press(&mut view, &["J", "j", "j", "j"]);
    let target = view.comment_target().unwrap();
    assert_eq!(
        (
            target.path.as_str(),
            target.line_number,
            target.position,
            target.line_text.as_str()
        ),
        ("README.md", 5, 4, "+Run `viewer FILE`, then press `j` and `k` to move.")
    );
    assert_eq!(target.body, "");
}

#[test]
fn each_comment_adds_a_remove_stop_after_the_files() {
    use DiffStop::{HunkTitle, RemoveComment};
    let mut view = with_two_comments();
    let stops = ring(&mut view);
    assert_eq!(
        stops[stops.len() - 3..],
        [HunkTitle(VIEWER, 1), RemoveComment(0), RemoveComment(1)]
    );
    view.focus_last();
    assert_eq!(view.stop(), RemoveComment(1));
    assert!(!view.focus_step(true));
}

#[test]
fn the_section_is_hidden_with_no_comments() {
    let mut view = view();
    assert!(!draw(&mut view, 80, 200).contains("Pending Comments:"));
    let first = comment(&view, 3, "x");
    view.add_comment(first);
    let screen = draw(&mut view, 80, 200);
    assert!(screen.contains("Pending Comments:"), "{screen}");
    assert!(screen.contains("README.md:5 - +Run `viewer FILE`"), "{screen}");
}

#[test]
fn remove_drops_the_first_the_last_and_the_only_comment() {
    use DiffStop::{HunkTitle, RemoveComment};
    // The first: focus on the second button follows it, now first.
    let mut view = with_two_comments();
    view.focus(RemoveComment(1));
    view.remove_comment(0);
    assert_eq!(bodies(&view), ["A heading for\nthe usage."]);
    assert_eq!(view.stop(), RemoveComment(0));
    // The last, focused: focus goes to the stop before it.
    let mut view = with_two_comments();
    view.focus(RemoveComment(1));
    press(&mut view, &["enter"]);
    assert_eq!(bodies(&view), ["Mention the arrow keys too."]);
    assert_eq!(view.stop(), RemoveComment(0));
    // The only one: the section hides and focus goes back to the files.
    press(&mut view, &["enter"]);
    assert!(view.pending().is_empty());
    assert_eq!(view.stop(), HunkTitle(VIEWER, 1));
    assert!(!draw(&mut view, 80, 200).contains("Pending Comments:"));
}

#[test]
fn a_click_on_remove_drops_its_comment() {
    let mut view = with_two_comments();
    view.focus(DiffStop::RemoveComment(1));
    let screen = draw(&mut view, 80, 40);
    // The focused button is revealed: find its label's row.
    let row = screen
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("Remove"))
        .map(|(row, _)| row)
        .last()
        .unwrap();
    let row = u16::try_from(row).unwrap();
    view.click(Position::new(8, row));
    assert_eq!(view.stop(), DiffStop::RemoveComment(1));
    let up = |column, row| MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    assert_eq!(view.mouse(up(8, row)), ViewMouse::Consumed);
    assert_eq!(bodies(&view), ["Mention the arrow keys too."]);
    // A release without a press on a button does nothing.
    assert_eq!(view.mouse(up(8, row)), ViewMouse::Ignored);
}

#[test]
fn pr_review_pending_comments() {
    let mut view = with_two_comments();
    view.focus(DiffStop::RemoveComment(1));
    let mut terminal = Terminal::new(TestBackend::new(96, 40)).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), &Theme::default(), true))
        .unwrap();
    insta::assert_snapshot!(buffer_to_styled_string(terminal.backend().buffer()));
}

// The review form (#273).

#[test]
fn the_form_s_stops_come_last_and_the_author_s_has_no_status() {
    use DiffStop::{RemoveComment, ReviewStatus, ReviewSubmit, ReviewSummary};
    let mut view = with_two_comments();
    view.show_review_form(false);
    let stops = ring(&mut view);
    assert_eq!(
        stops[stops.len() - 5..],
        [
            RemoveComment(0),
            RemoveComment(1),
            ReviewStatus,
            ReviewSummary,
            ReviewSubmit
        ]
    );
    let mut view = with_two_comments();
    view.show_review_form(true);
    let stops = ring(&mut view);
    assert_eq!(
        stops[stops.len() - 3..],
        [RemoveComment(1), ReviewSummary, ReviewSubmit]
    );
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

// Maximize (#288, #295).

#[test]
fn maximize_scope_is_the_section_or_the_tab_area_for_every_stop() {
    use DiffStop::{
        FileTitle, Filter, HunkTitle, Pane, RemoveComment, ReviewStatus, ReviewSubmit, ReviewSummary, Scroll,
    };
    for author in [false, true] {
        let mut view = with_two_comments();
        view.show_review_form(author);
        view.focus(HunkTitle(README, 0));
        press(&mut view, &["enter"]);
        let stops = ring(&mut view);
        assert!(stops.contains(&Pane(README, 0)));
        assert_eq!(stops.contains(&ReviewStatus), !author);
        for stop in stops {
            view.focus(stop);
            assert_eq!(view.stop(), stop);
            let expected = match stop {
                Scroll => DiffMaximize::Nothing,
                FileTitle(f) => DiffMaximize::Section(DiffSection::File(f)),
                HunkTitle(f, h) | Pane(f, h) => DiffMaximize::Section(DiffSection::Hunk(f, h)),
                Filter | RemoveComment(_) | ReviewStatus | ReviewSummary | ReviewSubmit => DiffMaximize::TabArea,
            };
            assert_eq!(view.maximize_scope(), expected, "{stop:?} (author: {author})");
        }
    }
    // Pin a few by hand, so the table above isn't the only statement of the rule.
    let mut view = with_two_comments();
    view.show_review_form(true);
    for (stop, expected) in [
        (Scroll, DiffMaximize::Nothing),
        (Filter, DiffMaximize::TabArea),
        (RemoveComment(1), DiffMaximize::TabArea),
        (ReviewSummary, DiffMaximize::TabArea),
        (ReviewSubmit, DiffMaximize::TabArea),
        (FileTitle(OLD), DiffMaximize::Section(DiffSection::File(OLD))),
        (
            HunkTitle(README, 0),
            DiffMaximize::Section(DiffSection::Hunk(README, 0)),
        ),
    ] {
        view.focus(stop);
        assert_eq!(view.maximize_scope(), expected, "{stop:?}");
    }
    assert_eq!(view.review_button(), Some("Add Comments"));
}
