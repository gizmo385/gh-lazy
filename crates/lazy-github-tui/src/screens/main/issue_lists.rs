//! The Pull Requests and Issues sections together: their shared first page and their tables. Python's
//! `SelectionsPane.fetch_issues_and_pull_requests` and the `IssuesAndPullRequestsFetched` message.
//!
//! One `GET /repos/{owner}/{repo}/issues` page 1 feeds both tables when their filters agree (the default); when they
//! don't, each table's own filters get their own request (Py bug B3's recommendation). Later pages belong to each
//! table's [`ListSection`].

use lazy_github_core::gh::GhError;
use lazy_github_core::github::IssueListQuery;
use lazy_github_core::models::{Issue, IssueListItem, PartialPullRequest, Repository};

use super::issues::{IssuesSource, issues_page, issues_query};
use super::list::{ListSection, PER_PAGE};
use super::pull_requests::{PullRequestsSource, pull_requests_page, pull_requests_query};
use crate::component::Cx;
use crate::components::table::SearchableTable;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::options::AppOptions;

/// The command-log line for a failed page whose body never decoded.
const UNEXPECTED_ERROR: &str = "Unexpected error fetching issues and PRs from Github API";
/// The command-log line for any other failed page.
const API_ERROR: &str = "API Error fetching issues and PRs from Github API";

/// Logs a failed issues-endpoint page as Python does: one ERROR line, no toast, then the error itself at debug.
pub(super) fn log_fetch_failure(error: &GhError) {
    if matches!(error, GhError::Decode(_)) {
        tracing::error!("{UNEXPECTED_ERROR}");
    } else {
        tracing::error!("{API_ERROR}");
    }
    tracing::debug!("Fetching issues and PRs failed: {error}");
}

/// The first-page requests in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstPage {
    Idle,
    /// One request feeds both tables.
    Shared(RequestId),
    /// One request per table, each with its own filters; `None` once it's answered.
    Split {
        prs: Option<RequestId>,
        issues: Option<RequestId>,
    },
}

/// Which tables a first-page response feeds.
#[derive(Debug, Clone, Copy)]
struct Feeds {
    prs: bool,
    issues: bool,
}

/// The PR and Issues sections' state.
#[derive(Debug)]
pub(super) struct IssueLists {
    prs: ListSection<PullRequestsSource>,
    issues: ListSection<IssuesSource>,
    first_page: FirstPage,
}

impl IssueLists {
    pub(super) fn new(options: AppOptions) -> Self {
        Self {
            prs: ListSection::new(options),
            issues: ListSection::new(options),
            first_page: FirstPage::Idle,
        }
    }

    /// The Issues table.
    pub(super) fn issues_table(&self) -> &SearchableTable<Issue> {
        self.issues.table()
    }

    /// The Issues table, for the screen's input and drawing paths.
    pub(super) fn issues_table_mut(&mut self) -> &mut SearchableTable<Issue> {
        self.issues.table_mut()
    }

    /// Whether the Issues section is compact.
    pub(super) fn issues_compact(&self) -> bool {
        self.issues.compact()
    }

    /// The PR table.
    pub(super) fn prs_table(&self) -> &SearchableTable<PartialPullRequest> {
        self.prs.table()
    }

    /// The PR table, for the screen's input and drawing paths.
    pub(super) fn prs_table_mut(&mut self) -> &mut SearchableTable<PartialPullRequest> {
        self.prs.table_mut()
    }

    /// Whether the PR section is compact.
    pub(super) fn prs_compact(&self) -> bool {
        self.prs.compact()
    }

    /// A repo was selected while the PR or the Issues section is shown: every table clears under its overlay, and
    /// the first page is requested, once when both tables' filters agree and once per table when they don't.
    pub(super) fn load_repo(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        self.prs.begin(repo, cx);
        self.issues.begin(repo, cx);
        self.request_first_page(repo, cx);
    }

    /// The palette showed the PR or the Issues section again with `repo` selected: Python's bare
    /// `fetch_issues_and_pull_requests`. The first page is requested as on selection, but nothing is read from the
    /// cache and the tables aren't cleared: their rows stay until the page lands and replaces them.
    pub(super) fn reload(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        // The rows may be for an older repo, if it was selected while both sections were hidden; the page and the
        // cache it writes are for this one.
        self.prs.retarget(repo);
        self.issues.retarget(repo);
        self.request_first_page(repo, cx);
    }

    /// Requests the first page for both tables, replacing any first page in flight: once when both tables' filters
    /// agree and once per table when they don't.
    fn request_first_page(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        for id in self.first_page_ids() {
            cx.effect(Effect::Cancel(id));
        }
        let prs = pull_requests_query(cx.config());
        let issues = issues_query(cx.config());
        let mut request = |query: IssueListQuery| {
            cx.request(GithubRequest::ListIssues {
                repo: Box::new(repo.clone()),
                query: IssueListQuery {
                    per_page: PER_PAGE,
                    ..query
                },
            })
        };
        self.first_page = if prs == issues {
            FirstPage::Shared(request(prs))
        } else {
            FirstPage::Split {
                prs: Some(request(prs)),
                issues: Some(request(issues)),
            }
        };
    }

