//! `gh` failing mid-session (#323): the main screen's own requests, and the dialogs it opens that only need a repo.
//! The PR details, the run details and the issue details have theirs next to their other tests.

use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{IssueListItem, Repository};

use super::pull_requests::{Kind, item};
use super::repos::repo;
use super::{EditOrigin, FocusTarget, MainAction, ProfileOrigin, SectionId};
use crate::action::Action;
use crate::components::table::{Handled, TableEvent};
use crate::effect::{Effect, GithubRequest};
use crate::keymap::BindingId;
use crate::modal::ModalKind;
use crate::test_util::{TestApp, assert_gh_failures_surface, gh_failures, logs};

fn ci() -> Repository {
    repo("octo", "ci", false)
}

/// Two pull requests and two issues, newest first.
fn first_page() -> Vec<IssueListItem> {
    let repo = ci();
    vec![
        item(&repo, 4, Kind::OpenPr),
        item(&repo, 3, Kind::Issue),
        item(&repo, 2, Kind::OpenPr),
        item(&repo, 1, Kind::Issue),
    ]
}

/// A full first page of runs, so the table pages.
fn runs() -> Vec<lazy_github_core::models::WorkflowRun> {
    let run = super::run_details::tests::fixture_run(42);
    (1..=30)
        .map(|n| lazy_github_core::models::WorkflowRun {
            id: n,
            run_number: n,
            ..run.clone()
        })
        .collect()
}

fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![ci()])
        .with_issue_page("octo/ci", 1, Ok(first_page()))
        .with_run_page("octo/ci", 1, Ok(runs()))
}

fn settle(app: &mut TestApp) {
    app.settle().now_or_never().expect("the fake answers at once");
}

/// The main screen with `octo/ci` listed, nothing selected.
fn listed() -> TestApp {
    TestApp::with_github(Config::default(), gh())
}

/// [`listed`] with `octo/ci` selected and loaded.
fn selected() -> TestApp {
    let mut app = listed();
    app.dispatch(Action::SelectRepo(Box::new(ci())));
    settle(&mut app);
    app
}

fn main(app: &TestApp) -> &super::MainScreen {
    app.app().main_screen().expect("on the main screen")
}

fn modals(app: &TestApp) -> Vec<ModalKind> {
    app.app().modals()
}

/// The GitHub requests made so far.
fn requests(app: &TestApp) -> Vec<GithubRequest> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. } => Some(request.clone()),
            _ => None,
        })
        .collect()
}

/// Types `text` into the focused input.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            '/' => "slash".to_owned(),
            c => c.to_string(),
        };
        app.press(&[key.as_str()]);
    }
}

/// The open dialog is `kind` and its submit asks again.
fn submit_works_again(app: &mut TestApp, kind: ModalKind) -> Result<(), String> {
    if modals(app) != [kind] {
        return Err(format!("dialogs: {:?}", modals(app)));
    }
    let before = requests(app).len();
    app.press(&["shift+enter"]);
    if requests(app).len() > before {
        Ok(())
    } else {
        Err("submit is stuck".into())
    }
}

/// No request was cancelled among the effects from `index` on: nothing was still in flight.
fn no_cancel_since(app: &TestApp, index: usize) -> Result<(), String> {
    match app.effects()[index..]
        .iter()
        .find(|effect| matches!(effect, Effect::Cancel(_)))
    {
        Some(cancel) => Err(format!("a request was still in flight: {cancel:?}")),
        None => Ok(()),
    }
}

#[test]
fn the_repos_list() {
    for (name, error) in gh_failures() {
        let mut app = TestApp::loading(Config::default(), gh().with_failure_from(0, error));
        let out = logs(|| settle(&mut app));
        assert!(
            out.contains("ERROR Error fetching repositories from Github API"),
            "{name}: {out}"
        );
        assert!(!main(&app).repos.table.is_loading(), "{name}");
        assert!(!app.screen(160, 48).contains("Loading"), "{name}");
    }
}

