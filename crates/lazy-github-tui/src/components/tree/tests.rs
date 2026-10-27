use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier, Style};

use super::*;
use crate::keymap::{KeyChord, Keymap};
use crate::test_util::{buffer_to_styled_string, key_event};

/// Run 42's jobs and steps, as the Jobs tab labels them. The icon colors stand in for the Jobs tab's.
/// A step's name and icon color.
type Step = (&'static str, Color);

const JOBS: &[(&str, Color, &[Step])] = &[
    (
        "build",
        Color::Rgb(0xad, 0xff, 0x2f),
        &[
            ("Set up job", Color::Rgb(0xad, 0xff, 0x2f)),
            ("Run cargo build", Color::Rgb(0xad, 0xff, 0x2f)),
            ("Complete job", Color::Rgb(0xad, 0xff, 0x2f)),
        ],
    ),
    (
        "test",
        Color::Red,
        &[
            ("Set up job", Color::Rgb(0xad, 0xff, 0x2f)),
            ("Run cargo test", Color::Red),
            ("Upload results", Color::DarkGray),
        ],
    ),
    ("lint", Color::DarkGray, &[]),
    (
        "deploy",
        Color::Yellow,
        &[("Set up job", Color::Rgb(0xad, 0xff, 0x2f)), ("Deploy", Color::Yellow)],
    ),
];

fn icon(color: Color) -> &'static str {
    match color {
        Color::Red => "✗",
        Color::DarkGray => "−",
        Color::Yellow => "○",
        _ => "✓",
    }
}

fn label(name: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(icon(color), Style::new().fg(color)),
        Span::raw(format!(" {name} (completed)")),
    ])
}

/// The tree with run 42's jobs collapsed, each node's data its plain name.
fn jobs() -> Tree<String> {
    let mut tree = Tree::new();
    for &(job, color, steps) in JOBS {
        let id = tree.add(None, label(job, color), job.to_owned(), true);
        for &(step, color) in steps {
            tree.add(Some(id), label(step, color), format!("{job}/{step}"), false);
        }
    }
    tree
}

fn node(tree: &Tree<String>, name: &str) -> NodeId {
    let index = tree.nodes.iter().position(|node| node.data == name).unwrap();
    NodeId(index)
}

fn cursor(tree: &Tree<String>) -> Option<&str> {
    tree.cursor_node().map(|(_, data)| data.as_str())
}

/// The visible lines' data, top to bottom.
fn lines(tree: &Tree<String>) -> Vec<&str> {
    tree.lines.iter().map(|&id| tree.data(id).as_str()).collect()
}

fn press(tree: &mut Tree<String>, actions: &[TreeAction]) {
    for &action in actions {
        let _ = tree.handle(action);
    }
}

/// What `action` reported, as the selected node's data.
fn selected(tree: &mut Tree<String>, action: TreeAction) -> Option<String> {
    tree.handle(action).map(|TreeEvent::Selected(_, data)| data.clone())
}

fn draw(tree: &mut Tree<String>, width: u16, height: u16, theme: &Theme, focused: bool) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| tree.render(frame, frame.area(), theme, focused))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn styled(tree: &mut Tree<String>, width: u16, height: u16, focused: bool) -> String {
    buffer_to_styled_string(&draw(tree, width, height, &Theme::default(), focused))
}

