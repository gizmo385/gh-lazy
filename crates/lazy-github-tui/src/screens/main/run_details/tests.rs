//! Selecting a workflow run, its tabs, its jobs request and its Overview in the app, against the `run-details`
//! fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{Repository, Workflow, WorkflowJob, WorkflowRun};
use serde_json::Value;

use crate::component::Owner;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::{FocusTarget, MainScreen, SectionId};
use crate::test_util::{TestApp, logs};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/run-details")
}

/// The response body of the fixture at `path` in the `run-details` set.
fn fixture_body(path: &str) -> Value {
    let path = fixtures().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

const ACTIONS: &str = "repos/parity-bot/run-fixture/actions";

/// `parity-bot/run-fixture`, first on the set's repos page.
fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// The repo's runs: 42 (failed), 41 (in progress) and 40 (no jobs).
fn fixture_runs() -> Vec<WorkflowRun> {
    let body = fixture_body(&format!("{ACTIONS}/runs/GET.page-1_per_page-30.json"));
    serde_json::from_value(body["workflow_runs"].clone()).unwrap()
}

/// Run number `number` of the repo.
pub(in crate::screens::main) fn fixture_run(number: u64) -> WorkflowRun {
    fixture_runs().into_iter().find(|run| run.run_number == number).unwrap()
}

/// The jobs of the run with id `run_id`.
pub(in crate::screens::main) fn fixture_jobs(run_id: u64) -> Vec<WorkflowJob> {
    let body = fixture_body(&format!("{ACTIONS}/runs/{run_id}/jobs/GET.json"));
    serde_json::from_value(body["jobs"].clone()).unwrap()
}

fn gh() -> FakeGithub {
    let name = "parity-bot/run-fixture";
    let workflows: Vec<Workflow> = serde_json::from_value(
        fixture_body(&format!("{ACTIONS}/workflows/GET.page-1_per_page-30.json"))["workflows"].clone(),
    )
    .unwrap();
    let mut gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_readme(name, Ok(Some("Workflow runs with jobs and logs.\n".into())))
        .with_workflow_page(name, 1, Ok(workflows))
        .with_run_page(name, 1, Ok(fixture_runs()));
    for run in fixture_runs() {
        gh = gh.with_run_jobs(run.id, Ok(fixture_jobs(run.id)));
    }
    gh
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The app with `parity-bot/run-fixture` selected and its runs listed, the Workflows table focused on run 42.
fn app_with(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["4"]);
    app
}

fn app() -> TestApp {
    app_with(gh())
}

/// [`app`] with run `number` selected and its jobs in.
fn selected(number: u64) -> TestApp {
    let mut app = app();
    for _ in 0..(42 - number) {
        app.press(&["j"]);
    }
    app.press(&["enter"]);
    settle(&mut app);
    app
}

/// The jobs requests among the app's effects, by run id.
fn jobs_requests(app: &TestApp) -> Vec<(RequestId, u64)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::WorkflowRunJobs { run_id, .. },
            } => Some((*id, *run_id)),
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

fn main(app: &TestApp) -> &MainScreen {
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

fn shown_run(app: &TestApp) -> Option<u64> {
    main(app).details_run().map(|run| run.run_number)
}

fn details(app: &TestApp) -> &super::RunDetails {
    main(app).details.run().unwrap()
}

fn shown_buttons(app: &TestApp) -> Vec<&str> {
    details(app).overview().shown_buttons()
}

fn focused_button(app: &TestApp) -> Option<&str> {
    details(app).overview().focused_button()
}

/// The row of the screen whose text contains `needle`, and the column it starts at.
fn find(app: &mut TestApp, needle: &str) -> (u16, u16) {
    let screen = app.screen(160, 48);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle} not on screen:\n{screen}"));
    let column = line[..line.find(needle).unwrap()].chars().count();
    (u16::try_from(row).unwrap(), u16::try_from(column).unwrap())
}

