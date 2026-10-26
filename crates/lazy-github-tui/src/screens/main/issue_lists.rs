//! The Pull Requests and Issues sections together: their shared first page and their tables. Python's
//! `SelectionsPane.fetch_issues_and_pull_requests` and the `IssuesAndPullRequestsFetched` message.
//!
//! One `GET /repos/{owner}/{repo}/issues` page 1 feeds both tables when their filters agree (the default); when they
//! don't, each table's own filters get their own request (Py bug B3's recommendation). Later pages belong to each
//! table's [`ListSection`].

use lazy_github_core::gh::GhError;
use lazy_github_core::github::IssueListQuery;
use lazy_github_core::models::{PartialPullRequest, Repository};

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
    #[expect(dead_code, reason = "#172 hands the issues table its half")]
    issues: bool,
}

/// The PR and Issues sections' state.
#[derive(Debug)]
pub(super) struct IssueLists {
    prs: ListSection<PullRequestsSource>,
    // #172 adds `issues: ListSection<IssuesSource>`.
    first_page: FirstPage,
}

impl IssueLists {
    pub(super) fn new(options: AppOptions) -> Self {
        Self {
            prs: ListSection::new(options),
            first_page: FirstPage::Idle,
        }
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
        for id in self.first_page_ids() {
            cx.effect(Effect::Cancel(id));
        }
        self.prs.begin(repo, cx);
        let prs = pull_requests_query(cx.config());
        let issues = {
            let settings = &cx.config().issues;
            IssueListQuery::first_page(settings.state_filter, settings.owner_filter)
        };
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

    /// Requests the page the PR table asked for.
    pub(super) fn load_more_prs(&mut self, generation: u64, page: u32, per_page: u32, cx: &mut Cx<'_>) {
        self.prs.load_more(generation, page, per_page, cx);
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
            return self.prs.on_response(id, GithubResponse::ListIssues(result), cx);
        };
        if let Err(error) = &result {
            log_fetch_failure(error);
        }
        // #172 splits the items between the tables when `feeds.issues`.
        if feeds.prs {
            self.prs.first_page(result.map(pull_requests_page), cx);
            self.prs.refresh_compact();
        }
        None
    }
}

#[cfg(test)]
mod tests;
