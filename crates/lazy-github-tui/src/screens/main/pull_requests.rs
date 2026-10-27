//! The pull requests section's rows: Python's `PullRequestsContainer` table, filled from the issues endpoint.

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::IssueListQuery;
use lazy_github_core::models::{IssueListItem, PartialPullRequest, Repository};

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

/// The pull requests among one issues-endpoint page, in API order. The page is the last one only when the API page
/// itself was empty, so a page of only issues still pages on (Py bug B44).
pub(super) fn pull_requests_page(items: Vec<IssueListItem>) -> Page<PartialPullRequest> {
    let last = items.is_empty();
    let items = items.into_iter().filter_map(IssueListItem::into_pull_request).collect();
    Page { items, last }
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

    /// With the `pull_requests.*` filters, as Python's `fetch_more_pull_requests`.
    fn page_request(repo: &Repository, page: u32, per_page: u32, config: &Config) -> GithubRequest {
        GithubRequest::ListIssues {
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
            GithubResponse::ListIssues(result) => Ok(result.map(pull_requests_page)),
            other => Err(other),
        }
    }

    /// Python's worker crashes here; Rust logs the first page's line and asks again on the next move (Py bug B43).
    fn page_failed(error: &GhError) -> PageFailure {
        log_fetch_failure(error);
        PageFailure::Retry
    }
}

/// Page 1 with the `pull_requests.*` filters. Python's first page uses the `issues.*` ones for both tables (Py bug B3).
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

/// Open pull requests numbered `numbers`, in that order.
#[cfg(test)]
pub(super) fn prs(repo: &Repository, numbers: impl IntoIterator<Item = u64>) -> Vec<IssueListItem> {
    numbers.into_iter().map(|n| item(repo, n, Kind::OpenPr)).collect()
}

#[cfg(test)]
mod tests {
    use lazy_github_core::config::{IssueOwnerFilter, IssueStateFilter};

    use super::*;
    use crate::components::table::CellValue;
    use crate::screens::main::repos::repo;

    fn pr(kind: Kind) -> PartialPullRequest {
        item(&repo("octo", "lists", false), 12, kind)
            .into_pull_request()
            .unwrap()
    }

    #[test]
    fn cells_for_open_closed_and_draft_prs() {
        let cells = |kind| pr(kind).cells(&());
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
        assert_eq!(pr(Kind::OpenPr).key(), 12);
    }

    #[test]
    fn a_title_with_brackets_is_drawn_as_typed() {
        // Python reads `[docs]` as markup and drops it (Py bug B46).
        let mut pr = pr(Kind::OpenPr);
        pr.issue.title = "[docs] Fix".into();
        assert_eq!(pr.cells(&())[3], Cell::text("[docs] Fix"));
    }

    #[test]
    fn a_page_keeps_its_prs_in_api_order_and_is_last_only_when_empty() {
        let repo = repo("octo", "lists", false);
        let mixed = vec![
            item(&repo, 9, Kind::Issue),
            item(&repo, 8, Kind::ClosedPr),
            item(&repo, 7, Kind::Issue),
            item(&repo, 6, Kind::DraftPr),
            item(&repo, 10, Kind::OpenPr),
        ];
        let page = pull_requests_page(mixed);
        let numbers: Vec<u64> = page.items.iter().map(|pr| pr.issue.number).collect();
        assert_eq!(numbers, [8, 6, 10]);
        assert!(!page.last);
        // A page of only issues still pages on (Py bug B44); an empty API page ends paging.
        assert!(!pull_requests_page(vec![item(&repo, 1, Kind::Issue)]).last);
        assert!(pull_requests_page(Vec::new()).last);
    }

    #[test]
    fn later_pages_use_the_pull_request_filters() {
        let mut config = Config::default();
        config.pull_requests.state_filter = IssueStateFilter::Open;
        config.pull_requests.owner_filter = IssueOwnerFilter::Mine;
        config.issues.state_filter = IssueStateFilter::Closed;
        let GithubRequest::ListIssues { repo: asked, query } =
            PullRequestsSource::page_request(&repo("octo", "lists", false), 3, 30, &config)
        else {
            panic!("an issues-endpoint request");
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
}
