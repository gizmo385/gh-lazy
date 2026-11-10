//! The New issue dialog on the compose sheet: focus, the footer, submitting, the answer, closing, the mouse, its size
//! and its drawing, opened with `I` against the `issue-write` and `issue-details` fixtures.

use std::path::Path;

use crossterm::event::{KeyModifiers, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::IssueListItem;
use serde_json::Value;

use super::*;
use crate::event::Event;
use crate::modal::issue_form::IssueField;
use crate::test_util::TestApp;

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

/// Issue 13, as `POST .../issues` answers.
fn created() -> Issue {
    let body = fixture_body("issue-write", &format!("repos/{REPO}/issues/POST.json"));
    Issue::from_api(body, &fixture_repo()).unwrap()
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

/// The repo and its issues; `create_issue` answers with `created`, or fails with a 422 for `None`.
fn gh(created: Option<Issue>) -> FakeGithub {
    let gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_list_page(REPO, 1, issues_page());
    match created {
        Some(issue) => gh.with_created_issue(Ok(issue)),
        None => gh,
    }
}

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

/// The repo selected and `I` pressed on the Issues table.
fn opened_with(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "I"]);
    assert!(is_open(&app), "I opens the dialog");
    app
}

fn opened() -> TestApp {
    opened_with(gh(Some(created())))
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::NewIssue]
}

/// Types `text` into the focused field, a key at a time; `\n` is `enter`.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            ' ' => "space".to_owned(),
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

/// The create requests the app sent.
fn creates(app: &TestApp) -> Vec<(RequestId, NewIssue)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::CreateIssue { repo, issue },
            } => {
                assert_eq!(repo.full_name, REPO);
                Some((*id, issue.clone()))
            }
            _ => None,
        })
        .collect()
}

/// A title and a two-line description typed, as the parity scenario types them, with the description focused.
fn typed() -> TestApp {
    let mut app = opened();
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    type_text(&mut app, "First line\nSecond line");
    app
}

