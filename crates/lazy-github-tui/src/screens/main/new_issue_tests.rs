//! `issue.new` on the main screen: the toast without a repo, and the created issue added to the Issues table and
//! opened. The dialog itself is tested in `modal::new_issue`.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{Issue, IssueListItem, Repository};
use serde_json::Value;

use super::{FocusTarget, MainAction, MainScreen};
use crate::action::Action;
use crate::effect::Effect;
use crate::modal::ModalKind;
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

/// Issues 12, 11, 10 and 9, and pull request 8.
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

fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()))
        .with_created_issue(Ok(created()))
}

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

/// The repo selected and its issues listed, with the Issues table focused.
fn app() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3"]);
    app
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

fn rows(app: &TestApp) -> Vec<u64> {
    main(app).issue_lists.issues_table().row_keys().copied().collect()
}

fn toasts(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

#[test]
fn without_a_repo_i_toasts_and_opens_nothing() {
    for keys in [&["I"][..], &["3", "I"], &["2", "I"]] {
        let mut app = TestApp::with_github(Config::default(), gh());
        app.press(keys);
        assert!(app.app().modals().is_empty(), "{keys:?}");
        assert_eq!(
            toasts(&app),
            [(
                Some("Cannot open new issue".to_owned()),
                "Please select a repository first!".to_owned()
            )],
            "{keys:?}"
        );
        assert_eq!(
            app.app().toasts().iter().next().unwrap().severity,
            crate::toast::Severity::Error
        );
    }
}

#[test]
fn i_opens_the_dialog_from_the_left_column_only() {
    let mut app = app();
    app.press(&["I"]);
    assert_eq!(app.app().modals(), [ModalKind::NewIssue]);
    app.press(&["escape", "5", "I"]);
    assert!(app.app().modals().is_empty(), "the details pane has no issue.new");
}

#[test]
fn the_created_issue_is_added_in_its_place_and_opened() {
    let mut app = app();
    app.press(&["I", "tab"]);
    for key in ["P", "space", "t"] {
        app.press(&[key]);
    }
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert_eq!(rows(&app), [13, 12, 11, 10, 9]);
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(13));
    assert_eq!(main(&app).details_title(), "[5] Issue #13 Details");
    assert_eq!(main(&app).details_tabs(), ["Overview", "Comments"]);
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    // The table cache has it.
    assert!(app.effects().iter().any(|effect| matches!(
        effect,
        Effect::WriteCache { text, .. } if text.contains("Parity title")
    )));
    let screen = app.screen(160, 48);
    assert!(screen.contains("13  open    parity-bot     Parity title"), "{screen}");
}

#[test]
fn a_created_issue_already_in_the_table_replaces_its_row() {
    let mut app = app();
    let IssueListItem::Issue(mut issue) = issues_page().remove(0) else {
        panic!("12 is an issue");
    };
    issue.title = "Edited".into();
    app.dispatch(Action::Main(MainAction::IssueCreated(Box::new(issue))));
    assert_eq!(rows(&app), [12, 11, 10, 9]);
    let row = main(&app).issue_lists.issues_table().get(&12).unwrap();
    assert_eq!(row.title, "Edited");
    assert_eq!(
        main(&app).details_issue().map(|issue| issue.title.as_str()),
        Some("Edited")
    );
}
