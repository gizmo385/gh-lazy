//! Selecting a workflow run, its tabs, its jobs request and its Overview in the app, against the `run-details`
//! fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{Repository, Workflow, WorkflowJob, WorkflowRun};
use serde_json::Value;

use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::screens::main::{FocusTarget, MainScreen, SectionId};
use crate::test_util::{TestApp, UNICODE_EXTRAS, logs, unicode_fixture};

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
        for job in fixture_jobs(run.id) {
            gh = gh.with_job_logs(job.id, fixture_logs(job.id));
        }
        gh = gh.with_run_jobs(run.id, Ok(fixture_jobs(run.id)));
    }
    gh
}

/// Job `job_id`'s logs fixture: the text, or a 404.
fn fixture_logs(job_id: u64) -> Result<Option<String>, GhError> {
    let path = fixtures().join(format!("{ACTIONS}/jobs/{job_id}/logs/GET.json"));
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    match fixture["response"]["body_text"].as_str() {
        Some(text) => Ok(Some(text.to_owned())),
        None => Err(GhError::Http {
            status: 404,
            message: "Not Found".into(),
            body: Vec::new(),
            headers: Vec::new(),
        }),
    }
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
            headers: Vec::new(),
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

// Re-run (#265).

const RUN_42: u64 = 5_000_042;

/// The re-runs sent, as `(run id, failed_only)`, and the run refetches, by run id.
fn reruns(app: &TestApp) -> (Vec<(u64, bool)>, Vec<u64>) {
    let mut reruns = Vec::new();
    let mut refetches = Vec::new();
    for call in app.gh().calls() {
        match call {
            GithubCall::RerunWorkflowRun(id) => reruns.push((id, false)),
            GithubCall::RerunFailedJobs(id) => reruns.push((id, true)),
            GithubCall::WorkflowRun(id) => refetches.push(id),
            _ => {}
        }
    }
    (reruns, refetches)
}

fn toasts(app: &TestApp) -> Vec<crate::toast::Toast> {
    app.app().toasts().iter().cloned().collect()
}

/// Run 42 selected on `gh`, with the button `tabs` presses after the strip focused (2: Re-run Workflow, 3: Re-run
/// Failed Jobs).
fn on_button(gh: FakeGithub, tabs: usize) -> TestApp {
    let mut app = app_with(gh);
    app.press(&["enter"]);
    settle(&mut app);
    for _ in 0..tabs {
        app.press(&["tab"]);
    }
    app
}

/// Run 42 as GitHub answers once it's re-run: titled differently, so the refetched run can be told apart.
fn rerun_42() -> WorkflowRun {
    let mut run = fixture_run(42);
    run.display_title = "Fix the flaky test (re-run)".into();
    run
}

#[test]
fn each_button_sends_its_request() {
    for (tabs, label, failed_only) in [(2, "Re-run Workflow", false), (3, "Re-run Failed Jobs", true)] {
        let mut app = on_button(gh(), tabs);
        assert_eq!(focused_button(&app), Some(label));
        app.press(&["enter"]);
        settle(&mut app);
        assert_eq!(reruns(&app).0, [(RUN_42, failed_only)], "{label}");
    }
}

#[test]
fn a_queued_rerun_toasts_and_shows_the_run_fetched_again() {
    for (presses, failed_only, message) in [
        (3, true, "Failed jobs queued for re-run"),
        (2, false, "Workflow run queued for re-run"),
    ] {
        let gh = gh()
            .with_rerun(RUN_42, failed_only, Ok(()))
            .with_workflow_run(RUN_42, Ok(rerun_42()));
        let mut app = on_button(gh, presses);
        app.press(&["enter"]);
        settle(&mut app);
        assert_eq!(reruns(&app), (vec![(RUN_42, failed_only)], vec![RUN_42]), "{message}");
        assert_eq!(
            toasts(&app),
            [crate::toast::Toast::info(message).with_title("Re-run Started")]
        );
        // Shown as a fresh selection: its jobs asked for again and the strip focused.
        assert_eq!(jobs_requests(&app).len(), 2);
        assert_eq!(focus(&app), FocusTarget::DetailsTabs);
        assert_eq!(tabs(&app), (vec!["Overview".into(), "Jobs".into(), "Logs".into()], 0));
        assert_eq!(details(&app).run().display_title, "Fix the flaky test (re-run)");
    }
}

#[test]
fn a_refused_rerun_toasts_and_changes_nothing() {
    for (tabs, message, line) in [
        (
            3,
            "Failed to re-run failed jobs",
            "ERROR Error re-running failed jobs for workflow run 5000042",
        ),
        (
            2,
            "Failed to re-run workflow",
            "ERROR Error re-running workflow run 5000042",
        ),
    ] {
        // Neither is canned: the fake refuses both with a 422.
        let mut app = on_button(gh(), tabs);
        app.press(&["enter"]);
        let out = settle(&mut app);
        assert!(out.contains(line), "{out}");
        assert_eq!(toasts(&app), [crate::toast::Toast::error(message).with_title("Error")]);
        assert!(reruns(&app).1.is_empty(), "no refetch");
        assert_eq!(jobs_requests(&app).len(), 1);
        assert_eq!(focus(&app), FocusTarget::DetailsContent);
    }
}

#[test]
fn a_failed_refetch_still_reselects_the_run() {
    let gh = gh().with_rerun(RUN_42, true, Ok(()));
    let mut app = on_button(gh, 3);
    app.press(&["enter"]);
    let out = settle(&mut app);
    assert!(
        out.contains("ERROR Error fetching workflow run 5000042 after a re-run"),
        "{out}"
    );
    assert_eq!(reruns(&app).1, [RUN_42]);
    assert_eq!(jobs_requests(&app).len(), 2);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    assert_eq!(details(&app).run(), &fixture_run(42));
}

#[test]
fn a_click_on_a_button_presses_it() {
    let mut app = on_button(gh().with_rerun(RUN_42, true, Ok(())), 0);
    let (row, column) = find(&mut app, "Re-run Failed Jobs");
    app.click(column, row);
    settle(&mut app);
    assert_eq!(reruns(&app).0, [(RUN_42, true)]);
}

#[test]
fn a_second_press_sends_a_second_request() {
    let mut app = on_button(gh(), 2);
    app.press(&["enter"]);
    // Past the button's debounce.
    app.advance(std::time::Duration::from_millis(300));
    app.press(&["enter"]);
    settle(&mut app);
    assert_eq!(reruns(&app).0, [(RUN_42, false), (RUN_42, false)]);
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

// The Jobs tab.

/// [`selected`] with the Jobs tab showing.
fn jobs_tab(number: u64) -> TestApp {
    let mut app = selected(number);
    app.press(&["right"]);
    app
}

/// The job logs requests among the app's effects, by job id.
fn log_requests(app: &TestApp) -> Vec<(RequestId, u64)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::JobLogs { job_id, .. },
            } => Some((*id, *job_id)),
            _ => None,
        })
        .collect()
}

