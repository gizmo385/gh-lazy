//! The command palette's section and command log toggles on the main screen: what they show and hide, the focus keys
//! that follow, the reloads on showing a section again and where focus goes when its section is hidden (Py bug B88).

use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::Repository;

use super::pull_requests::{Kind, item};
use super::repos::repo;
use super::workflows::run;
use super::*;
use crate::test_util::TestApp;

const REPOS: FocusTarget = FocusTarget::Table(SectionId::Repos);
const PRS: FocusTarget = FocusTarget::Table(SectionId::PullRequests);
const ISSUES: FocusTarget = FocusTarget::Table(SectionId::Issues);
const WORKFLOWS: FocusTarget = FocusTarget::Table(SectionId::Workflows);

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().expect("on the main screen")
}

fn focus(app: &TestApp) -> FocusTarget {
    main(app).focus()
}

fn shown(app: &TestApp, id: SectionId) -> bool {
    main(app).section(id).shown()
}

fn toggle(app: &mut TestApp, toggle: Toggle) {
    app.dispatch(Action::Toggle(toggle));
}

fn one() -> Repository {
    repo("octo", "one", false)
}

/// The requests and cache reads in the effects from `from` on, as short names.
fn loads(app: &TestApp, from: usize) -> Vec<&'static str> {
    app.effects()[from..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. } => match request {
                GithubRequest::ListIssues { .. } => Some("issues"),
                GithubRequest::ListWorkflows { .. } => Some("workflows"),
                GithubRequest::ListWorkflowRuns { .. } => Some("runs"),
                _ => None,
            },
            Effect::ReadCache { path, .. } => Some(if path.to_string_lossy().ends_with("_workflow_runs.json") {
                "runs cache"
            } else {
                "other cache"
            }),
            _ => None,
        })
        .collect()
}

/// An app with `octo/one` selected and every request answered: two PRs and an issue, and a run.
fn selected(config: Config) -> TestApp {
    let repo = one();
    let gh = FakeGithub::new()
        .with_repositories(vec![repo.clone(), repo_two()])
        .with_issue_page(
            "octo/one",
            1,
            Ok(vec![
                item(&repo, 3, Kind::OpenPr),
                item(&repo, 2, Kind::Issue),
                item(&repo, 1, Kind::OpenPr),
            ]),
        )
        .with_run_page(
            "octo/one",
            1,
            Ok(vec![run(
                &repo,
                1,
                "CI",
                1,
                "2026-10-20T09:00:00Z",
                Some("success"),
                "Run",
            )]),
        );
    let mut app = TestApp::with_github(config, gh);
    app.dispatch(Action::SelectRepo(Box::new(repo)));
    settle(&mut app);
    app
}

fn repo_two() -> Repository {
    repo("octo", "two", false)
}

fn settle(app: &mut TestApp) {
    app.settle().now_or_never().expect("the fake answers at once");
}

fn appearance(f: impl FnOnce(&mut AppearanceSettings)) -> Config {
    let mut config = Config::default();
    f(&mut config.appearance);
    config
}

#[test]
fn a_toggle_flips_a_section_and_shows_one_config_hides() {
    let mut app = TestApp::new(appearance(|a| a.show_workflows = false));
    assert!(!shown(&app, SectionId::Workflows));
    toggle(&mut app, Toggle::Workflows);
    assert!(shown(&app, SectionId::Workflows));
    toggle(&mut app, Toggle::Workflows);
    assert!(!shown(&app, SectionId::Workflows));
    assert!(main(&app).section(SectionId::Workflows).hidden_by_toggle);

    toggle(&mut app, Toggle::Issues);
    assert!(!shown(&app, SectionId::Issues));
    toggle(&mut app, Toggle::Issues);
    assert!(shown(&app, SectionId::Issues));
    // Session only: the config isn't written.
    assert!(
        !app.effects()
            .iter()
            .any(|effect| matches!(effect, Effect::SaveConfig(_)))
    );
}

#[test]
fn the_command_log_toggle_shows_and_hides_it() {
    let mut app = TestApp::new(Config::default());
    assert!(!main(&app).command_log().is_visible());
    toggle(&mut app, Toggle::CommandLog);
    assert!(main(&app).command_log().is_visible());
    assert!(main(&app).focus_ring(None).contains(&FocusTarget::CommandLog));
    toggle(&mut app, Toggle::CommandLog);
    assert!(!main(&app).command_log().is_visible());
}

