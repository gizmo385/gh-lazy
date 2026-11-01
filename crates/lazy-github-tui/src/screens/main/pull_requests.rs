//! The pull requests section's rows: Python's `PullRequestsContainer` table, filled from `/pulls`, or from a search
//! for the `mine` filter ([`Github::list_pull_requests`](lazy_github_core::github::Github::list_pull_requests)).
//! Python fills it from the issues endpoint (Py bug B180).

use lazy_github_core::config::{Config, IssueOwnerFilter};
use lazy_github_core::gh::GhError;
use lazy_github_core::github::IssueListQuery;
use lazy_github_core::github::search::SEARCH_RESULT_CAP;
#[cfg(test)]
use lazy_github_core::models::IssueListItem;
use lazy_github_core::models::{PartialPullRequest, Repository};

use super::issue_lists::log_fetch_failure;
use super::issues::{issue_cells, issue_columns};
use super::list::{ListSource, PageFailure};
use crate::components::table::{Cell, Column, Page, SortSpec, TableItem};
use crate::effect::{GithubRequest, GithubResponse};

impl TableItem for PartialPullRequest {
    type Key = u64;
    type Ctx = ();

    fn key(&self) -> u64 {
        self.issue.number
    }

    /// Python's `pull_request_to_cell`. A merged PR is `closed` and a draft `open`, as the API's `state` says. The
    /// title is drawn as typed (Py bug B46).
    fn cells(&self, (): &()) -> Vec<Cell> {
        issue_cells(&self.issue)
    }
}

/// One page of pull requests, asked for with `query`, in API order. It's the last one when it's empty (Py bug B44's
/// rule), or for a search when no later page can hold a result: page 34 of 30 reaches GitHub's 1,000-result cap,
/// and page 35 would answer 422.
pub(super) fn pull_requests_page(items: Vec<PartialPullRequest>, query: IssueListQuery) -> Page<PartialPullRequest> {
    let capped =
        query.owner == IssueOwnerFilter::Mine && query.page.saturating_mul(query.per_page) >= SEARCH_RESULT_CAP;
    Page {
        last: items.is_empty() || capped,
        items,
    }
}

/// What fills the pull requests section.
pub(super) struct PullRequestsSource;

impl ListSource for PullRequestsSource {
    type Item = PartialPullRequest;
    const CACHE_NAME: &'static str = "pull_requests";
    const TYPE_NAME: &'static str = "PartialPullRequest";

    fn columns() -> Vec<Column> {
        issue_columns()
    }

    fn sort() -> SortSpec {
        SortSpec::descending(0)
    }

    /// With the `pull_requests.*` filters, as Python's `fetch_more_pull_requests`, but from `/pulls` or a search
    /// rather than the issues endpoint (Py bug B180).
    fn page_request(repo: &Repository, page: u32, per_page: u32, config: &Config) -> GithubRequest {
        GithubRequest::ListPullRequests {
            repo: Box::new(repo.clone()),
            query: IssueListQuery {
                page,
                per_page,
                ..pull_requests_query(config)
            },
        }
    }

    fn page_items(response: GithubResponse) -> Result<Result<Page<PartialPullRequest>, GhError>, GithubResponse> {
        match response {
            GithubResponse::ListPullRequests { query, result } => {
                Ok(result.map(|items| pull_requests_page(items, query)))
            }
            other => Err(other),
        }
    }

    /// Python's worker crashes here; Rust logs the first page's line and asks again on the next move (Py bug B43).
    fn page_failed(error: &GhError) -> PageFailure {
        log_fetch_failure(error);
        PageFailure::Retry
    }
}

/// Page 1 with the `pull_requests.*` filters. Python's first page uses the `issues.*` ones for both tables (Py bug B3);
/// here the PR table always has its own.
pub(super) fn pull_requests_query(config: &Config) -> IssueListQuery {
    let settings = &config.pull_requests;
    IssueListQuery::first_page(settings.state_filter, settings.owner_filter)
}

/// An issues-endpoint item in `repo` for tests: issue `number` (open or closed), or a pull request (open, closed or
/// draft), as `kind` says. Titles are `Issue <n>` and `PR <n>`.
#[cfg(test)]
pub(super) fn item(repo: &Repository, number: u64, kind: Kind) -> IssueListItem {
    use lazy_github_core::models::{Issue, IssueState, User};

    let closed = matches!(kind, Kind::ClosedPr | Kind::ClosedIssue);
    let is_issue = matches!(kind, Kind::Issue | Kind::ClosedIssue);
    let issue = Issue {
        id: 7_000_000 + number,
        number,
        locked: false,
        state: if closed { IssueState::Closed } else { IssueState::Open },
        title: if is_issue {
            format!("Issue {number}")
        } else {
            format!("PR {number}")
        },
        body: None,
        user: User {
            login: "parity-bot".into(),
            id: 1,
            name: None,
            html_url: "https://github.com/parity-bot".into(),
        },
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
        closed_at: None,
        closed_by: None,
        assignee: None,
        assignees: None,
        comments_url: String::new(),
        html_url: String::new(),
        repo: repo.clone(),
    };
    match kind {
        Kind::Issue | Kind::ClosedIssue => IssueListItem::Issue(issue),
        Kind::OpenPr | Kind::ClosedPr | Kind::DraftPr => IssueListItem::PullRequest(PartialPullRequest {
            issue,
            draft: kind == Kind::DraftPr,
        }),
    }
}