fn text_rows(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    (area.top()..area.bottom())
        .map(|y| {
            let row: String = (area.left()..area.right()).map(|x| buf[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect()
}

fn click(tree: &mut Tree<String>, column: u16, row: u16) -> Option<String> {
    let event = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    tree.handle_mouse(&event, Rect::new(0, 0, 40, 10))
        .map(|TreeEvent::Selected(_, data)| data.clone())
}

use TreeAction as A;

#[test]
fn the_bindings_are_textuals_and_map_one_to_one() {
    let keymap = Keymap::defaults();
    let expected = [
        ("up", A::CursorUp),
        ("down", A::CursorDown),
        ("left", A::ScrollLeft),
        ("right", A::ScrollRight),
        ("home", A::Home),
        ("end", A::End),
        ("pageup", A::PageUp),
        ("pagedown", A::PageDown),
        ("ctrl+pageup", A::PageLeft),
        ("ctrl+pagedown", A::PageRight),
        ("shift+left", A::CursorParent),
        ("shift+right", A::CursorParentNextSibling),
        ("shift+up", A::CursorPreviousSibling),
        ("shift+down", A::CursorNextSibling),
        ("enter", A::Select),
        ("space", A::Toggle),
        ("shift+space", A::ToggleExpandAll),
    ];
    assert_eq!(TREE_BINDINGS.len(), expected.len());
    for (&id, (key, action)) in TREE_BINDINGS.iter().zip(expected) {
        assert_eq!(TreeAction::from_binding(id), Some(action), "{}", id.def().name);
        assert!(!id.def().show, "{} is hidden", id.def().name);
        assert_eq!(id.config_id(), None, "{} can't be overridden", id.def().name);
        let chord = KeyChord::from_event(&key_event(key)).unwrap();
        assert_eq!(keymap.match_in(TREE_BINDINGS, &chord), Some(id), "{key}");
    }
    assert_eq!(TreeAction::from_binding(BindingId::TextualScrollUp), None);
    assert_eq!(TreeAction::from_binding(BindingId::AppQuit), None);
}

#[test]
fn no_line_has_the_cursor_at_first() {
    let mut tree = jobs();
    assert_eq!(lines(&tree), ["build", "test", "lint", "deploy"]);
    assert_eq!(tree.cursor_node(), None);
    assert_eq!(tree.cursor_line(), None);
    // Enter, space and shift+space do nothing without a cursor.
    assert_eq!(selected(&mut tree, A::Select), None);
    press(&mut tree, &[A::Toggle, A::ToggleExpandAll]);
    // So do the parent and sibling moves.
    press(
        &mut tree,
        &[
            A::CursorParent,
            A::CursorParentNextSibling,
            A::CursorPreviousSibling,
            A::CursorNextSibling,
        ],
    );
    assert_eq!(lines(&tree), ["build", "test", "lint", "deploy"]);
    assert_eq!(tree.cursor_node(), None);
}

#[test]
fn down_and_up_from_no_cursor() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown]);
    assert_eq!(cursor(&tree), Some("build"));
    let mut tree = jobs();
    press(&mut tree, &[A::CursorUp]);
    assert_eq!(cursor(&tree), Some("deploy"));
}

#[test]
fn the_cursor_clamps_at_both_ends() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::CursorUp, A::CursorUp]);
    assert_eq!(tree.cursor_line(), Some(0));
    press(&mut tree, &[A::CursorDown; 6]);
    assert_eq!(cursor(&tree), Some("deploy"));
    assert_eq!(tree.cursor_line(), Some(3));
}

#[test]
fn home_end_and_paging() {
    let mut tree = jobs();
    for job in ["build", "test", "deploy"] {
        tree.set_expanded(node(&tree, job), true);
    }
    // 4 + 3 + 3 + 2 = 12 lines in 5 rows: a page is 4 lines.
    draw(&mut tree, 40, 5, &Theme::default(), true);
    press(&mut tree, &[A::End]);
    assert_eq!(tree.cursor_line(), Some(11));
    press(&mut tree, &[A::Home]);
    assert_eq!(tree.cursor_line(), Some(0));
    press(&mut tree, &[A::PageDown]);
    assert_eq!(tree.cursor_line(), Some(4));
    press(&mut tree, &[A::PageDown, A::PageDown, A::PageDown]);
    assert_eq!(tree.cursor_line(), Some(11));
    press(&mut tree, &[A::PageUp]);
    assert_eq!(tree.cursor_line(), Some(7));
    press(&mut tree, &[A::PageUp, A::PageUp]);
    assert_eq!(tree.cursor_line(), Some(0));

    // From no cursor, paging starts at the first line (down) or the last (up).
    let mut tree = jobs();
    draw(&mut tree, 40, 3, &Theme::default(), true);
    press(&mut tree, &[A::PageDown]);
    assert_eq!(tree.cursor_line(), Some(2));
    let mut tree = jobs();
    draw(&mut tree, 40, 3, &Theme::default(), true);
    press(&mut tree, &[A::PageUp]);
    assert_eq!(tree.cursor_line(), Some(1));
}

