//! Preloading the pull request for the working directory's commit when a repo loads (#198), against the `pr-preload`
//! and `pr-details` fixtures.

use std::path::Path;

use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullPullRequest, PartialPullRequest, Repository};
use serde_json::Value;

use super::pull_requests::{Kind, item};
use super::repos::repo;
use super::*;
use crate::effect::{Effect, GitRequest};
use crate::event::Event;
use crate::test_util::{TestApp, assert_gh_failures_surface, logs};
use crate::toast::Severity;

/// The harness's commit, which the fixtures look up.
const SHA: &str = "77a3f2ceb72bcca424cae588f347fc970b288bbf";

/// The response body of the parity fixture at `path`.
fn fixture_body(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot/pr-details`, as its repos page lists it: default branch `main`.
fn pr_details() -> Repository {
    let page = fixture_body("pr-details/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

fn other() -> Repository {
    repo("octo", "other", false)
}

/// The commit's pull requests as `set` answers them, tagged with a repo that isn't the one asked, so a test sees the
/// one asked replace it.
fn commit_prs(set: &str) -> Vec<PartialPullRequest> {
    let body = fixture_body(&format!(
        "{set}/repos/parity-bot/pr-details/commits/{SHA}/pulls/GET.json"
    ));
    body.as_array()
        .unwrap()
        .iter()
        .map(|pr| PartialPullRequest::from_api(pr.clone(), &other()).unwrap())
        .collect()
}

fn full_pr(number: u64) -> FullPullRequest {
    let body = fixture_body(&format!(
        "pr-details/repos/parity-bot/pr-details/pulls/{number}/GET.json"
    ));
    FullPullRequest::from_api(body, &pr_details()).unwrap()
}

fn http(status: u16) -> GhError {
    GhError::Http {
        status,
        message: String::new(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// GitHub with `pr-details` (PR 7 first in its list, then 5) and `octo/other`, the commit's lookup in `pr-details`
/// answering `prs`.
fn github(prs: Result<Vec<PartialPullRequest>, GhError>) -> FakeGithub {
    let repo = pr_details();
    FakeGithub::new()
        .with_repositories(vec![other(), pr_details()])
        .with_issue_page(
            "parity-bot/pr-details",
            1,
            Ok(vec![item(&repo, 7, Kind::OpenPr), item(&repo, 5, Kind::OpenPr)]),
        )
        .with_commit_pull_requests("parity-bot/pr-details", SHA, prs)
        .with_pull_request("parity-bot/pr-details", 5, Ok(full_pr(5)))
}

fn config(edit: impl FnOnce(&mut Config)) -> Config {
    let mut config = Config::default();
    config.pull_requests.preload_pull_request_for_current_commit = true;
    edit(&mut config);
    config
}

/// The main screen with `config`, `gh`, on branch `new-feature` at [`SHA`], loaded.
fn app_with(config: Config, gh: FakeGithub, branch: Option<&str>, commit: Option<&str>) -> TestApp {
    let mut app = TestApp::with_github(config, gh)
        .with_local_branch(branch)
        .with_local_commit(commit);
    settle(&mut app);
    app
}

fn app(prs: Result<Vec<PartialPullRequest>, GhError>) -> TestApp {
    app_with(config(|_| {}), github(prs), Some("new-feature"), Some(SHA))
}

fn settle(app: &mut TestApp) {
    app.settle().now_or_never().expect("the fake answers at once");
}

fn select(app: &mut TestApp, repo: Repository) {
    app.dispatch(Action::SelectRepo(Box::new(repo)));
    settle(app);
}

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().unwrap()
}

/// The commits looked up, by repo.
fn lookups(app: &TestApp) -> Vec<String> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::PullRequestsForCommit { full_name, sha } => {
                assert_eq!(sha, SHA);
                Some(full_name)
            }
            _ => None,
        })
        .collect()
}

/// The pull requests fetched, by repo and number.
fn fetched(app: &TestApp) -> Vec<(String, u64)> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::PullRequest { full_name, number } => Some((full_name, number)),
            _ => None,
        })
        .collect()
}

fn head_commit_asks(app: &TestApp) -> usize {
    app.effects()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Git {
                    request: GitRequest::HeadCommit,
                    ..
                }
            )
        })
        .count()
}

fn toasts(app: &TestApp) -> Vec<(Severity, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.message.to_string()))
        .collect()
}

/// The id of the last lookup asked.
fn lookup_id(app: &TestApp) -> RequestId {
    app.effects()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::PullRequestsForCommit { .. },
            } => Some(*id),
            _ => None,
        })
        .expect("a lookup")
}

#[test]
fn the_one_pr_for_the_commit_opens_without_moving_focus() {
    let mut app = app(Ok(commit_prs("pr-preload")));
    let out = logs(|| select(&mut app, pr_details()));
    assert!(out.contains("INFO Loading PR for your current commit"), "{out}");
    assert_eq!(lookups(&app), ["parity-bot/pr-details"]);
    // The PR carries the repo asked, not the one the answer was built with.
    assert_eq!(fetched(&app), [("parity-bot/pr-details".to_owned(), 5)]);
    let screen = main(&app);
    assert_eq!(screen.details_title(), "[5] PR #5 Details");
    assert_eq!(screen.details_tabs(), ["Overview", "Diff", "Conversation"]);
    assert_eq!(screen.focus(), FocusTarget::Table(SectionId::Repos));
    // The PR table's cursor stays on its first row.
    assert_eq!(
        screen.issue_lists.prs_table().selected().map(|pr| pr.issue.number),
        Some(7)
    );
    assert!(toasts(&app).is_empty());
}

#[test]
fn a_preloaded_pr_leaves_focus_on_the_repos() {
    let mut app = app(Ok(commit_prs("pr-preload")));
    select(&mut app, pr_details());
    insta::assert_snapshot!(app.screen(160, 48));
}

/// A case's name, config, the repo selected, and the working directory's branch and commit.
type Case = (
    &'static str,
    Config,
    Repository,
    Option<&'static str>,
    Option<&'static str>,
);

#[test]
fn no_lookup_unless_every_condition_holds() {
    let no_default = Repository {
        default_branch: None,
        ..pr_details()
    };
    let cases: [Case; 5] = [
        (
            "setting off",
            Config::default(),
            pr_details(),
            Some("new-feature"),
            Some(SHA),
        ),
        (
            "PRs and issues hidden",
            config(|c| {
                c.appearance.show_pull_requests = false;
                c.appearance.show_issues = false;
                c.appearance.show_workflows = true;
            }),
            pr_details(),
            Some("new-feature"),
            Some(SHA),
        ),
        (
            "no default branch",
            config(|_| {}),
            no_default,
            Some("new-feature"),
            Some(SHA),
        ),
        (
            "on the default branch",
            config(|_| {}),
            pr_details(),
            Some("main"),
            Some(SHA),
        ),
        ("no commit", config(|_| {}), pr_details(), Some("new-feature"), None),
    ];
    for (name, config, repo, branch, commit) in cases {
        let mut app = app_with(config, github(Ok(commit_prs("pr-preload"))), branch, commit);
        select(&mut app, repo);
        assert!(lookups(&app).is_empty(), "{name}");
        assert!(fetched(&app).is_empty(), "{name}");
        assert_eq!(
            main(&app).details_title(),
            "[5] parity-bot/pr-details Details",
            "{name}"
        );
    }
}

#[test]
fn the_lookup_runs_with_only_issues_shown_and_without_a_named_branch() {
    let only_issues = config(|c| c.appearance.show_pull_requests = false);
    let cases: [(&str, Config, Option<&str>); 3] = [
        ("only issues shown", only_issues, Some("new-feature")),
        ("no branch", config(|_| {}), None),
        ("detached HEAD", config(|_| {}), Some("HEAD")),
    ];
    for (name, config, branch) in cases {
        let mut app = app_with(config, github(Ok(commit_prs("pr-preload"))), branch, Some(SHA));
        select(&mut app, pr_details());
        assert_eq!(lookups(&app), ["parity-bot/pr-details"], "{name}");
        assert_eq!(main(&app).details_title(), "[5] PR #5 Details", "{name}");
    }
}

#[test]
fn a_selection_before_git_answers_looks_up_once_it_does_and_the_commit_is_asked_once() {
    let mut app = TestApp::with_github(config(|_| {}), github(Ok(commit_prs("pr-preload"))))
        .with_local_branch(Some("new-feature"))
        .with_local_commit(Some(SHA));
    // The startup auto-select can come before git's answers: the preload waits for them.
    app.dispatch(Action::SelectRepo(Box::new(pr_details())));
    assert_eq!(head_commit_asks(&app), 1);
    assert!(!app.effects().iter().any(|effect| matches!(
        effect,
        Effect::Github {
            request: GithubRequest::PullRequestsForCommit { .. },
            ..
        }
    )));
    settle(&mut app);
    assert_eq!(lookups(&app), ["parity-bot/pr-details"]);
    assert_eq!(main(&app).details_title(), "[5] PR #5 Details");

    // Later selections use the commit already read, and look it up at once.
    let before = app.effects().len();
    app.dispatch(Action::SelectRepo(Box::new(other())));
    assert!(app.effects()[before..].iter().any(|effect| matches!(
        effect,
        Effect::Github {
            request: GithubRequest::PullRequestsForCommit { .. },
            ..
        }
    )));
    settle(&mut app);
    assert_eq!(lookups(&app), ["parity-bot/pr-details", "octo/other"]);
    assert_eq!(head_commit_asks(&app), 1);
}

#[test]
fn no_commit_is_read_while_the_setting_is_off() {
    let mut app = app_with(
        Config::default(),
        github(Ok(commit_prs("pr-preload"))),
        Some("new-feature"),
        Some(SHA),
    );
    select(&mut app, pr_details());
    assert_eq!(head_commit_asks(&app), 0);
}

#[test]
fn none_or_several_prs_leave_the_repo_overview() {
    for (name, prs) in [("none", Vec::new()), ("two", commit_prs("pr-preload-skip"))] {
        let mut app = app(Ok(prs));
        let out = logs(|| select(&mut app, pr_details()));
        assert!(!out.contains("Loading PR for your current commit"), "{name}: {out}");
        assert_eq!(lookups(&app), ["parity-bot/pr-details"], "{name}");
        assert!(fetched(&app).is_empty(), "{name}");
        assert_eq!(
            main(&app).details_title(),
            "[5] parity-bot/pr-details Details",
            "{name}"
        );
    }
}

#[test]
fn a_failed_lookup_logs_an_error_without_a_toast() {
    let mut app = app(Err(http(500)));
    let out = logs(|| select(&mut app, pr_details()));
    assert!(out.contains("ERROR Error loading PRs for current commit"), "{out}");
    assert!(toasts(&app).is_empty());
    assert_eq!(main(&app).details_title(), "[5] parity-bot/pr-details Details");
}

/// Py bug B110's answer: GitHub's 422 for a commit the repo hasn't got is "no PRs", logged at debug only.
#[test]
fn a_commit_the_repo_has_not_got_is_no_prs() {
    let mut app = app(Err(http(422)));
    let out = logs(|| select(&mut app, pr_details()));
    assert!(!out.contains("ERROR"), "{out}");
    assert!(
        out.contains("DEBUG No preload: the current commit isn't in the selected repo"),
        "{out}"
    );
    assert!(toasts(&app).is_empty());
    assert_eq!(main(&app).details_title(), "[5] parity-bot/pr-details Details");
}

#[test]
fn a_failed_pr_fetch_toasts_as_a_table_selection_does() {
    let gh = github(Ok(commit_prs("pr-preload"))).with_pull_request("parity-bot/pr-details", 5, Err(http(500)));
    let mut app = app_with(config(|_| {}), gh, Some("new-feature"), Some(SHA));
    select(&mut app, pr_details());
    assert_eq!(
        toasts(&app),
        [(Severity::Error, "Error fetching pull request!".to_owned())]
    );
    assert_eq!(main(&app).details_title(), "[5] parity-bot/pr-details Details");
}

/// Newer selections, each replacing whatever the preload had under way.
fn newer_selections() -> Vec<(&'static str, Action)> {
    let repo = pr_details();
    let issue = item(&repo, 3, Kind::Issue).into_issue().unwrap();
    let row = item(&repo, 4, Kind::OpenPr).into_pull_request().unwrap();
    vec![
        ("a repo", Action::SelectRepo(Box::new(other()))),
        (
            "a PR row",
            Action::Main(MainAction::OpenPullRequest {
                target: PullRequestTarget::Full(Box::new(full_pr(4))),
                focus: true,
            }),
        ),
        (
            "a PR row to fetch",
            Action::Main(MainAction::OpenPullRequest {
                target: PullRequestTarget::Partial(Box::new(row)),
                focus: true,
            }),
        ),
        ("an issue", Action::Main(MainAction::OpenIssue(Box::new(issue)))),
        (
            "a run",
            Action::Main(MainAction::OpenWorkflowRun(Box::new(
                super::run_details::tests::fixture_run(42),
            ))),
        ),
    ]
}

/// The details title each of [`newer_selections`] leaves.
fn newer_title(name: &str) -> &'static str {
    match name {
        "a repo" => "[5] octo/other Details",
        "a PR row" | "a PR row to fetch" => "[5] PR #4 Details",
        "an issue" => "[5] Issue #3 Details",
        _ => "[5] Workflow Run #42 Details",
    }
}

/// Py bug B109's answer: a selection made while the lookup runs cancels it, and its late answer is dropped.
#[test]
fn a_newer_selection_drops_the_lookup() {
    for (name, action) in newer_selections() {
        let gh = github(Ok(commit_prs("pr-preload"))).with_pull_request("parity-bot/pr-details", 4, Ok(full_pr(4)));
        let mut app = app_with(config(|_| {}), gh, Some("new-feature"), Some(SHA));
        // A first selection reads the commit, so the next one looks it up at once.
        select(&mut app, other());
        app.dispatch(Action::SelectRepo(Box::new(pr_details())));
        let id = lookup_id(&app);
        app.dispatch(action);
        assert!(app.effects().contains(&Effect::Cancel(id)), "{name}");
        // The lookup answers anyway, late.
        settle(&mut app);
        assert!(
            !fetched(&app).contains(&("parity-bot/pr-details".to_owned(), 5)),
            "{name}"
        );
        assert_eq!(main(&app).details_title(), newer_title(name), "{name}");
    }
}

/// Py bug B109's answer: a selection made while the preloaded PR is fetched cancels the fetch.
#[test]
fn a_newer_selection_drops_the_preloaded_fetch() {
    for (name, action) in newer_selections() {
        let gh = github(Ok(commit_prs("pr-preload"))).with_pull_request("parity-bot/pr-details", 4, Ok(full_pr(4)));
        let mut app = app_with(config(|_| {}), gh, Some("new-feature"), Some(SHA));
        // A first selection reads the commit, so the next one looks it up at once.
        select(&mut app, other());
        app.dispatch(Action::SelectRepo(Box::new(pr_details())));
        // The lookup answers ahead of the queue, so the PR fetch is in flight.
        let id = lookup_id(&app);
        let answer = GithubResponse::PullRequestsForCommit(Ok(commit_prs("pr-preload")));
        app.send(Event::Github { id, response: answer });
        let fetch = app
            .effects()
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::Github {
                    id,
                    request: GithubRequest::PullRequest { number: 5, .. },
                } => Some(*id),
                _ => None,
            })
            .expect("the preloaded PR's fetch");
        app.dispatch(action);
        assert!(app.effects().contains(&Effect::Cancel(fetch)), "{name}");
        settle(&mut app);
        assert_eq!(main(&app).details_title(), newer_title(name), "{name}");
    }
}

/// A later repo load checks again: here the second repo's commit lookup preloads nothing, and the first's PR stays
/// replaced by that repo's overview.
#[test]
fn every_repo_load_checks_again() {
    let mut app = app(Ok(commit_prs("pr-preload")));
    select(&mut app, pr_details());
    assert_eq!(main(&app).details_title(), "[5] PR #5 Details");
    select(&mut app, other());
    assert_eq!(lookups(&app), ["parity-bot/pr-details", "octo/other"]);
    assert_eq!(main(&app).details_title(), "[5] octo/other Details");
    select(&mut app, pr_details());
    assert_eq!(main(&app).details_title(), "[5] PR #5 Details");
}

/// #323: `gh` failing mid-session fails the lookup with a log line, and leaves nothing loading.
#[test]
fn gh_failures_surface() {
    assert_gh_failures_surface(
        || {
            let mut app = app(Ok(commit_prs("pr-preload")));
            select(&mut app, other());
            app
        },
        |app| {
            app.dispatch(Action::SelectRepo(Box::new(pr_details())));
        },
        |app| {
            if fetched(app).is_empty() {
                Ok(())
            } else {
                Err(format!("a PR was fetched: {:?}", fetched(app)))
            }
        },
    );
}
