//! Open Bug Report: the requests it makes, when New issue opens, the failure toast, and the dialog it opens.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{Issue, Repository};

use super::*;
use crate::event::Event;
use crate::modal::ModalKind;
use crate::screens::main::FocusTarget;
use crate::test_util::{TestApp, buffer_to_styled_string};
use crate::toast::Severity;

fn fixture_body(set: &str, path: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// gizmo385/gh-lazy, as the parity fixtures answer it.
fn lazy_github() -> Repository {
    serde_json::from_value(fixture_body("base", "repos/gizmo385/gh-lazy/GET.json")).unwrap()
}

fn gh() -> FakeGithub {
    FakeGithub::new().with_repository(LAZY_GITHUB_REPO, Ok(Some(lazy_github())))
}

fn fixed_env() -> DebugEnv {
    DebugEnv {
        version: "0.1.0",
        rustc: "1.97.0 (fixed 2026-01-01)",
        gh: Some("2.89.0 (fake-gh)".to_owned()),
        os: "linux x86_64".to_owned(),
        terminal: "WezTerm 20240203-110809 (xterm-256color)".to_owned(),
    }
}

fn fixed_config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = "/home/user/.cache/lazy-github".into();
    config.core.logfile_path = "/home/user/.config/lazy-github/lazy_github.log".into();
    config
}

fn run_palette(app: &mut TestApp, query: &str) {
    app.press(&["ctrl+p"]);
    for c in query.chars() {
        let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
        app.press(&[&key]);
    }
    app.press(&["enter"]);
}

fn run_bug_report(app: &mut TestApp) {
    run_palette(app, "bug report");
}

/// The ids of the requests sent so far that `pick` matches, oldest first.
fn requests(app: &TestApp, pick: impl Fn(&GithubRequest) -> bool) -> Vec<RequestId> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { id, request } if pick(request) => Some(*id),
            _ => None,
        })
        .collect()
}

fn repo_requests(app: &TestApp) -> Vec<RequestId> {
    requests(
        app,
        |request| matches!(request, GithubRequest::Repository(name) if name == LAZY_GITHUB_REPO),
    )
}

fn gh_version_requests(app: &TestApp) -> Vec<RequestId> {
    requests(app, |request| matches!(request, GithubRequest::GhVersion))
}

fn answer(app: &mut TestApp, id: RequestId, response: GithubResponse) {
    app.send(Event::Github { id, response });
}

/// Answers the `n`th repo request with the repo.
fn answer_repo(app: &mut TestApp, n: usize) {
    let id = repo_requests(app)[n];
    answer(app, id, GithubResponse::Repository(Ok(Some(Box::new(lazy_github())))));
}

/// Answers the gh version request.
fn answer_gh(app: &mut TestApp) {
    let id = gh_version_requests(app)[0];
    answer(app, id, GithubResponse::GhVersion(Ok("2.89.0 (fake-gh)".to_owned())));
}

fn current(app: &TestApp) -> Option<&str> {
    app.app().current_repo().map(|repo| repo.full_name.as_str())
}

fn description(app: &TestApp) -> String {
    app.app()
        .top_dialog::<NewIssueDialog>()
        .expect("New issue is open")
        .description()
}

fn toasts(app: &TestApp) -> Vec<(Severity, Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.title.clone(), toast.message.to_string()))
        .collect()
}

#[test]
fn the_command_asks_for_the_repo_and_for_gh_only_when_unknown() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    assert!(
        app.app().modals().is_empty(),
        "the palette closed and nothing opened yet"
    );
    assert_eq!(repo_requests(&app).len(), 1);
    assert_eq!(gh_version_requests(&app).len(), 1);
    assert_eq!(current(&app), None, "nothing changes while it loads");

    answer_gh(&mut app);
    answer_repo(&mut app, 0);
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());

    // gh's version is known now: only the repo is asked for.
    run_bug_report(&mut app);
    assert_eq!(repo_requests(&app).len(), 2);
    assert_eq!(gh_version_requests(&app).len(), 1);
}

#[test]
fn the_repo_first_selects_it_and_new_issue_waits_for_gh() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    answer_repo(&mut app, 0);
    assert_eq!(current(&app), Some(LAZY_GITHUB_REPO));
    assert!(app.app().modals().is_empty(), "waits for gh");
    answer_gh(&mut app);
    assert_eq!(app.app().modals(), [ModalKind::NewIssue]);
    assert!(description(&app).contains("- Gh Version: 2.89.0 (fake-gh)\n"));
}

#[test]
fn gh_first_waits_for_the_repo() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    answer_gh(&mut app);
    assert!(app.app().modals().is_empty(), "waits for the repo");
    assert_eq!(current(&app), None);
    answer_repo(&mut app, 0);
    assert_eq!(current(&app), Some(LAZY_GITHUB_REPO));
    assert_eq!(app.app().modals(), [ModalKind::NewIssue]);
}

#[test]
fn a_failed_gh_answer_is_unknown() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    answer_repo(&mut app, 0);
    let id = gh_version_requests(&app)[0];
    answer(&mut app, id, GithubResponse::GhVersion(Err(GhError::NotInstalled)));
    assert_eq!(app.app().modals(), [ModalKind::NewIssue]);
    assert!(description(&app).contains("- Gh Version: unknown\n"));
}

