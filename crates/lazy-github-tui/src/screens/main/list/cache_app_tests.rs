//! The list sections' cache files through the whole app: what a repo selection shows when its fetches fail.

use std::path::{Path, PathBuf};

use futures_util::FutureExt as _;
use lazy_github_core::cache;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{IssueListItem, Repository};

use crate::action::Action;
use crate::screens::main::Toggle;
use crate::screens::main::pull_requests::{Kind, item};
use crate::screens::main::repos::repo;
use crate::screens::main::workflows::run;
use crate::test_util::{TestApp, logs};

fn cache_config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = PathBuf::from("cache-dir");
    config
}

fn cache_path(name: &str) -> PathBuf {
    Path::new("cache-dir").join(format!("octo_one_{name}.json"))
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
    }
}

/// `octo/one`'s cache files: three PRs, three issues and three runs.
fn with_caches(app: TestApp, repo: &Repository) -> TestApp {
    let mut prs = Vec::new();
    let mut issues = Vec::new();
    for (number, kind) in [
        (6, Kind::OpenPr),
        (5, Kind::Issue),
        (4, Kind::DraftPr),
        (3, Kind::ClosedIssue),
        (2, Kind::ClosedPr),
        (1, Kind::Issue),
    ] {
        match item(repo, number, kind) {
            IssueListItem::PullRequest(pr) => prs.push(pr),
            IssueListItem::Issue(issue) => issues.push(issue),
        }
    }
    let runs = vec![
        run(
            repo,
            3,
            "CI",
            3,
            "2026-10-21T09:00:00Z",
            Some("success"),
            "Cached run 3",
        ),
        run(
            repo,
            2,
            "CI",
            2,
            "2026-10-20T09:00:00Z",
            Some("failure"),
            "Cached run 2",
        ),
        run(repo, 1, "Lint", 1, "2026-10-19T09:00:00Z", None, "Cached run 1"),
    ];
    app.with_cache_file(cache_path("pull_requests"), cache::to_json(prs))
        .with_cache_file(cache_path("issues"), cache::to_json(issues))
        .with_cache_file(cache_path("workflow_runs"), cache::to_json(runs))
}

#[test]
fn restored_sections_after_failed_fetches_100x30() {
    let repo = repo("octo", "one", false);
    let gh = FakeGithub::new()
        .with_repositories(vec![
            repo.clone(),
            crate::screens::main::repos::repo("octo", "two", false),
        ])
        .with_issue_page("octo/one", 1, Err(server_error()))
        .with_run_page("octo/one", 1, Err(server_error()));
    let mut app = with_caches(TestApp::with_github(cache_config(), gh), &repo);
    app.press(&["enter"]);
    let out = logs(|| app.settle().now_or_never().expect("the fake answers at once"));
    assert!(
        out.contains("ERROR API Error fetching issues and PRs from Github API"),
        "{out}"
    );
    assert!(
        out.contains("ERROR Error retrieving action runs from the Github API"),
        "{out}"
    );
    let screen = app.screen(100, 30);
    for title in ["[2] Pull Requests ", "[3] Issues ", "[4] Workflows "] {
        assert!(screen.contains(title), "{title}\n{screen}");
    }
    assert!(!screen.contains("(empty)"), "{screen}");
    insta::assert_snapshot!(screen);
    // A failed fetch rewrites what was read.
    let runs: Vec<lazy_github_core::models::WorkflowRun> = cache::try_parse(
        Path::new("runs"),
        app.cache_file(&cache_path("workflow_runs")).unwrap(),
        "WorkflowRun",
    )
    .unwrap();
    assert_eq!(runs.iter().map(|run| run.id).collect::<Vec<_>>(), [3, 2, 1]);
}

/// The rest of `caches-drafts.workflow-runs-json`: the palette's "Toggle Workflows" reads the runs cache when it
/// shows the section again, as Python's `action_toggle_ui` does with `initialize_tables_from_cache`.
#[test]
fn showing_workflows_again_restores_the_runs_from_the_cache() {
    let repo = repo("octo", "one", false);
    let gh = FakeGithub::new()
        .with_repositories(vec![repo.clone()])
        .with_run_page("octo/one", 1, Err(server_error()));
    let mut config = cache_config();
    config.appearance.show_workflows = false;
    let mut app = with_caches(TestApp::with_github(config, gh), &repo);
    app.press(&["enter"]);
    logs(|| app.settle().now_or_never().expect("the fake answers at once"));
    assert!(!app.screen(100, 30).contains("[4] Workflows"));
    app.dispatch(Action::Toggle(Toggle::Workflows));
    logs(|| app.settle().now_or_never().expect("the fake answers at once"));
    let screen = app.screen(100, 30);
    for text in [
        "[4] Workflows ",
        "2026-10-21 09:00",
        "2026-10-20 09:00",
        "2026-10-19 09:00",
    ] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
}
