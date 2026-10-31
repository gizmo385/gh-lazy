//! The New comment dialog: its layout, the reply heading, the preview, posting, the answer, closing and its drawing.
//! Opened with `n` and `r` on issue 12's Comments tab against the `issue-write` and `issue-details` fixtures.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::IssueListItem;
use serde_json::Value;

use super::*;
use crate::event::Event;
use crate::test_util::{TestApp, buffer_to_styled_string};

const REPO: &str = "parity-bot/issue-fixture";

fn fixture_body(set: &str, path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn fixture_repo() -> Repository {
    let page = fixture_body(
        "issue-details",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    serde_json::from_value(page[0].clone()).unwrap()
}

fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body(
        "issue-details",
        &format!("repos/{REPO}/issues/GET.page-1_per_page-30_state-all.json"),
    );
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

/// Issue 12's comments: the second is from a deleted account.
fn comments() -> Vec<IssueComment> {
    serde_json::from_value(fixture_body(
        "issue-details",
        &format!("repos/{REPO}/issues/12/comments/GET.json"),
    ))
    .unwrap()
}

/// What `POST .../issues/12/comments` answers: `POST.json` for the new comment, `POST.2.json` for the reply.
fn posted(file: &str) -> IssueComment {
    serde_json::from_value(fixture_body(
        "issue-write",
        &format!("repos/{REPO}/issues/12/comments/{file}"),
    ))
    .unwrap()
}

fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()))
        .with_issue_comments(REPO, 12, Ok(comments()))
        .with_posted_comment("A **new** comment", Ok(posted("POST.json")))
        .with_posted_comment("Agreed, thanks.", Ok(posted("POST.2.json")))
}

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

/// Issue 12 shown, its Comments tab active and focus in its scroll area.
fn comments_tab() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "enter"]);
    settle(&mut app);
    app.press(&["5", "right", "tab"]);
    app
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::NewComment]
}

fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            ' ' => "space".to_owned(),
            '*' => "asterisk".to_owned(),
            '`' => "grave_accent".to_owned(),
            ',' => "comma".to_owned(),
            '.' => "full_stop".to_owned(),
            '\n' => "enter".to_owned(),
            c => c.to_string(),
        };
        app.press(&[key.as_str()]);
    }
}

fn toasts(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

fn footer(app: &mut TestApp) -> Vec<String> {
    crate::components::footer::footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

/// The comment requests the app sent: (number, body).
fn posts(app: &TestApp) -> Vec<(u64, String)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                request: GithubRequest::CreateIssueComment { repo, number, body },
                ..
            } => {
                assert_eq!(repo.full_name, REPO);
                Some((*number, body.clone()))
            }
            _ => None,
        })
        .collect()
}

// Layout.

#[test]
fn the_rows_follow_textuals_layout() {
    // Python at 160x48: the 48-row interior, without and with a reply.
    let rows = Rows::new(48, false);
    assert_eq!(rows.reply, None);
    assert_eq!((rows.editor_label, rows.editor.clone()), (0, 1..20));
    assert_eq!((rows.preview_label, rows.preview.clone()), (20, 21..40));
    assert_eq!(rows.buttons, 42..45);
    let rows = Rows::new(48, true);
    assert_eq!(rows.reply, Some(0..3));
    assert_eq!((rows.editor_label, rows.editor.clone()), (3, 4..24));
    assert_eq!((rows.preview_label, rows.preview), (24, 25..44));
    assert_eq!(rows.buttons, 44..47);
}

// The heading.

#[test]
fn the_heading_names_the_author_and_the_date() {
    let deleted = comments()[1].clone();
    assert_eq!(deleted.user, None);
    let target = ReplyTarget::IssueComment(Box::new(deleted));
    let (line, quote) = target.heading().unwrap();
    assert_eq!(line, "Replying to comment from Unknown at Wed Feb  4 11:00:00 2026");
    assert_eq!(quote, "A comment from a deleted account.");
    let first = comments()[0].clone();
    let login = first.user.clone().unwrap().login;
    let (line, _) = ReplyTarget::IssueComment(Box::new(first)).heading().unwrap();
    assert!(
        line.starts_with(&format!("Replying to comment from {login} at ")),
        "{line}"
    );
    assert_eq!(ReplyTarget::None.heading(), None);
}

