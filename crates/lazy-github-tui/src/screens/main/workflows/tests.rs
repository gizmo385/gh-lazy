use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;

use super::*;
use crate::action::Action;
use crate::components::table::{CellValue, Handled, TableEvent};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::repos::repo;
use crate::screens::main::{MainAction, SectionId};
use crate::test_util::{TestApp, logs};

fn ci_repo() -> Repository {
    repo("octo", "ci", false)
}

fn other_repo() -> Repository {
    repo("octo", "other", false)
}

/// `count` CI runs with ids and run numbers `first..`, a minute apart and newest first.
fn runs(repo: &Repository, first: u64, count: u64) -> Vec<WorkflowRun> {
    (0..count)
        .map(|i| {
            let n = first + i;
            let minute = 59 - (i % 60);
            let hour = 23 - (i / 60);
            run(
                repo,
                n,
                "CI",
                n,
                &format!("2026-10-20T{hour:02}:{minute:02}:00Z"),
                Some("success"),
                &format!("Run {n}"),
            )
        })
        .collect()
}

/// An app whose repos table holds `octo/ci` and `octo/other`, with workflow pages from `gh`.
fn app(config: Config, gh: FakeGithub) -> TestApp {
    TestApp::with_github(config, gh.with_repositories(vec![ci_repo(), other_repo()]))
}

fn pane(app: &TestApp) -> &WorkflowsPane {
    &app.app().main_screen().expect("on the main screen").workflows
}

fn select(app: &mut TestApp, repo: Repository) {
    app.dispatch(Action::SelectRepo(Box::new(repo)));
}

/// Answers every queued request, and returns what that logged.
fn settle(app: &mut TestApp) -> String {
    logs(|| app.settle().now_or_never().expect("the fake answers at once"))
}

/// Every workflows-endpoint request so far: its id and what it asked for.
fn requests(app: &TestApp) -> Vec<(RequestId, GithubRequest)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { id, request } => match request {
                GithubRequest::ListWorkflows { .. } | GithubRequest::ListWorkflowRuns { .. } => {
                    Some((*id, request.clone()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The pages of the runs requests so far.
fn run_pages(app: &TestApp) -> Vec<u32> {
    requests(app)
        .into_iter()
        .filter_map(|(_, request)| match request {
            GithubRequest::ListWorkflowRuns { page, .. } => Some(page.page),
            _ => None,
        })
        .collect()
}

fn cancels(app: &TestApp) -> Vec<RequestId> {
    let ids: Vec<RequestId> = requests(app).iter().map(|(id, _)| *id).collect();
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) if ids.contains(id) => Some(*id),
            _ => None,
        })
        .collect()
}

fn run_ids(app: &TestApp) -> Vec<u64> {
    pane(app).runs_table().row_keys().copied().collect()
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: br#"{"message": "Server Error"}"#.to_vec(),
    }
}

fn decode_error() -> GhError {
    GhError::Decode(serde_json::from_str::<u8>("x").unwrap_err())
}

/// Moves the runs table's cursor with `binding`, as a key press with the table focused would.
fn cursor(app: &mut TestApp, binding: BindingId) {
    let screen = app.app_mut().main_screen_mut().expect("on the main screen");
    if let Handled::Event(TableEvent::LoadMore {
        generation,
        page,
        per_page,
    }) = screen.workflows.runs_table_mut().on_binding(binding)
    {
        app.dispatch(Action::Main(MainAction::LoadMore {
            section: SectionId::Workflows,
            generation,
            page,
            per_page,
        }));
    }
}

// Cells.