fn log_job_ids(app: &TestApp) -> Vec<u64> {
    log_requests(app).into_iter().map(|(_, id)| id).collect()
}

fn log_calls(app: &TestApp) -> Vec<GithubCall> {
    app.gh()
        .calls()
        .into_iter()
        .filter(|call| matches!(call, GithubCall::JobLogs(_)))
        .collect()
}

fn tree_cursor(app: &TestApp) -> Option<usize> {
    details(app).jobs_tab().tree().cursor_line()
}

fn tree_focused(app: &TestApp) -> bool {
    details(app).jobs_tab().tree_focused()
}

fn footer(app: &mut TestApp) -> String {
    let screen = app.screen(160, 48);
    screen.lines().last().unwrap().trim_end().to_owned()
}

fn mouse(kind: crossterm::event::MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    })
}

#[test]
fn the_tab_shows_the_heading_the_collapsed_tree_and_the_hint() {
    let mut app = jobs_tab(42);
    let screen = app.screen(160, 48);
    for line in [
        "│Jobs for Run #42",
        "│▶ ✓ build (completed)",
        "│▶ ✗ test (completed)",
        "│▶ − lint (completed)",
        "│▶ ○ deploy (completed)",
        "│Job Logs",
        "│  Select a job and press 'l' to view logs.",
    ] {
        assert!(screen.contains(line), "{line}\n{screen}");
    }
    assert!(!screen.contains("Set up job"), "{screen}");
    assert!(log_requests(&app).is_empty());
}

