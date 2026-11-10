//! The New comment dialog: its layout, the reply heading, the preview, posting, the answer, closing and its drawing.
//! Opened with `n` and `r` on issue 12's Comments tab against the `issue-write` and `issue-details` fixtures.

use std::path::Path;

use crossterm::event::MouseButton;
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
        .with_list_page(REPO, 1, issues_page())
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

/// The dialog on top, as the app holds it.
fn dialog(app: &TestApp) -> &NewCommentDialog {
    app.app()
        .top_dialog::<NewCommentDialog>()
        .expect("the New comment dialog is on top")
}

/// The sheet's frame as last drawn, border included.
fn frame_of(app: &TestApp) -> Rect {
    let interior = app.app().top_dialog_interior().expect("a dialog was drawn");
    Rect::new(interior.x - 1, interior.y - 1, interior.width + 2, interior.height + 2)
}

/// `dialog` open over issue 12's Comments tab, its reply doing nothing.
fn opened(dialog: NewCommentDialog) -> TestApp {
    let mut app = comments_tab();
    app.with_cx(|cx| cx.open_dialog(dialog, |_| Action::Consumed));
    app
}

/// A reply to issue 12's first comment, its body replaced by `body`.
fn reply_to(body: &str) -> NewCommentDialog {
    let mut quoted = comments()[0].clone();
    quoted.body = body.into();
    NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::IssueComment(Box::new(quoted)))
}

/// A comment long enough to fill any sheet: the paragraphs of a long review answer, again and again.
fn long_body() -> String {
    let paragraphs = [
        "Agreed, and I think the README should say so explicitly. Here's what I'd add under Keys:",
        "- `j` / `k` move one line and stop at either end of the file; they never wrap around.\n- `G` jumps to the last line and `g` to the first.\n- `J` / `K` move by hunk, and stop at the first and the last hunk.",
        "I also checked the three cases you listed against the current build. All three behave as described, so I don't think we need a code change here, just the docs.",
        "One more thing: the help panel lists `j` as \"Line down\" but the footer says \"Down\". We should pick one.",
        "Finally, the screenshot in the README is from 0.5 and still shows the old title bar. I'll refresh it in the same PR.",
    ];
    let mut text = paragraphs.join("\n\n");
    for _ in 0..3 {
        text = format!("{text}\n\n{}", paragraphs.join("\n\n"));
    }
    text
}

/// A quote of nine lines, more than the cap at either size.
const LONG_QUOTE: &str = "No, it stops at the last line. The parser keeps going until it hits EOF and the viewer \
then shows the last page, so `j` at the bottom does nothing.\n\nSteps:\n1. Open a file longer than the screen\n2. \
Press `G`\n3. Press `j`\n\nExpected: nothing happens. Actual: nothing happens, which is right; see \
[the docs](https://example.com/docs).";

/// The cell where `needle` starts on `screen`, by chars (the frame's blocks are wider in bytes).
fn cell_of(screen: &str, needle: &str) -> (u16, u16) {
    screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let at = line.find(needle)?;
            let column = line[..at].chars().count();
            Some((u16::try_from(column).unwrap(), u16::try_from(row).unwrap()))
        })
        .unwrap_or_else(|| panic!("{needle:?} isn't on the screen:\n{screen}"))
}

const BODY_FOOTER: [&str; 5] = ["^s Post", "shift+⏎ Post", "^t Preview", "^o Expand", "esc Leave"];
const SHEET_FOOTER: [&str; 5] = ["^s Post", "shift+⏎ Post", "^t Preview", "^o Expand", "q Close"];

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
    assert_eq!(
        NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::None).heading,
        "New comment"
    );
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
    let target = ReplyTarget::Review(Box::new(review));
    assert_eq!(target.heading(), None);
    // Answered like a new comment.
    let dialog = NewCommentDialog::new(fixture_repo(), 5, target);
    assert_eq!(dialog.heading, "New comment");
    assert!(dialog.quote.is_none());
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
    app.with_cx(|cx| dialog.update(NewCommentAction::Compose(ComposeAction::Submit), cx));
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::Github {
            request: GithubRequest::ReplyToReviewComment { pr_number: 5, comment_id, .. },
            ..
        } if *comment_id == id
    )));
}

