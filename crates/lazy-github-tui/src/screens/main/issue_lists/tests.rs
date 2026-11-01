use futures_util::FutureExt as _;
use lazy_github_core::config::{Config, IssueOwnerFilter, IssueStateFilter};
use lazy_github_core::github::{FakeGithub, IssueListQuery};
use lazy_github_core::models::IssueListItem;

use super::*;
use crate::action::Action;
use crate::components::table::{Handled, TableEvent};
use crate::effect::{Effect, GithubRequest};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::issues::issues_page;
use crate::screens::main::pull_requests::{Kind, item, pr, prs};
use crate::screens::main::repos::repo;
use crate::screens::main::{MainAction, SectionId};
use crate::test_util::{TestApp, logs};

fn lists_repo() -> Repository {
    repo("octo", "lists", false)
}

fn other_repo() -> Repository {
    repo("octo", "other", false)
}

/// An app whose repos table holds `octo/lists` and `octo/other`, with issue and pull request pages from `gh`.
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

/// Which table a list request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
    Prs,
    Issues,
}

/// Every PR and issue list request so far: its id, table, repo and query.
fn list_requests(app: &TestApp) -> Vec<(RequestId, Table, String, IssueListQuery)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::ListPullRequests { repo, query },
            } => Some((*id, Table::Prs, repo.full_name.clone(), *query)),
            Effect::Github {
                id,
                request: GithubRequest::ListIssues { repo, query },
            } => Some((*id, Table::Issues, repo.full_name.clone(), *query)),
            _ => None,
        })
        .collect()
}

/// The requests for one table, as [`list_requests`].
fn table_requests(app: &TestApp, table: Table) -> Vec<(RequestId, String, IssueListQuery)> {
    list_requests(app)
        .into_iter()
        .filter(|(_, asked, ..)| *asked == table)
        .map(|(id, _, name, query)| (id, name, query))
        .collect()
}

/// The pages one table asked for, in order.
fn pages(app: &TestApp, table: Table) -> Vec<u32> {
    table_requests(app, table)
        .iter()
        .map(|(.., query)| query.page)
        .collect()
}

/// The cancelled list requests (the README request a selection also cancels is the overview's).
fn cancels(app: &TestApp) -> Vec<RequestId> {
    let list_ids: Vec<RequestId> = list_requests(app).iter().map(|(id, ..)| *id).collect();
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) if list_ids.contains(id) => Some(*id),
            _ => None,
        })
        .collect()
}

fn pr_numbers(app: &TestApp) -> Vec<u64> {
    lists(app).prs_table().row_keys().copied().collect()
}

fn issue_numbers(app: &TestApp) -> Vec<u64> {
    lists(app).issues_table().row_keys().copied().collect()
}

/// Open issues numbered `numbers`, in that order.
fn issues(repo: &Repository, numbers: impl IntoIterator<Item = u64>) -> Vec<IssueListItem> {
    numbers.into_iter().map(|n| item(repo, n, Kind::Issue)).collect()
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
        headers: Vec::new(),
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

// The first pages (#384: one per table, Py bugs B3 and B180).

#[test]
fn the_default_config_sends_a_first_page_per_table_with_its_own_filters() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_pull_request_page(
            "octo/lists",
            1,
            Ok(vec![pr(&repo, 8, Kind::ClosedPr), pr(&repo, 6, Kind::DraftPr)]),
        )
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, [9, 7])));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    assert!(lists(&app).prs_table().is_loading());
    assert!(lists(&app).issues_table().is_loading());
    let requests = list_requests(&app);
    let all = query(IssueStateFilter::All, IssueOwnerFilter::All, 1);
    assert_eq!(
        requests
            .iter()
            .map(|(_, table, name, query)| (*table, name.as_str(), *query))
            .collect::<Vec<_>>(),
        [(Table::Prs, "octo/lists", all), (Table::Issues, "octo/lists", all)]
    );
    assert_eq!(settle(&mut app), "");
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).prs_compact());
    assert_eq!(pr_numbers(&app), [8, 6]);
    assert_eq!(issue_numbers(&app), [9, 7]);
}

