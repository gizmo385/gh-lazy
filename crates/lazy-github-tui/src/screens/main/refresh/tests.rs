//! "Refresh from GitHub" (#407): the cache mark before any request, what each pane asks for again, cursors, the
//! active tab and focus kept, a second press replacing the first, and a Diff tab's unsent comments kept.

use std::path::Path;

use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Repository, WorkflowRun};

use super::super::pr_details::tests::{fixture_pr, fixture_repo};
use super::super::pull_requests::{Kind, item};
use super::super::repos::repo;
use super::super::workflows::{run, workflow};
use super::super::{FocusTarget, MainAction, MainScreen, SectionId, TabSwitch};
use crate::action::Action;
use crate::components::diff_view::{DiffLineRef, PendingComment};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::test_util::TestApp;

const FULL_NAME: &str = "parity-bot/pr-details";

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().expect("on the main screen")
}

fn settle(app: &mut TestApp) {
    app.settle().now_or_never().expect("the fake answers at once");
}

fn refresh(app: &mut TestApp) -> usize {
    let from = app.effects().len();
    app.dispatch(Action::Refresh);
    from
}

/// PR 5's diff, as the `pr-details` set answers it.
fn diff_5() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/GET.diff.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

/// PR 5 with `title`.
fn pr_5(title: &str) -> FullPullRequest {
    let mut pr = fixture_pr(5);
    pr.pr.issue.title = title.into();
    pr
}

fn the_run(id: u64, title: &str) -> WorkflowRun {
    run(
        &fixture_repo(),
        id,
        "CI",
        id,
        "2026-10-20T09:00:00Z",
        Some("success"),
        title,
    )
}

/// The pull requests (5 and 3) and issue 7 of the first page, then `extra` more items.
fn page(extra: &[IssueListItem]) -> Vec<IssueListItem> {
    let repo = fixture_repo();
    let mut items = vec![
        item(&repo, 5, Kind::OpenPr),
        item(&repo, 3, Kind::OpenPr),
        item(&repo, 7, Kind::Issue),
    ];
    items.extend_from_slice(extra);
    items
}

/// GitHub with `parity-bot/pr-details` and `octo/other`, two PRs and an issue, runs 2 and 1, PR 5, issue 7 and run
/// 2 to refetch.
fn github() -> FakeGithub {
    let fixture = fixture_repo();
    let issue_7 = item(&fixture, 7, Kind::Issue);
    FakeGithub::new()
        .with_repositories(vec![fixture.clone(), repo("octo", "other", false)])
        .with_repository(FULL_NAME, Ok(Some(fixture)))
        .with_list_page(FULL_NAME, 1, page(&[]))
        .with_workflow_page(FULL_NAME, 1, Ok(vec![workflow(1, "CI")]))
        .with_run_page(FULL_NAME, 1, Ok(vec![the_run(2, "Second"), the_run(1, "First")]))
        .with_pull_request(FULL_NAME, 5, Ok(pr_5("Before")))
        .with_pull_request_diff(FULL_NAME, 5, Ok(diff_5()))
        .with_issue(FULL_NAME, 7, Ok(issue_7))
        .with_workflow_run(2, Ok(the_run(2, "Second")))
}

/// An app with `parity-bot/pr-details` selected and loaded.
fn selected(config: Config) -> TestApp {
    let mut app = TestApp::with_github(config, github());
    settle(&mut app);
    app.dispatch(Action::SelectRepo(Box::new(fixture_repo())));
    settle(&mut app);
    app
}

/// [`selected`], with PR 5 picked from its table and its Conversation tab active.
fn pr_shown() -> TestApp {
    let mut app = selected(Config::default());
    app.press(&["2", "enter"]);
    settle(&mut app);
    assert_eq!(main(&app).details_title(), "[5] PR #5 Details");
    app.dispatch(Action::Main(MainAction::SwitchDetailsTab(TabSwitch::To(2))));
    settle(&mut app);
    app
}