#[test]
fn a_reply_to_a_diff_thread_quotes_its_comment_and_goes_to_the_root() {
    let mut app = comments_tab();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../parity/fixtures/pr-diff-comments/repos/parity-bot/pr-details/pulls/5/reviews/502/comments/GET.json",
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let reply: ReviewComment = serde_json::from_value(fixture["response"]["body"][0].clone()).unwrap();
    assert_eq!((reply.comment.id, reply.in_reply_to_id), (9002, Some(9001)));
    let target = ReplyTarget::Thread {
        quoted: Box::new(reply),
        root: 9001,
    };
    let mut dialog = NewCommentDialog::new(fixture_repo(), 5, target);
    assert_eq!(
        dialog.heading,
        "Replying to comment from parity-friend at Mon Feb  2 11:00:00 2026"
    );
    assert!(dialog.quote.is_some());
    app.with_cx(|cx| dialog.update(NewCommentAction::Compose(ComposeAction::Submit), cx));
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::Github {
            request: GithubRequest::ReplyToReviewComment {
                pr_number: 5,
                comment_id: 9001,
                ..
            },
            ..
        }
    )));
}

// Opening, focus and the footer.

#[test]
fn n_opens_the_sheet_with_the_body_focused() {
    let mut app = comments_tab();
    app.press(&["n"]);
    assert!(is_open(&app));
    assert_eq!(footer(&mut app), BODY_FOOTER);
    let screen = app.screen(160, 48);
    assert!(screen.contains("█ New comment "), "{screen}");
    assert!(screen.contains("█  Write a comment (markdown) "), "{screen}");
    // `q` types.
    app.press(&["q"]);
    assert!(is_open(&app));
    assert_eq!(dialog(&app).text(), "q");
}

#[test]
fn escape_in_the_body_leaves_it_and_a_second_escape_closes() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Keep me");
    app.press(&["escape"]);
    assert!(is_open(&app), "the first escape only leaves the body");
    assert_eq!(footer(&mut app), SHEET_FOOTER);
    // Printable keys do nothing on the sheet.
    app.press(&["x"]);
    assert_eq!(dialog(&app).text(), "Keep me");
    // `tab` goes back, `shift+tab` too.
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), BODY_FOOTER);
    app.press(&["escape", "shift+tab"]);
    assert_eq!(footer(&mut app), BODY_FOOTER);
    // `tab` in the body does nothing: it's the only field.
    app.press(&["tab"]);
    assert_eq!(footer(&mut app), BODY_FOOTER);
    app.press(&["escape", "escape"]);
    assert!(!is_open(&app));
    settle(&mut app);
    assert!(posts(&app).is_empty());
}

#[test]
fn q_on_the_sheet_closes() {
    let mut app = comments_tab();
    app.press(&["n", "escape", "q"]);
    assert!(!is_open(&app));
}

#[test]
fn a_click_on_the_body_focuses_it_again() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Hello");
    app.press(&["escape"]);
    let (column, row) = cell_of(&app.screen(160, 48), " Hello ");
    app.click(column + 3, row);
    assert_eq!(footer(&mut app), BODY_FOOTER);
    type_text(&mut app, "X");
    assert_eq!(dialog(&app).text(), "HeXllo");
}

// The preview.

#[test]
fn ctrl_t_shows_the_rendered_text_and_brings_the_editor_back() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "A **new** comment");
    let screen = app.screen(160, 48);
    assert!(screen.contains("█  A **new** comment "), "{screen}");
    app.press(&["left", "left"]);
    app.press(&["ctrl+t"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("█  A new comment "), "{screen}");
    assert!(!screen.contains("**"), "{screen}");
    assert_eq!(
        footer(&mut app),
        ["^s Post", "shift+⏎ Post", "^t Write", "^o Expand", "esc Leave"]
    );
    // Printable keys and pastes do nothing; the text and the cursor are kept.
    app.press(&["z", "q"]);
    app.send(Event::Paste("pasted".into()));
    assert!(is_open(&app));
    app.press(&["ctrl+t"]);
    assert_eq!(dialog(&app).text(), "A **new** comment");
    type_text(&mut app, "X");
    assert_eq!(dialog(&app).text(), "A **new** commeXnt");
}