#[test]
fn enter_toggles_an_expandable_node_and_reports_it() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown]);
    assert_eq!(selected(&mut tree, A::Select).as_deref(), Some("build"));
    assert_eq!(
        lines(&tree),
        [
            "build",
            "build/Set up job",
            "build/Run cargo build",
            "build/Complete job",
            "test",
            "lint",
            "deploy"
        ]
    );
    assert_eq!(selected(&mut tree, A::Select).as_deref(), Some("build"));
    assert_eq!(lines(&tree), ["build", "test", "lint", "deploy"]);

    // `lint` has no steps but is expandable: it toggles, and still shows nothing below it.
    press(&mut tree, &[A::CursorDown, A::CursorDown]);
    assert_eq!(selected(&mut tree, A::Select).as_deref(), Some("lint"));
    assert!(tree.is_expanded(node(&tree, "lint")));
    assert_eq!(lines(&tree), ["build", "test", "lint", "deploy"]);
}

#[test]
fn enter_on_a_leaf_only_reports_it() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::Select, A::CursorDown]);
    let before = lines(&tree).len();
    assert_eq!(selected(&mut tree, A::Select).as_deref(), Some("build/Set up job"));
    assert_eq!(lines(&tree).len(), before);
    assert_eq!(cursor(&tree), Some("build/Set up job"));
}

#[test]
fn space_toggles_without_reporting() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::CursorDown]);
    assert_eq!(selected(&mut tree, A::Toggle), None);
    assert!(tree.is_expanded(node(&tree, "test")));
    assert_eq!(lines(&tree).len(), 7);
    press(&mut tree, &[A::Toggle]);
    assert!(!tree.is_expanded(node(&tree, "test")));
    // On a leaf it does nothing.
    press(&mut tree, &[A::Toggle, A::CursorDown]);
    assert_eq!(cursor(&tree), Some("test/Set up job"));
    press(&mut tree, &[A::Toggle]);
    assert_eq!(lines(&tree).len(), 7);
}

#[test]
fn shift_space_expands_all_siblings_when_all_are_collapsed() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::CursorDown, A::ToggleExpandAll]);
    for job in ["build", "test", "lint", "deploy"] {
        assert!(tree.is_expanded(node(&tree, job)), "{job}");
    }
    assert_eq!(lines(&tree).len(), 12);
    assert_eq!(cursor(&tree), Some("test"));
}

#[test]
fn shift_space_collapses_all_siblings_when_one_is_expanded() {
    let mut tree = jobs();
    tree.set_expanded(node(&tree, "deploy"), true);
    press(&mut tree, &[A::CursorDown, A::ToggleExpandAll]);
    for job in ["build", "test", "lint", "deploy"] {
        assert!(!tree.is_expanded(node(&tree, job)), "{job}");
    }
    assert_eq!(cursor(&tree), Some("build"));

    // Among steps, which are all leaves, it changes nothing.
    press(&mut tree, &[A::Select, A::CursorDown, A::ToggleExpandAll]);
    assert_eq!(lines(&tree).len(), 7);
    assert_eq!(cursor(&tree), Some("build/Set up job"));
}

