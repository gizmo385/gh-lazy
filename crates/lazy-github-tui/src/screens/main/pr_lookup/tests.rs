//! The pull request lookup: its kind on its own, then `O` on the PR table against the `pr-details` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{FullPullRequest, IssueListItem, Repository};
use ratatui::style::Color;
use rstest::rstest;
use serde_json::Value;

use super::*;
use crate::effect::{Effect, RequestId};
use crate::modal::ModalKind;
use crate::screens::main::{FocusTarget, MainScreen, SectionId};
use crate::test_util::{TestApp, logs};

const REPO: &str = "parity-bot/pr-details";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/pr-details")
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

fn fixture_pr(number: u64) -> FullPullRequest {
    let body = fixture_body(&format!("repos/{REPO}/pulls/{number}/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

/// PRs 5, 4, 3 and 2.
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
    }
}

/// PR 9 exists but isn't on the issues page, 42 doesn't exist, and 5 answers with a new title.
fn gh() -> FakeGithub {
    let mut renamed = fixture_pr(5);
    renamed.pr.issue.title = "Renamed since the list".into();
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()))
        .with_pull_request(REPO, 9, Ok(fixture_pr(9)))
        .with_pull_request(REPO, 5, Ok(renamed))
        .with_pull_request(REPO, 42, Err(not_found()))
}

fn config() -> Config {
    let mut config = Config::default();
    config.appearance.show_issues = false;
    config.appearance.show_workflows = false;
    config
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The repo selected, its PRs listed and the PR table focused.
fn app() -> TestApp {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
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
    app.app().modals() == [ModalKind::Lookup(LookupTarget::PullRequest)]
}

fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            '-' => "minus".to_owned(),
            '+' => "plus".to_owned(),
            '.' => "full_stop".to_owned(),
            '_' => "underscore".to_owned(),
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
        Some("Invalid PR Number".into()),
        "Must enter a valid pull request number!".into(),
    )
}

fn unknown() -> (Option<String>, String) {
    (Some("Unknown PR".into()), "Could not find pull request!".into())
}

/// The pull request requests the app sent, by number.
fn pr_requests(app: &TestApp) -> Vec<u64> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                request: GithubRequest::PullRequest { number, .. },
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
    main(app).issue_lists.prs_table().row_keys().copied().collect()
}

fn shown_pr(app: &TestApp) -> Option<u64> {
    main(app).details_pull_request().map(FullPullRequest::number)
}

// The kind.

#[rstest]
#[case("9", true)]
#[case("42", true)]
#[case("1.5", true)]
#[case("-3", true)]
#[case("1e2", true)]
#[case("", false)]
#[case("-", false)]
fn a_number_is_valid(#[case] text: &str, #[case] valid: bool) {
    assert_eq!(PrLookup::is_valid(text), valid, "{text:?}");
}

#[test]
fn a_whole_number_requests_that_pull_request_in_the_current_repo() {
    let mut app = app();
    for (text, number) in [("9", 9), ("+5", 5), ("1_0", 10), ("0", 0)] {
        let request = app.with_cx(|cx| PrLookup::request(text, cx));
        let Ok(GithubRequest::PullRequest { repo, number: asked }) = request else {
            panic!("{text:?}: {request:?}");
        };
        assert_eq!((repo.full_name.as_str(), asked), (REPO, number), "{text:?}");
    }
}

#[test]
fn anything_else_is_refused_at_submit() {
    let mut app = app();
    for text in ["1.5", "", "1e2", ".", "-"] {
        let toast = app.with_cx(|cx| PrLookup::request(text, cx)).unwrap_err();
        assert_eq!((toast.title, toast.message.to_string()), invalid_number(), "{text:?}");
    }
    // A negative number is no pull request, so it's unknown without asking.
    let toast = app.with_cx(|cx| PrLookup::request("-3", cx)).unwrap_err();
    assert_eq!((toast.title, toast.message.to_string()), unknown());
}

#[test]
fn without_a_repo_the_request_toasts() {
    let mut app = TestApp::with_github(config(), gh());
    let toast = app.with_cx(|cx| PrLookup::request("9", cx)).unwrap_err();
    assert_eq!(toast, select_repository_first());
}

#[test]
fn the_answer_is_the_pull_request_or_unknown() {
    let found = PrLookup::found(GithubResponse::PullRequest(Ok(Box::new(fixture_pr(9)))));
    assert_eq!(found.ok().and_then(Result::ok).map(|pr| pr.number()), Some(9));
    for error in [not_found(), GhError::NotInstalled] {
        let Ok(Err(toast)) = PrLookup::found(GithubResponse::PullRequest(Err(error))) else {
            panic!("a toast");
        };
        assert_eq!((toast.title, toast.message.to_string()), unknown());
    }
    let other = GithubResponse::Repository(Ok(None));
    assert!(PrLookup::found(other).is_err(), "another response is handed back");
}

// Opening.

#[test]
fn o_on_the_pr_table_opens_the_dialog_with_its_footer_key() {
    let mut app = app();
    let footer: Vec<String> = crate::components::footer::footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect();
    assert!(footer.contains(&"O Lookup Pull Request".to_owned()), "{footer:?}");
    app.press(&["O"]);
    assert!(is_open(&app));
    let screen = app.screen(160, 48);
    for text in [
        "Search for a pull request by number:",
        "Pull Request Number:",
        "Pull request number",
        "Lookup",
        "Cancel",
    ] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
}