#[test]
fn a_click_on_the_tabs_switches_and_focuses_the_body() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Some **bold**");
    app.press(&["escape"]);
    let (column, row) = cell_of(&app.screen(160, 48), "Preview ");
    app.click(column + 1, row);
    assert_eq!(dialog(&app).body.mode(), BodyMode::Preview);
    assert_eq!(footer(&mut app)[2], "^t Write");
    assert_eq!(footer(&mut app)[4], "esc Leave");
    assert!(app.screen(160, 48).contains("█  Some bold "));
    let (column, _) = cell_of(&app.screen(160, 48), " Write ");
    app.click(column + 1, row);
    assert_eq!(dialog(&app).body.mode(), BodyMode::Write);
}

// Size and place.

#[test]
fn the_sheet_is_seven_rows_empty_and_nine_with_a_one_line_quote() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.render(160, 48);
    assert_eq!(frame_of(&app), Rect::new(30, 20, 100, 7));
    app.render(100, 30);
    assert_eq!(frame_of(&app), Rect::new(2, 11, 96, 7));
    let mut app = opened(reply_to("No, it stops at the last line."));
    app.render(160, 48);
    assert_eq!(frame_of(&app).height, 9);
}

#[test]
fn the_body_grows_with_no_scrollbar_until_it_overflows() {
    // At 160x48 the sheet may be 45 rows: 41 of body under the heading and the tabs.
    let mut app = comments_tab();
    app.press(&["n"]);
    app.render(160, 48);
    let top = frame_of(&app).y;
    for lines in 1..=41u16 {
        if lines > 1 {
            app.press(&["enter"]);
        }
        type_text(&mut app, "line");
        app.render(160, 48);
        let frame = frame_of(&app);
        assert_eq!(frame.height, lines.max(3) + 4, "{lines} lines");
        assert!(
            !dialog(&app).body.editor().shows_scrollbar(),
            "a scrollbar at {lines} lines"
        );
        // The top stays until the bottom reaches the margin row, then the sheet grows up.
        assert_eq!(frame.y, top.min(46 - frame.height), "{lines} lines");
        assert!(frame.y >= 1);
    }
    // One more line: the sheet can't grow, so the body scrolls with its bar, the cursor in view.
    app.press(&["enter"]);
    type_text(&mut app, "last");
    let screen = app.screen(160, 48);
    assert_eq!(frame_of(&app), Rect::new(30, 1, 100, 45));
    assert!(dialog(&app).body.editor().shows_scrollbar());
    assert!(screen.contains("█  last "), "{screen}");
    // Deleting the last line takes the bar away, and deleting another shrinks the sheet again.
    let backspaces = ["backspace"; 5];
    app.press(&backspaces);
    app.render(160, 48);
    assert_eq!(frame_of(&app).height, 45);
    assert!(!dialog(&app).body.editor().shows_scrollbar());
    app.press(&backspaces);
    app.render(160, 48);
    assert_eq!(frame_of(&app).height, 44);
}

#[test]
fn expanding_fills_the_screen_and_collapsing_centers_again() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Short");
    app.render(100, 30);
    let before = frame_of(&app);
    app.press(&["ctrl+o"]);
    app.render(100, 30);
    assert_eq!(frame_of(&app), Rect::new(0, 0, 100, 29));
    assert_eq!(footer(&mut app)[3], "^o Collapse");
    type_text(&mut app, " and more");
    app.press(&["ctrl+o"]);
    app.render(100, 30);
    assert_eq!(frame_of(&app), before);
    assert_eq!(dialog(&app).text(), "Short and more");
}