#[test]
fn a_first_issues_page_of_only_issues_still_fills_the_pr_table() {
    // B180's case: the newest issues-endpoint items are all issues, and the PR table has PRs all the same.
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, (46..=75).rev())))
        .with_pull_request_page("octo/lists", 1, Ok(prs(&repo, [44, 43, 41])));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    assert_eq!(pr_numbers(&app), [44, 43, 41]);
    assert!(!lists(&app).prs_compact());
    assert_eq!(lists(&app).issues_table().row_count(), 30);
}

#[test]
fn rows_sort_by_number_highest_first() {
    let repo = lists_repo();
    let gh = FakeGithub::new().with_pull_request_page("octo/lists", 1, Ok(prs(&repo, [3, 12, 7])));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    assert_eq!(pr_numbers(&app), [12, 7, 3]);
}

#[test]
fn differing_filters_send_each_tables_own() {
    let mut config = Config::default();
    config.pull_requests.state_filter = IssueStateFilter::Open;
    config.pull_requests.owner_filter = IssueOwnerFilter::Mine;
    config.issues.state_filter = IssueStateFilter::Closed;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, lists_repo());
    let queries: Vec<(Table, IssueListQuery)> = list_requests(&app)
        .into_iter()
        .map(|(_, table, _, query)| (table, query))
        .collect();
    assert_eq!(
        queries,
        [
            (Table::Prs, query(IssueStateFilter::Open, IssueOwnerFilter::Mine, 1)),
            (Table::Issues, query(IssueStateFilter::Closed, IssueOwnerFilter::All, 1)),
        ]
    );
    settle(&mut app);
    assert!(!lists(&app).prs_table().is_loading());
    assert!(lists(&app).prs_compact());
    assert!(lists(&app).issues_compact());
}

#[test]
fn each_table_fills_from_its_own_response_in_either_order() {
    let repo = lists_repo();
    for prs_first in [true, false] {
        let mut app = app(Config::default(), FakeGithub::new());
        select(&mut app, repo.clone());
        let [(prs_id, ..)] = table_requests(&app, Table::Prs)[..] else {
            panic!("one PR first page");
        };
        let [(issues_id, ..)] = table_requests(&app, Table::Issues)[..] else {
            panic!("one issues first page");
        };
        let prs_answer = || Event::Github {
            id: prs_id,
            response: GithubResponse::ListPullRequests {
                query: query(IssueStateFilter::All, IssueOwnerFilter::All, 1),
                result: Ok(prs(&repo, [3])),
            },
        };
        // The Issues table drops the PRs on its page.
        let issues_answer = || Event::Github {
            id: issues_id,
            response: GithubResponse::ListIssues(Ok(vec![item(&repo, 4, Kind::Issue), item(&repo, 3, Kind::OpenPr)])),
        };
        if prs_first {
            app.send(prs_answer());
            assert_eq!(pr_numbers(&app), [3]);
            assert!(!lists(&app).prs_table().is_loading());
            assert!(
                lists(&app).issues_table().is_loading(),
                "the other table waits for its own page"
            );
            assert_eq!(issue_numbers(&app), Vec::<u64>::new());
            app.send(issues_answer());
        } else {
            app.send(issues_answer());
            assert_eq!(issue_numbers(&app), [4]);
            assert!(
                lists(&app).prs_table().is_loading(),
                "the other table waits for its own page"
            );
            assert_eq!(pr_numbers(&app), Vec::<u64>::new());
            app.send(prs_answer());
        }
        assert_eq!(pr_numbers(&app), [3]);
        assert_eq!(issue_numbers(&app), [4]);
        assert!(!lists(&app).prs_table().is_loading());
        assert!(!lists(&app).issues_table().is_loading());
    }
}

#[test]
fn an_empty_pr_page_makes_the_section_compact_without_an_error() {
    // What a repo with pull requests turned off answers: `/pulls`' 404 is an empty page in the core.
    let repo = lists_repo();
    let gh = FakeGithub::new().with_issue_page("octo/lists", 1, Ok(issues(&repo, [2, 1])));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    assert_eq!(settle(&mut app), "");
    assert!(lists(&app).prs_compact());
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).issues_compact());
}

