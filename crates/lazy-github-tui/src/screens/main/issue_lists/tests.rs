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

/// The cancelled issue-list requests (the README request a selection also cancels is the overview's).
fn cancels(app: &TestApp) -> Vec<RequestId> {
    let issue_ids: Vec<RequestId> = issue_requests(app).iter().map(|(id, ..)| *id).collect();
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) if issue_ids.contains(id) => Some(*id),
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
    assert!(lists(&app).issues_compact());
    assert_eq!(issue_numbers(&app), Vec::<u64>::new());
}

// The Issues section (#172).

fn issue_numbers(app: &TestApp) -> Vec<u64> {
    lists(app).issues_table().row_keys().copied().collect()
}

/// Open issues numbered `numbers`, in that order.
fn issues(repo: &Repository, numbers: impl IntoIterator<Item = u64>) -> Vec<IssueListItem> {
    numbers.into_iter().map(|n| item(repo, n, Kind::Issue)).collect()
}

/// Moves the Issues table's cursor with `binding`, as a key press with the table focused would.
fn issue_cursor(app: &mut TestApp, binding: BindingId) {
    let screen = app.app_mut().main_screen_mut().expect("on the main screen");
    if let Handled::Event(TableEvent::LoadMore {
        generation,
        page,
        per_page,
    }) = screen.issue_lists.issues_table_mut().on_binding(binding)
    {
        app.dispatch(Action::Main(MainAction::LoadMore {
            section: SectionId::Issues,
            generation,
            page,
            per_page,
        }));
    }
}

#[test]
fn the_shared_first_page_feeds_issues_and_prs_to_their_tables_in_api_order() {
    let repo = lists_repo();
    let mixed = vec![
        item(&repo, 5, Kind::Issue),
        item(&repo, 8, Kind::ClosedPr),
        item(&repo, 9, Kind::Issue),
        item(&repo, 6, Kind::DraftPr),
        item(&repo, 7, Kind::ClosedIssue),
    ];
    let mut app = app(
        Config::default(),
        FakeGithub::new().with_issue_page("octo/lists", 1, Ok(mixed)),
    );
    select(&mut app, repo);
    assert!(lists(&app).issues_table().is_loading());
    assert_eq!(issue_requests(&app).len(), 1, "one request for both tables");
    settle(&mut app);
    // Each table holds its own kind in API order, and shows it highest number first.
    let issue_items: Vec<u64> = lists(&app).issues_table().items().map(|issue| issue.number).collect();
    let pr_items: Vec<u64> = lists(&app).prs_table().items().map(|pr| pr.issue.number).collect();
    assert_eq!(issue_items, [5, 9, 7]);
    assert_eq!(pr_items, [8, 6]);
    assert_eq!(issue_numbers(&app), [9, 7, 5]);
    assert_eq!(pr_numbers(&app), [8, 6]);
    assert!(!lists(&app).issues_table().is_loading());
    assert!(!lists(&app).issues_compact());
}

#[test]
fn a_first_page_of_only_prs_leaves_issues_compact() {
    let repo = lists_repo();
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Ok(prs(&repo, 1..=30)));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    assert!(lists(&app).issues_compact());
    assert!(!lists(&app).prs_compact());
    assert_eq!(lists(&app).issues_table().row_count(), 0);
}

#[test]
fn differing_filters_fill_the_issues_table_from_its_own_request() {
    let repo = lists_repo();
    let mut config = Config::default();
    config.issues.state_filter = IssueStateFilter::Closed;
    config.issues.owner_filter = IssueOwnerFilter::Mine;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, repo.clone());
    let requests = issue_requests(&app);
    let [(prs_id, ..), (issues_id, _, issues_query)] = requests[..] else {
        panic!("one first page per table: {requests:?}");
    };
    // The Issues table's own filters (Py bug B3).
    assert_eq!(issues_query, query(IssueStateFilter::Closed, IssueOwnerFilter::Mine, 1));
    // The PR request's issues don't reach the Issues table.
    app.send(Event::Github {
        id: prs_id,
        response: GithubResponse::ListIssues(Ok(vec![item(&repo, 4, Kind::Issue), item(&repo, 3, Kind::OpenPr)])),
    });
    assert_eq!(pr_numbers(&app), [3]);
    assert!(lists(&app).issues_table().is_loading());
    assert_eq!(issue_numbers(&app), Vec::<u64>::new());
    // And its own request's PRs don't reach the PR table.
    app.send(Event::Github {
        id: issues_id,
        response: GithubResponse::ListIssues(Ok(vec![
            item(&repo, 2, Kind::ClosedIssue),
            item(&repo, 1, Kind::OpenPr),
        ])),
    });
    assert_eq!(issue_numbers(&app), [2]);
    assert_eq!(pr_numbers(&app), [3]);
    assert!(!lists(&app).issues_table().is_loading());
    assert_eq!(lists(&app).first_page, FirstPage::Idle);
}