#[test]
fn the_tree_is_empty_until_the_jobs_arrive() {
    let mut app = app();
    app.press(&["enter", "5", "right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Jobs for Run #42"), "{screen}");
    assert!(!screen.contains("build (completed)"), "{screen}");
    assert!(!screen.contains("No jobs found"), "{screen}");
    settle(&mut app);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│▶ ✓ build (completed)"), "{screen}");
}

#[test]
fn the_hint_names_the_overridden_key() {
    let mut config = Config::default();
    config
        .bindings
        .overrides
        .insert("workflows.view_job_logs".into(), "v".into());
    let mut app = TestApp::with_github(config, gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["4", "enter"]);
    settle(&mut app);
    app.press(&["right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Select a job and press 'v' to view logs."), "{screen}");
    // The key does what `l` did.
    app.press(&["tab", "tab", "down", "v"]);
    assert_eq!(log_job_ids(&app), [6_000_001]);
}

#[test]
fn tab_walks_the_strip_the_body_and_the_tree() {
    let mut app = jobs_tab(42);
    app.render(160, 48);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.press(&["tab"]);
    assert_eq!((focus(&app), tree_focused(&app)), (FocusTarget::DetailsContent, false));
    app.press(&["tab"]);
    assert_eq!((focus(&app), tree_focused(&app)), (FocusTarget::DetailsContent, true));
    // Focusing the tree doesn't move its cursor.
    assert_eq!(tree_cursor(&app), None);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Repos));
    app.press(&["shift+tab"]);
    assert_eq!((focus(&app), tree_focused(&app)), (FocusTarget::DetailsContent, true));
    app.press(&["shift+tab"]);
    assert_eq!((focus(&app), tree_focused(&app)), (FocusTarget::DetailsContent, false));
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn the_tree_keys_move_the_cursor_and_expand() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down"]);
    assert_eq!(tree_cursor(&app), Some(0));
    app.press(&["enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│▼ ✓ build (completed)"), "{screen}");
    assert!(screen.contains("│├── ✓ Set up job (completed)"), "{screen}");
    assert!(screen.contains("│└── ✓ Complete job (completed)"), "{screen}");
    app.press(&["down"]);
    assert_eq!(tree_cursor(&app), Some(1));
    // `end` moves the tree's cursor rather than scrolling the body.
    app.press(&["end"]);
    assert_eq!(tree_cursor(&app), Some(6));
    assert_eq!(details(&app).jobs_tab().offset(), 0);
}

#[test]
fn l_on_a_job_shows_its_logs() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "l"]);
    assert_eq!(log_job_ids(&app), [6_000_001]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Logs for Job: build"), "{screen}");
    assert!(screen.contains("│  Loading logs..."), "{screen}");
    assert!(!screen.contains("Select a job and press"), "{screen}");
    let out = settle(&mut app);
    assert!(!out.contains("ERROR"), "{out}");
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("│    2026-03-04T05:06:10.0000000Z ##[group]Run cargo build"),
        "{screen}"
    );
    assert!(
        screen.contains("│    2026-03-04T05:06:14.0000000Z     Finished dev profile in 61.2s"),
        "{screen}"
    );
    assert!(!screen.contains("Loading logs..."), "{screen}");
    assert_eq!(
        details(&app).job_logs().get(&6_000_001).cloned().flatten(),
        fixture_logs(6_000_001).unwrap()
    );
}

#[test]
fn l_on_a_step_shows_its_jobs_logs() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "down", "space", "down", "down", "l"]);
    assert_eq!(log_job_ids(&app), [6_000_002]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Logs for Job: test"), "{screen}");
    assert!(
        screen.contains("##[error]Process completed with exit code 101."),
        "{screen}"
    );
}

#[test]
fn l_without_a_cursor_or_on_no_jobs_found_does_nothing() {
    // With the body focused and with the tree just focused, no line has the cursor.
    let mut app = jobs_tab(42);
    app.press(&["tab", "l", "tab", "l"]);
    assert!(log_requests(&app).is_empty());
    assert!(app.screen(160, 48).contains("Select a job and press"));

    let mut app = jobs_tab(40);
    app.press(&["tab", "tab", "down"]);
    assert_eq!(tree_cursor(&app), Some(0));
    app.press(&["l"]);
    assert!(log_requests(&app).is_empty());
    let screen = app.screen(160, 48);
    assert!(screen.contains("│No jobs found"), "{screen}");
    assert!(screen.contains("Select a job and press"), "{screen}");
}

