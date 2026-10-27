//! Selecting an issue, its tabs and its Overview in the app, against the `issue-details` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{Issue, IssueListItem, Repository};
use serde_json::Value;

use crate::action::Action;
use crate::app::Maximized;
use crate::component::Owner;
use crate::keymap::BindingId;
use crate::screens::main::{FocusTarget, MainAction, SectionId};
use crate::test_util::{TestApp, logs};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/issue-details")
}

/// The response body of the fixture at `path` in the `issue-details` set.
fn fixture_body(path: &str) -> Value {
    let path = fixtures().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot/issue-fixture`, first on the set's repos page.
fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// The repo's first issues page: issues 12 (open, with a body), 11 (closed, no body), 10 and 9, and PR 8.
fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body("repos/parity-bot/issue-fixture/issues/GET.page-1_per_page-30_state-all.json");
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

/// Issue `number` of `parity-bot/issue-fixture`, as the Issues table holds it.
pub(in crate::screens::main) fn fixture_issue(number: u64) -> Issue {
    issues_page()
        .into_iter()
        .find_map(|item| match item {
            IssueListItem::Issue(issue) if issue.number == number => Some(issue),
            _ => None,
        })
        .unwrap()
}

fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page("parity-bot/issue-fixture", 1, Ok(issues_page()))
        .with_readme(
            "parity-bot/issue-fixture",
            Ok(Some("Issues with bodies and comments.\n".into())),
        )
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The app with `parity-bot/issue-fixture` selected and its issues listed, the Issues table focused on issue 12.
fn app() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3"]);
    app
}

/// [`app`] with issue `12` selected after `moves` cursor moves down.
fn selected(moves: usize) -> TestApp {
    let mut app = app();
    for _ in 0..moves {
        app.press(&["j"]);
    }
    app.press(&["enter"]);
    settle(&mut app);
    app
}

fn main(app: &TestApp) -> &crate::screens::main::MainScreen {
    app.app().main_screen().unwrap()
}

fn focus(app: &TestApp) -> FocusTarget {
    main(app).focus()
}

fn title(app: &TestApp) -> &str {
    main(app).details_title()
}

fn tabs(app: &TestApp) -> (Vec<String>, usize) {
    let main = main(app);
    (main.details_tabs().to_vec(), main.details.tabs().active())
}

fn shown_issue(app: &TestApp) -> Option<u64> {
    main(app).details_issue().map(|issue| issue.number)
}

fn issues_cursor(app: &TestApp) -> Option<u64> {
    main(app)
        .issue_lists
        .issues_table()
        .selected()
        .map(|issue| issue.number)
}

fn footer(app: &mut TestApp) -> String {
    app.screen(160, 48).lines().last().unwrap().to_owned()
}

/// The row of the screen whose text contains `needle`.
fn row_of(app: &mut TestApp, needle: &str) -> u16 {
    let screen = app.screen(160, 48);
    let row = screen.lines().position(|line| line.contains(needle)).unwrap();
    u16::try_from(row).unwrap()
}

#[test]
fn enter_space_and_a_click_each_select_the_issue() {
    let mut app = app();
    let out = logs(|| {
        app.press(&["enter"]);
    });
    assert!(out.contains("INFO Selected Issue: #12"), "{out}");
    assert_eq!(shown_issue(&app), Some(12));

    let mut app = self::app();
    app.press(&["j", "space"]);
    assert_eq!(shown_issue(&app), Some(11));

    // A click on the cursor's cell: the press focuses the table, the release selects. Elsewhere on the row it only
    // moves the cursor.
    let mut app = self::app();
    let row = row_of(&mut app, "│ 12  open");
    app.click(6, row);
    assert_eq!(shown_issue(&app), None);
    app.render(160, 48);
    app.click(2, row);
    assert_eq!(shown_issue(&app), None);
    app.render(160, 48);
    let out = logs(|| {
        app.click(2, row);
    });
    assert!(out.contains("INFO Selected Issue: #12"), "{out}");
    assert_eq!(shown_issue(&app), Some(12));
    assert!(!out.contains("ERROR"), "{out}");
}

#[test]
fn selecting_shows_the_two_tabs_and_focuses_the_strip() {
    let mut app = selected(0);
    assert_eq!(title(&app), "[5] Issue #12 Details");
    assert_eq!(tabs(&app), (vec!["Overview".into(), "Comments".into()], 0));
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    // The Issues table's cursor doesn't move.
    assert_eq!(issues_cursor(&app), Some(12));
    let screen = app.screen(160, 48);
    assert!(screen.contains("┌─ [5] Issue #12 Details ─"), "{screen}");
    assert!(screen.contains("│ Overview  Comments "), "{screen}");
    assert!(
        screen.contains("│Open Crash when the config file is empty (#12) by parity-friend "),
        "{screen}"
    );
}

#[test]
fn the_title_key_follows_the_overrides() {
    let mut config = Config::default();
    config
        .bindings
        .overrides
        .insert("main.details.focus".into(), "9".into());
    let mut app = TestApp::with_github(config, gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "enter"]);
    assert_eq!(title(&app), "[9] Issue #12 Details");
}

#[test]
fn selecting_the_same_issue_again_rebuilds_the_tabs() {
    let mut app = selected(0);
    app.render(100, 20);
    app.press(&["tab", "down", "down"]);
    let offset = |app: &TestApp| main(app).details.issue().unwrap().overview().offset();
    assert_eq!(offset(&app), 2);
    app.press(&["5", "right"]);
    assert_eq!(tabs(&app).1, 1);
    app.press(&["3", "enter"]);
    assert_eq!(tabs(&app).1, 0);
    assert_eq!(offset(&app), 0);
    assert_eq!(shown_issue(&app), Some(12));
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn a_repo_selection_replaces_the_issue_tabs() {
    let mut app = selected(0);
    app.press(&["tab"]);
    app.press(&["1", "enter"]);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(shown_issue(&app), None);
    assert_eq!(title(&app), "[5] parity-bot/issue-fixture Details");
}

#[test]
fn the_open_issue_action_shows_an_issue_at_once() {
    let mut app = app();
    app.dispatch(Action::Main(MainAction::OpenIssue(Box::new(fixture_issue(10)))));
    assert_eq!(shown_issue(&app), Some(10));
    assert_eq!(title(&app), "[5] Issue #10 Details");
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn an_issue_selection_cancels_a_pending_pull_request() {
    let mut app = app();
    app.press(&["2", "enter"]);
    let pending = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            crate::effect::Effect::Github {
                id,
                request: crate::effect::GithubRequest::PullRequest { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    app.press(&["3", "enter"]);
    assert!(app.effects().contains(&crate::effect::Effect::Cancel(pending)));
    assert_eq!(shown_issue(&app), Some(12));
}

#[test]
fn edit_shows_only_with_the_body_focused_and_is_unimplemented() {
    let mut app = selected(0);
    assert!(!footer(&mut app).contains("Edit Issue"));
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    let line = footer(&mut app);
    assert!(
        line.starts_with(" E Edit Issue  ^v Paste link  f Follow link"),
        "{line}"
    );
    let out = logs(|| {
        app.press(&["E"]);
    });
    assert!(out.contains("is not implemented yet"), "{out}");
    let main = app.app_mut().main_screen_mut().unwrap();
    assert!(matches!(
        main.details.on_binding(Owner::IssueOverview, BindingId::IssueEdit),
        Some(Action::Unimplemented(BindingId::IssueEdit))
    ));
    // `tab` again leaves the body: there are no stops inside the Overview.
    app.press(&["tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
    // The Comments tab's body doesn't have the Overview's binding.
    app.press(&["5", "right", "tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert!(!footer(&mut app).contains("Edit Issue"));
}

#[test]
fn switching_tabs_keeps_the_overview_scroll_position() {
    let mut app = selected(0);
    app.render(100, 20);
    app.press(&["tab", "down", "down", "down"]);
    let offset = |app: &TestApp| main(app).details.issue().unwrap().overview().offset();
    assert_eq!(offset(&app), 3);
    app.press(&["5", "right", "left"]);
    app.render(100, 20);
    assert_eq!(offset(&app), 3);
}

#[test]
fn maximize_works_on_the_strip_but_not_the_body() {
    let mut app = selected(0);
    app.press(&["tab", "M"]);
    assert_eq!(app.app().maximized(), None);
    app.press(&["5", "M"]);
    assert_eq!(
        app.app().maximized(),
        Some(Maximized {
            target: FocusTarget::DetailsTabs
        })
    );
}

#[test]
fn the_issues_section_cursor_stays_when_focus_returns() {
    let mut app = selected(1);
    app.press(&["3"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Issues));
    assert_eq!(issues_cursor(&app), Some(11));
}

#[test]
fn issue_overview_160x48() {
    let mut app = selected(0);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_overview_100x30() {
    let mut app = selected(0);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn issue_overview_closed_no_body() {
    let mut app = selected(1);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_overview_body_focused() {
    let mut app = selected(0);
    app.press(&["tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_overview_scrolled_to_the_end_100x30() {
    // Issue 12's body just fits at 100x30: a longer one overflows, with a scrollbar.
    let mut issue = fixture_issue(12);
    let body = issue.body.take().unwrap();
    issue.body = Some(format!("{body}\n{}\nThe last line.\n", "More detail.\n\n".repeat(6)));
    let mut app = app();
    app.dispatch(Action::Main(MainAction::OpenIssue(Box::new(issue))));
    app.render(100, 30);
    app.press(&["tab", "end"]);
    let screen = app.screen(100, 30);
    assert!(screen.contains("The last line."), "{screen}");
    assert!(!screen.contains("Open Crash"), "{screen}");
    insta::assert_snapshot!(screen);
}