#[test]
fn cells_are_a_utc_minute_the_result_names_and_the_run_number() {
    let run = run(
        &ci_repo(),
        9,
        "Release",
        3,
        "2026-10-21T09:00:50-05:00",
        Some("failure"),
        "[docs] Release v1",
    );
    assert_eq!(
        run.cells(&()),
        [
            // 09:00 at -05:00 is 14:00 UTC (Py bug B26).
            Cell::text("2026-10-21 14:00"),
            Cell::text("failure"),
            Cell::text("Release"),
            // Drawn as typed (Py bug B46).
            Cell::text("[docs] Release v1"),
            Cell::int(3),
        ]
    );
    assert_eq!(run.cells(&())[4].value, CellValue::Int(3));
    assert_eq!(run.key(), 9);
}

#[test]
fn the_status_shows_while_there_is_no_conclusion() {
    let mut run = run(&ci_repo(), 1, "CI", 1, "2026-10-21T09:00:00Z", None, "Fix");
    assert_eq!(run.cells(&())[1], Cell::text("in_progress"));
    run.status = "queued".into();
    assert_eq!(run.cells(&())[1], Cell::text("queued"));
    // Python's `conclusion or status`: an empty conclusion is no conclusion.
    run.conclusion = Some(String::new());
    assert_eq!(run.cells(&())[1], Cell::text("queued"));
    run.conclusion = Some("cancelled".into());
    assert_eq!(run.cells(&())[1], Cell::text("cancelled"));
}

#[test]
fn runs_sort_by_minute_newest_first_and_keep_api_order_within_one() {
    let repo = ci_repo();
    let page = vec![
        run(&repo, 1, "CI", 11, "2026-10-21T09:00:05Z", None, "In flight"),
        run(
            &repo,
            2,
            "Release",
            3,
            "2026-10-21T09:00:50Z",
            Some("failure"),
            "Release v1",
        ),
        run(
            &repo,
            3,
            "CI",
            12,
            "2026-10-20T10:15:00Z",
            Some("success"),
            "Fix the thing",
        ),
        run(
            &repo,
            4,
            "CI",
            10,
            "2026-10-22T01:02:00Z",
            Some("cancelled"),
            "Cancelled one",
        ),
    ];
    let mut app = app(
        Config::default(),
        FakeGithub::new().with_run_page("octo/ci", 1, Ok(page)),
    );
    select(&mut app, repo);
    settle(&mut app);
    // 09:00:05 stays above 09:00:50: the sort is by the minute string, ties in API order.
    assert_eq!(run_ids(&app), [4, 1, 2, 3]);
}

#[test]
fn runs_of_different_workflows_with_one_run_number_are_both_rows() {
    // Python keys by `run_number`, so one replaces the other (Py bug B4).
    let repo = ci_repo();
    let page = vec![
        run(&repo, 70, "CI", 5, "2026-10-21T09:00:00Z", Some("success"), "CI five"),
        run(
            &repo,
            71,
            "Lint",
            5,
            "2026-10-21T08:00:00Z",
            Some("success"),
            "Lint five",
        ),
    ];
    let mut app = app(
        Config::default(),
        FakeGithub::new().with_run_page("octo/ci", 1, Ok(page)),
    );
    select(&mut app, repo);
    settle(&mut app);
    assert_eq!(run_ids(&app), [70, 71]);
}

// Loading.

#[test]
fn a_selection_requests_the_workflows_and_the_first_runs_page_together() {
    let mut app = app(Config::default(), FakeGithub::new());
    select(&mut app, ci_repo());
    assert!(pane(&app).runs_table().is_loading());
    let asked: Vec<GithubRequest> = requests(&app).into_iter().map(|(_, request)| request).collect();
    let first = PageQuery { page: 1, per_page: 30 };
    assert_eq!(
        asked,
        [
            GithubRequest::ListWorkflows {
                repo: Box::new(ci_repo()),
                page: first
            },
            GithubRequest::ListWorkflowRuns {
                repo: Box::new(ci_repo()),
                page: first
            },
        ]
    );
}

