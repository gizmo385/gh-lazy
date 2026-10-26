use futures_util::FutureExt as _;
use lazy_github_core::config::{Config, IssueOwnerFilter, IssueStateFilter};
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::IssueListItem;

use super::*;
use crate::action::Action;
use crate::components::table::{Handled, TableEvent};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::pull_requests::{Kind, item, prs};
use crate::screens::main::repos::repo;
use crate::screens::main::{MainAction, SectionId};
use crate::test_util::{TestApp, logs};

fn lists_repo() -> Repository {
    repo("octo", "lists", false)
}

fn other_repo() -> Repository {
    repo("octo", "other", false)
}

/// An app whose repos table holds `octo/lists` and `octo/other`, with issue pages from `gh`.
fn app(config: Config, gh: FakeGithub) -> TestApp {
    TestApp::with_github(config, gh.with_repositories(vec![lists_repo(), other_repo()]))
}

fn lists(app: &TestApp) -> &IssueLists {
    &app.app().main_screen().expect("on the main screen").issue_lists
}

fn select(app: &mut TestApp, repo: Repository) {
    app.dispatch(Action::SelectRepo(Box::new(repo)));
}

/// Answers every queued request, and returns what that logged.
fn settle(app: &mut TestApp) -> String {
    logs(|| app.settle().now_or_never().expect("the fake answers at once"))
}

/// Every issues-endpoint request so far: its id, repo and query.
fn issue_requests(app: &TestApp) -> Vec<(RequestId, String, IssueListQuery)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::ListIssues { repo, query },
            } => Some((*id, repo.full_name.clone(), *query)),
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

fn pr_numbers(app: &TestApp) -> Vec<u64> {
    lists(app).prs_table().row_keys().copied().collect()
}

fn query(state: IssueStateFilter, owner: IssueOwnerFilter, page: u32) -> IssueListQuery {
    IssueListQuery {
        state,
        owner,
        page,
        per_page: 30,
    }
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: br#"{"message": "Server Error"}"#.to_vec(),
    }
}

/// Moves the PR table's cursor with `binding`, as a key press with the table focused would.
fn pr_cursor(app: &mut TestApp, binding: BindingId) {
    let screen = app.app_mut().main_screen_mut().expect("on the main screen");
    if let Handled::Event(TableEvent::LoadMore {
        generation,
        page,
        per_page,
    }) = screen.issue_lists.prs_table_mut().on_binding(binding)
    {
        app.dispatch(Action::Main(MainAction::LoadMore {
            section: SectionId::PullRequests,
            generation,
            page,
            per_page,
        }));
    }
}

#[test]
fn the_default_config_sends_one_shared_first_page() {
    let repo = lists_repo();
    let mixed = vec![
        item(&repo, 9, Kind::Issue),
        item(&repo, 8, Kind::ClosedPr),
        item(&repo, 7, Kind::Issue),
        item(&repo, 6, Kind::DraftPr),
    ];
    let mut app = app(
        Config::default(),
        FakeGithub::new().with_issue_page("octo/lists", 1, Ok(mixed)),
    );
    select(&mut app, repo);
    assert!(lists(&app).prs_table().is_loading());
    let requests = issue_requests(&app);
    let [(_, ref name, asked)] = requests[..] else {
        panic!("one first-page request: {requests:?}");
    };
    assert_eq!(name, "octo/lists");
    assert_eq!(asked, query(IssueStateFilter::All, IssueOwnerFilter::All, 1));
    assert_eq!(settle(&mut app), "");
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).prs_compact());
    // Only the PRs, highest number first.
    assert_eq!(pr_numbers(&app), [8, 6]);
}

#[test]
fn rows_sort_by_number_highest_first() {
    let repo = lists_repo();
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Ok(prs(&repo, [3, 12, 7])));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    assert_eq!(pr_numbers(&app), [12, 7, 3]);
}

#[test]
fn differing_filters_send_one_first_page_per_table() {
    let mut config = Config::default();
    config.pull_requests.state_filter = IssueStateFilter::Open;
    config.issues.owner_filter = IssueOwnerFilter::Mine;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, lists_repo());
    let queries: Vec<IssueListQuery> = issue_requests(&app).into_iter().map(|(.., query)| query).collect();
    // The PR table's own filters, not the issues ones (Py bug B3).
    assert_eq!(
        queries,
        [
            query(IssueStateFilter::Open, IssueOwnerFilter::All, 1),
            query(IssueStateFilter::All, IssueOwnerFilter::Mine, 1),
        ]
    );
    settle(&mut app);
    assert!(!lists(&app).prs_table().is_loading());
    assert!(lists(&app).prs_compact());
    assert_eq!(lists(&app).first_page, FirstPage::Idle);
}

#[test]
fn nothing_loads_when_neither_section_is_shown() {
    let mut config = Config::default();
    config.appearance.show_pull_requests = false;
    config.appearance.show_issues = false;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, lists_repo());
    assert_eq!(issue_requests(&app), []);
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).prs_compact());
}