#[test]
fn enter_space_and_a_click_each_select_the_run_and_request_its_jobs_once() {
    let mut app = app();
    app.press(&["enter"]);
    assert_eq!(
        jobs_requests(&app).iter().map(|&(_, id)| id).collect::<Vec<_>>(),
        [5_000_042]
    );

    let mut app = self::app();
    app.press(&["j", "space"]);
    assert_eq!(
        jobs_requests(&app).iter().map(|&(_, id)| id).collect::<Vec<_>>(),
        [5_000_041]
    );
    assert_eq!(shown_run(&app), Some(41));

    // A click on another cell of the row focuses the table and moves the cursor there; a second click on it selects.
    let mut app = self::app();
    app.press(&["1"]);
    let (row, column) = find(&mut app, "failure      CI");
    app.click(column, row);
    assert!(jobs_requests(&app).is_empty());
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Workflows));
    app.render(160, 48);
    app.click(column, row);
    assert_eq!(
        jobs_requests(&app).iter().map(|&(_, id)| id).collect::<Vec<_>>(),
        [5_000_042]
    );
    let out = settle(&mut app);
    assert_eq!(
        app.gh()
            .calls()
            .into_iter()
            .filter(|call| matches!(call, GithubCall::WorkflowRunJobs(_)))
            .collect::<Vec<_>>(),
        [GithubCall::WorkflowRunJobs(5_000_042)]
    );
    assert!(!out.contains("ERROR"), "{out}");
}