#[test]
fn parent_and_sibling_moves_at_the_top_level() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::CursorDown]);
    assert_eq!(cursor(&tree), Some("test"));
    // The root is hidden: no parent to go to, and so no parent's sibling.
    press(&mut tree, &[A::CursorParent]);
    assert_eq!(cursor(&tree), Some("test"));
    press(&mut tree, &[A::CursorParentNextSibling]);
    assert_eq!(cursor(&tree), Some("test"));

    press(&mut tree, &[A::CursorNextSibling]);
    assert_eq!(cursor(&tree), Some("lint"));
    press(&mut tree, &[A::CursorNextSibling, A::CursorNextSibling]);
    assert_eq!(cursor(&tree), Some("deploy"));
    press(&mut tree, &[A::CursorPreviousSibling]);
    assert_eq!(cursor(&tree), Some("lint"));
    press(
        &mut tree,
        &[
            A::CursorPreviousSibling,
            A::CursorPreviousSibling,
            A::CursorPreviousSibling,
        ],
    );
    assert_eq!(cursor(&tree), Some("build"));
}

#[test]
fn parent_and_sibling_moves_inside_a_node() {
    let mut tree = jobs();
    tree.set_expanded(node(&tree, "build"), true);
    tree.set_expanded(node(&tree, "test"), true);
    press(&mut tree, &[A::CursorDown, A::CursorDown, A::CursorDown]);
    assert_eq!(cursor(&tree), Some("build/Run cargo build"));

    press(&mut tree, &[A::CursorParent]);
    assert_eq!(cursor(&tree), Some("build"));

    press(&mut tree, &[A::CursorDown, A::CursorDown, A::CursorParentNextSibling]);
    assert_eq!(cursor(&tree), Some("test"));

    // Previous sibling, then the parent from the first step.
    press(
        &mut tree,
        &[A::End, A::Home, A::CursorDown, A::CursorDown, A::CursorPreviousSibling],
    );
    assert_eq!(cursor(&tree), Some("build/Set up job"));
    press(&mut tree, &[A::CursorPreviousSibling]);
    assert_eq!(cursor(&tree), Some("build"));

    // Next sibling, then the parent's next sibling from the last step.
    press(&mut tree, &[A::CursorDown, A::CursorNextSibling]);
    assert_eq!(cursor(&tree), Some("build/Run cargo build"));
    press(&mut tree, &[A::CursorNextSibling, A::CursorNextSibling]);
    assert_eq!(cursor(&tree), Some("test"));

    // The last step of the last job with steps: the parent has a next sibling.
    press(
        &mut tree,
        &[
            A::CursorDown,
            A::CursorNextSibling,
            A::CursorNextSibling,
            A::CursorNextSibling,
        ],
    );
    assert_eq!(cursor(&tree), Some("lint"));
    // From a last step whose job is the last: nowhere to go.
    tree.set_expanded(node(&tree, "deploy"), true);
    press(&mut tree, &[A::End, A::CursorNextSibling, A::CursorParentNextSibling]);
    assert_eq!(cursor(&tree), Some("deploy/Deploy"));
}

#[test]
fn the_cursor_stays_on_its_node_when_lines_shift() {
    let mut tree = jobs();
    press(&mut tree, &[A::End]);
    assert_eq!(tree.cursor_line(), Some(3));
    tree.set_expanded(node(&tree, "build"), true);
    assert_eq!(cursor(&tree), Some("deploy"));
    assert_eq!(tree.cursor_line(), Some(6));
    tree.set_expanded(node(&tree, "build"), false);
    assert_eq!(tree.cursor_line(), Some(3));
}

#[test]
fn a_collapse_that_hides_the_cursor_leaves_no_cursor() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::Select, A::CursorDown, A::CursorDown]);
    assert_eq!(cursor(&tree), Some("build/Run cargo build"));
    // A click on build's icon collapses it under the cursor.
    assert_eq!(click(&mut tree, 0, 0), None);
    assert_eq!(lines(&tree), ["build", "test", "lint", "deploy"]);
    assert_eq!(tree.cursor_node(), None);
    assert_eq!(tree.cursor_line(), None);
    // Down starts over from the first line.
    press(&mut tree, &[A::CursorDown]);
    assert_eq!(cursor(&tree), Some("build"));
}