fn mouse(app: &mut TestApp, kind: MouseEventKind, column: u16, row: u16) {
    app.send(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

/// The dialog on top, as the app holds it.
fn dialog(app: &TestApp) -> &NewIssueDialog {
    app.app()
        .top_dialog::<NewIssueDialog>()
        .expect("the New issue dialog is on top")
}

/// The sheet's frame as last drawn, border included.
fn frame_of(app: &TestApp) -> Rect {
    let interior = app.app().top_dialog_interior().expect("a dialog was drawn");
    Rect::new(interior.x - 1, interior.y - 1, interior.width + 2, interior.height + 2)
}

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

/// A description far longer than any sheet: 60 numbered lines.
fn long_body() -> String {
    (1..=60)
        .map(|n| format!("Step {n}: what happened, and what should have"))
        .collect::<Vec<_>>()
        .join("\n")
}

const FIELD_FOOTER: [&str; 5] = ["^s Save", "shift+⏎ Save", "^t Preview", "^o Expand", "esc Leave"];
const SHEET_FOOTER: [&str; 5] = ["^s Save", "shift+⏎ Save", "^t Preview", "^o Expand", "q Close"];

// Focus and the footer.

#[test]
fn it_opens_on_the_title_and_tab_cycles_the_title_and_the_description() {
    let mut dialog = NewIssueDialog::new(fixture_repo(), None);
    assert_eq!(dialog.form.focused(), Some(IssueField::Title));
    let mut seen = Vec::new();
    for _ in 0..3 {
        dialog.move_focus(FocusMove::Next);
        seen.push(dialog.form.focused().unwrap());
    }
    assert_eq!(
        seen,
        [IssueField::Description, IssueField::Title, IssueField::Description]
    );
    dialog.move_focus(FocusMove::Previous);
    assert_eq!(dialog.form.focused(), Some(IssueField::Title));
}

#[test]
fn escape_leaves_a_field_and_tab_comes_back_to_the_first_or_the_last() {
    let mut app = opened();
    assert_eq!(footer(&mut app), FIELD_FOOTER);
    app.press(&["escape"]);
    assert!(is_open(&app), "the first escape only leaves the title");
    assert_eq!(footer(&mut app), SHEET_FOOTER);
    assert_eq!(dialog(&app).form.focused(), None);
    // Printable keys do nothing on the sheet.
    type_text(&mut app, "x");
    assert_eq!(dialog(&app).title(), "");
    app.press(&["tab"]);
    assert_eq!(dialog(&app).form.focused(), Some(IssueField::Title));
    app.press(&["escape", "shift+tab"]);
    assert_eq!(dialog(&app).form.focused(), Some(IssueField::Description));
    assert_eq!(footer(&mut app), FIELD_FOOTER);
}

#[test]
fn q_types_in_the_fields_and_closes_from_the_sheet() {
    let mut app = opened();
    app.press(&["q", "tab", "q"]);
    assert!(is_open(&app));
    assert_eq!(
        (dialog(&app).title(), dialog(&app).description()),
        ("q", "q".to_owned())
    );
    app.press(&["escape", "q"]);
    assert!(!is_open(&app));
    assert!(creates(&app).is_empty());
}

// Submitting.

#[test]
fn an_empty_title_toasts_marks_the_title_and_focuses_it() {
    let mut app = opened();
    app.press(&["tab"]);
    type_text(&mut app, "Body");
    app.press(&["ctrl+s"]);
    assert_eq!(toasts(&app), [(None, "Must have a non-empty issue title!".to_owned())]);
    assert_eq!(
        app.app().toasts().iter().next().unwrap().severity,
        crate::toast::Severity::Error
    );
    assert!(creates(&app).is_empty());
    assert!(is_open(&app));
    assert_eq!(dialog(&app).form.focused(), Some(IssueField::Title));
    assert!(dialog(&app).form.is_title_invalid());
    // The label and the placeholder are in the error color.
    let buffer = app.render(160, 48);
    let screen = app.screen(160, 48);
    let theme = crate::theme::Theme::default();
    let error = theme.color(
        theme.roles().text.error,
        theme.roles().surface.dialog.over(theme.background()),
    );
    let (column, row) = cell_of(&screen, "█ Title ");
    let column = column + 2;
    assert_eq!(buffer[(column, row)].fg, error);
    // Typing a title draws it plain again.
    type_text(&mut app, "T");
    assert!(!dialog(&app).form.is_title_invalid());
}

#[test]
fn ctrl_s_and_shift_enter_save() {
    for key in ["ctrl+s", "shift+enter"] {
        let mut app = typed();
        app.press(&[key]);
        let sent: Vec<NewIssue> = creates(&app).into_iter().map(|(_, issue)| issue).collect();
        assert_eq!(
            sent,
            [NewIssue {
                title: "Parity title".into(),
                body: "First line\nSecond line".into(),
            }],
            "{key}"
        );
    }
}

#[test]
fn enter_in_the_title_doesnt_save() {
    let mut app = opened();
    type_text(&mut app, "Title\n");
    assert!(creates(&app).is_empty());
}

/// Py bug B25: Python refuses an empty body.
#[test]
fn a_blank_title_and_an_empty_description_are_sent() {
    let mut app = opened();
    app.press(&["space", "shift+enter"]);
    assert_eq!(toasts(&app), [(None, "Creating new issue...".to_owned())]);
    let sent: Vec<NewIssue> = creates(&app).into_iter().map(|(_, issue)| issue).collect();
    assert_eq!(
        sent,
        [NewIssue {
            title: " ".into(),
            body: String::new(),
        }]
    );
}

#[test]
fn while_saving_the_heading_says_so_and_the_submit_keys_are_disabled() {
    let mut app = typed();
    app.press(&["ctrl+s"]);
    assert!(dialog(&app).form.is_saving());
    let screen = app.screen(160, 48);
    assert!(screen.contains("Saving… █"), "{screen}");
    let active = app.app_mut().active_bindings();
    let items = crate::components::footer::footer_items(&active);
    assert!(!items[0].enabled && !items[1].enabled);
    assert_eq!((items[0].description, items[1].description), ("Save", "Save"));
    // A second submit, by either key, sends nothing.
    app.press(&["ctrl+s", "shift+enter"]);
    assert_eq!(creates(&app).len(), 1);
    assert_eq!(toasts(&app), [(None, "Creating new issue...".to_owned())]);
}

#[test]
fn success_closes_with_the_issue() {
    let mut app = typed();
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert_eq!(
        toasts(&app),
        [
            (None, "Creating new issue...".to_owned()),
            (None, "Successfully created issue #13".to_owned()),
        ]
    );
}

#[test]
fn a_failure_toasts_and_keeps_everything_typed() {
    let mut app = opened_with(gh(None));
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    type_text(&mut app, "Body");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(is_open(&app));
    assert_eq!(
        toasts(&app),
        [
            (None, "Creating new issue...".to_owned()),
            (
                Some("Error creating issue".to_owned()),
                "Could not create the issue".to_owned()
            ),
        ]
    );
    let screen = app.screen(160, 48);
    assert!(screen.contains("Parity title"), "{screen}");
    assert!(screen.contains("1  Body"), "{screen}");
    assert!(!screen.contains("Saving…"), "{screen}");
    // It can be sent again.
    app.press(&["shift+enter"]);
    assert_eq!(creates(&app).len(), 2);
}

#[test]
fn a_failed_answer_is_logged() {
    let mut dialog = NewIssueDialog::new(fixture_repo(), None);
    let mut app = opened();
    dialog.form.set_title("T");
    let id = app.with_cx(|cx| {
        dialog.update(NewIssueAction::Compose(ComposeAction::Submit), cx);
        dialog.pending.unwrap()
    });
    let out = crate::test_util::logs(|| {
        app.with_cx(|cx| {
            let handed_back = dialog.on_response(id, GithubResponse::CreateIssue(Err(GhError::NotInstalled)), cx);
            assert!(handed_back.is_none());
        });
    });
    assert!(out.contains("ERROR Error creating issue:"), "{out}");
    assert!(!dialog.is_pending());
    assert!(!dialog.form.is_saving());
    assert!(dialog.take_output().is_none());
}

#[test]
fn another_response_is_handed_back() {
    let mut dialog = NewIssueDialog::new(fixture_repo(), None);
    let mut app = opened();
    dialog.form.set_title("T");
    app.with_cx(|cx| dialog.update(NewIssueAction::Compose(ComposeAction::Submit), cx));
    app.with_cx(|cx| {
        let other = dialog.on_response(
            RequestId::new(u64::MAX),
            GithubResponse::CreateIssue(Ok(Box::new(created()))),
            cx,
        );
        assert!(other.is_some(), "not its request");
        assert!(dialog.is_pending());
    });
}

// Closing.

#[test]
fn escape_twice_or_escape_and_q_close_without_a_request() {
    for keys in [
        &["escape", "escape"][..],
        &["escape", "q"],
        &["tab", "escape", "escape"],
    ] {
        let mut app = opened();
        app.press(keys);
        assert!(!is_open(&app), "{keys:?}");
        settle(&mut app);
        assert!(creates(&app).is_empty(), "{keys:?}");
        assert!(toasts(&app).is_empty(), "{keys:?}");
    }
}

#[test]
fn closing_in_flight_cancels_the_request() {
    let mut app = typed();
    app.press(&["shift+enter", "escape", "escape"]);
    assert!(!is_open(&app));
    let (id, _) = creates(&app)[0].clone();
    assert!(app.effects().contains(&Effect::Cancel(id)));
}

// Text.

#[test]
fn a_prefilled_description_is_in_the_editor() {
    let dialog = NewIssueDialog::new(fixture_repo(), Some("## Steps\n"));
    assert_eq!(dialog.description(), "## Steps\n");
    assert_eq!(dialog.title(), "");
}

#[test]
fn a_paste_goes_to_the_focused_field() {
    let mut app = opened();
    app.send(Event::Paste("Pasted title\nsecond".into()));
    app.press(&["tab"]);
    app.send(Event::Paste("One\nTwo".into()));
    app.press(&["escape"]);
    app.send(Event::Paste("ignored".into()));
    let screen = app.screen(160, 48);
    assert!(screen.contains("Pasted title "), "{screen}");
    assert!(!screen.contains("ignored"), "{screen}");
    assert!(screen.contains(" 2  Two"), "{screen}");
}

// The mouse.

#[test]
fn a_click_focuses_the_field_under_it_its_label_included() {
    let mut app = opened();
    let screen = app.screen(160, 48);
    let (column, row) = cell_of(&screen, "Description");
    app.click(column, row);
    assert_eq!(dialog(&app).form.focused(), Some(IssueField::Description));
    let (column, row) = cell_of(&screen, "█ Title ");
    let column = column + 2;
    app.click(column, row);
    assert_eq!(dialog(&app).form.focused(), Some(IssueField::Title));
    type_text(&mut app, "Clicked");
    // A click in the body moves the editor's cursor there.
    let (column, row) = cell_of(&screen, "Describe the issue");
    app.click(column, row + 1);
    type_text(&mut app, "Body");
    assert_eq!(
        (dialog(&app).title(), dialog(&app).description()),
        ("Clicked", "Body".to_owned())
    );
}

#[test]
fn the_wheel_over_a_long_body_scrolls_it() {
    let mut app = opened();
    app.press(&["tab"]);
    app.send(Event::Paste(long_body()));
    let screen = app.screen(100, 30);
    assert!(!screen.contains("Step 1:"), "the cursor's end is in view:\n{screen}");
    let (column, row) = cell_of(&screen, "Step 60:");
    for _ in 0..30 {
        mouse(&mut app, MouseEventKind::ScrollUp, column, row);
    }
    assert!(app.screen(100, 30).contains("Step 1:"));
}

// Size.

#[test]
fn the_empty_sheet_is_twelve_rows() {
    let mut app = opened();
    app.render(160, 48);
    assert_eq!(frame_of(&app), Rect::new(30, 17, 100, 12));
    app.render(100, 30);
    assert_eq!(frame_of(&app), Rect::new(2, 8, 96, 12));
}

#[test]
fn a_long_body_grows_the_sheet_to_the_cap_then_scrolls() {
    let mut app = opened();
    app.press(&["tab"]);
    app.send(Event::Paste(long_body()));
    app.render(160, 48);
    assert_eq!(frame_of(&app), Rect::new(30, 1, 100, 45));
    app.render(100, 30);
    assert_eq!(frame_of(&app), Rect::new(2, 1, 96, 27));
}

#[test]
fn the_bug_report_template_opens_the_sheet_at_its_height() {
    let template = "**Describe the bug**\n\n**To Reproduce**\n\n**Expected behavior**\n\n**Debug Info:**";
    let mut app = TestApp::with_github(Config::default(), gh(Some(created())));
    app.with_cx(|cx| {
        cx.open_dialog(NewIssueDialog::new(fixture_repo(), Some(template)), |_| {
            Action::Consumed
        });
    });
    app.render(160, 48);
    // Seven lines of body under the heading, the title and the tabs.
    assert_eq!(frame_of(&app).height, 7 + 7);
}

#[test]
fn expanding_fills_the_screen_and_collapsing_brings_it_back() {
    let mut app = typed();
    app.render(100, 30);
    let before = frame_of(&app);
    app.press(&["ctrl+o"]);
    app.render(100, 30);
    assert_eq!(frame_of(&app), Rect::new(0, 0, 100, 29));
    assert_eq!(footer(&mut app)[3], "^o Collapse");
    app.press(&["ctrl+o"]);
    app.render(100, 30);
    assert_eq!(frame_of(&app), before);
}

#[test]
fn every_row_keeps_its_borders_in_a_tiny_terminal() {
    // The frame fits the screen, so its edges are on screen beside every row of the scrolled interior (#334).
    let mut app = opened();
    let _ = app.render(48, 16);
    app.press(&["tab"]);
    let screen = app.render(48, 16);
    let frame = frame_of(&app);
    for y in frame.y..frame.bottom() {
        assert_eq!(screen[(frame.x, y)].symbol(), "█", "row {y} has its left border");
        assert_eq!(
            screen[(frame.right() - 1, y)].symbol(),
            "█",
            "row {y} has its right border"
        );
    }
}

// Drawing.

#[test]
fn new_issue_dialog_opened() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_issue_dialog_typed() {
    let mut app = typed();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn new_issue_dialog_opened_100x30() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn new_issue_dialog_typed_100x30() {
    let mut app = opened();
    app.screen(100, 30);
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    type_text(&mut app, "First line\nSecond line");
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn new_issue_long_body_100x30() {
    let mut app = opened();
    app.screen(100, 30);
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    app.send(Event::Paste(long_body()));
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn new_issue_preview() {
    let mut app = opened();
    type_text(&mut app, "Parity title");
    app.press(&["tab"]);
    app.send(Event::Paste(
        "## Steps\n\n1. Open the **New issue** sheet\n2. Press `ctrl+t`\n\nSee [the docs](https://example.com/docs)."
            .into(),
    ));
    app.press(&["ctrl+t"]);
    insta::assert_snapshot!(app.screen(160, 48));
}