#[test]
fn selecting_logs_shows_the_tabs_at_once_and_focuses_the_strip() {
    let mut app = app();
    let out = logs(|| {
        app.press(&["enter"]);
    });
    assert!(out.contains("INFO Selected workflow run #42"), "{out}");
    assert_eq!(title(&app), "[5] Workflow Run #42 Details");
    assert_eq!(tabs(&app), (vec!["Overview".into(), "Jobs".into(), "Logs".into()], 0));
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    assert_eq!(shown_run(&app), Some(42));
    assert!(details(&app).jobs().is_none());
    let screen = app.screen(160, 48);
    assert!(screen.contains("┌─ [5] Workflow Run #42 Details ─"), "{screen}");
    assert!(screen.contains("│ Overview  Jobs  Logs "), "{screen}");
    assert!(screen.contains("│Loading jobs..."), "{screen}");

    settle(&mut app);
    let names: Vec<&str> = details(&app)
        .jobs()
        .unwrap()
        .iter()
        .map(|job| job.name.as_str())
        .collect();
    assert_eq!(names, ["build", "test", "lint", "deploy"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│○ deploy (9s)"), "{screen}");
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
    app.press(&["4", "enter"]);
    assert_eq!(title(&app), "[9] Workflow Run #42 Details");
}

#[test]
fn a_second_selection_cancels_the_jobs_request_and_drops_its_late_answer() {
    let mut app = app();
    app.press(&["enter"]);
    let (first, _) = jobs_requests(&app)[0];
    app.press(&["4", "j", "enter"]);
    assert!(cancels(&app).contains(&first));
    let (second, run_id) = jobs_requests(&app)[1];
    assert_eq!(run_id, 5_000_041);
    // The first answer arrives anyway, as one already sent can.
    let out = logs(|| {
        app.send(Event::Github {
            id: first,
            response: GithubResponse::WorkflowRunJobs(Ok(fixture_jobs(5_000_042))),
        });
    });
    assert!(out.contains("DEBUG Dropping the jobs response"), "{out}");
    assert!(details(&app).jobs().is_none());
    app.send(Event::Github {
        id: second,
        response: GithubResponse::WorkflowRunJobs(Ok(fixture_jobs(5_000_041))),
    });
    assert_eq!(details(&app).jobs().unwrap().len(), 2);
    assert_eq!(shown_run(&app), Some(41));
}

#[test]
fn selecting_a_repo_cancels_the_jobs_request_and_replaces_the_tabs() {
    let mut app = app();
    app.press(&["enter"]);
    let (pending, _) = jobs_requests(&app)[0];
    app.press(&["1", "enter"]);
    assert!(cancels(&app).contains(&pending));
    app.send(Event::Github {
        id: pending,
        response: GithubResponse::WorkflowRunJobs(Ok(fixture_jobs(5_000_042))),
    });
    settle(&mut app);
    assert_eq!(shown_run(&app), None);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(title(&app), "[5] parity-bot/run-fixture Details");
}

#[test]
fn a_finished_request_is_not_cancelled() {
    let mut app = selected(42);
    let before = cancels(&app).len();
    app.press(&["1", "enter"]);
    let (first, _) = jobs_requests(&app)[0];
    assert_eq!(cancels(&app)[before..].iter().filter(|&&id| id == first).count(), 0);
}

#[test]
fn selecting_the_same_run_again_rebuilds_and_asks_again() {
    let mut app = selected(42);
    app.press(&["right", "right"]);
    assert_eq!(tabs(&app).1, 2);
    app.press(&["4", "enter"]);
    assert_eq!(tabs(&app).1, 0);
    assert!(details(&app).jobs().is_none());
    settle(&mut app);
    assert_eq!(jobs_requests(&app).len(), 2);
    assert_eq!(details(&app).jobs().unwrap().len(), 4);
}

#[test]
fn a_failed_request_logs_an_error_and_shows_no_jobs() {
    let mut app = app_with(gh().with_run_jobs(
        5_000_042,
        Err(GhError::Http {
            status: 500,
            message: "Server Error".into(),
            body: Vec::new(),
        }),
    ));
    app.press(&["enter"]);
    let out = settle(&mut app);
    assert!(
        out.contains("ERROR Error retrieving jobs for workflow run 5000042 from the Github API"),
        "{out}"
    );
    assert!(app.app().toasts().is_empty());
    assert!(details(&app).jobs().unwrap().is_empty());
    let screen = app.screen(160, 48);
    assert!(screen.contains("│No jobs found"), "{screen}");
    assert_eq!(shown_buttons(&app), ["Re-run Workflow"]);
}

#[test]
fn re_run_failed_jobs_shows_only_after_the_jobs_and_with_a_failed_job() {
    let mut app = app();
    app.press(&["enter"]);
    assert_eq!(shown_buttons(&app), ["Re-run Workflow"]);
    settle(&mut app);
    assert_eq!(shown_buttons(&app), ["Re-run Workflow", "Re-run Failed Jobs"]);
    // In progress, or without jobs: no failed job.
    assert_eq!(shown_buttons(&selected(41)), ["Re-run Workflow"]);
    assert_eq!(shown_buttons(&selected(40)), ["Re-run Workflow"]);
    // A failure that hasn't completed doesn't count.
    let mut jobs = fixture_jobs(5_000_042);
    jobs[1].status = lazy_github_core::models::JobStatus::InProgress;
    let mut app = app_with(gh().with_run_jobs(5_000_042, Ok(jobs)));
    app.press(&["enter"]);
    settle(&mut app);
    assert_eq!(shown_buttons(&app), ["Re-run Workflow"]);
}

#[test]
fn tab_walks_the_strip_the_body_and_the_buttons() {
    let mut app = selected(42);
    app.render(160, 48);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.press(&["tab"]);
    assert_eq!((focus(&app), focused_button(&app)), (FocusTarget::DetailsContent, None));
    app.press(&["tab"]);
    assert_eq!(
        (focus(&app), focused_button(&app)),
        (FocusTarget::DetailsContent, Some("Re-run Workflow"))
    );
    app.press(&["tab"]);
    assert_eq!(
        (focus(&app), focused_button(&app)),
        (FocusTarget::DetailsContent, Some("Re-run Failed Jobs"))
    );
    // Then on round the main ring: the command log is hidden, so the repos table.
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Repos));
    // Back: the last button first.
    app.press(&["shift+tab"]);
    assert_eq!(focused_button(&app), Some("Re-run Failed Jobs"));
    app.press(&["shift+tab", "shift+tab"]);
    assert_eq!((focus(&app), focused_button(&app)), (FocusTarget::DetailsContent, None));
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);

    // With one button, it's the last stop.
    let mut app = selected(40);
    app.press(&["tab", "tab"]);
    assert_eq!(focused_button(&app), Some("Re-run Workflow"));
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Repos));
}

#[test]
fn focusing_a_button_scrolls_it_into_view() {
    let mut app = selected(42);
    app.render(100, 30);
    app.press(&["tab", "tab"]);
    let offset = details(&app).overview().offset();
    assert!(offset > 0, "{offset}");
    let screen = app.screen(100, 30);
    assert!(screen.contains("Re-run Workflow"), "{screen}");
}