#[test]
fn a_click_on_a_label_moves_the_cursor_and_selects() {
    let mut tree = jobs();
    draw(&mut tree, 40, 10, &Theme::default(), true);
    // On `test`'s label: the cursor moves there, and the select expands it.
    assert_eq!(click(&mut tree, 5, 1).as_deref(), Some("test"));
    assert_eq!(cursor(&tree), Some("test"));
    assert!(tree.is_expanded(node(&tree, "test")));
    // Past the end of a step's label and on its guides: the line still takes the click.
    assert_eq!(click(&mut tree, 38, 2).as_deref(), Some("test/Set up job"));
    assert_eq!(click(&mut tree, 1, 3).as_deref(), Some("test/Run cargo test"));
    assert_eq!(lines(&tree).len(), 7);
    // Below the last line, nothing.
    assert_eq!(click(&mut tree, 5, 8), None);
    assert_eq!(cursor(&tree), Some("test/Run cargo test"));
    // Outside the area, nothing.
    let outside = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 50,
        row: 1,
        modifiers: KeyModifiers::NONE,
    };
    assert!(tree.handle_mouse(&outside, Rect::new(0, 0, 40, 10)).is_none());
    // A press isn't a click.
    let press = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        ..outside
    };
    assert!(
        tree.handle_mouse(&MouseEvent { column: 5, ..press }, Rect::new(0, 0, 40, 10))
            .is_none()
    );
    assert_eq!(cursor(&tree), Some("test/Run cargo test"));
}

#[test]
fn a_click_on_the_icon_only_toggles() {
    let mut tree = jobs();
    draw(&mut tree, 40, 10, &Theme::default(), true);
    assert_eq!(click(&mut tree, 1, 1), None);
    assert!(tree.is_expanded(node(&tree, "test")));
    assert_eq!(tree.cursor_node(), None);
    assert_eq!(click(&mut tree, 0, 1), None);
    assert!(!tree.is_expanded(node(&tree, "test")));
    // The column after the icon is the label's.
    assert_eq!(click(&mut tree, 2, 1).as_deref(), Some("test"));
}

#[test]
fn the_cursor_is_kept_in_view() {
    let mut tree = jobs();
    for job in ["build", "test", "deploy"] {
        tree.set_expanded(node(&tree, job), true);
    }
    draw(&mut tree, 40, 4, &Theme::default(), true);
    press(&mut tree, &[A::CursorDown; 6]);
    assert_eq!(tree.cursor_line(), Some(5));
    assert_eq!(tree.scroll_offset(), (2, 0));
    press(&mut tree, &[A::End]);
    assert_eq!(tree.scroll_offset(), (8, 0));
    press(&mut tree, &[A::CursorUp; 4]);
    assert_eq!(tree.cursor_line(), Some(7));
    assert_eq!(tree.scroll_offset(), (7, 0));
    // A sibling move far up.
    press(
        &mut tree,
        &[A::CursorParent, A::CursorPreviousSibling, A::CursorPreviousSibling],
    );
    assert_eq!(cursor(&tree), Some("build"));
    assert_eq!(tree.scroll_offset(), (0, 0));
    let buf = draw(&mut tree, 40, 4, &Theme::default(), true);
    assert!(text_rows(&buf)[0].starts_with("▼ ✓ build"), "{:?}", text_rows(&buf));
}

