//! The Trigger workflow dialog and its opener, `T` on the Workflows section, against the `run-details` and
//! `workflow-write` fixtures.

use std::path::Path;
use std::sync::{Arc, Mutex};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{Branch, Repository, Workflow, WorkflowRun};
use serde_json::Value;

use super::*;
use crate::test_util::{TestApp, buffer_to_styled_string, logs};

const NAME: &str = "parity-bot/run-fixture";
const RUN_FIXTURE: &str = "repos/parity-bot/run-fixture";

/// The response body of the fixture at `path` in fixture set `set`.
fn fixture_body(set: &str, path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot/run-fixture`, whose default branch is `main`.
fn fixture_repo() -> Repository {
    let page = fixture_body(
        "run-details",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    serde_json::from_value(page[0].clone()).unwrap()
}

/// The workflows CI (7000001) and Release (7000002).
fn workflows() -> Vec<Workflow> {
    let body = fixture_body(
        "workflow-write",
        &format!("{RUN_FIXTURE}/actions/workflows/GET.page-1_per_page-30.json"),
    );
    serde_json::from_value(body["workflows"].clone()).unwrap()
}

/// The branches `main`, `release` and `run-details`.
fn branches() -> Vec<Branch> {
    serde_json::from_value(fixture_body(
        "workflow-write",
        &format!("{RUN_FIXTURE}/branches/GET.page-1_per_page-30.json"),
    ))
    .unwrap()
}

fn runs() -> Vec<WorkflowRun> {
    let body = fixture_body(
        "run-details",
        &format!("{RUN_FIXTURE}/actions/runs/GET.page-1_per_page-30.json"),
    );
    serde_json::from_value(body["workflow_runs"].clone()).unwrap()
}

fn unprocessable() -> GhError {
    GhError::Http {
        status: 422,
        message: "Unprocessable Entity".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// The fake with the repo, its workflows, runs and branches, Release dispatching on `release` and CI refused.
fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_workflow_page(NAME, 1, Ok(workflows()))
        .with_run_page(NAME, 1, Ok(runs()))
        .with_branches(NAME, Ok(branches()))
        .with_dispatch(7_000_002, "release", Ok(()))
        .with_dispatch(7_000_001, "main", Err(unprocessable()))
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

type Replies = Arc<Mutex<Vec<bool>>>;

/// An app on `gh` with the dialog for `repo` open on its own, its replies recorded, and the branches answered.
fn open_on(gh: FakeGithub, repo: Repository) -> (TestApp, Replies) {
    let mut app = TestApp::with_github(Config::default(), gh);
    settle(&mut app);
    let replies = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&replies);
    app.with_cx(|cx| {
        let dialog = TriggerWorkflowDialog::new(repo, &workflows(), cx);
        cx.open_dialog(dialog, move |triggered| {
            sink.lock().unwrap().push(triggered);
            Action::Consumed
        });
    });
    settle(&mut app);
    (app, replies)
}

fn open() -> (TestApp, Replies) {
    open_on(gh(), fixture_repo())
}

fn modals(app: &TestApp) -> Vec<ModalKind> {
    app.app().modals()
}

/// The dispatches sent, as (workflow id, ref).
fn dispatches(app: &TestApp) -> Vec<(u64, String)> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::DispatchWorkflow {
                full_name,
                workflow_id,
                git_ref,
            } => {
                assert_eq!(full_name, NAME);
                Some((workflow_id, git_ref))
            }
            _ => None,
        })
        .collect()
}

fn toasts(app: &TestApp) -> Vec<Toast> {
    app.app().toasts().iter().cloned().collect()
}

/// The branch input's row on a 160x48 screen.
fn branch_row(app: &mut TestApp) -> String {
    app.screen(160, 48).lines().nth(25).unwrap().to_owned()
}

/// The dialog's frame at 160x48, where the `workflow-trigger` scenario's captures have it, with its styles.
fn frame_snapshot(app: &mut TestApp) -> String {
    let screen = app.render(160, 48);
    let frame = Rect::new(50, 11, 60, 25);
    let mut dialog = ratatui::buffer::Buffer::empty(frame);
    for (x, y) in frame.positions().map(|p| (p.x, p.y)) {
        dialog[(x, y)] = screen[(x, y)].clone();
    }
    buffer_to_styled_string(&dialog)
}

#[test]
fn it_asks_for_the_branches_on_open_and_prefills_the_default_branch() {
    let (app, _) = open();
    assert!(app.gh().calls().contains(&GithubCall::ListBranches(NAME.into())));
    assert_eq!(modals(&app), [ModalKind::TriggerWorkflow]);
    let mut app = app;
    assert!(branch_row(&mut app).contains("▊  main "), "{}", branch_row(&mut app));
}

#[test]
fn a_repo_without_a_default_branch_prefills_main() {
    let mut repo = fixture_repo();
    repo.default_branch = None;
    let mut app = TestApp::new(Config::default());
    let dialog = app.with_cx(|cx| TriggerWorkflowDialog::new(repo.clone(), &workflows(), cx));
    assert_eq!(dialog.branch(), "main");
    repo.default_branch = Some("trunk".into());
    let dialog = app.with_cx(|cx| TriggerWorkflowDialog::new(repo, &workflows(), cx));
    assert_eq!(dialog.branch(), "trunk");
}

#[test]
fn trigger_without_a_workflow_toasts_and_sends_nothing() {
    let (mut app, replies) = open();
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(dispatches(&app).is_empty());
    assert_eq!(
        toasts(&app),
        [Toast::error("You must select a workflow!").with_title("Validation Error")]
    );
    assert_eq!(modals(&app), [ModalKind::TriggerWorkflow]);
    assert!(replies.lock().unwrap().is_empty());
}

#[test]
fn an_empty_branch_toasts_and_marks_the_input() {
    let (mut app, _) = open();
    // Release, then the branch emptied and Trigger pressed.
    app.press(&["enter", "down", "down", "enter", "tab", "backspace", "tab", "enter"]);
    settle(&mut app);
    assert!(dispatches(&app).is_empty());
    assert_eq!(
        toasts(&app),
        [Toast::error("You must enter a branch!").with_title("Validation Error")]
    );
    assert_eq!(modals(&app), [ModalKind::TriggerWorkflow]);
    assert!(branch_row(&mut app).contains("Choose a branch"));
}

#[test]
fn a_dispatch_closes_with_true() {
    let (mut app, replies) = open();
    app.press(&["enter", "down", "down", "enter", "tab"]);
    app.send(crate::event::Event::Paste("release".into()));
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(dispatches(&app), [(7_000_002, "release".into())]);
    assert!(modals(&app).is_empty());
    assert_eq!(*replies.lock().unwrap(), [true]);
}

#[test]
fn a_refused_dispatch_toasts_and_keeps_the_dialog() {
    let (mut app, replies) = open();
    // CI on main, which GitHub refuses (Py bug B68's recommendation).
    app.press(&["enter", "down", "enter", "shift+enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR Error creating workflow dispatch event!"), "{out}");
    assert_eq!(dispatches(&app), [(7_000_001, "main".into())]);
    assert_eq!(
        toasts(&app),
        [
            Toast::error("Could not trigger build - are you sure this workflow supports dispatch events?")
                .with_title("Error Triggering Build")
        ]
    );
    assert_eq!(modals(&app), [ModalKind::TriggerWorkflow]);
    assert!(replies.lock().unwrap().is_empty());
}

#[test]
fn q_escape_and_cancel_close_with_false_and_send_nothing() {
    for keys in [&["q"][..], &["escape"], &["tab", "tab", "tab", "enter"]] {
        let (mut app, replies) = open();
        app.press(keys);
        settle(&mut app);
        assert!(modals(&app).is_empty(), "{keys:?}");
        assert_eq!(*replies.lock().unwrap(), [false], "{keys:?}");
        assert!(dispatches(&app).is_empty(), "{keys:?}");
    }
}

#[test]
fn closing_before_the_branches_arrive_cancels_their_request() {
    let mut app = TestApp::with_github(Config::default(), gh());
    settle(&mut app);
    app.with_cx(|cx| {
        let dialog = TriggerWorkflowDialog::new(fixture_repo(), &workflows(), cx);
        cx.open_dialog(dialog, |_| Action::Consumed);
    });
    let id = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::ListBranches { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    app.press(&["q"]);
    assert!(app.effects().contains(&Effect::Cancel(id)));
}

#[test]
fn the_footers_follow_the_focused_field() {
    let (mut app, _) = open();
    let footer = |app: &mut TestApp| app.screen(160, 48).lines().last().unwrap().trim_end().to_owned();
    assert_eq!(footer(&mut app), " shift+⏎ Submit  q Close");
    app.press(&["enter"]);
    assert_eq!(footer(&mut app), " esc Dismiss menu  shift+⏎ Submit");
    app.press(&["escape", "tab"]);
    assert_eq!(footer(&mut app), " shift+⏎ Submit  esc Close");
}

#[test]
fn the_branch_names_complete_and_right_accepts() {
    let (mut app, _) = open();
    app.press(&["tab", "r", "e", "l"]);
    assert!(branch_row(&mut app).contains("▊  release "));
    app.press(&["right", "shift+tab", "tab"]);
    assert_eq!(dispatches(&app), []);
    app.press(&["shift+tab", "enter", "end", "enter", "shift+enter"]);
    settle(&mut app);
    assert_eq!(dispatches(&app), [(7_000_002, "release".into())]);
}

#[test]
fn failed_branches_leave_the_input_without_suggestions() {
    let (mut app, _) = open_on(gh().with_branches(NAME, Err(unprocessable())), fixture_repo());
    // Python's worker raises, which ends the app: here it's the generic API error (#323).
    assert_eq!(
        toasts(&app),
        [Toast::error("GitHub API request failed (HTTP 422): Unprocessable Entity").with_title("API Error")]
    );
    app.press(&["tab", "r", "e", "l"]);
    let row = branch_row(&mut app);
    assert!(row.contains("▊  rel "), "{row}");
}

#[test]
fn a_click_on_the_box_opens_the_list_and_a_click_on_a_row_picks_it() {
    let (mut app, _) = open();
    app.press(&["tab"]);
    app.render(160, 48);
    app.click(70, 21);
    app.render(160, 48);
    // The list: the blank on row 24, CI on 25, Release on 26.
    app.click(70, 26);
    app.press(&["tab", "shift+enter"]);
    settle(&mut app);
    // Release on main isn't canned: the fake refuses it.
    assert_eq!(dispatches(&app), [(7_000_002, "main".into())]);
}

#[test]
fn trigger_workflow_open() {
    let (mut app, _) = open();
    insta::assert_snapshot!(frame_snapshot(&mut app));
}

#[test]
fn trigger_workflow_select_open() {
    let (mut app, _) = open();
    app.press(&["enter"]);
    insta::assert_snapshot!(frame_snapshot(&mut app));
}

#[test]
fn trigger_workflow_suggested() {
    let (mut app, _) = open();
    app.press(&["enter", "down", "down", "enter", "tab", "r", "e", "l"]);
    insta::assert_snapshot!(frame_snapshot(&mut app));
}

// The opener: `T` on the Workflows section.

/// The app with the Workflows table focused, and `parity-bot/run-fixture` selected when `select`.
fn workflows_focused(gh: FakeGithub, select: bool) -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh);
    if select {
        app.press(&["enter"]);
        settle(&mut app);
    }
    app.press(&["4"]);
    app
}

fn workflow_lists(app: &TestApp) -> usize {
    app.gh()
        .calls()
        .iter()
        .filter(|call| matches!(call, GithubCall::ListWorkflows { .. }))
        .count()
}

#[test]
fn t_without_a_repo_warns() {
    let mut app = workflows_focused(gh(), false);
    app.press(&["T"]);
    settle(&mut app);
    assert_eq!(toasts(&app), [Toast::warning("No repository selected")]);
    assert!(modals(&app).is_empty());
}

#[test]
fn t_opens_the_dialog_with_the_workflows_loaded_with_the_repo() {
    let mut app = workflows_focused(gh(), true);
    assert_eq!(workflow_lists(&app), 1);
    app.press(&["T"]);
    settle(&mut app);
    assert_eq!(workflow_lists(&app), 1, "no second request");
    assert_eq!(modals(&app), [ModalKind::TriggerWorkflow]);
    // Release on release, and the success toast once it closes.
    app.press(&["enter", "down", "down", "enter", "tab"]);
    app.send(crate::event::Event::Paste("release".into()));
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(modals(&app).is_empty());
    assert_eq!(toasts(&app), [Toast::info("Successfully triggered workflow")]);
    assert_eq!(
        app.app().main_screen().unwrap().focus(),
        crate::screens::main::FocusTarget::Table(crate::screens::main::SectionId::Workflows)
    );
}

#[test]
fn without_workflows_t_fetches_them_first() {
    // None load with the repo; the second request finds them.
    let gh = gh().with_workflow_page(NAME, 1, Ok(Vec::new()));
    let mut app = workflows_focused(gh, true);
    let _ = app.gh().clone().with_workflow_page(NAME, 1, Ok(workflows()));
    app.press(&["T"]);
    assert!(modals(&app).is_empty(), "the dialog waits for the workflows");
    settle(&mut app);
    assert_eq!(workflow_lists(&app), 2);
    assert_eq!(modals(&app), [ModalKind::TriggerWorkflow]);
}

#[test]
fn still_no_workflows_warns() {
    for page in [Ok(Vec::new()), Err(unprocessable())] {
        let mut app = workflows_focused(gh().with_workflow_page(NAME, 1, page), true);
        app.press(&["T"]);
        settle(&mut app);
        assert_eq!(workflow_lists(&app), 2);
        assert_eq!(toasts(&app), [Toast::warning("No workflows found in this repository")]);
        assert!(modals(&app).is_empty());
    }
}

// `gh` failing mid-session (#323): the branches and the dispatch.

mod gh_failures {
    use super::*;
    use crate::test_util::assert_gh_failures_surface;

    #[test]
    fn the_branches() {
        assert_gh_failures_surface(
            || TestApp::with_github(Config::default(), gh()),
            |app| {
                app.with_cx(|cx| {
                    let dialog = TriggerWorkflowDialog::new(fixture_repo(), &workflows(), cx);
                    cx.open_dialog(dialog, |_| Action::Consumed);
                });
            },
            |app| {
                // Open, with the default branch and no suggestions.
                if modals(app) == [ModalKind::TriggerWorkflow] {
                    Ok(())
                } else {
                    Err(format!("{:?}", modals(app)))
                }
            },
        );
    }

    #[test]
    fn the_dispatch() {
        assert_gh_failures_surface(
            || open().0,
            |app| {
                app.press(&["enter", "down", "enter", "shift+enter"]);
            },
            |app| {
                if modals(app) != [ModalKind::TriggerWorkflow] {
                    return Err(format!("{:?}", modals(app)));
                }
                let before = dispatches(app).len();
                app.press(&["shift+enter"]);
                futures_util::FutureExt::now_or_never(app.settle()).unwrap();
                (dispatches(app).len() > before)
                    .then_some(())
                    .ok_or_else(|| "Trigger is stuck".into())
            },
        );
    }
}
