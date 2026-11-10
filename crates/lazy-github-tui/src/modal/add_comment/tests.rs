//! Add Comment on its own: opened over an empty app with a reply that records what it closed with.

use std::sync::{Arc, Mutex};

use lazy_github_core::config::Config;

use super::*;
use crate::event::Event;
use crate::test_util::TestApp;

const LINE: &str = "+Run `viewer FILE`, then press `j` and `k` to move.";

/// What the dialog closed with: `None` while it's open, then its output.
type Closed = Arc<Mutex<Option<Option<String>>>>;

/// An app with Add Comment open on `line`, and where its output goes.
fn open_on(line: &str) -> (TestApp, Closed) {
    let mut app = TestApp::new(Config::default());
    let closed: Closed = Arc::default();
    let reply = Arc::clone(&closed);
    let dialog = AddCommentDialog::new(line);
    app.with_cx(|cx| {
        cx.open_dialog(dialog, move |body| {
            *reply.lock().unwrap() = Some(body);
            Action::Consumed
        });
    });
    (app, closed)
}

/// An app with Add Comment open on [`LINE`].
fn open() -> (TestApp, Closed) {
    open_on(LINE)
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::AddComment]
}

fn toasts(app: &TestApp) -> Vec<String> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

fn footer(app: &mut TestApp) -> Vec<String> {
    crate::components::footer::footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

fn dialog(app: &TestApp) -> &AddCommentDialog {
    app.app()
        .top_dialog::<AddCommentDialog>()
        .expect("Add Comment is on top")
}

/// The sheet's frame as last drawn, border included.
fn frame_of(app: &TestApp) -> Rect {
    let interior = app.app().top_dialog_interior().expect("a dialog was drawn");
    Rect::new(interior.x - 1, interior.y - 1, interior.width + 2, interior.height + 2)
}

/// A comment long enough to pass any sheet's cap.
fn long_body() -> String {
    let paragraph = "This line is the one the README's Keys section should mention: `j` and `k` stop at either end \
of the file and never wrap around, and `G` / `g` jump to the last and the first line.";
    [paragraph; 12].join("\n\n")
}

const BODY_FOOTER: [&str; 5] = [
    "^s Add Comment",
    "shift+⏎ Add Comment",
    "^t Preview",
    "^o Expand",
    "esc Leave",
];
const SHEET_FOOTER: [&str; 5] = [
    "^s Add Comment",
    "shift+⏎ Add Comment",
    "^t Preview",
    "^o Expand",
    "q Close",
];

#[test]
fn the_sheet_is_80_wide_and_nine_rows_empty() {
    let (mut app, _) = open();
    let screen = app.screen(160, 48);
    // The heading, the quote, a blank row, the tabs and three rows of body, in the border.
    assert_eq!(frame_of(&app), Rect::new(40, 19, 80, 9));
    assert!(screen.contains("Add Comment"), "{screen}");
    assert!(screen.contains(&format!("▎ {LINE}")), "{screen}");
    assert!(screen.contains("Write a review comment (markdown)"), "{screen}");
    assert!(!screen.contains("Commenting on:"), "{screen}");
    assert_eq!(dialog(&app).line_text(), LINE);
    // At 100x30 too, centered.
    app.screen(100, 30);
    assert_eq!(frame_of(&app), Rect::new(10, 10, 80, 9));
}

#[test]
fn the_sheet_grows_to_its_cap_then_the_body_scrolls() {
    let (mut app, _) = open();
    app.screen(100, 30);
    app.send(Event::Paste(long_body()));
    app.screen(100, 30);
    // 27 rows at 100x30: the screen less the footer and a margin row above and below.
    assert_eq!(frame_of(&app), Rect::new(10, 1, 80, 27));
    app.screen(160, 48);
    assert_eq!(frame_of(&app).height, 45);
}

#[test]
fn the_quote_cuts_a_long_line_with_an_ellipsis() {
    let long = format!("+{}", "word ".repeat(40));
    let (mut app, _) = open_on(long.trim_end());
    let screen = app.screen(160, 48);
    let row = screen.lines().find(|line| line.contains("▎ +word")).unwrap();
    // Cut at the sheet's right padding, the ellipsis in its last cell.
    assert!(row.contains("wo… █"), "{row}");
}

#[test]
fn the_body_is_focused_with_the_sheet_s_footer() {
    let (mut app, _) = open();
    assert_eq!(footer(&mut app), BODY_FOOTER);
    // The dialog has a footer of its own now: the diff's doesn't show under it.
    let screen = app.screen(160, 48);
    let last = screen.lines().last().unwrap();
    assert!(last.contains("^s Add Comment"), "{last}");
    // `q` types.
    app.press(&["q"]);
    assert!(is_open(&app));
    assert_eq!(dialog(&app).text(), "q");
}

#[test]
fn an_empty_or_blank_comment_is_refused_and_the_sheet_stays() {
    let (mut app, closed) = open();
    app.press(&["ctrl+s"]);
    assert!(is_open(&app));
    assert_eq!(toasts(&app), ["Comment cannot be empty!"]);
    app.press(&["space", "enter", "space", "shift+enter"]);
    assert!(is_open(&app));
    assert_eq!(toasts(&app).len(), 2);
    assert_eq!(*closed.lock().unwrap(), None);
}

#[test]
fn ctrl_s_and_shift_enter_close_with_the_comment_as_typed() {
    for submit in ["ctrl+s", "shift+enter"] {
        let (mut app, closed) = open();
        app.press(&["space", "o", "k", submit]);
        assert!(!is_open(&app), "{submit}");
        assert_eq!(*closed.lock().unwrap(), Some(Some(" ok".to_owned())), "{submit}");
        assert!(toasts(&app).is_empty(), "{submit}");
    }
}

#[test]
fn enter_in_the_body_types_a_new_line() {
    let (mut app, _) = open();
    app.press(&["a", "enter", "b"]);
    assert!(is_open(&app));
    assert_eq!(dialog(&app).text(), "a\nb");
}

#[test]
fn escape_leaves_the_body_and_a_second_closes_with_nothing() {
    let (mut app, closed) = open();
    app.press(&["o", "k", "escape"]);
    assert!(is_open(&app), "the first escape only leaves the body");
    assert_eq!(footer(&mut app), SHEET_FOOTER);
    app.press(&["x"]);
    assert_eq!(dialog(&app).text(), "ok");
    // `tab` goes back to the body.
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), BODY_FOOTER);
    app.press(&["escape", "escape"]);
    assert!(!is_open(&app));
    assert_eq!(*closed.lock().unwrap(), Some(None));
}