#[test]
fn the_wheel_scrolls_without_moving_the_cursor() {
    let mut tree = jobs();
    for job in ["build", "test", "deploy"] {
        tree.set_expanded(node(&tree, job), true);
    }
    draw(&mut tree, 40, 4, &Theme::default(), true);
    press(&mut tree, &[A::CursorDown]);
    let wheel = |kind| MouseEvent {
        kind,
        column: 3,
        row: 1,
        modifiers: KeyModifiers::NONE,
    };
    let area = Rect::new(0, 0, 40, 4);
    assert!(tree.handle_mouse(&wheel(MouseEventKind::ScrollDown), area).is_none());
    assert_eq!(tree.scroll_offset(), (2, 0));
    for _ in 0..5 {
        let _ = tree.handle_mouse(&wheel(MouseEventKind::ScrollDown), area);
    }
    assert_eq!(tree.scroll_offset(), (8, 0));
    let _ = tree.handle_mouse(&wheel(MouseEventKind::ScrollUp), area);
    assert_eq!(tree.scroll_offset(), (6, 0));
    assert_eq!(tree.cursor_line(), Some(0));
}

#[test]
fn left_and_right_scroll_a_wide_tree_sideways() {
    let mut tree = jobs();
    tree.set_expanded(node(&tree, "build"), true);
    // `├── ✓ Run cargo build (completed)` is 33 cells wide.
    draw(&mut tree, 20, 10, &Theme::default(), true);
    assert_eq!(tree.viewport, (20, 9));
    press(&mut tree, &[A::ScrollRight, A::ScrollRight]);
    assert_eq!(tree.scroll_offset(), (0, 2));
    press(&mut tree, &[A::ScrollLeft]);
    assert_eq!(tree.scroll_offset(), (0, 1));
    press(&mut tree, &[A::PageRight]);
    assert_eq!(tree.scroll_offset(), (0, 13));
    press(&mut tree, &[A::PageLeft]);
    assert_eq!(tree.scroll_offset(), (0, 0));
    // Nothing to scroll in a tree that fits.
    let mut tree = jobs();
    draw(&mut tree, 40, 10, &Theme::default(), true);
    press(&mut tree, &[A::ScrollRight, A::PageRight]);
    assert_eq!(tree.scroll_offset(), (0, 0));
}

#[test]
fn a_tree_without_nodes_ignores_everything() {
    let mut tree: Tree<String> = Tree::new();
    assert!(tree.is_empty());
    press(
        &mut tree,
        &[
            A::CursorDown,
            A::CursorUp,
            A::Home,
            A::End,
            A::PageDown,
            A::PageUp,
            A::Toggle,
        ],
    );
    assert_eq!(tree.cursor_node(), None);
    assert_eq!(selected(&mut tree, A::Select), None);
    let buf = draw(&mut tree, 10, 2, &Theme::default(), true);
    assert!(text_rows(&buf).iter().all(String::is_empty));
}

#[test]
fn a_child_added_under_an_expanded_node_shows_at_once() {
    let mut tree: Tree<String> = Tree::new();
    let job = tree.add(None, Line::from("job"), "job".to_owned(), true);
    tree.add(Some(job), Line::from("one"), "one".to_owned(), false);
    assert_eq!(lines(&tree), ["job"]);
    tree.set_expanded(job, true);
    tree.add(Some(job), Line::from("two"), "two".to_owned(), false);
    assert_eq!(lines(&tree), ["job", "one", "two"]);
    // A leaf never expands.
    let leaf = tree.add(None, Line::from("No jobs found"), "none".to_owned(), false);
    tree.set_expanded(leaf, true);
    assert!(!tree.is_expanded(leaf));
}

#[test]
fn deeper_levels_draw_continuation_guides() {
    let mut tree: Tree<String> = Tree::new();
    let a = tree.add(None, Line::from("a"), "a".to_owned(), true);
    let b = tree.add(Some(a), Line::from("b"), "b".to_owned(), true);
    tree.add(Some(b), Line::from("b1"), "b1".to_owned(), false);
    tree.add(Some(b), Line::from("b2"), "b2".to_owned(), false);
    let c = tree.add(Some(a), Line::from("c"), "c".to_owned(), true);
    tree.add(Some(c), Line::from("c1"), "c1".to_owned(), false);
    for id in [a, b, c] {
        tree.set_expanded(id, true);
    }
    let buf = draw(&mut tree, 20, 7, &Theme::default(), false);
    assert_eq!(
        text_rows(&buf),
        [
            "▼ a",
            "├── ▼ b",
            "│   ├── b1",
            "│   └── b2",
            "└── ▼ c",
            "    └── c1",
            ""
        ]
    );
}