#[test]
fn the_section_turns_compact_only_once_both_requests_answered() {
    let mut app = app(Config::default(), FakeGithub::new());
    select(&mut app, ci_repo());
    let ids: Vec<RequestId> = requests(&app).iter().map(|(id, _)| *id).collect();
    let [workflows, runs] = ids[..] else {
        panic!("two requests: {ids:?}");
    };
    app.send(Event::Github {
        id: runs,
        response: GithubResponse::ListWorkflowRuns(Ok(Vec::new())),
    });
    assert!(!pane(&app).runs_table().is_loading(), "the runs clear the overlay");
    assert!(!pane(&app).compact(), "the workflows are still out");
    app.send(Event::Github {
        id: workflows,
        response: GithubResponse::ListWorkflows(Ok(vec![workflow(1, "CI")])),
    });
    assert!(pane(&app).compact());
    assert_eq!(pane(&app).workflows(), [workflow(1, "CI")]);
}

#[test]
fn the_workflows_answering_first_also_waits_for_the_runs() {
    let mut app = app(Config::default(), FakeGithub::new());
    select(&mut app, ci_repo());
    let ids: Vec<RequestId> = requests(&app).iter().map(|(id, _)| *id).collect();
    app.send(Event::Github {
        id: ids[0],
        response: GithubResponse::ListWorkflows(Ok(Vec::new())),
    });
    assert!(pane(&app).runs_table().is_loading());
    assert!(!pane(&app).compact());
    app.send(Event::Github {
        id: ids[1],
        response: GithubResponse::ListWorkflowRuns(Ok(runs(&ci_repo(), 1, 2))),
    });
    assert!(!pane(&app).compact());
    assert_eq!(run_ids(&app), [1, 2]);
}

#[test]
fn nothing_is_requested_while_the_section_is_hidden_by_config() {
    let mut config = Config::default();
    config.appearance.show_workflows = false;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, ci_repo());
    assert_eq!(requests(&app), []);
    assert!(!pane(&app).runs_table().is_loading());
}

// Errors.

#[test]
fn failed_requests_log_their_lines_with_no_toast_and_the_section_collapses() {
    let gh = FakeGithub::new()
        .with_workflow_page("octo/ci", 1, Err(server_error()))
        .with_run_page("octo/ci", 1, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, ci_repo());
    let out = settle(&mut app);
    let errors: Vec<&str> = out.lines().filter(|line| line.starts_with("ERROR")).collect();
    assert_eq!(
        errors,
        [
            "ERROR Error retrieving actions from the Github API",
            "ERROR Error retrieving action runs from the Github API",
        ],
        "{out}"
    );
    assert!(
        out.contains("DEBUG Error retrieving action runs from the Github API: "),
        "{out}"
    );
    assert!(app.app().toasts().is_empty());
    assert!(!pane(&app).runs_table().is_loading());
    assert!(pane(&app).compact());
}

#[test]
fn a_workflows_decode_error_logs_like_an_http_error() {
    let gh = FakeGithub::new()
        .with_workflow_page("octo/ci", 1, Err(decode_error()))
        .with_run_page("octo/ci", 1, Ok(runs(&ci_repo(), 1, 3)));
    let mut app = app(Config::default(), gh);
    select(&mut app, ci_repo());
    let out = settle(&mut app);
    assert!(
        out.contains("ERROR Error retrieving actions from the Github API\n"),
        "{out}"
    );
    assert!(!out.contains("action runs"), "{out}");
    assert!(app.app().toasts().is_empty());
    assert!(!pane(&app).compact());
    assert_eq!(pane(&app).workflows(), []);
}