#[test]
fn a_long_quote_is_capped_by_the_screen() {
    let mut app = opened(reply_to(LONG_QUOTE));
    let screen = app.screen(160, 48);
    assert!(screen.contains("▎ ⋯ "), "{screen}");
    let screen = app.screen(100, 30);
    let quoted = screen.lines().filter(|line| line.contains("█ ▎")).count();
    assert_eq!(quoted, 4, "{screen}");
}

// Posting.

#[test]
fn ctrl_s_and_shift_enter_send_the_same_request() {
    for key in ["ctrl+s", "shift+enter"] {
        let mut app = comments_tab();
        app.press(&["n"]);
        type_text(&mut app, "A **new** comment");
        app.press(&[key]);
        assert_eq!(posts(&app), [(12, "A **new** comment".to_owned())], "{key}");
    }
}

#[test]
fn the_compose_keys_follow_bindings_overrides() {
    let mut config = Config::default();
    // `f6` and `f7` are the editor's select line and select all.
    for (id, keys) in [
        ("compose.submit", "f5"),
        ("compose.toggle_preview", "f8"),
        ("compose.expand", "f9"),
    ] {
        config.bindings.overrides.insert(id.into(), keys.into());
    }
    let mut app = TestApp::with_github(config, gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "enter"]);
    settle(&mut app);
    app.press(&["5", "right", "tab", "n"]);
    let mut items = footer(&mut app);
    items.sort();
    assert_eq!(
        items,
        ["esc Leave", "f5 Post", "f8 Preview", "f9 Expand", "shift+⏎ Post"]
    );
    type_text(&mut app, "Hi");
    // The old keys do nothing now.
    app.press(&["ctrl+t", "ctrl+o", "ctrl+s"]);
    assert_eq!(dialog(&app).body.mode(), BodyMode::Write);
    assert!(posts(&app).is_empty());
    app.press(&["f8"]);
    assert_eq!(dialog(&app).body.mode(), BodyMode::Preview);
    app.press(&["f9"]);
    assert!(footer(&mut app).contains(&"f9 Collapse".to_owned()));
    app.press(&["f5"]);
    assert_eq!(posts(&app), [(12, "Hi".to_owned())]);
}

#[test]
fn ctrl_s_posts_from_the_sheet_and_the_preview_too() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "A **new** comment");
    app.press(&["ctrl+t", "escape", "ctrl+s"]);
    assert_eq!(posts(&app), [(12, "A **new** comment".to_owned())]);
}

#[test]
fn posting_closes_and_appends_the_comment() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "A **new** comment");
    app.press(&["ctrl+s"]);
    // In flight: the heading says so and the submit keys are greyed.
    let screen = app.screen(160, 48);
    assert!(screen.contains("Posting… █"), "{screen}");
    let active = app.app_mut().active_bindings();
    let items = crate::components::footer::footer_items(&active);
    assert!(!items[0].enabled && !items[1].enabled);
    assert_eq!((items[0].description, items[1].description), ("Post", "Post"));
    settle(&mut app);
    assert!(!is_open(&app));
    assert!(toasts(&app).is_empty(), "nothing is toasted");
    let screen = app.screen(160, 48);
    assert!(screen.contains("parity-bot • Tue Mar  3 09:00:00 2026"), "{screen}");
}

#[test]
fn an_empty_comment_is_sent_as_is() {
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
    let screen = app.screen(160, 48);
    assert!(screen.contains("█  Nope "), "{screen}");
    assert!(!screen.contains("Posting…"), "{screen}");
    assert!(footer_enabled(&mut app));
}

fn footer_enabled(app: &mut TestApp) -> bool {
    let active = app.app_mut().active_bindings();
    crate::components::footer::footer_items(&active)
        .iter()
        .all(|item| item.enabled)
}

#[test]
fn a_submit_while_one_is_in_flight_is_ignored() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Hi");
    app.press(&["shift+enter", "shift+enter", "ctrl+s"]);
    assert_eq!(posts(&app).len(), 1);
}