#[test]
fn a_review_has_a_heading_only_once_submitted() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/reviews/GET.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut review: lazy_github_core::models::Review =
        serde_json::from_value(fixture["response"]["body"][0].clone()).unwrap();
    let at = review.submitted_at.unwrap();
    let target = ReplyTarget::Review(Box::new(review.clone()));
    let (line, quote) = target.heading().unwrap();
    assert_eq!(
        line,
        format!(
            "Replying to comment from {} at {}",
            review.user.login,
            time::c_format(at)
        )
    );
    assert_eq!(quote, review.body);
    review.submitted_at = None;
    assert_eq!(ReplyTarget::Review(Box::new(review)).heading(), None);
}

#[test]
fn a_reply_to_a_review_comment_goes_to_its_thread() {
    let mut app = comments_tab();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/reviews/501/comments/GET.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let comment: ReviewComment = serde_json::from_value(fixture["response"]["body"][0].clone()).unwrap();
    let id = comment.comment.id;
    let mut dialog = NewCommentDialog::new(fixture_repo(), 5, ReplyTarget::ReviewComment(Box::new(comment)));
    app.with_cx(|cx| dialog.update(NewCommentAction::Submit, cx));
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::Github {
            request: GithubRequest::ReplyToReviewComment { pr_number: 5, comment_id, .. },
            ..
        } if *comment_id == id
    )));
}

// Opening and focus.

#[test]
fn n_opens_the_dialog_with_the_editor_focused() {
    let mut app = comments_tab();
    app.press(&["n"]);
    assert!(is_open(&app));
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "esc Close"]);
    // `q` types.
    app.press(&["q"]);
    assert!(is_open(&app));
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close"]);
    app.press(&["tab", "tab"]);
    assert_eq!(
        footer(&mut app),
        ["shift+⏎ Submit", "esc Close"],
        "tab wraps to the editor"
    );
}

// The preview.

#[test]
fn the_preview_follows_each_edit() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "A **new** comment");
    let screen = app.screen(160, 48);
    assert!(screen.contains("█ A **new** comment "), "{screen}");
    assert!(screen.contains("█ A new comment "), "{screen}");
    app.press(&["backspace"]);
    assert!(app.screen(160, 48).contains("█ A new commen "));
    app.send(Event::Paste("t, pasted".into()));
    assert!(app.screen(160, 48).contains("█ A new comment, pasted "));
}

// Posting.

#[test]
fn post_comment_sends_the_text_and_appends_the_comment() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "A **new** comment");
    app.press(&["tab", "enter"]);
    assert_eq!(posts(&app), [(12, "A **new** comment".to_owned())]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert!(toasts(&app).is_empty(), "nothing is toasted");
    let screen = app.screen(160, 48);
    assert!(screen.contains("parity-bot • Tue Mar  3 09:00:00 2026"), "{screen}");
}

#[test]
fn modal_submit_sends_the_text_and_an_empty_one_is_sent_as_is() {
    let mut app = comments_tab();
    app.press(&["n", "shift+enter"]);
    assert_eq!(posts(&app), [(12, String::new())]);
    // GitHub refuses it (the fake's 422): the dialog stays.
    settle(&mut app);
    assert!(is_open(&app));
}

#[test]
fn a_failure_toasts_and_keeps_the_text() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Nope");
    app.press(&["shift+enter"]);
    let out = crate::test_util::logs(|| settle(&mut app));
    assert!(out.contains("ERROR Error while posting comment for issue #12"), "{out}");
    assert!(is_open(&app));
    assert_eq!(
        toasts(&app),
        [(
            Some("Error posting comment".to_owned()),
            "Could not post the comment".to_owned()
        )]
    );
    assert!(app.screen(160, 48).contains("█ Nope "));
}

