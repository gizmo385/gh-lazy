//! The Add reaction dialog: what it lists, its keys, its output and its look.

use lazy_github_core::models::ReactionKind::{
    self, Confused, Eyes, Heart, Hooray, Laugh, Rocket, ThumbsDown, ThumbsUp,
};

use super::*;
use crate::test_util::TestApp;

/// The dialog for a user who already gave 👍, 🎉 and 👀, as parity-bot has on PR 5.
fn pr5_dialog() -> AddReactionDialog {
    AddReactionDialog::new(&[ThumbsUp, Hooray, Eyes])
}

/// An app with `dialog` open, its replies recorded as the test dialog's.
fn open(dialog: AddReactionDialog) -> (TestApp, std::sync::Arc<std::sync::Mutex<Vec<Vec<ReactionKind>>>>) {
    let mut app = TestApp::new(lazy_github_core::config::Config::default());
    let replies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&replies);
    app.with_cx(|cx| {
        cx.open_dialog(dialog, move |kinds| {
            sink.lock().unwrap().push(kinds);
            Action::Consumed
        });
    });
    (app, replies)
}

fn modals(app: &TestApp) -> Vec<ModalKind> {
    app.app().modals()
}

#[test]
fn the_kinds_already_given_are_left_out() {
    assert_eq!(pr5_dialog().listed(), [ThumbsDown, Laugh, Confused, Heart, Rocket]);
    assert_eq!(AddReactionDialog::new(&[]).listed(), ReactionKind::ALL);
}

#[test]
fn space_and_enter_check_and_submit_closes_with_them_in_the_order_checked() {
    let (mut app, replies) = open(pr5_dialog());
    // 🚀 with enter, then 😄 with space, then 👎 checked and unchecked, then checked again.
    app.press(&["end", "enter", "home", "down", "space", "up", "space", "space", "space"]);
    assert_eq!(modals(&app), [ModalKind::AddReaction]);
    app.press(&["tab", "enter"]);
    assert!(modals(&app).is_empty());
    assert_eq!(*replies.lock().unwrap(), [vec![Rocket, Laugh, ThumbsDown]]);
}

#[test]
fn submit_with_nothing_checked_closes_with_nothing() {
    let (mut app, replies) = open(pr5_dialog());
    app.press(&["tab", "enter"]);
    assert_eq!(*replies.lock().unwrap(), [Vec::<ReactionKind>::new()]);
}

#[test]
fn cancel_q_and_escape_close_with_nothing() {
    for keys in [
        &["space", "tab", "tab", "enter"][..],
        &["space", "q"],
        &["space", "escape"],
    ] {
        let (mut app, replies) = open(pr5_dialog());
        app.press(keys);
        assert!(modals(&app).is_empty(), "{keys:?}");
        assert_eq!(*replies.lock().unwrap(), [Vec::<ReactionKind>::new()], "{keys:?}");
    }
}

#[test]
fn the_footer_shows_only_close() {
    let (mut app, _) = open(pr5_dialog());
    let screen = app.screen(160, 48);
    assert_eq!(screen.lines().last().unwrap().trim_end(), " q Close");
    // `shift+enter` isn't bound: Python binds only `modal.close`.
    app.press(&["shift+enter"]);
    assert_eq!(modals(&app), [ModalKind::AddReaction]);
}

#[test]
fn a_click_on_a_row_checks_it_and_focuses_the_list() {
    let (mut app, replies) = open(pr5_dialog());
    let screen = app.screen(160, 48);
    let row = screen.lines().position(|line| line.contains("▐X▌ ❤️")).unwrap();
    app.press(&["tab"]);
    app.click(70, u16::try_from(row).unwrap());
    app.press(&["space", "space"]);
    app.press(&["tab", "enter"]);
    // The click checked ❤️ and focused the list, whose space keys unchecked and checked it again.
    assert_eq!(*replies.lock().unwrap(), [vec![Heart]]);
}

#[test]
fn add_reaction_two_checked() {
    let (mut app, _) = open(pr5_dialog());
    app.press(&["down", "space", "down", "down", "down", "enter"]);
    // The frame's cells at 160x48, where the `pr-reactions` scenario's `checked` capture has them.
    let screen = app.render(160, 48);
    let frame = Rect::new(55, 13, 50, 20);
    let mut dialog = ratatui::buffer::Buffer::empty(frame);
    for (x, y) in frame.positions().map(|p| (p.x, p.y)) {
        dialog[(x, y)] = screen[(x, y)].clone();
    }
    insta::assert_snapshot!(crate::test_util::buffer_to_styled_string(&dialog));
}