#[test]
fn a_tracked_repo_is_left_out_as_python_does() {
    let mut config = Config::default();
    config.repositories.additional_repos_to_track = vec!["octo/tracked".into()];
    for (name, error) in gh_failures() {
        // The list works; the tracked repo, the call after it, fails.
        let mut app = TestApp::loading(config.clone(), gh().with_failure_from(1, error));
        settle(&mut app);
        assert_eq!(
            app.gh().calls().get(1),
            Some(&GithubCall::Repository("octo/tracked".into())),
            "{name}"
        );
        assert!(!main(&app).repos.table.is_loading(), "{name}");
        assert_eq!(main(&app).repos.table.row_count(), 1, "{name}");
    }
}

#[test]
fn selecting_a_repo() {
    // The issues and pull requests, the workflows, the runs and the README.
    assert_gh_failures_surface(
        listed,
        |app| {
            app.dispatch(Action::SelectRepo(Box::new(ci())));
        },
        |app| {
            let main = main(app);
            let loading = [
                main.issue_lists.prs_table().is_loading(),
                main.issue_lists.issues_table().is_loading(),
                main.workflows.runs_table().is_loading(),
            ];
            if loading.contains(&true) {
                return Err(format!("still loading: {loading:?}"));
            }
            let screen = app.screen(160, 48);
            if screen.contains("No README found.") {
                Ok(())
            } else {
                Err(format!("no README line\n{screen}"))
            }
        },
    );
}

/// Moves `section`'s table cursor with `binding`, which asks for the next page near the end when paging is on.
fn move_cursor(app: &mut TestApp, section: SectionId, binding: BindingId) {
    let main = app.app_mut().main_screen_mut().expect("on the main screen");
    let handled = match section {
        SectionId::PullRequests => main.issue_lists.prs_table_mut().on_binding(binding),
        SectionId::Workflows => main.workflows.runs_table_mut().on_binding(binding),
        other => panic!("{other:?} doesn't page here"),
    };
    if let Handled::Event(TableEvent::LoadMore {
        generation,
        page,
        per_page,
    }) = handled
    {
        app.dispatch(Action::Main(MainAction::LoadMore {
            section,
            generation,
            page,
            per_page,
        }));
    }
}

/// [`selected`] with a full first page of pull requests, so the table pages.
fn paging_prs() -> TestApp {
    let repo = ci();
    let page: Vec<IssueListItem> = (1..=30).rev().map(|n| item(&repo, n, Kind::OpenPr)).collect();
    let gh = gh().with_issue_page("octo/ci", 1, Ok(page));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.dispatch(Action::SelectRepo(Box::new(ci())));
    settle(&mut app);
    app
}

/// How many later pages `section` asked for.
fn page_requests(app: &TestApp, section: SectionId) -> usize {
    requests(app)
        .iter()
        .filter(|request| match (section, request) {
            (SectionId::PullRequests, GithubRequest::ListIssues { query, .. }) => query.page > 1,
            (SectionId::Workflows, GithubRequest::ListWorkflowRuns { page, .. }) => page.page > 1,
            _ => false,
        })
        .count()
}

#[test]
fn a_later_page_of_pull_requests() {
    assert_gh_failures_surface(
        paging_prs,
        |app| move_cursor(app, SectionId::PullRequests, BindingId::TableScrollBottom),
        |app| {
            let asked = page_requests(app, SectionId::PullRequests);
            if asked != 1 {
                return Err(format!("{asked} page requests"));
            }
            if main(app).issue_lists.prs_table().row_count() != 30 {
                return Err("the rows changed".into());
            }
            // Python retries a failed page: the next move asks for it again.
            move_cursor(app, SectionId::PullRequests, BindingId::TableCursorUp);
            if page_requests(app, SectionId::PullRequests) == 2 {
                Ok(())
            } else {
                Err("paging is stuck".into())
            }
        },
    );
}

#[test]
fn a_later_page_of_runs() {
    assert_gh_failures_surface(
        selected,
        |app| move_cursor(app, SectionId::Workflows, BindingId::TableScrollBottom),
        |app| {
            let asked = page_requests(app, SectionId::Workflows);
            if asked != 1 {
                return Err(format!("{asked} page requests"));
            }
            // A failed runs page ends paging, as in Python, so nothing is left waiting.
            let lazy = main(app)
                .workflows
                .runs_table()
                .lazy_load()
                .map(crate::components::table::LazyLoad::can_load_more);
            match lazy {
                Some(false) => Ok(()),
                other => Err(format!("paging: {other:?}")),
            }
        },
    );
}