#[test]
fn focus_keys_follow_the_toggles() {
    let mut app = TestApp::new(Config::default());
    toggle(&mut app, Toggle::Issues);
    toggle(&mut app, Toggle::PullRequests);
    app.press(&["3"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["2"]);
    assert_eq!(focus(&app), REPOS);
    toggle(&mut app, Toggle::Issues);
    app.press(&["3"]);
    assert_eq!(focus(&app), ISSUES);
    toggle(&mut app, Toggle::PullRequests);
    app.press(&["2"]);
    assert_eq!(focus(&app), PRS);

    // `4` still focuses the runs table while Workflows is hidden.
    toggle(&mut app, Toggle::Workflows);
    app.press(&["4"]);
    assert_eq!(focus(&app), WORKFLOWS);

    // The log takes `6` only while it's shown, whatever config said.
    app.press(&["1", "6"]);
    assert_eq!(focus(&app), REPOS);
    toggle(&mut app, Toggle::CommandLog);
    app.press(&["6"]);
    assert_eq!(focus(&app), FocusTarget::CommandLog);
}

#[test]
fn a_config_hidden_section_shown_by_a_toggle_takes_its_key_and_hidden_again_ignores_it() {
    let mut app = TestApp::new(appearance(|a| a.show_issues = false));
    toggle(&mut app, Toggle::Issues);
    app.press(&["3"]);
    assert_eq!(focus(&app), ISSUES);
    toggle(&mut app, Toggle::Issues);
    app.press(&["3"]);
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn showing_workflows_with_a_repo_reads_its_runs_cache_and_requests_workflows_and_runs() {
    let mut app = selected(appearance(|a| a.show_workflows = false));
    let from = app.effects().len();
    toggle(&mut app, Toggle::Workflows);
    assert_eq!(loads(&app, from), ["runs cache", "workflows", "runs"]);
    settle(&mut app);
    assert_eq!(main(&app).workflows.runs_table().row_count(), 1);
}

#[test]
fn showing_issues_or_prs_requests_the_first_page_once_without_clearing() {
    for (section, id) in [
        (Toggle::Issues, SectionId::Issues),
        (Toggle::PullRequests, SectionId::PullRequests),
    ] {
        let mut app = selected(Config::default());
        let rows = |app: &TestApp| {
            let lists = &main(app).issue_lists;
            (lists.prs_table().row_count(), lists.issues_table().row_count())
        };
        assert_eq!(rows(&app), (2, 1));
        let from = app.effects().len();
        toggle(&mut app, section);
        assert!(!shown(&app, id));
        assert!(loads(&app, from).is_empty(), "hiding requests nothing");
        toggle(&mut app, section);
        assert!(shown(&app, id));
        // One page for both tables, no cache read, and the rows stay until it lands.
        assert_eq!(loads(&app, from), ["issues"]);
        assert_eq!(rows(&app), (2, 1));
        assert!(!main(&app).issue_lists.issues_table().is_loading());
        settle(&mut app);
        // The page replaces the rows (Py bug B53's answer).
        assert_eq!(rows(&app), (2, 1));
    }
}

#[test]
fn showing_a_section_selected_while_both_were_hidden_loads_the_current_repo() {
    let mut app = selected(appearance(|a| {
        a.show_issues = false;
        a.show_pull_requests = false;
    }));
    assert_eq!(main(&app).issue_lists.issues_table().row_count(), 0);
    toggle(&mut app, Toggle::Issues);
    settle(&mut app);
    assert_eq!(main(&app).issue_lists.issues_table().row_count(), 1);
    assert_eq!(main(&app).issue_lists.prs_table().row_count(), 2);
}

#[test]
fn showing_with_no_repo_and_hiding_request_nothing() {
    let mut app = TestApp::new(appearance(|a| {
        a.show_workflows = false;
        a.show_issues = false;
    }));
    let from = app.effects().len();
    for t in [
        Toggle::Workflows,
        Toggle::Issues,
        Toggle::PullRequests,
        Toggle::CommandLog,
    ] {
        toggle(&mut app, t);
        toggle(&mut app, t);
    }
    assert!(loads(&app, from).is_empty());
}

#[test]
fn hiding_the_focused_table_search_box_or_log_focuses_the_first_ring_stop() {
    let mut app = TestApp::new(Config::default());
    app.press(&["3"]);
    toggle(&mut app, Toggle::Issues);
    assert_eq!(focus(&app), REPOS);

    // The search box: its section's search, focused.
    let mut app = TestApp::new(Config::default());
    app.press(&["2"]);
    app.app_mut()
        .main_screen_mut()
        .expect("on the main screen")
        .open_search(SectionId::PullRequests);
    assert_eq!(focus(&app), FocusTarget::Search(SectionId::PullRequests));
    toggle(&mut app, Toggle::PullRequests);
    assert_eq!(focus(&app), REPOS);

    let mut app = TestApp::new(Config::default());
    toggle(&mut app, Toggle::CommandLog);
    app.press(&["6"]);
    toggle(&mut app, Toggle::CommandLog);
    assert_eq!(focus(&app), REPOS);

    // With the repos search box shown, it's the first stop.
    let mut app = TestApp::new(Config::default());
    app.app_mut()
        .main_screen_mut()
        .expect("on the main screen")
        .open_search(SectionId::Repos);
    app.press(&["4"]);
    toggle(&mut app, Toggle::Workflows);
    assert_eq!(focus(&app), FocusTarget::Search(SectionId::Repos));
}

#[test]
fn hiding_an_unfocused_section_keeps_focus() {
    let mut app = TestApp::new(Config::default());
    app.press(&["2"]);
    toggle(&mut app, Toggle::Issues);
    assert_eq!(focus(&app), PRS);
}

#[test]
fn hiding_the_maximized_section_restores_the_layout_first() {
    let mut app = TestApp::new(Config::default());
    app.press(&["3", "M"]);
    assert!(app.app().maximized().is_some());
    toggle(&mut app, Toggle::Issues);
    assert!(app.app().maximized().is_none());
    assert_eq!(focus(&app), REPOS);

    // Another section maximized stays so.
    let mut app = TestApp::new(Config::default());
    app.press(&["2", "M"]);
    toggle(&mut app, Toggle::Issues);
    assert_eq!(app.app().maximized().map(|m| m.target), Some(PRS));
    assert_eq!(focus(&app), PRS);
}

#[test]
fn issues_and_workflows_toggled_off_160x48() {
    let mut app = TestApp::new(Config::default());
    toggle(&mut app, Toggle::Issues);
    toggle(&mut app, Toggle::Workflows);
    insta::assert_snapshot!(app.screen(160, 48));
}
