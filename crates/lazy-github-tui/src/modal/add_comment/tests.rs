//! Add Comment on its own: opened over an empty app with a reply that records what it closed with.

use std::sync::{Arc, Mutex};

use lazy_github_core::config::Config;

use super::*;
use crate::test_util::TestApp;

const LINE: &str = "+Run `viewer FILE`, then press `j` and `k` to move.";

/// What the dialog closed with: `None` while it's open, then its output.
type Closed = Arc<Mutex<Option<Option<String>>>>;

/// An app with Add Comment open on [`LINE`], and where its output goes.
fn open() -> (TestApp, Closed) {
    let mut app = TestApp::new(Config::default());
    let closed: Closed = Arc::default();
    let reply = Arc::clone(&closed);
    app.with_cx(|cx| {
        cx.open_dialog(AddCommentDialog::new(LINE), move |body| {
            *reply.lock().unwrap() = Some(body);
            Action::Consumed
        });
    });
    (app, closed)
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

#[test]
fn the_dialog_is_as_tall_as_its_content() {
    let dialog = AddCommentDialog::new(LINE);
    // Python's 80 x 29 at 160x48: 25 rows of content, its padding and the border.
    assert_eq!(dialog.size(Rect::new(0, 0, 160, 47)), (80, 29));
    assert_eq!(dialog.frame(Rect::new(0, 0, 160, 48)), Rect::new(40, 9, 80, 29));
    assert_eq!(dialog.line_text(), LINE);
}

#[test]
fn focus_starts_in_the_editor_and_tab_walks_the_buttons() {
    let (mut app, _) = open();
    let field = |app: &TestApp| app.app().binding_chain().first().map(|link| link.owner);
    assert_eq!(
        field(&app),
        Some(crate::component::Owner::DialogField(
            ModalKind::AddComment,
            FieldKind::TextEditor
        ))
    );
    app.press(&["tab"]);
    assert_eq!(
        field(&app),
        Some(crate::component::Owner::DialogField(
            ModalKind::AddComment,
            FieldKind::Button
        ))
    );
    app.press(&["tab", "tab"]);
    assert_eq!(
        field(&app),
        Some(crate::component::Owner::DialogField(
            ModalKind::AddComment,
            FieldKind::TextEditor
        ))
    );
}

#[test]
fn an_empty_or_blank_comment_is_refused_and_the_dialog_stays() {
    let (mut app, closed) = open();
    app.press(&["shift+enter"]);
    assert!(is_open(&app));
    assert_eq!(toasts(&app), ["Comment cannot be empty!"]);
    app.press(&["space", "enter", "space"]);
    app.press(&["tab", "enter"]);
    assert!(is_open(&app));
    assert_eq!(toasts(&app).len(), 2);
    assert_eq!(*closed.lock().unwrap(), None);
}

#[test]
fn submit_closes_with_the_comment_as_typed() {
    let (mut app, closed) = open();
    app.press(&["space", "o", "k"]);
    app.press(&["shift+enter"]);
    assert!(!is_open(&app));
    assert_eq!(*closed.lock().unwrap(), Some(Some(" ok".to_owned())));
    assert!(toasts(&app).is_empty());
}

#[test]
fn add_comment_closes_with_the_comment() {
    let (mut app, closed) = open();
    app.press(&["o", "k", "tab", "enter"]);
    assert_eq!(*closed.lock().unwrap(), Some(Some("ok".to_owned())));
}

#[test]
fn cancel_escape_and_q_close_with_nothing() {
    for keys in [
        &["o", "k", "tab", "tab", "enter"][..],
        &["o", "k", "escape"],
        &["tab", "q"],
    ] {
        let (mut app, closed) = open();
        app.press(keys);
        assert!(!is_open(&app), "{keys:?}");
        assert_eq!(*closed.lock().unwrap(), Some(None), "{keys:?}");
    }
}

#[test]
fn q_in_the_editor_types() {
    let (mut app, closed) = open();
    app.press(&["q"]);
    assert!(is_open(&app));
    app.press(&["shift+enter"]);
    assert_eq!(*closed.lock().unwrap(), Some(Some("q".to_owned())));
}

#[test]
fn the_dialog_has_no_footer_of_its_own() {
    let (mut app, _) = open();
    let screen = app.screen(160, 48);
    let footer = screen.lines().last().unwrap();
    assert!(!footer.contains("Submit"), "{footer}");
}

#[test]
fn a_click_on_cancel_closes() {
    let (mut app, closed) = open();
    app.screen(160, 48);
    // Cancel's label row, from the frame at (40, 9): the interior's padding, then 22 rows down.
    app.click(85, 9 + 1 + 1 + 23);
    assert_eq!(*closed.lock().unwrap(), Some(None));
}