#[test]
fn nothing_loads_when_neither_section_is_shown() {
    let mut config = Config::default();
    config.appearance.show_pull_requests = false;
    config.appearance.show_issues = false;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, lists_repo());
    assert_eq!(list_requests(&app), []);
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).prs_compact());
}

#[test]
fn both_first_pages_load_when_only_issues_are_shown() {
    let mut config = Config::default();
    config.appearance.show_pull_requests = false;
    let mut app = app(config, FakeGithub::new());
    select(&mut app, lists_repo());
    assert_eq!(pages(&app, Table::Prs), [1]);
    assert_eq!(pages(&app, Table::Issues), [1]);
}

// Failures.

#[test]
fn a_failed_pr_page_logs_once_and_leaves_the_issues_filled() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Err(server_error()))
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, [2, 1])));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
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
    assert_eq!(issue_numbers(&app), [2, 1]);
    assert!(!lists(&app).issues_compact());
}

#[test]
fn a_failed_issues_page_logs_once_and_leaves_the_prs_filled() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Ok(prs(&repo, [5])))
        .with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR").count(), 1, "{out}");
    assert_eq!(pr_numbers(&app), [5]);
    assert!(lists(&app).issues_compact());
    assert!(!lists(&app).issues_table().is_loading());
}

#[test]
fn both_failing_log_twice_and_both_sections_are_compact() {
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Err(server_error()))
        .with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = app(Config::default(), gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR API Error").count(), 2, "{out}");
    assert!(lists(&app).issues_compact());
    assert!(lists(&app).prs_compact());
}

#[test]
fn a_decode_error_logs_the_unexpected_line() {
    let decode = serde_json::from_str::<u8>("x").unwrap_err();
    let gh = FakeGithub::new().with_pull_request_page("octo/lists", 1, Err(GhError::Decode(decode)));
    let mut app = app(Config::default(), gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert!(
        out.starts_with("ERROR Unexpected error fetching issues and PRs from Github API\n"),
        "{out}"
    );
    assert!(lists(&app).prs_compact());
}

// PR paging.

#[test]
fn a_later_pr_page_uses_the_pull_request_filters_and_the_loaded_repo() {
    let repo = lists_repo();
    let mut config = Config::default();
    config.pull_requests.state_filter = IssueStateFilter::Open;
    let gh = FakeGithub::new().with_pull_request_page("octo/lists", 1, Ok(prs(&repo, (1..=30).rev())));
    let mut app = app(config, gh);
    select(&mut app, repo);
    settle(&mut app);
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    let requests = table_requests(&app, Table::Prs);
    let (_, name, asked) = requests.last().expect("a page request");
    assert_eq!(name, "octo/lists");
    assert_eq!(*asked, query(IssueStateFilter::Open, IssueOwnerFilter::All, 2));
    // No issues page for the PR table.
    assert_eq!(pages(&app, Table::Issues), [1]);
}

#[test]
fn pr_paging_adds_prs_and_ends_at_an_empty_page() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Ok(prs(&repo, (31..=60).rev())))
        .with_pull_request_page("octo/lists", 2, Ok(prs(&repo, (1..=30).rev())));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    settle(&mut app);
    assert_eq!(lists(&app).prs_table().row_count(), 60);
    let lazy = lists(&app).prs_table().lazy_load().expect("paging is on");
    assert!(lazy.can_load_more());
    // Page 3 isn't in the fake, so it's empty: paging ends.
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    settle(&mut app);
    let lazy = lists(&app).prs_table().lazy_load().expect("paging is on");
    assert!(!lazy.can_load_more());
    assert_eq!(pages(&app, Table::Prs), [1, 2, 3]);
    pr_cursor(&mut app, BindingId::TableCursorUp);
    assert_eq!(pages(&app, Table::Prs), [1, 2, 3], "nothing after the last page");
}