#[test]
fn a_runs_decode_error_logs_the_runs_line_and_the_section_collapses() {
    let gh = FakeGithub::new().with_run_page("octo/ci", 1, Err(decode_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, ci_repo());
    let out = settle(&mut app);
    assert!(
        out.contains("ERROR Error retrieving action runs from the Github API\n"),
        "{out}"
    );
    assert!(pane(&app).compact());
}

// Later pages.

#[test]
fn later_pages_append_until_an_empty_one() {
    let repo = ci_repo();
    let gh = FakeGithub::new()
        .with_run_page("octo/ci", 1, Ok(runs(&repo, 1, 30)))
        .with_run_page("octo/ci", 2, Ok(runs(&repo, 31, 8)));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    cursor(&mut app, BindingId::TableScrollBottom);
    settle(&mut app);
    assert_eq!(pane(&app).runs_table().row_count(), 38);
    cursor(&mut app, BindingId::TableScrollBottom);
    settle(&mut app);
    // Page 3 isn't in the fake, so it's empty: paging ends.
    assert!(
        !pane(&app)
            .runs_table()
            .lazy_load()
            .expect("paging is on")
            .can_load_more()
    );
    assert_eq!(run_pages(&app), [1, 2, 3]);
    let asked = requests(&app);
    let (_, GithubRequest::ListWorkflowRuns { repo, page }) = asked.last().expect("a page request") else {
        panic!("a runs request");
    };
    assert_eq!(repo.full_name, "octo/ci");
    assert_eq!(*page, PageQuery { page: 3, per_page: 30 });
}

#[test]
fn a_failed_later_page_logs_the_runs_line_and_ends_paging() {
    let repo = ci_repo();
    let gh = FakeGithub::new()
        .with_run_page("octo/ci", 1, Ok(runs(&repo, 1, 30)))
        .with_run_page("octo/ci", 2, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    cursor(&mut app, BindingId::TableScrollBottom);
    let out = settle(&mut app);
    assert!(
        out.starts_with("ERROR Error retrieving action runs from the Github API\n"),
        "{out}"
    );
    assert!(app.app().toasts().is_empty());
    assert_eq!(pane(&app).runs_table().row_count(), 30);
    assert!(
        !pane(&app)
            .runs_table()
            .lazy_load()
            .expect("paging is on")
            .can_load_more()
    );
    cursor(&mut app, BindingId::TableCursorUp);
    assert_eq!(run_pages(&app), [1, 2]);
    assert_eq!(RunsSource::page_failed(&server_error()), PageFailure::End);
}

// Another repo.

#[test]
fn selecting_another_repo_cancels_both_requests_and_drops_late_answers() {
    let repo = ci_repo();
    let mut app = app(Config::default(), FakeGithub::new());
    select(&mut app, repo.clone());
    let first: Vec<RequestId> = requests(&app).iter().map(|(id, _)| *id).collect();
    select(&mut app, other_repo());
    assert_eq!(cancels(&app), first);
    for (id, response) in [
        (first[0], GithubResponse::ListWorkflows(Ok(vec![workflow(1, "CI")]))),
        (first[1], GithubResponse::ListWorkflowRuns(Ok(runs(&repo, 1, 3)))),
    ] {
        app.send(Event::Github { id, response });
    }
    assert!(pane(&app).runs_table().is_loading());
    assert_eq!(run_ids(&app), Vec::<u64>::new());
    assert_eq!(pane(&app).workflows(), []);
    assert!(!pane(&app).compact());
}

#[test]
fn selecting_another_repo_cancels_a_later_page_in_flight() {
    let repo = ci_repo();
    let gh = FakeGithub::new().with_run_page("octo/ci", 1, Ok(runs(&repo, 1, 30)));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo.clone());
    settle(&mut app);
    cursor(&mut app, BindingId::TableScrollBottom);
    let (page_2, _) = requests(&app).last().cloned().expect("page 2");
    select(&mut app, other_repo());
    assert_eq!(cancels(&app), [page_2]);
    app.send(Event::Github {
        id: page_2,
        response: GithubResponse::ListWorkflowRuns(Ok(runs(&repo, 31, 5))),
    });
    assert_eq!(pane(&app).runs_table().row_count(), 0);
    settle(&mut app);
    assert!(pane(&app).compact(), "octo/other has no runs");
}