#[test]
fn trigger_s_workflows() {
    // None loaded with the repo, so `T` asks for them; a failure is "No workflows found", as in Python.
    assert_gh_failures_surface(
        selected,
        |app| {
            app.press(&["4", "T"]);
        },
        |app| {
            if !modals(app).is_empty() {
                return Err("a dialog opened".into());
            }
            let warned = app
                .app()
                .toasts()
                .iter()
                .any(|toast| toast.message.to_string() == "No workflows found in this repository");
            warned.then_some(()).ok_or_else(|| "no warning".into())
        },
    );
}

#[test]
fn a_profile() {
    assert_gh_failures_surface(
        listed,
        |app| {
            app.dispatch(Action::Main(MainAction::ViewProfile(ProfileOrigin::Repos)));
        },
        |app| {
            if !modals(app).is_empty() {
                return Err("a dialog opened".into());
            }
            // `u` asks again.
            let before = requests(app).len();
            app.dispatch(Action::Main(MainAction::ViewProfile(ProfileOrigin::Repos)));
            (requests(app).len() == before + 2)
                .then_some(())
                .ok_or_else(|| "u is stuck".into())
        },
    );
}

#[test]
fn looking_up_a_repository() {
    let kind = ModalKind::Lookup(crate::modal::LookupTarget::Repository);
    assert_gh_failures_surface(
        || {
            let mut app = listed();
            app.press(&["O"]);
            type_text(&mut app, "octo/elsewhere");
            app
        },
        |app| {
            app.press(&["shift+enter"]);
        },
        |app| submit_works_again(app, kind),
    );
}

#[test]
fn looking_up_a_pull_request_and_an_issue() {
    for (section, target) in [
        ("2", crate::modal::LookupTarget::PullRequest),
        ("3", crate::modal::LookupTarget::Issue),
    ] {
        let kind = ModalKind::Lookup(target);
        assert_gh_failures_surface(
            || {
                let mut app = selected();
                app.press(&[section, "O"]);
                type_text(&mut app, "7");
                app
            },
            |app| {
                app.press(&["shift+enter"]);
            },
            |app| submit_works_again(app, kind),
        );
    }
}

#[test]
fn a_new_issue() {
    assert_gh_failures_surface(
        || {
            let mut app = selected();
            app.dispatch(Action::Main(MainAction::NewIssue));
            app.press(&["tab"]);
            type_text(&mut app, "Title");
            app
        },
        |app| {
            app.press(&["shift+enter"]);
        },
        |app| submit_works_again(app, ModalKind::NewIssue),
    );
}

#[test]
fn editing_an_issue() {
    assert_gh_failures_surface(
        || {
            let mut app = selected();
            app.press(&["3"]);
            app.dispatch(Action::Main(MainAction::EditIssue(EditOrigin::Table)));
            app
        },
        |app| {
            app.press(&["shift+enter"]);
        },
        |app| submit_works_again(app, ModalKind::EditIssue),
    );
}

#[test]
fn fetching_a_pull_request_to_edit() {
    assert_gh_failures_surface(
        || {
            let mut app = selected();
            app.press(&["2"]);
            app
        },
        |app| {
            app.dispatch(Action::Main(MainAction::EditPullRequest(EditOrigin::Table)));
        },
        |app| {
            if !modals(app).is_empty() {
                return Err("a dialog opened".into());
            }
            // `E` asks again, with nothing in flight to cancel.
            let before = app.effects().len();
            app.dispatch(Action::Main(MainAction::EditPullRequest(EditOrigin::Table)));
            no_cancel_since(app, before)?;
            if main(app).focus() != FocusTarget::Table(SectionId::PullRequests) {
                return Err(format!("focus moved to {:?}", main(app).focus()));
            }
            Ok(())
        },
    );
}