#[tokio::test]
async fn the_dialog_has_the_template_an_empty_title_and_the_scroll_area_focused() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    app.settle().await;
    let dialog = app.app().top_dialog::<NewIssueDialog>().expect("New issue is open");
    assert_eq!(dialog.title(), "");
    let text = dialog.description();
    assert!(text.starts_with("**Describe the bug**"), "{text}");
    assert!(text.contains("**Debug Info:**\n"), "{text}");
    assert!(text.contains("# Library Versions\n"), "{text}");
    assert!(text.contains("# LazyGithub Config\n"), "{text}");
    app.render(160, 48);
    let footer: Vec<String> = crate::components::footer::footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect();
    assert_eq!(footer, ["shift+⏎ Submit", "q Close"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Current repo: gizmo385/gh-lazy"), "{screen}");

    // Cancel keeps the repo selected.
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(current(&app), Some(LAZY_GITHUB_REPO));
}

#[test]
fn a_missing_repo_or_a_failure_toasts_and_changes_nothing() {
    for response in [
        GithubResponse::Repository(Ok(None)),
        GithubResponse::Repository(Err(GhError::Http {
            status: 500,
            message: "Server Error".to_owned(),
            body: Vec::new(),
            headers: Vec::new(),
        })),
    ] {
        let mut app = TestApp::with_github(Config::default(), gh());
        let focus = app.app().main_screen().unwrap().focus();
        run_bug_report(&mut app);
        let id = repo_requests(&app)[0];
        answer(&mut app, id, response);
        answer_gh(&mut app);
        assert!(app.app().modals().is_empty());
        assert_eq!(current(&app), None);
        assert_eq!(app.app().main_screen().unwrap().focus(), focus);
        assert_eq!(
            toasts(&app),
            [(
                Severity::Error,
                Some("Error opening bug report".to_owned()),
                "Could not load LazyGithub repo! Please open the bug report on Github".to_owned()
            )]
        );
    }
}

#[test]
fn running_it_again_cancels_the_first_request() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    run_bug_report(&mut app);
    let ids = repo_requests(&app);
    assert_eq!(ids.len(), 2);
    assert!(app.effects().contains(&Effect::Cancel(ids[0])));
    assert_eq!(gh_version_requests(&app).len(), 1, "the gh request is shared");
    // The first answer, if it comes anyway, is dropped.
    answer_repo(&mut app, 0);
    answer_gh(&mut app);
    assert_eq!(current(&app), None);
    assert!(app.app().modals().is_empty());
    answer_repo(&mut app, 1);
    assert_eq!(app.app().modals(), [ModalKind::NewIssue]);
}

#[test]
fn the_template_holds_the_config_when_the_modal_opens() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    answer_repo(&mut app, 0);
    app.with_cx(|cx| cx.config_mut().appearance.show_workflows = false);
    answer_gh(&mut app);
    assert!(description(&app).contains("\"show_workflows\": false"));
}

#[test]
fn a_debug_info_and_a_bug_report_share_the_gh_request() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_palette(&mut app, "debug info");
    run_bug_report(&mut app);
    assert_eq!(gh_version_requests(&app).len(), 1);
    answer_repo(&mut app, 0);
    answer_gh(&mut app);
    assert_eq!(app.app().modals(), [ModalKind::DebugInfo, ModalKind::NewIssue]);
}

#[test]
fn a_bug_report_alone_does_not_open_debug_info() {
    let mut app = TestApp::with_github(Config::default(), gh());
    run_bug_report(&mut app);
    let id = repo_requests(&app)[0];
    answer(&mut app, id, GithubResponse::Repository(Ok(None)));
    answer_gh(&mut app);
    assert!(app.app().modals().is_empty());
}

#[test]
fn a_created_issue_goes_to_the_issues_table_and_opens() {
    let created = Issue::from_api(
        fixture_body("issue-write", "repos/parity-bot/issue-fixture/issues/POST.json"),
        &lazy_github(),
    )
    .unwrap();
    let mut app = TestApp::with_github(Config::default(), gh().with_created_issue(Ok(created)));
    run_bug_report(&mut app);
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
    app.press(&["tab", "B"]);
    app.press(&["shift+enter"]);
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
    assert!(app.app().modals().is_empty());
    let main = app.app().main_screen().unwrap();
    assert_eq!(main.details_issue().map(|issue| issue.number), Some(13));
    assert_eq!(main.focus(), FocusTarget::DetailsTabs);
}

#[test]
fn bug_report_160x48() {
    let mut app = TestApp::with_github(Config::default(), FakeGithub::new());
    app.dispatch(Action::SelectRepo(Box::new(lazy_github())));
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
    app.with_cx(|cx| {
        cx.open_dialog(dialog(lazy_github(), &fixed_env(), &fixed_config()), |_| {
            Action::Consumed
        });
    });
    insta::assert_snapshot!(buffer_to_styled_string(&app.render(160, 48)));
}

// `gh` failing mid-session (#323): the repo and gh's version.

mod gh_failures {
    use super::*;
    use crate::test_util::assert_gh_failures_surface;

    #[test]
    fn the_repo_and_the_gh_version() {
        assert_gh_failures_surface(
            || TestApp::with_github(Config::default(), gh()),
            run_bug_report,
            |app| {
                if !app.app().modals().is_empty() {
                    return Err(format!("{:?}", app.app().modals()));
                }
                // It can run again.
                let before = repo_requests(app).len();
                run_bug_report(app);
                (repo_requests(app).len() > before)
                    .then_some(())
                    .ok_or_else(|| "the command is stuck".into())
            },
        );
    }

    #[test]
    fn the_gh_version_for_debug_info() {
        assert_gh_failures_surface(
            || TestApp::with_github(Config::default(), gh()),
            |app| run_palette(app, "debug info"),
            |app| {
                if app.app().modals() == [ModalKind::DebugInfo] {
                    Ok(())
                } else {
                    Err(format!("{:?}", app.app().modals()))
                }
            },
        );
    }
}