#[test]
fn l_from_the_body_uses_the_trees_cursor() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "down", "shift+tab", "l"]);
    assert_eq!(log_job_ids(&app), [6_000_002]);
}

#[test]
fn a_failure_logs_an_error_once_and_shows_the_fallback() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "down", "down", "l"]);
    let out = settle(&mut app);
    assert_eq!(
        out.matches("ERROR Error retrieving logs for job 6000003").count(),
        1,
        "{out}"
    );
    assert!(out.contains("DEBUG Error retrieving logs for job 6000003: "), "{out}");
    assert!(app.app().toasts().is_empty());
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Logs for Job: lint"), "{screen}");
    assert!(
        screen.contains("│  No logs available or failed to fetch logs."),
        "{screen}"
    );
    assert_eq!(details(&app).job_logs().get(&6_000_003), Some(&None));
}

#[test]
fn an_empty_body_is_no_logs() {
    let mut app = app_with(gh().with_job_logs(6_000_001, Ok(None)));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["right", "tab", "tab", "down", "l"]);
    let out = settle(&mut app);
    assert_eq!(
        out.matches("ERROR Error retrieving logs for job 6000001").count(),
        1,
        "{out}"
    );
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("│  No logs available or failed to fetch logs."),
        "{screen}"
    );
}

#[test]
fn logs_already_fetched_show_without_a_request() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "l"]);
    settle(&mut app);
    app.press(&["down", "l"]);
    settle(&mut app);
    assert!(app.screen(160, 48).contains("│Logs for Job: test"));
    app.press(&["up", "l"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Logs for Job: build"), "{screen}");
    assert!(screen.contains("Finished dev profile in 61.2s"), "{screen}");
    assert!(!screen.contains("Loading logs..."), "{screen}");
    assert_eq!(
        log_calls(&app),
        [GithubCall::JobLogs(6_000_001), GithubCall::JobLogs(6_000_002)]
    );
    // A job whose logs failed is asked for again.
    app.press(&["down", "down", "l"]);
    settle(&mut app);
    app.press(&["l"]);
    settle(&mut app);
    assert_eq!(
        log_calls(&app)[2..],
        [GithubCall::JobLogs(6_000_003), GithubCall::JobLogs(6_000_003)]
    );
}

#[test]
fn a_late_answer_to_an_earlier_l_is_dropped() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "l", "down", "l"]);
    let requests = log_requests(&app);
    let [(first, 6_000_001), (second, 6_000_002)] = requests[..] else {
        panic!("{requests:?}");
    };
    assert!(cancels(&app).contains(&first));
    let out = logs(|| {
        app.send(Event::Github {
            id: first,
            response: GithubResponse::JobLogs(Ok(Some("stale\n".into()))),
        });
    });
    assert!(out.contains("DEBUG Dropping the job logs response"), "{out}");
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Logs for Job: test"), "{screen}");
    assert!(screen.contains("Loading logs..."), "{screen}");
    assert!(!screen.contains("stale"), "{screen}");
    app.send(Event::Github {
        id: second,
        response: GithubResponse::JobLogs(Ok(Some("fresh\n".into()))),
    });
    let screen = app.screen(160, 48);
    assert!(screen.contains("│    fresh"), "{screen}");
}

#[test]
fn another_selection_cancels_the_logs_request() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "l"]);
    let (pending, _) = log_requests(&app)[0];
    app.press(&["4", "j", "enter"]);
    assert!(cancels(&app).contains(&pending));
    settle(&mut app);
    assert!(details(&app).job_logs().is_empty());
}

#[test]
fn the_footer_shows_l_with_the_body_or_the_tree_focused() {
    let mut app = jobs_tab(42);
    let strip = footer(&mut app);
    assert!(!strip.contains("View Job Logs"), "{strip}");
    app.press(&["tab"]);
    let body = footer(&mut app);
    assert!(body.starts_with(" l View Job Logs  ^v Paste link"), "{body}");
    app.press(&["tab"]);
    let tree = footer(&mut app);
    assert!(tree.starts_with(" l View Job Logs  ^v Paste link"), "{tree}");
    // Not on the Overview.
    app.press(&["shift+tab", "shift+tab", "left", "tab"]);
    let overview = footer(&mut app);
    assert!(!overview.contains("View Job Logs"), "{overview}");
}