#[test]
fn q_on_the_sheet_closes_with_nothing() {
    let (mut app, closed) = open();
    app.press(&["o", "k", "escape", "q"]);
    assert!(!is_open(&app));
    assert_eq!(*closed.lock().unwrap(), Some(None));
}

#[test]
fn ctrl_t_previews_the_comment_and_ctrl_o_expands() {
    let (mut app, closed) = open();
    for key in ["asterisk", "asterisk", "o", "k", "asterisk", "asterisk"] {
        app.press(&[key]);
    }
    app.press(&["ctrl+t"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains(" ok "), "{screen}");
    assert!(!screen.contains("**ok**"), "{screen}");
    assert_eq!(
        footer(&mut app),
        [
            "^s Add Comment",
            "shift+⏎ Add Comment",
            "^t Write",
            "^o Expand",
            "esc Leave"
        ]
    );
    app.press(&["ctrl+t", "ctrl+o"]);
    app.screen(160, 48);
    assert_eq!(frame_of(&app), Rect::new(0, 0, 160, 47));
    assert!(footer(&mut app).contains(&"^o Collapse".to_owned()));
    app.press(&["ctrl+o"]);
    app.screen(160, 48);
    assert_eq!(frame_of(&app).width, 80);
    app.press(&["ctrl+s"]);
    assert_eq!(*closed.lock().unwrap(), Some(Some("**ok**".to_owned())));
}

#[test]
fn add_comment_empty_160x48() {
    let (mut app, _) = open();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn add_comment_long_100x30() {
    let (mut app, _) = open();
    app.screen(100, 30);
    app.send(Event::Paste(long_body()));
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn add_comment_preview() {
    let (mut app, _) = open();
    app.send(Event::Paste(
        "Mention the **arrow keys** too, and `G` / `g`.".to_owned(),
    ));
    app.press(&["ctrl+t"]);
    insta::assert_snapshot!(app.screen(100, 30));
}
