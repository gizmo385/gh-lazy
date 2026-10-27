//! The viewer on its own: its focus ring, the filter, `J` / `K` and the line cursor, against PR 5's diff from the
//! `pr-details` fixtures.

use std::path::Path;

use lazy_github_core::diff;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;

use super::*;
use crate::keymap::{KeySpec, Keymap};
use crate::test_util::buffer_to_string;

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
        if view.handle_key(&chord, &keymap).is_some() {
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