#[test]
fn maximize_works_from_the_tree_but_not_the_body() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "M"]);
    assert!(app.app().maximized().is_none());
    app.press(&["tab", "M"]);
    assert_eq!(app.app().maximized().unwrap().target, FocusTarget::DetailsContent);
    let screen = app.screen(160, 48);
    assert!(screen.contains("▶ ✓ build (completed)"), "{screen}");
    assert!(!screen.contains("[1] Repositories"), "{screen}");
}

#[test]
fn a_click_on_the_tree_focuses_it_and_toggles_the_job() {
    let mut app = jobs_tab(42);
    let (row, column) = find(&mut app, "test (completed)");
    app.click(column, row);
    assert_eq!((focus(&app), tree_focused(&app)), (FocusTarget::DetailsContent, true));
    assert_eq!(tree_cursor(&app), Some(1));
    let screen = app.screen(160, 48);
    assert!(screen.contains("│▼ ✗ test (completed)"), "{screen}");
    let (row, column) = find(&mut app, "Job Logs");
    app.click(column, row);
    assert_eq!((focus(&app), tree_focused(&app)), (FocusTarget::DetailsContent, false));
}

#[test]
fn the_body_scrolls_when_the_logs_dont_fit() {
    let mut app = jobs_tab(42);
    app.render(100, 30);
    app.press(&["tab", "tab", "down", "l"]);
    settle(&mut app);
    app.render(100, 30);
    // The wheel off the tree scrolls the body.
    let screen = app.screen(100, 30);
    let row = screen
        .lines()
        .position(|line| line.contains("Jobs for Run #42"))
        .unwrap();
    let (row, column) = (u16::try_from(row).unwrap(), 60);
    app.send(mouse(crossterm::event::MouseEventKind::ScrollDown, column, row));
    assert_eq!(details(&app).jobs_tab().offset(), 2);
    app.press(&["shift+tab", "end"]);
    app.render(100, 30);
    assert!(details(&app).jobs_tab().offset() > 0);
    let screen = app.screen(100, 30);
    assert!(screen.contains("Finished dev prof"), "{screen}");
    assert!(!screen.contains("Jobs for Run #42"), "{screen}");
    // Focusing the tree scrolls the body so it shows from its first row.
    app.press(&["tab"]);
    app.render(100, 30);
    assert_eq!(details(&app).jobs_tab().offset(), 1);
}

#[test]
fn the_wheel_scrolls_the_tree_over_it() {
    // 40 jobs: more lines than the tree's rows.
    let build = fixture_jobs(5_000_042).remove(0);
    let jobs = (0..40)
        .map(|n| WorkflowJob {
            id: 7_000_000 + n,
            name: format!("job {n}"),
            ..build.clone()
        })
        .collect();
    let mut app = app_with(gh().with_run_jobs(5_000_042, Ok(jobs)));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["right"]);
    app.render(160, 48);
    let (row, column) = find(&mut app, "job 0 (completed)");
    app.send(mouse(crossterm::event::MouseEventKind::ScrollDown, column, row + 1));
    assert_eq!(details(&app).jobs_tab().tree().scroll_offset().0, 2);
    assert_eq!(details(&app).jobs_tab().offset(), 0);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│▶ ✓ job 2 (completed)"), "{screen}");
    assert!(!screen.contains("job 0 (completed)"), "{screen}");
    // Elsewhere it's the body's (which fits here): the tree stays.
    let (row, column) = find(&mut app, "Select a job and press");
    app.send(mouse(crossterm::event::MouseEventKind::ScrollDown, column, row));
    assert_eq!(details(&app).jobs_tab().tree().scroll_offset().0, 2);
}

#[test]
fn run_jobs_160x48() {
    let mut app = jobs_tab(42);
    insta::assert_snapshot!(app.screen(160, 48));
}