#[test]
fn another_response_is_handed_back() {
    let mut app = comments_tab();
    let mut dialog = NewCommentDialog::new(fixture_repo(), 12, ReplyTarget::None);
    app.with_cx(|cx| dialog.update(NewCommentAction::Compose(ComposeAction::Submit), cx));
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

#[test]
fn closing_in_flight_cancels_the_request() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Hi");
    app.press(&["shift+enter", "escape", "escape"]);
    assert!(!is_open(&app));
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
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("█ Replying to comment from Unknown at Wed Feb  4 11:00:00 2026 "),
        "{screen}"
    );
    assert!(screen.contains("█ ▎ A comment from a deleted account. "), "{screen}");
    assert!(screen.contains("█  Write a reply (markdown) "), "{screen}");
    type_text(&mut app, "Agreed, thanks.");
    app.press(&["ctrl+s"]);
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

#[test]
fn new_comment_sheet_focused() {
    let mut app = comments_tab();
    app.press(&["n"]);
    type_text(&mut app, "Agreed, thanks.");
    app.press(&["escape"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_comment_reply_with_a_two_line_quote() {
    // A hard line break: two rows.
    let mut app = opened(reply_to("First line of the quote  \nand its second line."));
    type_text(&mut app, "Agreed.\n\nThanks for checking!");
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_comment_preview_with_bold_and_a_code_span() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.send(Event::Paste("Some **bold** and `code`".into()));
    app.press(&["ctrl+t"]);
    insta::assert_snapshot!(buffer_to_styled_string(&app.render(100, 30)));
}

#[test]
fn new_comment_long_body_160x48() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.send(Event::Paste(long_body()));
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_comment_long_body_100x30() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.send(Event::Paste(long_body()));
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn new_comment_long_quote() {
    let mut app = opened(reply_to(LONG_QUOTE));
    app.send(Event::Paste(long_body()));
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_comment_expanded_100x30() {
    let mut app = opened(reply_to(LONG_QUOTE));
    app.send(Event::Paste("Short for now.".into()));
    app.press(&["ctrl+o"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

// Links (#355): Textual's `Markdown` keeps `open_links` in the dialog, so a click on a link in the quote or the preview
// opens it in the browser, with nothing shown in the app.

fn opened_urls(app: &TestApp) -> Vec<String> {
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
    // Writing, the link is only text.
    app.render(160, 48);
    assert!(app.app().dialog_links().iter().next().is_none());
    app.press(&["ctrl+t"]);
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
    assert_eq!(opened_urls(&app), ["https://example.com/docs"]);
    assert!(is_open(&app));
    assert!(toasts(&app).is_empty(), "{:?}", toasts(&app));
    assert!(posts(&app).is_empty());
}

#[test]
fn a_press_on_a_link_and_a_release_off_it_open_nothing() {
    let mut app = comments_tab();
    app.press(&["n"]);
    app.send(Event::Paste("see [zzdocs](https://example.com/docs) here".into()));
    app.press(&["ctrl+t"]);
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
    assert!(opened_urls(&app).is_empty());
    assert!(is_open(&app));
}

#[test]
fn links_in_the_quote_and_the_preview_land_where_they_are_drawn() {
    let mut app = opened(reply_to("Read [the guide](https://example.com/guide) first"));
    app.send(Event::Paste("Line one\n\nand <https://example.com/auto>".into()));
    app.press(&["ctrl+t"]);
    let buffer = app.render(160, 48);
    let drawn: Vec<(String, String)> = app
        .app()
        .dialog_links()
        .iter()
        .map(|link| (link.target.to_string(), link_text(&buffer, link)))
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

#[test]
fn the_quote_s_cut_links_aren_t_clickable() {
    let mut app = opened(reply_to(LONG_QUOTE));
    app.render(100, 30);
    // The docs link is on the quote's last line, past the cap.
    assert!(
        app.app()
            .dialog_links()
            .iter()
            .all(|link| &*link.target != "https://example.com/docs"),
        "{:?}",
        app.app().dialog_links()
    );
}