/// The GitHub requests among the effects from `from` on.
fn requests(app: &TestApp, from: usize) -> Vec<(RequestId, GithubRequest)> {
    app.effects()[from..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect()
}

/// The requests from `from` on, as short names, notifications left out.
fn asked(app: &TestApp, from: usize) -> Vec<String> {
    requests(app, from)
        .into_iter()
        .filter_map(|(_, request)| {
            Some(match request {
                GithubRequest::ListRepositories(_) => "repos".to_owned(),
                GithubRequest::ListPullRequests { query, .. } => format!("pulls page {}", query.page),
                GithubRequest::ListIssues { query, .. } => format!("issues page {}", query.page),
                GithubRequest::ListWorkflows { .. } => "workflows".to_owned(),
                GithubRequest::ListWorkflowRuns { page, .. } => format!("runs page {}", page.page),
                GithubRequest::Repository(name) => format!("repo {name}"),
                GithubRequest::PullRequest { number, .. } => format!("pr {number}"),
                GithubRequest::Issue { number, .. } => format!("issue {number}"),
                GithubRequest::WorkflowRun { run_id, .. } => format!("run {run_id}"),
                GithubRequest::CheckAuth | GithubRequest::UnreadNotificationCount => return None,
                other => format!("{other:?}"),
            })
        })
        .collect()
}

fn cancelled(app: &TestApp, from: usize) -> Vec<RequestId> {
    app.effects()[from..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn the_cache_mark_comes_before_every_request() {
    let mut app = pr_shown();
    let from = refresh(&mut app);
    let effects = &app.effects()[from..];
    let mark = effects
        .iter()
        .position(|effect| *effect == Effect::RefreshGithubCache)
        .expect("the cache is marked");
    let first_request = effects
        .iter()
        .position(|effect| matches!(effect, Effect::Github { .. }))
        .expect("requests");
    assert!(mark < first_request, "{effects:?}");
    // The fake sees the mark before the reads the refresh makes.
    let calls_before = app.gh().calls().len();
    settle(&mut app);
    let calls = app.gh().calls();
    assert_eq!(calls[calls_before - 1], GithubCall::RefreshAll);
    assert!(matches!(calls[calls_before], GithubCall::ListRepositories(_)));
}

#[test]
fn with_a_pr_shown_every_pane_asks_again_and_tab_and_focus_stay() {
    let mut app = pr_shown();
    app.press(&["5"]);
    assert_eq!(main(&app).focus(), FocusTarget::DetailsTabs);
    // Clones of the fake share its canned answers, so this changes what the refresh gets.
    let _ = app.gh().clone().with_pull_request(FULL_NAME, 5, Ok(pr_5("After")));
    let from = refresh(&mut app);
    assert_eq!(
        asked(&app, from),
        [
            "repos",
            "pulls page 1",
            "issues page 1",
            "workflows",
            "runs page 1",
            "pr 5"
        ]
    );
    // Nothing changes until the pull request lands.
    assert_eq!(
        main(&app).details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("Before")
    );
    let from = app.effects().len();
    settle(&mut app);
    let main = main(&app);
    assert_eq!(
        main.details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("After")
    );
    assert_eq!(main.details_title(), "[5] PR #5 Details");
    assert_eq!(main.details.tabs().active(), 2);
    assert_eq!(main.focus(), FocusTarget::DetailsTabs);
    // Overview, Diff and Conversation load as on a pick.
    let asked = asked(&app, from);
    assert!(
        asked.iter().any(|request| request.starts_with("PullRequestDiff")),
        "{asked:?}"
    );
    assert!(
        asked.iter().any(|request| request.starts_with("IssueComments")),
        "{asked:?}"
    );
}

#[test]
fn with_nothing_selected_only_the_repos_reload() {
    let mut app = TestApp::with_github(Config::default(), github());
    settle(&mut app);
    let from = refresh(&mut app);
    assert_eq!(asked(&app, from), ["repos"]);
    assert!(!app.app().notifications_refreshing());

    let mut config = Config::default();
    config.notifications.enabled = true;
    let mut app = TestApp::with_github(config, github().with_unread_count(Ok(2)));
    settle(&mut app);
    let from = refresh(&mut app);
    assert_eq!(asked(&app, from), ["repos"]);
    assert!(app.app().notifications_refreshing());
}

#[test]
fn a_refresh_after_one_that_landed_reloads_again() {
    let mut app = pr_shown();
    refresh(&mut app);
    settle(&mut app);
    let _ = app.gh().clone().with_pull_request(FULL_NAME, 5, Ok(pr_5("Again")));
    let from = refresh(&mut app);
    settle(&mut app);
    assert_eq!(
        main(&app).details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("Again")
    );
    let asked = asked(&app, from);
    assert!(
        asked.iter().any(|request| request.starts_with("IssueComments")),
        "{asked:?}"
    );
}

#[test]
fn a_second_press_cancels_what_the_first_still_waits_for() {
    let mut app = pr_shown();
    let first = refresh(&mut app);
    let first_ids: Vec<RequestId> = requests(&app, first).into_iter().map(|(id, _)| id).collect();
    assert_eq!(first_ids.len(), 6);
    let second = refresh(&mut app);
    let cancelled = cancelled(&app, second);
    for id in &first_ids {
        assert!(cancelled.contains(id), "{id} isn't cancelled: {cancelled:?}");
    }
    assert_eq!(asked(&app, second).len(), 6);
}

#[test]
fn an_answer_from_before_the_press_doesnt_land() {
    let mut app = pr_shown();
    let first = refresh(&mut app);
    let first_requests = requests(&app, first);
    let _ = refresh(&mut app);
    let old_pr = first_requests
        .iter()
        .find_map(|(id, request)| matches!(request, GithubRequest::PullRequest { .. }).then_some(*id))
        .unwrap();
    let (old_pulls, query) = first_requests
        .iter()
        .find_map(|(id, request)| match request {
            GithubRequest::ListPullRequests { query, .. } => Some((*id, *query)),
            _ => None,
        })
        .unwrap();
    app.dispatch(Action::Github(
        old_pr,
        GithubResponse::PullRequest(Ok(Box::new(pr_5("Stale")))),
    ));
    app.dispatch(Action::Github(
        old_pulls,
        GithubResponse::ListPullRequests {
            query,
            result: Ok(vec![]),
        },
    ));
    let main = main(&app);
    assert_eq!(
        main.details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("Before")
    );
    assert_eq!(main.issue_lists.prs_table().row_count(), 2);
}

#[test]
fn a_diff_with_an_unsent_comment_keeps_it() {
    let mut app = pr_shown();
    let comment = PendingComment {
        path: "README.md".into(),
        line: DiffLineRef {
            file: 0,
            hunk: 0,
            line: 3,
        },
        position: 4,
        line_number: 6,
        line_text: "+Run `viewer FILE`, then press `j` and `k` to move.".into(),
        body: "Nice".into(),
    };
    app.dispatch(Action::Main(MainAction::DiffCommentAdded(Box::new(comment))));
    let pending = |app: &TestApp| {
        main(app)
            .details
            .pull_request()
            .and_then(|details| details.diff().view())
            .map(|view| view.pending().len())
    };
    assert_eq!(pending(&app), Some(1));
    let _ = app.gh().clone().with_pull_request(FULL_NAME, 5, Ok(pr_5("After")));
    refresh(&mut app);
    let from = app.effects().len();
    settle(&mut app);
    assert_eq!(pending(&app), Some(1));
    assert_eq!(
        main(&app).details_pull_request().map(|pr| pr.pr.issue.title.as_str()),
        Some("After")
    );
    let asked = asked(&app, from);
    assert!(
        !asked.iter().any(|request| request.starts_with("PullRequestDiff")),
        "{asked:?}"
    );
    assert!(
        asked.iter().any(|request| request.starts_with("IssueComments")),
        "{asked:?}"
    );
}

#[test]
fn the_repo_overview_an_issue_and_a_run_each_fetch_their_subject() {
    // The repo's overview: the repo, then its README.
    let mut app = selected(Config::default());
    let from = refresh(&mut app);
    assert!(asked(&app, from).contains(&format!("repo {FULL_NAME}")));
    let from = app.effects().len();
    settle(&mut app);
    assert!(
        asked(&app, from)
            .iter()
            .any(|request| request.starts_with("RepoReadme"))
    );
    assert!(main(&app).details_repo().is_some());

    // An issue: the issue, then its comments.
    let mut app = selected(Config::default());
    let IssueListItem::Issue(issue) = item(&fixture_repo(), 7, Kind::Issue) else {
        unreachable!("an issue kind")
    };
    app.dispatch(Action::Main(MainAction::OpenIssue(Box::new(issue))));
    settle(&mut app);
    app.dispatch(Action::Main(MainAction::SwitchDetailsTab(TabSwitch::To(1))));
    let from = refresh(&mut app);
    assert!(asked(&app, from).contains(&"issue 7".to_owned()));
    let from = app.effects().len();
    settle(&mut app);
    assert!(
        asked(&app, from)
            .iter()
            .any(|request| request.starts_with("IssueComments"))
    );
    assert_eq!(main(&app).details_issue().map(|issue| issue.number), Some(7));
    assert_eq!(main(&app).details.tabs().active(), 1);

    // A run: the run, then its jobs.
    let mut app = selected(Config::default());
    app.dispatch(Action::Main(MainAction::OpenWorkflowRun(Box::new(the_run(
        2, "Second",
    )))));
    settle(&mut app);
    let _ = app.gh().clone().with_workflow_run(2, Ok(the_run(2, "Renamed")));
    let from = refresh(&mut app);
    assert!(asked(&app, from).contains(&"run 2".to_owned()));
    let from = app.effects().len();
    settle(&mut app);
    assert!(
        asked(&app, from)
            .iter()
            .any(|request| request.starts_with("WorkflowRunJobs"))
    );
    assert_eq!(
        main(&app).details_run().map(|run| run.display_title.as_str()),
        Some("Renamed")
    );
}

#[test]
fn a_subject_from_another_repo_is_fetched_from_its_own() {
    let mut app = selected(Config::default());
    let other: Repository = repo("octo", "other", false);
    let mut run = the_run(9, "Elsewhere");
    run.repository = other.clone();
    app.dispatch(Action::Main(MainAction::OpenWorkflowRun(Box::new(run))));
    settle(&mut app);
    let from = refresh(&mut app);
    let asked_from = requests(&app, from).into_iter().find_map(|(_, request)| match request {
        GithubRequest::WorkflowRun { repo, run_id: 9 } => Some(repo.full_name),
        _ => None,
    });
    assert_eq!(asked_from.as_deref(), Some("octo/other"));
}

#[test]
fn cursors_stay_on_the_same_repo_number_and_run() {
    let mut app = selected(Config::default());
    // The repos table: the cursor on octo/other, which a new repo pushes down.
    app.press(&["1", "down"]);
    assert_eq!(
        main(&app).repos.table.selected().map(|repo| repo.full_name.as_str()),
        Some("octo/other")
    );
    // The PR table: on PR 3, which a new PR 6 pushes down.
    app.press(&["2", "down"]);
    assert_eq!(
        main(&app).issue_lists.prs_table().selected().map(|pr| pr.issue.number),
        Some(3)
    );
    // The runs: on run 1, below a new run 3.
    app.press(&["4", "down"]);
    assert_eq!(main(&app).workflows.runs_table().selected().map(|run| run.id), Some(1));

    let fixture = fixture_repo();
    let _ = app
        .gh()
        .clone()
        .with_repositories(vec![
            repo("octo", "aaa", false),
            fixture.clone(),
            repo("octo", "other", false),
        ])
        .with_list_page(FULL_NAME, 1, {
            let mut items = vec![item(&fixture, 6, Kind::OpenPr)];
            items.extend(page(&[]));
            items
        })
        .with_run_page(
            FULL_NAME,
            1,
            Ok(vec![the_run(3, "Third"), the_run(2, "Second"), the_run(1, "First")]),
        );
    refresh(&mut app);
    settle(&mut app);
    let main = main(&app);
    assert_eq!(main.repos.table.row_count(), 3);
    assert_eq!(
        main.repos.table.selected().map(|repo| repo.full_name.as_str()),
        Some("octo/other")
    );
    assert_eq!(main.issue_lists.prs_table().row_count(), 3);
    assert_eq!(
        main.issue_lists.prs_table().selected().map(|pr| pr.issue.number),
        Some(3)
    );
    assert_eq!(main.workflows.runs_table().row_count(), 3);
    assert_eq!(main.workflows.runs_table().selected().map(|run| run.id), Some(1));
    assert_eq!(main.focus(), FocusTarget::Table(SectionId::Workflows));
}

#[test]
fn a_cursor_whose_row_is_gone_goes_to_the_top() {
    let mut app = selected(Config::default());
    app.press(&["2", "down"]);
    assert_eq!(
        main(&app).issue_lists.prs_table().selected().map(|pr| pr.issue.number),
        Some(3)
    );
    let repo = fixture_repo();
    let _ = app.gh().clone().with_list_page(
        FULL_NAME,
        1,
        vec![item(&repo, 5, Kind::OpenPr), item(&repo, 8, Kind::OpenPr)],
    );
    refresh(&mut app);
    settle(&mut app);
    let table = main(&app).issue_lists.prs_table();
    assert_eq!(table.cursor(), 0);
}

#[test]
fn the_rows_stay_until_the_new_page_lands() {
    let mut app = selected(Config::default());
    refresh(&mut app);
    let main = main(&app);
    assert_eq!(main.repos.table.row_count(), 2);
    assert_eq!(main.issue_lists.prs_table().row_count(), 2);
    assert_eq!(main.workflows.runs_table().row_count(), 2);
    assert!(!main.workflows.workflows().is_empty());
}

#[test]
fn ctrl_r_refreshes_from_any_focus_and_is_rebindable() {
    for focus in ["1", "2", "5"] {
        let mut app = selected(Config::default());
        app.press(&[focus]);
        let from = app.effects().len();
        app.press(&["ctrl+r"]);
        assert!(
            app.effects()[from..].contains(&Effect::RefreshGithubCache),
            "focus {focus}"
        );
    }
    // The table's search box has focus.
    let mut app = selected(Config::default());
    app.press(&["1", "/"]);
    assert_eq!(main(&app).focus(), FocusTarget::Search(SectionId::Repos));
    let from = app.effects().len();
    app.press(&["ctrl+r"]);
    assert!(app.effects()[from..].contains(&Effect::RefreshGithubCache));

    let mut config = Config::default();
    config.bindings.overrides.insert("main.refresh".into(), "f5".into());
    let mut app = selected(config);
    let from = app.effects().len();
    app.press(&["ctrl+r"]);
    assert!(!app.effects()[from..].contains(&Effect::RefreshGithubCache));
    app.press(&["f5"]);
    assert!(app.effects()[from..].contains(&Effect::RefreshGithubCache));
}

#[test]
fn ctrl_r_is_mark_all_as_read_in_the_notifications_modal() {
    let mut config = Config::default();
    config.notifications.enabled = true;
    let mut app = TestApp::with_github(config, github().with_unread_count(Ok(2)));
    settle(&mut app);
    app.press(&["ctrl+n"]);
    settle(&mut app);
    let from = app.effects().len();
    app.press(&["ctrl+r"]);
    assert!(!app.effects()[from..].contains(&Effect::RefreshGithubCache));
}

#[test]
fn the_key_is_neither_in_the_footer_nor_in_the_help_panel() {
    let mut app = selected(Config::default());
    let footer = app.screen(160, 48);
    assert!(!footer.contains("Refresh from GitHub"));
    app.press(&["?"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Cursor up"), "the help panel is open");
    assert!(!screen.contains("Refresh"), "{screen}");
}