#[test]
fn the_cursor_styles_follow_textual() {
    let theme = Theme::default();
    let mut tree = jobs();
    tree.set_expanded(node(&tree, "build"), true);
    press(&mut tree, &[A::CursorDown]);
    let buf = draw(&mut tree, 40, 10, &theme, true);
    let rgb = |hex: u32| {
        let [_, r, g, b] = hex.to_be_bytes();
        Color::Rgb(r, g, b)
    };
    // `run-details` `job-expanded`: the icon in the tree's colors, the label in the block cursor's, the children's
    // guides in `$primary`, all on the focused `#272727`.
    assert_eq!((buf[(0, 0)].fg, buf[(0, 0)].bg), (rgb(0xe0_e0_e0), rgb(0x27_27_27)));
    for x in [2, 4, 10] {
        let cell = &buf[(x, 0)];
        assert_eq!((cell.fg, cell.bg), (rgb(0xdd_ed_f9), rgb(0x01_78_d4)), "{x}");
        assert!(cell.modifier.contains(Modifier::BOLD));
    }
    assert_eq!((buf[(0, 1)].fg, buf[(0, 1)].bg), (rgb(0x01_78_d4), rgb(0x27_27_27)));
    assert_eq!(buf[(25, 0)].bg, rgb(0x27_27_27));
    // Blurred: `$surface`, the blurred cursor under `$foreground`, and plain guides.
    let buf = draw(&mut tree, 40, 10, &theme, false);
    assert_eq!(buf[(0, 0)].bg, rgb(0x1e_1e_1e));
    assert_eq!((buf[(2, 0)].fg, buf[(2, 0)].bg), (rgb(0xe0_e0_e0), rgb(0x15_38_54)));
    assert_eq!(buf[(0, 1)].fg, rgb(0x3e_3e_3e));
}

#[test]
fn snapshot_four_collapsed_jobs() {
    insta::assert_snapshot!(styled(&mut jobs(), 40, 6, false));
}

#[test]
fn snapshot_one_job_expanded() {
    let mut tree = jobs();
    tree.set_expanded(node(&tree, "build"), true);
    insta::assert_snapshot!(styled(&mut tree, 40, 8, false));
}

#[test]
fn snapshot_two_jobs_expanded() {
    let mut tree = jobs();
    tree.set_expanded(node(&tree, "build"), true);
    tree.set_expanded(node(&tree, "test"), true);
    insta::assert_snapshot!(styled(&mut tree, 40, 11, false));
}

#[test]
fn snapshot_focused_cursor_on_a_step() {
    let mut tree = jobs();
    press(
        &mut tree,
        &[A::CursorDown, A::CursorDown, A::Select, A::CursorDown, A::CursorDown],
    );
    insta::assert_snapshot!(styled(&mut tree, 40, 8, true));
}

#[test]
fn snapshot_unfocused_with_a_cursor() {
    let mut tree = jobs();
    press(&mut tree, &[A::CursorDown, A::Select]);
    insta::assert_snapshot!(styled(&mut tree, 40, 8, false));
}

#[test]
fn snapshot_label_wider_than_the_area() {
    let mut tree = jobs();
    for job in ["build", "test", "deploy"] {
        tree.set_expanded(node(&tree, job), true);
    }
    press(&mut tree, &[A::CursorDown, A::CursorDown]);
    draw(&mut tree, 24, 6, &Theme::default(), true);
    press(&mut tree, &[A::ScrollRight, A::ScrollRight]);
    insta::assert_snapshot!(styled(&mut tree, 24, 6, true));
}