#[test]
fn search_paging_stops_after_page_34() {
    let repo = lists_repo();
    let mut config = Config::default();
    config.pull_requests.owner_filter = IssueOwnerFilter::Mine;
    // 34 full pages, numbered down from 1,020; the fake would answer page 35 too.
    let mut gh = FakeGithub::new();
    for page in 1..=35_u32 {
        let top = u64::from((35 - page) * 30 + 30);
        gh = gh.with_pull_request_page("octo/lists", page, Ok(prs(&repo, (top - 29..=top).rev())));
    }
    let mut app = app(config, gh);
    select(&mut app, repo);
    settle(&mut app);
    for _ in 0..40 {
        pr_cursor(&mut app, BindingId::TableScrollBottom);
        settle(&mut app);
    }
    assert_eq!(pages(&app, Table::Prs), (1..=34).collect::<Vec<_>>());
    assert!(
        table_requests(&app, Table::Prs)
            .iter()
            .all(|(.., query)| query.owner == IssueOwnerFilter::Mine)
    );
    assert_eq!(lists(&app).prs_table().row_count(), 34 * 30);
    let lazy = lists(&app).prs_table().lazy_load().expect("paging is on");
    assert!(!lazy.can_load_more());
}

#[test]
fn a_failed_later_page_logs_keeps_the_rows_and_asks_again() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Ok(prs(&repo, 1..=30)))
        .with_pull_request_page("octo/lists", 2, Err(server_error()));
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
    assert_eq!(pages(&app, Table::Prs), [1, 2, 2]);
}

#[test]
fn selecting_another_repo_cancels_the_requests_in_flight_and_drops_late_answers() {
    let repo = lists_repo();
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Ok(prs(&repo, 1..=30)))
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, 31..=60)));
    let mut app = app(Config::default(), gh);
    select(&mut app, repo.clone());
    settle(&mut app);
    // A later page in flight, then new first pages in flight, then another repo.
    pr_cursor(&mut app, BindingId::TableScrollBottom);
    let (page_2, ..) = *table_requests(&app, Table::Prs).last().unwrap();
    select(&mut app, repo.clone());
    let (prs_first, ..) = *table_requests(&app, Table::Prs).last().unwrap();
    let (issues_first, ..) = *table_requests(&app, Table::Issues).last().unwrap();
    assert_eq!(cancels(&app), [page_2]);
    select(&mut app, other_repo());
    assert_eq!(cancels(&app), [page_2, prs_first, issues_first]);
    // Late answers for the cancelled requests change nothing.
    for id in [page_2, prs_first] {
        app.send(Event::Github {
            id,
            response: GithubResponse::ListPullRequests {
                query: query(IssueStateFilter::All, IssueOwnerFilter::All, 1),
                result: Ok(prs(&repo, 61..=90)),
            },
        });
    }
    app.send(Event::Github {
        id: issues_first,
        response: GithubResponse::ListIssues(Ok(issues(&repo, 91..=95))),
    });
    assert!(lists(&app).prs_table().is_loading());
    assert_eq!(lists(&app).prs_table().row_count(), 0);
    assert_eq!(lists(&app).issues_table().row_count(), 0);
    settle(&mut app);
    assert!(lists(&app).prs_compact(), "octo/other has no pull requests");
    assert_eq!(pr_numbers(&app), Vec::<u64>::new());
    assert!(lists(&app).issues_compact());
    assert_eq!(issue_numbers(&app), Vec::<u64>::new());
}

// The Issues section (#172).

#[test]
fn the_issues_page_keeps_its_issues_in_api_order() {
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
    settle(&mut app);
    // The table holds its issues in API order, and shows them highest number first. The PRs aren't its rows, and
    // the PR table doesn't take them either: its rows come from its own page, empty here.
    let issue_items: Vec<u64> = lists(&app).issues_table().items().map(|issue| issue.number).collect();
    assert_eq!(issue_items, [5, 9, 7]);
    assert_eq!(issue_numbers(&app), [9, 7, 5]);
    assert_eq!(pr_numbers(&app), Vec::<u64>::new());
    assert!(!lists(&app).issues_table().is_loading());
    assert!(!lists(&app).issues_compact());
}