#[test]
fn without_a_repo_o_toasts_and_opens_nothing() {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["2", "O"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(toasts(&app), [(None, "Please select a repository first!".to_owned())]);
}

// The input.

#[test]
fn the_input_takes_only_what_can_make_a_number() {
    let mut app = opened();
    type_text(&mut app, "ab1.x5e-");
    let screen = app.screen(160, 48);
    assert!(screen.contains("▊  1.5e-"), "{screen}");
    // Pasted text too: only a paste whose result can make a number goes in.
    app.send(crate::event::Event::Paste("2z".into()));
    app.send(crate::event::Event::Paste("2".into()));
    assert!(app.screen(160, 48).contains("▊  1.5e-2 "));
}

/// The color of the input's top border.
fn input_border(app: &mut TestApp) -> Color {
    let screen = app.screen(160, 48);
    let text_row = screen
        .lines()
        .position(|line| line.contains("Pull Request Number:"))
        .unwrap();
    app.render(160, 48)[(60, u16::try_from(text_row + 1).unwrap())].fg
}

#[test]
fn an_empty_input_is_invalid_once_checked() {
    let mut app = opened();
    let normal = input_border(&mut app);
    app.press(&["tab"]);
    let blurred_invalid = input_border(&mut app);
    assert_ne!(normal, blurred_invalid);
    app.press(&["shift+tab"]);
    type_text(&mut app, "1.5");
    // A number, if not a whole one: valid.
    assert_eq!(input_border(&mut app), normal);
}

// Submitting.

#[test]
fn an_empty_or_fractional_number_toasts_and_keeps_the_dialog() {
    for text in ["", "1.5"] {
        let mut app = opened();
        type_text(&mut app, text);
        app.press(&["tab", "enter"]);
        assert_eq!(toasts(&app), [invalid_number()], "{text:?}");
        assert!(pr_requests(&app).is_empty(), "{text:?}");
        assert!(is_open(&app), "{text:?}");
    }
}

#[test]
fn an_unknown_number_toasts_and_keeps_the_text() {
    let mut app = opened();
    type_text(&mut app, "42");
    app.press(&["tab", "enter"]);
    assert_eq!(pr_requests(&app), [42]);
    settle(&mut app);
    assert_eq!(toasts(&app), [unknown()]);
    assert!(is_open(&app));
    assert!(app.screen(160, 48).contains("▊  42 "));
    assert_eq!(rows(&app), [5, 4, 3, 2]);
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn a_found_pull_request_is_added_sorted_and_selected_without_a_second_request() {
    let mut app = opened();
    type_text(&mut app, "9");
    app.press(&["shift+enter"]);
    let out = settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert!(toasts(&app).is_empty());
    assert!(out.contains("INFO Looked up PR #9"), "{out}");
    assert_eq!(rows(&app), [9, 5, 4, 3, 2]);
    assert_eq!(pr_requests(&app), [9], "the lookup's answer is shown as is");
    assert_eq!(shown_pr(&app), Some(9));
    assert_eq!(main(&app).details_title(), "[5] PR #9 Details");
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    // The table cache has it too.
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::WriteCache { text, .. } if text.contains("Clamp the cursor to the last line")
    )));
}

#[test]
fn a_pull_request_already_in_the_table_is_neither_added_nor_replaced() {
    let mut app = opened();
    type_text(&mut app, "5");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(rows(&app), [5, 4, 3, 2]);
    let row = main(&app).issue_lists.prs_table().get(&5).unwrap();
    assert_ne!(row.issue.title, "Renamed since the list");
    // The details show the answer.
    assert_eq!(
        main(&app).details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("Renamed since the list")
    );
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
}

#[test]
fn cancel_and_close_change_nothing() {
    for keys in [&["escape"][..], &["tab", "tab", "enter"]] {
        let mut app = opened();
        type_text(&mut app, "9");
        app.press(keys);
        assert!(app.app().modals().is_empty(), "{keys:?}");
        settle(&mut app);
        assert!(pr_requests(&app).is_empty(), "{keys:?}");
        assert_eq!(rows(&app), [5, 4, 3, 2]);
        assert_eq!(shown_pr(&app), None);
        assert_eq!(main(&app).focus(), FocusTarget::Table(SectionId::PullRequests));
    }
}

#[test]
fn closing_in_flight_cancels_the_lookup() {
    let mut app = opened();
    type_text(&mut app, "9");
    app.press(&["shift+enter", "escape"]);
    let id: RequestId = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::PullRequest { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    assert!(app.effects().contains(&Effect::Cancel(id)));
    settle(&mut app);
    assert_eq!(rows(&app), [5, 4, 3, 2]);
    assert_eq!(shown_pr(&app), None);
}

// Drawing.

#[test]
fn pr_lookup_dialog() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_lookup_invalid_empty() {
    let mut app = opened();
    app.press(&["tab", "enter"]);
    insta::assert_snapshot!(app.screen(160, 48));
}
