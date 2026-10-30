//! The issue lookup: its kind on its own, then `O` on the Issues table against the `issue-details` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{IssueListItem, Repository};
use rstest::rstest;
use serde_json::Value;

use super::*;
use crate::effect::{Effect, RequestId};
use crate::modal::ModalKind;
use crate::screens::main::{FocusTarget, MainScreen, SectionId};
use crate::test_util::{TestApp, logs};

const REPO: &str = "parity-bot/issue-fixture";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/issue-details")
}

fn fixture_body(path: &str) -> Value {
    let path = fixtures().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// What `GET .../issues/<number>` answers: issues 3 and 12, pull request 8.
fn fixture_item(number: u64) -> IssueListItem {
    let body = fixture_body(&format!("repos/{REPO}/issues/{number}/GET.json"));
    IssueListItem::from_api_item(body, &fixture_repo()).unwrap()
}

/// Issues 12, 11, 10 and 9, and pull request 8.
fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body(&format!("repos/{REPO}/issues/GET.page-1_per_page-30_state-all.json"));
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

fn not_found() -> GhError {
    GhError::Http {
        status: 404,
        message: "Not Found".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// Issue 3 exists but isn't on the issues page, 404 doesn't exist, 12 answers with a new title and 8 is a pull
/// request.
fn gh() -> FakeGithub {
    let IssueListItem::Issue(mut renamed) = fixture_item(12) else {
        panic!("12 is an issue");
    };
    renamed.title = "Renamed since the list".into();
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()))
        .with_issue(REPO, 3, Ok(fixture_item(3)))
        .with_issue(REPO, 12, Ok(IssueListItem::Issue(renamed)))
        .with_issue(REPO, 8, Ok(fixture_item(8)))
        .with_issue(REPO, 404, Err(not_found()))
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The repo selected, its issues listed and the Issues table focused, its cursor on issue 12.
fn app() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3"]);
    app
}

/// [`app`] with `O` pressed.
fn opened() -> TestApp {
    let mut app = app();
    app.press(&["O"]);
    assert!(is_open(&app), "O opens the dialog");
    app
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::Lookup(LookupTarget::Issue)]
}

fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            '.' => "full_stop".to_owned(),
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

fn invalid_number() -> (Option<String>, String) {
    (
        Some("Invalid Issue Number".into()),
        "Must enter a valid issue number!".into(),
    )
}

fn unknown() -> (Option<String>, String) {
    (Some("Unknown Issue".into()), "Could not find issue!".into())
}

/// The issue requests the app sent, by number.
fn issue_requests(app: &TestApp) -> Vec<u64> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                request: GithubRequest::Issue { number, .. },
                ..
            } => Some(*number),
            _ => None,
        })
        .collect()
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

fn rows(app: &TestApp) -> Vec<u64> {
    main(app).issue_lists.issues_table().row_keys().copied().collect()
}

fn issues_cursor(app: &TestApp) -> Option<u64> {
    main(app)
        .issue_lists
        .issues_table()
        .selected()
        .map(|issue| issue.number)
}

fn shown_issue(app: &TestApp) -> Option<u64> {
    main(app).details_issue().map(|issue| issue.number)
}

/// Types `text` into the open dialog and submits it, letting the fake answer.
fn look_up(app: &mut TestApp, text: &str) -> String {
    type_text(app, text);
    app.press(&["shift+enter"]);
    settle(app)
}

// The kind.

#[rstest]
#[case("3", true)]
#[case("1.5", true)]
#[case("-3", true)]
#[case("1e2", true)]
#[case("", false)]
#[case(".", false)]
fn a_number_is_valid(#[case] text: &str, #[case] valid: bool) {
    assert_eq!(IssueLookup::is_valid(text), valid, "{text:?}");
}

#[test]
fn a_whole_number_requests_that_issue_in_the_current_repo() {
    let mut app = app();
    for (text, number) in [("3", 3), ("+5", 5), ("1_0", 10), ("0", 0)] {
        let request = app.with_cx(|cx| IssueLookup::request(text, cx));
        let Ok(GithubRequest::Issue { repo, number: asked }) = request else {
            panic!("{text:?}: {request:?}");
        };
        assert_eq!((repo.full_name.as_str(), asked), (REPO, number), "{text:?}");
    }
}

#[test]
fn anything_else_is_refused_at_submit() {
    let mut app = app();
    for text in ["", "1.5", "1e3", ".", "-", "1__0"] {
        let toast = app.with_cx(|cx| IssueLookup::request(text, cx)).unwrap_err();
        assert_eq!((toast.title, toast.message.to_string()), invalid_number(), "{text:?}");
    }
    // A negative number is no issue, so it's unknown without asking.
    let toast = app.with_cx(|cx| IssueLookup::request("-3", cx)).unwrap_err();
    assert_eq!((toast.title, toast.message.to_string()), unknown());
}

#[test]
fn without_a_repo_the_request_toasts() {
    let mut app = TestApp::with_github(Config::default(), gh());
    let toast = app.with_cx(|cx| IssueLookup::request("3", cx)).unwrap_err();
    assert_eq!(toast, select_repository_first());
}

#[test]
fn the_answer_is_the_item_or_unknown() {
    let found = IssueLookup::found(GithubResponse::Issue(Ok(Box::new(fixture_item(8)))));
    assert_eq!(found.ok().and_then(Result::ok), Some(fixture_item(8)));
    for error in [not_found(), GhError::NotInstalled] {
        let Ok(Err(toast)) = IssueLookup::found(GithubResponse::Issue(Err(error))) else {
            panic!("a toast");
        };
        assert_eq!((toast.title, toast.message.to_string()), unknown());
    }
    let other = GithubResponse::Repository(Ok(None));
    assert!(IssueLookup::found(other).is_err(), "another response is handed back");
}