/// What [`item`] makes.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Issue,
    ClosedIssue,
    OpenPr,
    ClosedPr,
    DraftPr,
}

/// A pull request in `repo` for tests, as [`item`] makes it. `kind` must be a pull request kind.
#[cfg(test)]
pub(super) fn pr(repo: &Repository, number: u64, kind: Kind) -> PartialPullRequest {
    item(repo, number, kind)
        .into_pull_request()
        .expect("a pull request kind")
}

/// Open pull requests numbered `numbers`, in that order.
#[cfg(test)]
pub(super) fn prs(repo: &Repository, numbers: impl IntoIterator<Item = u64>) -> Vec<PartialPullRequest> {
    numbers.into_iter().map(|n| pr(repo, n, Kind::OpenPr)).collect()
}

#[cfg(test)]
mod tests {
    use lazy_github_core::config::IssueStateFilter;

    use super::*;
    use crate::components::table::CellValue;
    use crate::screens::main::repos::repo;

    fn pr_12(kind: Kind) -> PartialPullRequest {
        pr(&repo("octo", "lists", false), 12, kind)
    }

    #[test]
    fn cells_for_open_closed_and_draft_prs() {
        let cells = |kind| pr_12(kind).cells(&());
        let row = |state: &str| {
            vec![
                Cell::int(12),
                Cell::text(state),
                Cell::text("parity-bot"),
                Cell::text("PR 12"),
            ]
        };
        assert_eq!(cells(Kind::OpenPr), row("open"));
        // Merged PRs are `closed` too, as the API's state says; a draft is `open`.
        assert_eq!(cells(Kind::ClosedPr), row("closed"));
        assert_eq!(cells(Kind::DraftPr), row("open"));
        assert_eq!(cells(Kind::OpenPr)[0].value, CellValue::Int(12));
        assert_eq!(pr_12(Kind::OpenPr).key(), 12);
    }

    #[test]
    fn a_title_with_brackets_is_drawn_as_typed() {
        // Python reads `[docs]` as markup and drops it (Py bug B46).
        let mut pr = pr_12(Kind::OpenPr);
        pr.issue.title = "[docs] Fix".into();
        assert_eq!(pr.cells(&())[3], Cell::text("[docs] Fix"));
    }

    fn query(owner: IssueOwnerFilter, page: u32) -> IssueListQuery {
        IssueListQuery {
            page,
            ..IssueListQuery::first_page(IssueStateFilter::All, owner)
        }
    }

    #[test]
    fn a_page_keeps_its_prs_in_api_order_and_is_last_only_when_empty() {
        let repo = repo("octo", "lists", false);
        let items = vec![
            pr(&repo, 8, Kind::ClosedPr),
            pr(&repo, 6, Kind::DraftPr),
            pr(&repo, 10, Kind::OpenPr),
        ];
        let page = pull_requests_page(items, query(IssueOwnerFilter::All, 1));
        let numbers: Vec<u64> = page.items.iter().map(|pr| pr.issue.number).collect();
        assert_eq!(numbers, [8, 6, 10]);
        assert!(!page.last);
        // An empty page ends paging, whatever the filter.
        assert!(pull_requests_page(Vec::new(), query(IssueOwnerFilter::All, 2)).last);
        assert!(pull_requests_page(Vec::new(), query(IssueOwnerFilter::Mine, 2)).last);
    }

    #[test]
    fn a_search_page_is_the_last_once_it_reaches_the_result_cap() {
        let repo = repo("octo", "lists", false);
        let full = || prs(&repo, (1..=30).rev());
        assert!(!pull_requests_page(full(), query(IssueOwnerFilter::Mine, 33)).last);
        // 34 * 30 >= 1,000: page 35 would answer 422.
        assert!(pull_requests_page(full(), query(IssueOwnerFilter::Mine, 34)).last);
        // `/pulls` has no cap.
        assert!(!pull_requests_page(full(), query(IssueOwnerFilter::All, 34)).last);
        assert!(!pull_requests_page(full(), query(IssueOwnerFilter::All, 40)).last);
    }

    #[test]
    fn later_pages_use_the_pull_request_filters() {
        let mut config = Config::default();
        config.pull_requests.state_filter = IssueStateFilter::Open;
        config.pull_requests.owner_filter = IssueOwnerFilter::Mine;
        config.issues.state_filter = IssueStateFilter::Closed;
        let GithubRequest::ListPullRequests { repo: asked, query } =
            PullRequestsSource::page_request(&repo("octo", "lists", false), 3, 30, &config)
        else {
            panic!("a pull request list request");
        };
        assert_eq!(asked.full_name, "octo/lists");
        assert_eq!(
            query,
            IssueListQuery {
                state: IssueStateFilter::Open,
                owner: IssueOwnerFilter::Mine,
                page: 3,
                per_page: 30,
            }
        );
    }

    #[test]
    fn a_page_is_read_from_a_pull_request_list_response_only() {
        let repo = repo("octo", "lists", false);
        let asked = query(IssueOwnerFilter::Mine, 34);
        let response = GithubResponse::ListPullRequests {
            query: asked,
            result: Ok(prs(&repo, [2, 1])),
        };
        let page = PullRequestsSource::page_items(response).unwrap().unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.last, "the query it was asked with decides the cap");
        let issues = GithubResponse::ListIssues(Ok(Vec::new()));
        assert!(matches!(
            PullRequestsSource::page_items(issues),
            Err(GithubResponse::ListIssues(_))
        ));
    }
}