/// The build job's log from the `unicode` fixture set (#327), with a line of what the parity scenario leaves out: a
/// ZWJ sequence and control characters, ANSI colors among them. Python doesn't color by ANSI escapes in a log either
/// (it shows the log in a markdown fence), so they're drawn as text with U+FFFD for the ESC.
#[test]
fn run_jobs_logs_unicode_160x48() {
    let log = unicode_fixture("repos/parity-bot/run-fixture/actions/jobs/6000001/logs/GET.json");
    let log = format!(
        "{}2026-03-04T05:06:15.0000000Z \x1b[1;32mok\x1b[0m{UNICODE_EXTRAS}\n",
        log.as_str().unwrap()
    );
    let first_job = fixture_jobs(fixture_run(42).id)[0].id;
    let mut app = app_with(gh().with_job_logs(first_job, Ok(Some(log))));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["right", "tab", "tab", "down", "enter", "l"]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    assert!(screen.contains("�[1;32mok�[0m"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn run_jobs_logs_160x48() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "enter", "l"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_jobs_no_logs_160x48() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "down", "down", "l"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_jobs_in_progress_160x48() {
    let mut app = jobs_tab(41);
    // Both jobs expanded, the queued one's steps included.
    app.press(&["tab", "tab", "down", "shift+space"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_jobs_no_jobs_160x48() {
    let mut app = jobs_tab(40);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_jobs_logs_100x30() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "enter", "l"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(100, 30));
}

// The Logs tab.

/// [`selected`] with the Logs tab showing, its requests not yet answered.
fn logs_tab(number: u64) -> TestApp {
    let mut app = selected(number);
    app.press(&["right", "right"]);
    app
}

fn logs_state(app: &TestApp) -> super::logs::LogsState {
    details(app).logs_tab().state()
}

#[test]
fn nothing_is_requested_until_the_tab_shows() {
    let mut app = selected(42);
    assert_eq!(logs_state(&app), super::logs::LogsState::NotStarted);
    app.press(&["right"]);
    settle(&mut app);
    assert!(log_requests(&app).is_empty());
    app.press(&["right"]);
    assert_eq!(log_job_ids(&app), [6_000_001]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Full Logs for Run #42"), "{screen}");
    assert!(screen.contains("Loading full workflow logs..."), "{screen}");
}

#[test]
fn the_logs_are_requested_one_at_a_time_in_job_order() {
    let mut app = logs_tab(42);
    let requests = log_requests(&app);
    let [(first, 6_000_001)] = requests[..] else {
        panic!("{requests:?}");
    };
    app.send(Event::Github {
        id: first,
        response: GithubResponse::JobLogs(fixture_logs(6_000_001)),
    });
    assert_eq!(log_job_ids(&app), [6_000_001, 6_000_002]);
    assert!(app.screen(160, 48).contains("Loading full workflow logs..."));
    let out = settle(&mut app);
    assert_eq!(log_job_ids(&app), [6_000_001, 6_000_002, 6_000_003, 6_000_004]);
    // Lint's logs answer 404: Python's ERROR line, and the fallback in its section.
    assert_eq!(
        out.matches("ERROR Error retrieving logs for job 6000003").count(),
        1,
        "{out}"
    );
    assert!(app.app().toasts().is_empty());
    let screen = app.screen(160, 48);
    for line in [
        "│    Job: build (Status: completed, Conclusion: success)",
        "│    2026-03-04T05:06:14.0000000Z     Finished dev profile in 61.2s",
        "│    Job: lint (Status: completed, Conclusion: skipped)",
        "│    (No logs available for this job)",
    ] {
        assert!(screen.contains(line), "{line}\n{screen}");
    }
    assert!(!screen.contains("Loading full workflow logs..."), "{screen}");
    // Showing the tab again asks for nothing more.
    app.press(&["left", "right"]);
    assert_eq!(log_requests(&app).len(), 4);
}

#[test]
fn a_tab_shown_before_the_jobs_starts_when_they_arrive() {
    let mut app = app();
    app.press(&["enter", "5", "right", "right"]);
    assert!(log_requests(&app).is_empty());
    assert_eq!(logs_state(&app), super::logs::LogsState::WaitingForJobs);
    assert!(app.screen(160, 48).contains("Loading full workflow logs..."));
    settle(&mut app);
    assert_eq!(log_job_ids(&app), [6_000_001, 6_000_002, 6_000_003, 6_000_004]);
    assert_eq!(logs_state(&app), super::logs::LogsState::Done);
}

#[test]
fn logs_the_jobs_tab_fetched_are_reused_and_the_other_way_round() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "l"]);
    settle(&mut app);
    app.press(&["5", "right"]);
    settle(&mut app);
    assert_eq!(
        log_calls(&app),
        [6_000_001, 6_000_002, 6_000_003, 6_000_004].map(GithubCall::JobLogs)
    );
    assert!(app.screen(160, 48).contains("Finished dev profile in 61.2s"));
    // Back on the Jobs tab, `l` on test shows what the Logs tab fetched.
    app.press(&["left", "tab", "tab", "down", "l"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Logs for Job: test"), "{screen}");
    assert!(!screen.contains("Loading logs..."), "{screen}");
    assert_eq!(log_calls(&app).len(), 4);
}

#[test]
fn a_job_whose_logs_failed_on_the_jobs_tab_is_asked_again() {
    let mut app = jobs_tab(42);
    app.press(&["tab", "tab", "down", "down", "down", "l"]);
    settle(&mut app);
    app.press(&["5", "right"]);
    settle(&mut app);
    assert_eq!(
        log_calls(&app),
        [6_000_003, 6_000_001, 6_000_002, 6_000_003, 6_000_004].map(GithubCall::JobLogs)
    );
}

#[test]
fn no_jobs_shows_a_line_and_asks_for_nothing() {
    let mut app = logs_tab(40);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│Full Logs for Run #40"), "{screen}");
    assert!(screen.contains("No jobs found for this workflow run."), "{screen}");
    assert!(log_requests(&app).is_empty());

    let mut app = app_with(gh().with_run_jobs(
        5_000_042,
        Err(GhError::Http {
            status: 500,
            message: "Server Error".into(),
            body: Vec::new(),
            headers: Vec::new(),
        }),
    ));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["right", "right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("No jobs found for this workflow run."), "{screen}");
    assert!(log_requests(&app).is_empty());
}

#[test]
fn another_selection_cancels_the_logs_tabs_request_and_drops_its_answer() {
    let mut app = logs_tab(42);
    let (pending, _) = log_requests(&app)[0];
    app.press(&["4", "j", "enter"]);
    assert!(cancels(&app).contains(&pending));
    let out = logs(|| {
        app.send(Event::Github {
            id: pending,
            response: GithubResponse::JobLogs(Ok(Some("stale\n".into()))),
        });
    });
    assert!(out.contains("DEBUG Dropping the job logs response"), "{out}");
    assert!(details(&app).job_logs().is_empty());
    assert_eq!(log_requests(&app).len(), 1);
    assert_eq!(shown_run(&app), Some(41));
}

#[test]
fn tab_goes_to_the_body_with_no_inner_stops_and_m_does_nothing() {
    let mut app = logs_tab(42);
    settle(&mut app);
    app.render(160, 48);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["M"]);
    assert!(app.app().maximized().is_none());
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Repos));
}

#[test]
fn the_scroll_keys_and_the_wheel_scroll_the_body() {
    let mut app = logs_tab(42);
    settle(&mut app);
    app.render(100, 30);
    app.press(&["tab", "down", "down"]);
    assert_eq!(details(&app).logs_tab().offset(), 2);
    let screen = app.screen(100, 30);
    let row = screen.lines().position(|line| line.contains("Job: build")).unwrap();
    app.send(mouse(
        crossterm::event::MouseEventKind::ScrollDown,
        60,
        u16::try_from(row).unwrap(),
    ));
    assert_eq!(details(&app).logs_tab().offset(), 4);
    app.press(&["end"]);
    app.render(100, 30);
    let screen = app.screen(100, 30);
    assert!(screen.contains("##[error]The operatio"), "{screen}");
    assert!(!screen.contains("Full Logs for Run #42"), "{screen}");
}

#[test]
fn run_logs_loading_160x48() {
    let mut app = logs_tab(42);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_logs_160x48() {
    let mut app = logs_tab(42);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn run_logs_end_100x30() {
    let mut app = logs_tab(42);
    settle(&mut app);
    app.render(100, 30);
    app.press(&["tab", "end"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn run_logs_no_jobs_160x48() {
    let mut app = logs_tab(40);
    insta::assert_snapshot!(app.screen(160, 48));
}