// Opening.

#[test]
fn o_on_the_issues_table_opens_the_dialog_with_its_footer_key() {
    let mut app = app();
    let footer: Vec<String> = crate::components::footer::footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect();
    assert!(footer.contains(&"O Lookup Issue".to_owned()), "{footer:?}");
    app.press(&["O"]);
    assert!(is_open(&app));
    let screen = app.screen(160, 48);
    for text in [
        "Search for an issue by number:",
        "Issue Number:",
        "Issue number",
        "Lookup",
        "Cancel",
    ] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
}

#[test]
fn without_a_repo_o_toasts_and_opens_nothing() {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["3", "O"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(toasts(&app), [(None, "Please select a repository first!".to_owned())]);
}

#[test]
fn the_input_takes_only_what_can_make_a_number() {
    let mut app = opened();
    type_text(&mut app, "ab1.x");
    assert!(app.screen(160, 48).contains("▊  1. "));
}

// Submitting.

#[test]
fn an_invalid_number_toasts_and_keeps_the_dialog_and_its_text() {
    for text in ["", "1.5"] {
        let mut app = opened();
        type_text(&mut app, text);
        app.press(&["tab", "enter"]);
        assert_eq!(toasts(&app), [invalid_number()], "{text:?}");
        assert!(issue_requests(&app).is_empty(), "{text:?}");
        assert!(is_open(&app), "{text:?}");
        if !text.is_empty() {
            assert!(app.screen(160, 48).contains("▊  1.5 "), "{text:?}");
        }
    }
}

#[test]
fn an_unknown_number_toasts_and_keeps_the_text() {
    let mut app = opened();
    look_up(&mut app, "404");
    assert_eq!(issue_requests(&app), [404]);
    assert_eq!(toasts(&app), [unknown()]);
    assert!(is_open(&app));
    assert!(app.screen(160, 48).contains("▊  404 "));
    assert_eq!(rows(&app), [12, 11, 10, 9]);
    assert_eq!(shown_issue(&app), None);
}

#[test]
fn a_found_issue_is_added_in_number_order_and_selected() {
    let mut app = opened();
    let out = look_up(&mut app, "3");
    assert!(app.app().modals().is_empty());
    assert!(toasts(&app).is_empty());
    assert!(out.contains("INFO Looked up Issue #3"), "{out}");
    assert_eq!(rows(&app), [12, 11, 10, 9, 3]);
    assert_eq!(issue_requests(&app), [3]);
    assert_eq!(shown_issue(&app), Some(3));
    assert_eq!(main(&app).details_title(), "[5] Issue #3 Details");
    assert_eq!(main(&app).details_tabs(), ["Overview", "Comments"]);
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    assert_eq!(issues_cursor(&app), Some(12), "the table's cursor doesn't move");
    // The table cache has it too.
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::WriteCache { text, .. } if text.contains("An old issue on no listed page")
    )));
}

#[test]
fn an_issue_already_in_the_table_is_neither_added_nor_replaced() {
    let mut app = opened();
    let out = look_up(&mut app, "12");
    assert!(out.contains("INFO Looked up Issue #12"), "{out}");
    assert_eq!(rows(&app), [12, 11, 10, 9]);
    let row = main(&app).issue_lists.issues_table().get(&12).unwrap();
    assert_ne!(row.title, "Renamed since the list");
    // The details show the answer.
    assert_eq!(
        main(&app).details_issue().map(|issue| issue.title.as_str()),
        Some("Renamed since the list")
    );
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
}

/// Py bug B61: Python adds the pull request to the Issues table too.
#[test]
fn a_pull_requests_number_is_selected_as_an_issue_but_not_added() {
    let mut app = opened();
    let out = look_up(&mut app, "8");
    assert!(out.contains("INFO Looked up Issue #8"), "{out}");
    assert_eq!(rows(&app), [12, 11, 10, 9]);
    assert_eq!(shown_issue(&app), Some(8));
    assert_eq!(main(&app).details_title(), "[5] Issue #8 Details");
    assert_eq!(main(&app).details_tabs(), ["Overview", "Comments"]);
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    assert_eq!(issues_cursor(&app), Some(12));
}

#[test]
fn cancel_and_close_change_nothing() {
    for keys in [&["escape"][..], &["tab", "tab", "enter"]] {
        let mut app = opened();
        type_text(&mut app, "3");
        app.press(keys);
        assert!(app.app().modals().is_empty(), "{keys:?}");
        settle(&mut app);
        assert!(issue_requests(&app).is_empty(), "{keys:?}");
        assert_eq!(rows(&app), [12, 11, 10, 9]);
        assert_eq!(shown_issue(&app), None);
        assert_eq!(main(&app).focus(), FocusTarget::Table(SectionId::Issues));
    }
}

#[test]
fn closing_in_flight_cancels_the_lookup() {
    let mut app = opened();
    type_text(&mut app, "3");
    app.press(&["shift+enter", "escape"]);
    let id: RequestId = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::Issue { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    assert!(app.effects().contains(&Effect::Cancel(id)));
    settle(&mut app);
    assert_eq!(rows(&app), [12, 11, 10, 9]);
    assert_eq!(shown_issue(&app), None);
}

// Drawing.

#[test]
fn issue_lookup_dialog() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}