#[test]
fn a_later_issues_page_uses_the_issue_filters_and_the_loaded_repo() {
    let repo = lists_repo();
    let mut config = Config::default();
    config.issues.state_filter = IssueStateFilter::Open;
    config.issues.owner_filter = IssueOwnerFilter::Mine;
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Ok(issues(&repo, (1..=30).rev())));
    let mut app = app(config, gh);
    select(&mut app, repo);
    settle(&mut app);
    issue_cursor(&mut app, BindingId::TableScrollBottom);
    let requests = issue_requests(&app);
    let (_, name, asked) = requests.last().expect("a page request");
    assert_eq!(name, "octo/lists");
    assert_eq!(*asked, query(IssueStateFilter::Open, IssueOwnerFilter::Mine, 2));
}

#[test]
fn a_page_of_only_prs_keeps_the_issues_paging_on() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, 1..=30)))
        .with_issue_page("octo/lists", 2, Ok(prs(&repo, 31..=60)));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    issue_cursor(&mut app, BindingId::TableScrollBottom);
    settle(&mut app);
    let lazy = lists(&app).issues_table().lazy_load().expect("paging is on");
    assert!(
        lazy.can_load_more(),
        "page 2 had no issues but wasn't empty (Py bug B44)"
    );
    // Page 3 isn't in the fake, so it's empty: paging ends.
    issue_cursor(&mut app, BindingId::TableCursorUp);
    settle(&mut app);
    let lazy = lists(&app).issues_table().lazy_load().expect("paging is on");
    assert!(!lazy.can_load_more());
    assert_eq!(lists(&app).issues_table().row_count(), 30);
}

#[test]
fn a_failed_shared_request_logs_once_and_both_sections_are_compact() {
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR").count(), 1, "{out}");
    assert!(!lists(&app).issues_table().is_loading());
    assert!(lists(&app).issues_compact());
    assert!(lists(&app).prs_compact());
}

#[test]
fn a_failed_later_issues_page_logs_keeps_the_rows_and_asks_again() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, 1..=30)))
        .with_issue_page("octo/lists", 2, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    issue_cursor(&mut app, BindingId::TableScrollBottom);
    let out = settle(&mut app);
    assert!(
        out.starts_with("ERROR API Error fetching issues and PRs from Github API\n"),
        "{out}"
    );
    assert!(app.app().toasts().is_empty());
    assert_eq!(lists(&app).issues_table().row_count(), 30);
    issue_cursor(&mut app, BindingId::TableCursorUp);
    let pages: Vec<u32> = issue_requests(&app).iter().map(|(.., query)| query.page).collect();
    assert_eq!(pages, [1, 2, 2]);
}

// The tables' cache files (#228).

fn cache_config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = std::path::PathBuf::from("cache-dir");
    config
}

fn cache_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new("cache-dir").join(format!("octo_lists_{name}.json"))
}

/// The cache files' text for `items`: the PRs', then the issues'.
fn cache_texts(items: Vec<IssueListItem>) -> (String, String) {
    let (prs, issues): (Vec<_>, Vec<_>) = items
        .into_iter()
        .partition(|item| matches!(item, IssueListItem::PullRequest(_)));
    (
        lazy_github_core::cache::to_json(pull_requests_page(prs).items),
        lazy_github_core::cache::to_json(issues_page(issues).items),
    )
}

/// An app whose `octo/lists` cache files hold PRs 4 and 2 and issues 3 and 1, with issue pages from `gh`.
fn cached_app(gh: FakeGithub) -> TestApp {
    let repo = lists_repo();
    let (prs, issues) = cache_texts(vec![
        item(&repo, 4, Kind::OpenPr),
        item(&repo, 3, Kind::Issue),
        item(&repo, 2, Kind::DraftPr),
        item(&repo, 1, Kind::Issue),
    ]);
    app(cache_config(), gh)
        .with_cache_file(cache_path("pull_requests"), prs)
        .with_cache_file(cache_path("issues"), issues)
}

#[test]
fn the_shared_first_page_replaces_each_tables_cached_rows_and_writes_each_file() {
    let repo = lists_repo();
    let mixed = vec![
        item(&repo, 9, Kind::Issue),
        item(&repo, 8, Kind::ClosedPr),
        item(&repo, 3, Kind::Issue),
        item(&repo, 6, Kind::DraftPr),
    ];
    let mut app = cached_app(FakeGithub::new().with_issue_page("octo/lists", 1, Ok(mixed.clone())));
    select(&mut app, repo);
    assert_eq!(issue_requests(&app).len(), 1, "one request for both tables");
    assert_eq!(settle(&mut app), "");
    assert_eq!(pr_numbers(&app), [8, 6]);
    assert_eq!(issue_numbers(&app), [9, 3]);
    let (prs, issues) = cache_texts(mixed);
    assert_eq!(app.cache_file(&cache_path("pull_requests")), Some(prs.as_str()));
    assert_eq!(app.cache_file(&cache_path("issues")), Some(issues.as_str()));
}

#[test]
fn a_failed_shared_first_page_keeps_each_tables_cached_rows() {
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = cached_app(gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR").count(), 1, "{out}");
    assert_eq!(pr_numbers(&app), [4, 2]);
    assert_eq!(issue_numbers(&app), [3, 1]);
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).prs_compact());
    assert!(!lists(&app).issues_compact());
}
