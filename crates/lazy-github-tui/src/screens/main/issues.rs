//! The issues section's rows: Python's `IssuesContainer` table, filled from the issues endpoint.

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::IssueListQuery;
use lazy_github_core::models::{Issue, IssueListItem, Repository};

use super::issue_lists::log_fetch_failure;
use super::list::{ListSource, PageFailure};
use crate::components::table::{Cell, Column, Page, SortSpec, TableItem};
use crate::effect::{GithubRequest, GithubResponse};

/// The issue and PR tables' columns: Python's `#`, `Status`, `Author` and `Title`.
pub(super) fn issue_columns() -> Vec<Column> {
    ["#", "Status", "Author", "Title"].map(Column::new).into()
}

/// Python's `issue_to_cell` and `pull_request_to_cell`: the number as an integer, the API's `state`, the author's
/// login and the title drawn as typed (Py bug B46).
pub(super) fn issue_cells(issue: &Issue) -> Vec<Cell> {
    vec![
        Cell::int(i64::try_from(issue.number).unwrap_or(i64::MAX)),
        Cell::text(issue.state.as_str()),
        Cell::text(issue.user.login.clone()),
        Cell::text(issue.title.clone()),
    ]
}

impl TableItem for Issue {
    type Key = u64;
    type Ctx = ();

    fn key(&self) -> u64 {
        self.number
    }

    fn cells(&self, (): &()) -> Vec<Cell> {
        issue_cells(self)
    }
}

/// The issues among one issues-endpoint page, in API order. The page is the last one only when the API page itself
/// was empty, so a page of only PRs still pages on (Py bug B44).
pub(super) fn issues_page(items: Vec<IssueListItem>) -> Page<Issue> {
    let last = items.is_empty();
    let items = items.into_iter().filter_map(IssueListItem::into_issue).collect();
    Page { items, last }
}

/// What fills the issues section.
pub(super) struct IssuesSource;

impl ListSource for IssuesSource {
    type Item = Issue;

    fn columns() -> Vec<Column> {
        issue_columns()
    }

    fn sort() -> SortSpec {
        SortSpec::descending(0)
    }

    /// With the `issues.*` filters, as Python's `fetch_more_issues`.
    fn page_request(repo: &Repository, page: u32, per_page: u32, config: &Config) -> GithubRequest {
        GithubRequest::ListIssues {
            repo: Box::new(repo.clone()),
            query: IssueListQuery {
                page,
                per_page,
                ..issues_query(config)
            },
        }
    }

    fn page_items(response: GithubResponse) -> Result<Result<Page<Issue>, GhError>, GithubResponse> {
        match response {
            GithubResponse::ListIssues(result) => Ok(result.map(issues_page)),
            other => Err(other),
        }
    }

    /// Python's worker crashes here; Rust logs the first page's line and asks again on the next move (Py bug B43).
    fn page_failed(error: &GhError) -> PageFailure {
        log_fetch_failure(error);
        PageFailure::Retry
    }
}

/// Page 1 with the `issues.*` filters.
pub(super) fn issues_query(config: &Config) -> IssueListQuery {
    let settings = &config.issues;
    IssueListQuery::first_page(settings.state_filter, settings.owner_filter)
}

#[cfg(test)]
mod tests {
    use lazy_github_core::config::{IssueOwnerFilter, IssueStateFilter};

    use super::*;
    use crate::components::table::CellValue;
    use crate::screens::main::pull_requests::{Kind, item};
    use crate::screens::main::repos::repo;

    fn issue(number: u64, closed: bool) -> Issue {
        let kind = if closed { Kind::ClosedIssue } else { Kind::Issue };
        item(&repo("octo", "lists", false), number, kind).into_issue().unwrap()
    }

    #[test]
    fn cells_for_an_open_and_a_closed_issue() {
        let row = |number: i64, state: &str| {
            vec![
                Cell::int(number),
                Cell::text(state),
                Cell::text("parity-bot"),
                Cell::text(format!("Issue {number}")),
            ]
        };
        assert_eq!(issue(9, false).cells(&()), row(9, "open"));
        assert_eq!(issue(7, true).cells(&()), row(7, "closed"));
        assert_eq!(issue(9, false).cells(&())[0].value, CellValue::Int(9));
        assert_eq!(issue(9, false).key(), 9);
    }

    #[test]
    fn a_title_with_brackets_is_drawn_as_typed() {
        // Python reads `[docs]` as markup and drops it (Py bug B46).
        let mut issue = issue(5, false);
        issue.title = "[docs] Fix".into();
        assert_eq!(issue.cells(&())[3], Cell::text("[docs] Fix"));
    }

    #[test]
    fn a_page_keeps_its_issues_in_api_order_and_is_last_only_when_empty() {
        let repo = repo("octo", "lists", false);
        let mixed = vec![
            item(&repo, 9, Kind::Issue),
            item(&repo, 8, Kind::ClosedPr),
            item(&repo, 4, Kind::Issue),
            item(&repo, 6, Kind::DraftPr),
            item(&repo, 7, Kind::Issue),
        ];
        let page = issues_page(mixed);
        let numbers: Vec<u64> = page.items.iter().map(|issue| issue.number).collect();
        assert_eq!(numbers, [9, 4, 7]);
        assert!(!page.last);
        // A page of only PRs still pages on (Py bug B44); an empty API page ends paging.
        assert!(!issues_page(vec![item(&repo, 1, Kind::OpenPr)]).last);
        assert!(issues_page(Vec::new()).last);
    }

    #[test]
    fn later_pages_use_the_issue_filters() {
        let mut config = Config::default();
        config.issues.state_filter = IssueStateFilter::Closed;
        config.issues.owner_filter = IssueOwnerFilter::Mine;
        config.pull_requests.state_filter = IssueStateFilter::Open;
        let GithubRequest::ListIssues { repo: asked, query } =
            IssuesSource::page_request(&repo("octo", "lists", false), 2, 30, &config)
        else {
            panic!("an issues-endpoint request");
        };
        assert_eq!(asked.full_name, "octo/lists");
        assert_eq!(
            query,
            IssueListQuery {
                state: IssueStateFilter::Closed,
                owner: IssueOwnerFilter::Mine,
                page: 2,
                per_page: 30,
            }
        );
    }
}