#[test]
fn enter_on_a_button_does_nothing_yet() {
    let mut app = selected(42);
    app.press(&["tab", "tab"]);
    let before = app.screen(160, 48);
    let out = logs(|| {
        app.press(&["enter"]);
    });
    assert!(out.contains("TextualButtonPress is not implemented yet"), "{out}");
    assert_eq!(app.screen(160, 48), before);
    assert_eq!(jobs_requests(&app).len(), 1);
    let main = app.app_mut().main_screen_mut().unwrap();
    assert!(matches!(
        main.details
            .on_binding(Owner::DetailsButton, BindingId::TextualButtonPress),
        Some(crate::action::Action::Unimplemented(BindingId::TextualButtonPress))
    ));
}

#[test]
fn the_footer_has_only_the_screen_and_app_items() {
    let mut app = selected(42);
    let expected = " ^v Paste link  f Follow link  F Follow link (browser)  q Quit  ^p Command Palette  M Maximize  ? \
                    Keybindings";
    for keys in [&[][..], &["tab"], &["tab"]] {
        app.press(keys);
        let screen = app.screen(160, 48);
        let footer = screen.lines().last().unwrap().trim_end().to_owned();
        assert_eq!(footer, expected, "{keys:?}");
    }
}

#[test]
fn maximize_works_from_the_strip_and_a_button_but_not_the_body() {
    let mut app = selected(42);
    app.press(&["tab", "M"]);
    assert!(app.app().maximized().is_none());
    app.press(&["tab", "M"]);
    let maximized = app.app().maximized().unwrap();
    assert_eq!(maximized.target, FocusTarget::DetailsContent);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Overview  Jobs  Logs"), "{screen}");
    assert!(!screen.contains("[1] Repositories"), "{screen}");

    let mut app = selected(42);
    app.press(&["M"]);
    assert_eq!(app.app().maximized().unwrap().target, FocusTarget::DetailsTabs);
}

#[test]
fn a_click_on_a_button_focuses_it_and_one_on_the_body_focuses_the_body() {
    let mut app = selected(42);
    let (row, column) = find(&mut app, "Re-run Failed Jobs");
    app.click(column, row);
    assert_eq!(
        (focus(&app), focused_button(&app)),
        (FocusTarget::DetailsContent, Some("Re-run Failed Jobs"))
    );
    assert_eq!(jobs_requests(&app).len(), 1);
    let (row, column) = find(&mut app, "Commit Information");
    app.click(column, row);
    assert_eq!((focus(&app), focused_button(&app)), (FocusTarget::DetailsContent, None));
}

#[test]
fn the_wheel_and_the_scroll_keys_scroll_the_body() {
    let mut app = selected(42);
    app.render(100, 30);
    app.press(&["tab", "down", "down"]);
    assert_eq!(details(&app).overview().offset(), 2);
    app.press(&["5", "right", "left"]);
    app.render(100, 30);
    assert_eq!(details(&app).overview().offset(), 2);
    // A new selection starts at the top.
    app.press(&["4", "enter"]);
    assert_eq!(details(&app).overview().offset(), 0);
}

#[test]
fn run_overview_loading_160x48() {
    let mut app = app();
    app.press(&["enter"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_overview_160x48() {
    let mut app = selected(42);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_overview_100x30() {
    let mut app = selected(42);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn run_overview_in_progress_160x48() {
    let mut app = selected(41);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_overview_no_jobs_160x48() {
    let mut app = selected(40);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_overview_button_focused_160x48() {
    let mut app = selected(42);
    app.press(&["tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

/// The focused button drawn as Textual draws a focused one, the other as a blurred one: the Actions rows with their
/// styles.
#[test]
fn run_overview_button_focused_styles() {
    let mut app = selected(42);
    app.press(&["tab", "tab"]);
    let (row, column) = find(&mut app, "Actions");
    let buffer = app.render(160, 48);
    let area = ratatui::layout::Rect::new(column, row, 30, 9);
    let mut actions = ratatui::buffer::Buffer::empty(area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            actions[(x, y)] = buffer[(x, y)].clone();
        }
    }
    insta::assert_snapshot!(crate::test_util::buffer_to_styled_string(&actions));
}