#[test]
fn a_second_submit_while_one_is_in_flight_is_ignored() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Hi");
    app.press(&["shift+enter", "shift+enter"]);
    assert_eq!(posts(&app).len(), 1);
}

#[test]
fn another_response_is_handed_back() {
    let mut app = comments_tab();
    let mut dialog = NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::None);
    app.with_cx(|cx| dialog.update(NewCommentAction::Submit, cx));
    let id = dialog.pending.unwrap();
    app.with_cx(|cx| {
        let other = dialog.on_response(id, GithubResponse::UpdateIssue(Err(GhError::NotInstalled)), cx);
        assert!(other.is_some());
        assert!(dialog.is_pending());
        let none = dialog.on_response(
            id,
            GithubResponse::CreateIssueComment(Ok(Box::new(posted("POST.json")))),
            cx,
        );
        assert!(none.is_none());
    });
    assert_eq!(
        dialog.take_output(),
        Some(Some(PostedComment::Issue(Box::new(posted("POST.json")))))
    );
}

// Closing.

#[test]
fn cancel_and_escape_close_without_a_request() {
    for keys in [&["tab", "tab", "enter"][..], &["escape"], &["tab", "q"]] {
        let mut app = comments_tab();
        app.press(&["n"]);
        app.press(keys);
        assert!(!is_open(&app), "{keys:?}");
        settle(&mut app);
        assert!(posts(&app).is_empty(), "{keys:?}");
    }
}

#[test]
fn closing_in_flight_cancels_the_request() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Hi");
    app.press(&["shift+enter", "escape"]);
    let id = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::CreateIssueComment { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    assert!(app.effects().contains(&Effect::Cancel(id)));
}

// Replying.

#[test]
fn r_on_a_comment_replies_to_it_and_the_reply_is_appended() {
    let mut app = comments_tab();
    // The second block, the deleted account's comment.
    app.press(&["tab", "tab", "r"]);
    assert!(is_open(&app));
    assert!(
        app.screen(160, 48)
            .contains("Replying to comment from Unknown at Wed Feb  4 11:00:00 2026")
    );
    type_text(&mut app, "Agreed, thanks.");
    // Post Comment is below the screen's edge: tab scrolls it into view.
    app.press(&["tab"]);
    assert!(app.screen(160, 48).contains("Post Comment"));
    app.press(&["enter"]);
    assert_eq!(posts(&app), [(12, "Agreed, thanks.".to_owned())]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert!(app.screen(160, 48).contains("parity-bot • Tue Mar  3 09:05:00 2026"));
}

// Drawing.

#[test]
fn new_comment_dialog_160x48() {
    let mut app = comments_tab();
    app.press(&["n"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_comment_dialog_100x30() {
    let mut app = comments_tab();
    app.press(&["n"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

/// The dialog alone, drawn whole into a 100 x 50 frame on a screen tall enough for it.
fn draw(dialog: &mut NewCommentDialog) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let theme = Theme::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 51)).unwrap();
    terminal
        .draw(|frame| {
            let rect = Rect::new(0, 0, 100, 50);
            frame.render_widget(
                super::super::DialogFrame {
                    theme: &theme,
                    background: theme.palette.surface_lighten_3,
                    border: super::super::default_border(theme.palette),
                },
                rect,
            );
            let keymap = Keymap::defaults();
            let cx = RenderCx {
                theme: &theme,
                keymap: &keymap,
                now: tokio::time::Instant::now(),
                focused: true,
                focus_within: true,
                links: None,
                hyperlinks: None,
            };
            dialog.render(frame, super::super::DialogFrame::interior(rect), &cx);
        })
        .unwrap();
    buffer_to_styled_string(terminal.backend().buffer())
}

#[test]
fn new_comment_reply_with_a_two_line_quote() {
    let mut quoted = comments()[0].clone();
    quoted.body = "First line of the quote\nand its second line.".into();
    let mut dialog = NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::IssueComment(Box::new(quoted)));
    insta::assert_snapshot!(draw(&mut dialog));
}

#[test]
fn new_comment_preview_with_bold_and_a_code_span() {
    let mut dialog = NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::None);
    let now = tokio::time::Instant::now();
    let _ = dialog.editor.handle_paste("Some **bold** and `code`", now);
    dialog.preview_stale = true;
    insta::assert_snapshot!(draw(&mut dialog));
}

// Links (#355): Textual's `Markdown` keeps `open_links` in the dialog, so a click on a link in the quote or the preview
// opens it in the browser, with nothing shown in the app.

fn opened(app: &TestApp) -> Vec<String> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::OpenUrl(url) => Some(url.clone()),
            _ => None,
        })
        .collect()
}