#[test]
fn the_first_page_loads_when_only_issues_are_shown() {
    let mut config = Config::default();
    config.appearance.show_pull_requests = false;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, lists_repo());
    assert_eq!(issue_requests(&app).len(), 1);
}

#[test]
fn a_later_page_uses_the_pull_request_filters_and_the_loaded_repo() {
    let repo = lists_repo();
    let mut config = Config::default();
    config.pull_requests.state_filter = IssueStateFilter::Open;
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Ok(prs(&repo, (1..=30).rev())));
    let mut app = app(config, gh);
    select(&mut app, repo);
    settle(&mut app);
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    let requests = issue_requests(&app);
    let (_, name, asked) = requests.last().expect("a page request");
    assert_eq!(name, "octo/lists");
    assert_eq!(*asked, query(IssueStateFilter::Open, IssueOwnerFilter::All, 2));
}

#[test]
fn a_page_of_only_issues_pages_on_and_an_empty_page_ends_paging() {
    let repo = lists_repo();
    let issues: Vec<IssueListItem> = (31..=60).map(|n| item(&repo, n, Kind::Issue)).collect();
    let gh = FakeGithub::new()
        .with_issue_page("octo/lists", 1, Ok(prs(&repo, 1..=30)))
        .with_issue_page("octo/lists", 2, Ok(issues));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    settle(&mut app);
    let lazy = lists(&app).prs_table().lazy_load().expect("paging is on");
    assert!(lazy.can_load_more(), "page 2 had no PRs but wasn't empty (Py bug B44)");
    // Page 3 isn't in the fake, so it's empty: paging ends.
    pr_cursor(&mut app, BindingId::TableCursorUp);
    settle(&mut app);
    let lazy = lists(&app).prs_table().lazy_load().expect("paging is on");
    assert!(!lazy.can_load_more());
    let pages: Vec<u32> = issue_requests(&app).iter().map(|(.., query)| query.page).collect();
    assert_eq!(pages, [1, 2, 3]);
    assert_eq!(lists(&app).prs_table().row_count(), 30);
}

#[test]
fn an_http_error_logs_the_api_line_once_and_the_section_is_compact() {
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0], "ERROR API Error fetching issues and PRs from Github API",
        "{out}"
    );
    assert!(lines[1].starts_with("DEBUG Fetching issues and PRs failed: "), "{out}");
    assert_eq!(lines.len(), 2, "{out}");
    assert!(app.app().toasts().is_empty());
    assert!(!lists(&app).prs_table().is_loading());
    assert!(lists(&app).prs_compact());
}

#[test]
fn a_decode_error_logs_the_unexpected_line() {
    let decode = serde_json::from_str::<u8>("x").unwrap_err();
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Err(GhError::Decode(decode)));
    let mut app = app(Config::default(), gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert!(
        out.starts_with("ERROR Unexpected error fetching issues and PRs from Github API\n"),
        "{out}"
    );
    assert!(lists(&app).prs_compact());
}

#[test]
fn differing_filters_log_once_per_failed_request() {
    let mut config = Config::default();
    config.issues.state_filter = IssueStateFilter::Open;
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = app(config, gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR API Error").count(), 2, "{out}");
}

#[test]
fn a_failed_later_page_logs_keeps_the_rows_and_asks_again() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_issue_page("octo/lists", 1, Ok(prs(&repo, 1..=30)))
        .with_issue_page("octo/lists", 2, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    let out = settle(&mut app);
    assert!(
        out.starts_with("ERROR API Error fetching issues and PRs from Github API\n"),
        "{out}"
    );
    assert!(app.app().toasts().is_empty());
    assert_eq!(lists(&app).prs_table().row_count(), 30);
    pr_cursor(&mut app, BindingId::TableCursorUp);
    let pages: Vec<u32> = issue_requests(&app).iter().map(|(.., query)| query.page).collect();
    assert_eq!(pages, [1, 2, 2]);
}

#[test]
fn selecting_another_repo_cancels_the_requests_in_flight_and_drops_late_answers() {
    let repo = lists_repo();
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Ok(prs(&repo, 1..=30)));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo.clone());
    settle(&mut app);
    // A later page in flight, then a new first page in flight, then another repo.
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    let (page_2, ..) = *issue_requests(&app).last().unwrap();
    select(&mut app, repo.clone());
    let (first, ..) = *issue_requests(&app).last().unwrap();
    assert_eq!(cancels(&app), [page_2]);
    select(&mut app, other_repo());
    assert_eq!(cancels(&app), [page_2, first]);
    // Late answers for the cancelled requests change nothing.
    for id in [page_2, first] {
        app.send(Event::Github {
            id,
            response: GithubResponse::ListIssues(Ok(prs(&repo, 31..=60))),
        });
    }
    assert!(lists(&app).prs_table().is_loading());
    assert_eq!(lists(&app).prs_table().row_count(), 0);
    settle(&mut app);
    assert!(lists(&app).prs_compact(), "octo/other has no issues");
    assert_eq!(pr_numbers(&app), Vec::<u64>::new());
}
