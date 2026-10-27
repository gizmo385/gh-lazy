//! Selecting a pull request, its tabs and its Overview in the app, against the `pr-details` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Repository};
use serde_json::Value;

use crate::action::Action;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::{TestApp, logs};
use crate::toast::Toast;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/pr-details")
}

/// The response body of the fixture at `path` in the `pr-details` set.
fn fixture_body(path: &str) -> Value {
    let path = fixtures().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot/pr-details`, first on the set's repos page.
pub(in crate::screens::main) fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// Pull request `number` of `parity-bot/pr-details`, as the pull request endpoint returns it.
pub(in crate::screens::main) fn fixture_pr(number: u64) -> FullPullRequest {
    let body = fixture_body(&format!("repos/parity-bot/pr-details/pulls/{number}/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

/// The repo's first issues page: PRs 5 (open), 4 (closed), 3 (merged) and 2 (draft).
fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body("repos/parity-bot/pr-details/issues/GET.page-1_per_page-30_state-all.json");
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
    }
}

fn gh() -> FakeGithub {
    let name = "parity-bot/pr-details";
    let mut gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(name, 1, Ok(issues_page()))
        .with_pull_request(name, 2, Err(server_error()));
    for number in [5, 4, 3, 9] {
        gh = gh.with_pull_request(name, number, Ok(fixture_pr(number)));
    }
    gh
}

/// The `pr-details` scenario's config: Issues and Workflows hidden.
fn config() -> Config {
    let mut config = Config::default();
    config.appearance.show_issues = false;
    config.appearance.show_workflows = false;
    config
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The app with `parity-bot/pr-details` selected and its PRs listed, the PR table focused on PR 5.
fn app() -> TestApp {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app
}

/// The pull request requests among the app's effects, by number.
fn pr_requests(app: &TestApp) -> Vec<(RequestId, u64)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::PullRequest { number, .. },
            } => Some((*id, *number)),
            _ => None,
        })
        .collect()
}

fn cancels(app: &TestApp) -> Vec<RequestId> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) => Some(*id),
            _ => None,
        })
        .collect()
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

fn shown_pr(app: &TestApp) -> Option<u64> {
    main(app).details_pull_request().map(FullPullRequest::number)
}

/// The row of the screen whose text contains `needle`.
fn row_of(app: &mut TestApp, needle: &str) -> u16 {
    let screen = app.screen(160, 48);
    let row = screen.lines().position(|line| line.contains(needle)).unwrap();
    u16::try_from(row).unwrap()
}

#[test]
fn enter_space_and_a_click_each_request_the_pull_request_once() {
    let mut app = app();
    app.press(&["enter"]);
    assert_eq!(pr_requests(&app).iter().map(|&(_, n)| n).collect::<Vec<_>>(), [5]);

    let mut app = self::app();
    app.press(&["j", "space"]);
    assert_eq!(pr_requests(&app).iter().map(|&(_, n)| n).collect::<Vec<_>>(), [4]);

    // A click on the cursor's cell (the number column of the first row): the press focuses the table, the release
    // selects. Elsewhere on the row it only moves the cursor.
    let mut app = self::app();
    let row = row_of(&mut app, "│ 5  open");
    app.click(6, row);
    assert!(pr_requests(&app).is_empty());
    app.render(160, 48);
    app.click(2, row);
    assert!(pr_requests(&app).is_empty());
    app.render(160, 48);
    app.click(2, row);
    assert_eq!(pr_requests(&app).iter().map(|&(_, n)| n).collect::<Vec<_>>(), [5]);
    let out = settle(&mut app);
    assert_eq!(
        app.gh()
            .calls()
            .into_iter()
            .filter(|call| matches!(call, GithubCall::PullRequest { .. }))
            .collect::<Vec<_>>(),
        [GithubCall::PullRequest {
            full_name: "parity-bot/pr-details".into(),
            number: 5
        }]
    );
    assert_eq!(shown_pr(&app), Some(5));
    assert!(!out.contains("ERROR"), "{out}");
}

#[test]
fn selecting_logs_and_nothing_changes_until_the_answer() {
    let mut app = app();
    let before = app.screen(160, 48);
    let out = logs(|| {
        app.press(&["enter"]);
    });
    assert!(out.contains("INFO Selected PR: #5"), "{out}");
    assert_eq!(app.screen(160, 48), before);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::PullRequests));
    assert_eq!(title(&app), "[5] parity-bot/pr-details Details");
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn the_answer_shows_the_three_tabs_and_focuses_the_strip() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    assert_eq!(title(&app), "[5] PR #5 Details");
    assert_eq!(
        tabs(&app),
        (vec!["Overview".into(), "Diff".into(), "Conversation".into()], 0)
    );
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    let screen = app.screen(160, 48);
    assert!(screen.contains("┌─ [5] PR #5 Details ─"), "{screen}");
    assert!(screen.contains("│ Overview  Diff  Conversation "), "{screen}");
    assert!(
        screen.contains("│Open Add a cursor to the viewer (#5) by parity-friend "),
        "{screen}"
    );
}

#[test]
fn the_title_key_follows_the_overrides() {
    let mut config = config();
    config
        .bindings
        .overrides
        .insert("main.details.focus".into(), "9".into());
    let mut app = TestApp::with_github(config, gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    assert_eq!(title(&app), "[9] PR #5 Details");
}

#[test]
fn a_failure_toasts_and_changes_nothing_else() {
    let mut app = app();
    app.press(&["j", "enter"]);
    settle(&mut app);
    assert_eq!(shown_pr(&app), Some(4));
    app.press(&["2", "j", "j", "enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR API error loading PR!"), "{out}");
    assert!(
        app.app()
            .toasts()
            .iter()
            .eq([&Toast::error("Error fetching pull request!").with_title("API Error")])
    );
    assert_eq!(shown_pr(&app), Some(4));
    assert_eq!(title(&app), "[5] PR #4 Details");
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::PullRequests));
}

#[test]
fn a_missing_pull_request_fails_the_same_way() {
    let mut app = TestApp::with_github(
        config(),
        gh().with_pull_request("parity-bot/pr-details", 5, Err(GhError::NotInstalled)),
    );
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR API error loading PR!"), "{out}");
    assert_eq!(app.app().toasts().len(), 1);
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn a_second_selection_cancels_the_first_and_drops_its_late_answer() {
    let mut app = app();
    app.press(&["enter"]);
    let (first, _) = pr_requests(&app)[0];
    app.press(&["j", "enter"]);
    assert!(cancels(&app).contains(&first));
    // The first answer arrives anyway, as one already sent can.
    let out = logs(|| {
        app.send(Event::Github {
            id: first,
            response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
        });
    });
    assert!(out.contains("DEBUG Dropping the pull request response"), "{out}");
    assert_eq!(shown_pr(&app), None);
    settle(&mut app);
    assert_eq!(shown_pr(&app), Some(4));
    assert_eq!(title(&app), "[5] PR #4 Details");
}

#[test]
fn selecting_a_repo_drops_a_pending_pull_request() {
    let mut app = app();
    app.press(&["enter"]);
    let (pending, _) = pr_requests(&app)[0];
    app.press(&["1", "enter"]);
    assert!(cancels(&app).contains(&pending));
    app.send(Event::Github {
        id: pending,
        response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
    });
    settle(&mut app);
    assert_eq!(shown_pr(&app), None);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(title(&app), "[5] parity-bot/pr-details Details");
}

#[test]
fn selecting_a_repo_replaces_the_pull_request_tabs() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["1", "enter"]);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn selecting_the_same_pull_request_again_fetches_and_rebuilds() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["right", "right"]);
    assert_eq!(tabs(&app).1, 2);
    app.press(&["2", "enter"]);
    settle(&mut app);
    assert_eq!(pr_requests(&app).len(), 2);
    assert_eq!(tabs(&app).1, 0);
    assert_eq!(shown_pr(&app), Some(5));
}

#[test]
fn right_and_left_switch_tabs_and_wrap() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    let active = |app: &TestApp| tabs(app).1;
    app.press(&["right"]);
    assert_eq!(active(&app), 1);
    app.press(&["right"]);
    assert_eq!(active(&app), 2);
    app.press(&["right"]);
    assert_eq!(active(&app), 0);
    app.press(&["left"]);
    assert_eq!(active(&app), 2);
    app.press(&["left", "left"]);
    assert_eq!(active(&app), 0);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn with_one_tab_the_keys_do_nothing() {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["5", "right"]);
    assert_eq!(tabs(&app), (vec!["Introduction".into()], 0));
    app.press(&["1", "enter", "5", "left", "right"]);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
}

#[test]
fn a_click_on_a_tab_activates_it_and_focuses_the_strip() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    let screen = app.screen(160, 48);
    let row = row_of(&mut app, "Overview  Diff  Conversation");
    let line = screen.lines().nth(usize::from(row)).unwrap();
    let column = |name: &str| u16::try_from(line[..line.find(name).unwrap()].chars().count()).unwrap();
    app.click(column("Conversation") + 3, row);
    assert_eq!(tabs(&app).1, 2);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.click(column("Diff"), row);
    assert_eq!(tabs(&app).1, 1);
    // The underline row only focuses.
    app.click(column("Overview"), row + 1);
    assert_eq!(tabs(&app).1, 1);
}

#[test]
fn each_tab_keeps_its_scroll_position() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.render(100, 20);
    app.press(&["tab", "down", "down"]);
    let offset = |app: &TestApp| main(app).details.pull_request().unwrap().overview().offset();
    assert_eq!(offset(&app), 2);
    app.press(&["5", "right", "left"]);
    app.render(100, 20);
    assert_eq!(offset(&app), 2);
}

#[test]
fn tab_from_the_strip_focuses_the_body_and_its_footer() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    let footer = |app: &mut TestApp| app.screen(160, 48).lines().last().unwrap().to_owned();
    assert!(!footer(&mut app).contains("Merge PR"));
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    let footer = footer(&mut app);
    assert!(
        footer.starts_with(" ^m Merge PR  E Edit PR  R Add reaction  ^v Paste link"),
        "{footer}"
    );
    // Not built yet: consumed and logged.
    for key in ["E", "R"] {
        let out = logs(|| {
            app.press(&[key]);
        });
        assert!(out.contains("is not implemented yet"), "{key}: {out}");
    }
    // Merge, edit and react belong to the Overview: the other tabs' bodies don't have them.
    app.press(&["5", "right", "tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    let screen = app.screen(160, 48);
    assert!(!screen.lines().last().unwrap().contains("Merge PR"), "{screen}");
}

#[test]
fn the_pull_request_bindings_answer_unimplemented() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["tab"]);
    let main = app.app_mut().main_screen_mut().unwrap();
    for id in [
        BindingId::PullRequestMerge,
        BindingId::PullRequestEdit,
        BindingId::ReactAdd,
    ] {
        assert!(matches!(
            main.details.on_binding(crate::component::Owner::PrOverview, id),
            Some(Action::Unimplemented(got)) if got == id
        ));
    }
}

#[test]
fn a_full_pull_request_shows_at_once() {
    let mut app = app();
    app.dispatch(Action::Main(crate::screens::main::MainAction::OpenPullRequest {
        target: super::PullRequestTarget::Full(Box::new(fixture_pr(9))),
        focus: false,
    }));
    assert!(pr_requests(&app).is_empty());
    assert_eq!(shown_pr(&app), Some(9));
    assert_eq!(title(&app), "[5] PR #9 Details");
    // Without `focus`, focus stays where it was.
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::PullRequests));
}

#[test]
fn pr_overview_open_160x48() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_overview_merged_160x48() {
    let mut app = app();
    app.press(&["j", "j", "enter"]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("Opened on Sat Jan 10 14:00:00 2026 • Merged on Mon Jan 12 16:20:00 2026"),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn pr_overview_closed_100x30() {
    let mut app = app();
    app.press(&["j", "enter"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn pr_details_tabs_switched() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["left"]);
    insta::assert_snapshot!(app.screen(160, 48));
}