/// The text in `buffer` under `link`'s cells.
fn link_text(buffer: &Buffer, link: &crate::components::link_map::ScreenLink) -> String {
    (link.x..link.x + link.width)
        .map(|x| buffer[(x, link.y)].symbol().to_owned())
        .collect()
}

#[test]
fn a_click_on_a_link_in_the_preview_opens_it_in_the_browser() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.send(Event::Paste("see [zzdocs](https://example.com/docs) here".into()));
    let buffer = app.render(160, 48);
    let links: Vec<_> = app.app().dialog_links().iter().cloned().collect();
    assert_eq!(links.len(), 1, "{links:?}");
    let link = &links[0];
    assert_eq!(&*link.target, "https://example.com/docs");
    assert_eq!(link_text(&buffer, link), "zzdocs");
    // The main screen's links aren't clickable under the dialog.
    assert!(
        app.app().links().iter().all(|l| !l.contains(link.x, link.y)),
        "{:?}",
        app.app().links()
    );

    app.click(link.x + 2, link.y);
    assert_eq!(opened(&app), ["https://example.com/docs"]);
    assert!(is_open(&app));
    assert!(toasts(&app).is_empty(), "{:?}", toasts(&app));
    assert!(posts(&app).is_empty());
}

#[test]
fn a_press_on_a_link_and_a_release_off_it_open_nothing() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.send(Event::Paste("see [zzdocs](https://example.com/docs) here".into()));
    app.render(160, 48);
    let link = app.app().dialog_links().iter().next().cloned().unwrap();
    app.mouse(
        crossterm::event::MouseEventKind::Down(MouseButton::Left),
        link.x,
        link.y,
    );
    app.mouse(
        crossterm::event::MouseEventKind::Up(MouseButton::Left),
        link.x + link.width,
        link.y,
    );
    assert!(opened(&app).is_empty());
    assert!(is_open(&app));
}

#[test]
fn links_in_the_quote_and_the_preview_land_where_they_are_drawn() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut quoted = comments()[0].clone();
    quoted.body = "Read [the guide](https://example.com/guide) first".into();
    let mut dialog = NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::IssueComment(Box::new(quoted)));
    let _ = dialog.editor.handle_paste(
        "Line one\n\nand <https://example.com/auto>",
        tokio::time::Instant::now(),
    );
    dialog.preview_stale = true;

    let theme = Theme::default();
    let keymap = Keymap::defaults();
    let sink = crate::components::link_map::LinkSink::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 51)).unwrap();
    terminal
        .draw(|frame| {
            let cx = RenderCx {
                theme: &theme,
                keymap: &keymap,
                now: tokio::time::Instant::now(),
                focused: true,
                focus_within: true,
                links: Some(&sink),
                hyperlinks: None,
            };
            let rect = Rect::new(0, 0, 100, 50);
            dialog.render(frame, super::super::DialogFrame::interior(rect), &cx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let links: Vec<_> = sink.take().iter().cloned().collect();
    let drawn: Vec<(String, String)> = links
        .iter()
        .map(|link| (link.target.to_string(), link_text(buffer, link)))
        .collect();
    assert_eq!(
        drawn,
        [
            ("https://example.com/guide".to_owned(), "the guide".to_owned()),
            (
                "https://example.com/auto".to_owned(),
                "https://example.com/auto".to_owned()
            ),
        ]
    );
}