    /// The first-page requests still in flight.
    fn first_page_ids(&self) -> Vec<RequestId> {
        match self.first_page {
            FirstPage::Idle => Vec::new(),
            FirstPage::Shared(id) => vec![id],
            FirstPage::Split { prs, issues } => prs.into_iter().chain(issues).collect(),
        }
    }

    /// Which tables a response to `id` feeds, if it's a first page in flight, and forgets the request.
    fn take_first_page(&mut self, id: RequestId) -> Option<Feeds> {
        let feeds = match &mut self.first_page {
            FirstPage::Shared(shared) if *shared == id => {
                self.first_page = FirstPage::Idle;
                return Some(Feeds {
                    prs: true,
                    issues: true,
                });
            }
            FirstPage::Idle | FirstPage::Shared(_) => return None,
            FirstPage::Split { prs, issues } => {
                if *prs == Some(id) {
                    *prs = None;
                    Feeds {
                        prs: true,
                        issues: false,
                    }
                } else if *issues == Some(id) {
                    *issues = None;
                    Feeds {
                        prs: false,
                        issues: true,
                    }
                } else {
                    return None;
                }
            }
        };
        if self.first_page
            == (FirstPage::Split {
                prs: None,
                issues: None,
            })
        {
            self.first_page = FirstPage::Idle;
        }
        Some(feeds)
    }

    /// Adds a looked-up pull request to the PR table unless it's there: [`ListSection::insert_if_missing`].
    pub(super) fn insert_pr_if_missing(&mut self, pr: PartialPullRequest, cx: &mut Cx<'_>) {
        self.prs.insert_if_missing(pr, cx);
    }

    /// Adds a looked-up issue to the Issues table unless it's there: [`ListSection::insert_if_missing`].
    pub(super) fn insert_issue_if_missing(&mut self, issue: Issue, cx: &mut Cx<'_>) {
        self.issues.insert_if_missing(issue, cx);
    }

    /// Adds a created pull request to the PR table, or replaces its row: [`ListSection::upsert`].
    pub(super) fn upsert_pr(&mut self, pr: PartialPullRequest, cx: &mut Cx<'_>) {
        self.prs.upsert(pr, cx);
    }

    /// Adds a created issue to the Issues table, or replaces its row: [`ListSection::upsert`].
    pub(super) fn upsert_issue(&mut self, issue: Issue, cx: &mut Cx<'_>) {
        self.issues.upsert(issue, cx);
    }

    /// Requests the page the PR table asked for.
    pub(super) fn load_more_prs(&mut self, generation: u64, page: u32, per_page: u32, cx: &mut Cx<'_>) {
        self.prs.load_more(generation, page, per_page, cx);
    }

    /// Requests the page the Issues table asked for.
    pub(super) fn load_more_issues(&mut self, generation: u64, page: u32, per_page: u32, cx: &mut Cx<'_>) {
        self.issues.load_more(generation, page, per_page, cx);
    }

    /// Takes a response to a first page or a table's later page, and hands back any other. A first page that was
    /// cancelled by a newer selection is handed back, so the app drops it (Py bug B45).
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let GithubResponse::ListIssues(result) = response else {
            return Some(response);
        };
        let Some(feeds) = self.take_first_page(id) else {
            let response = self.prs.on_response(id, GithubResponse::ListIssues(result), cx)?;
            return self.issues.on_response(id, response, cx);
        };
        // A failure logs once, whichever tables it feeds, and leaves them with their cached rows.
        let (prs, issues) = match result {
            Ok(items) => {
                // Each table keeps its own kind, in API order. A first page's `last` isn't read, so splitting first
                // is fine.
                let (prs, issues): (Vec<_>, Vec<_>) = items
                    .into_iter()
                    .partition(|item| matches!(item, IssueListItem::PullRequest(_)));
                (Some(pull_requests_page(prs)), Some(issues_page(issues)))
            }
            Err(error) => {
                log_fetch_failure(&error);
                (None, None)
            }
        };
        if feeds.prs {
            self.prs.first_page(prs, cx);
            self.prs.refresh_compact();
        }
        if feeds.issues {
            self.issues.first_page(issues, cx);
            self.issues.refresh_compact();
        }
        None
    }

    /// Takes the result of either table's cache read, and hands back any other.
    pub(super) fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        let result = self.prs.on_cache_read(id, result, cx)?;
        self.issues.on_cache_read(id, result, cx)
    }
}

#[cfg(test)]
mod tests;