#[test]
fn a_first_issues_page_of_only_prs_leaves_issues_compact() {
    let repo = lists_repo();
    let gh = FakeGithub::new().with_issue_page(
        "octo/lists",
        1,
        Ok(prs(&repo, 1..=30).into_iter().map(IssueListItem::PullRequest).collect()),
    );
    let mut app = app(Config::default(), gh);
    select(&mut app, repo);
    settle(&mut app);
    assert!(lists(&app).issues_compact());
    assert_eq!(lists(&app).issues_table().row_count(), 0);
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
    let requests = table_requests(&app, Table::Issues);
    let (_, name, asked) = requests.last().expect("a page request");
    assert_eq!(name, "octo/lists");
    assert_eq!(*asked, query(IssueStateFilter::Open, IssueOwnerFilter::Mine, 2));
    assert_eq!(
        pages(&app, Table::Prs),
        [1],
        "the Issues table's paging asks nothing of /pulls"
    );
}

#[test]
fn a_page_of_only_prs_keeps_the_issues_paging_on() {
    let repo = lists_repo();
    let only_prs = prs(&repo, 31..=60)
        .into_iter()
        .map(IssueListItem::PullRequest)
        .collect();
    let gh = FakeGithub::new()
        .with_issue_page("octo/lists", 1, Ok(issues(&repo, 1..=30)))
        .with_issue_page("octo/lists", 2, Ok(only_prs));
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
    assert_eq!(pages(&app, Table::Issues), [1, 2, 2]);
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

/// The PR cache file's text for `prs`.
fn prs_cache(prs: &[PartialPullRequest]) -> String {
    lazy_github_core::cache::to_json(prs.iter())
}

/// The issues cache file's text for the issues among `items`.
fn issues_cache(items: Vec<IssueListItem>) -> String {
    lazy_github_core::cache::to_json(issues_page(items).items)
}

/// An app whose `octo/lists` cache files hold PRs 4 and 2 and issues 3 and 1, with pages from `gh`.
fn cached_app(gh: FakeGithub) -> TestApp {
    let repo = lists_repo();
    let prs = prs_cache(&[pr(&repo, 4, Kind::OpenPr), pr(&repo, 2, Kind::DraftPr)]);
    let issues = issues_cache(issues(&repo, [3, 1]));
    app(cache_config(), gh)
        .with_cache_file(cache_path("pull_requests"), prs)
        .with_cache_file(cache_path("issues"), issues)
}

#[test]
fn each_first_page_replaces_its_tables_cached_rows_and_writes_its_file() {
    let repo = lists_repo();
    let page_prs = vec![pr(&repo, 8, Kind::ClosedPr), pr(&repo, 6, Kind::DraftPr)];
    let page_issues = vec![item(&repo, 9, Kind::Issue), item(&repo, 3, Kind::Issue)];
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Ok(page_prs.clone()))
        .with_issue_page("octo/lists", 1, Ok(page_issues.clone()));
    let mut app = cached_app(gh);
    select(&mut app, repo);
    assert_eq!(settle(&mut app), "");
    assert_eq!(pr_numbers(&app), [8, 6]);
    assert_eq!(issue_numbers(&app), [9, 3]);
    assert_eq!(
        app.cache_file(&cache_path("pull_requests")),
        Some(prs_cache(&page_prs).as_str())
    );
    assert_eq!(
        app.cache_file(&cache_path("issues")),
        Some(issues_cache(page_issues).as_str())
    );
}

#[test]
fn failed_first_pages_keep_each_tables_cached_rows() {
    let gh = FakeGithub::new()
        .with_pull_request_page("octo/lists", 1, Err(server_error()))
        .with_issue_page("octo/lists", 1, Err(server_error()));
    let mut app = cached_app(gh);
    select(&mut app, lists_repo());
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR").count(), 2, "{out}");
    assert_eq!(pr_numbers(&app), [4, 2]);
    assert_eq!(issue_numbers(&app), [3, 1]);
    assert!(!lists(&app).prs_table().is_loading());
    assert!(!lists(&app).prs_compact());
    assert!(!lists(&app).issues_compact());
}
