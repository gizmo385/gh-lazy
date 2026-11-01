//! The Pull Requests and Issues sections together: Python's `SelectionsPane.fetch_issues_and_pull_requests` and the
//! `IssuesAndPullRequestsFetched` message.
//!
//! Python fetches one issues-endpoint page 1 for both tables, with the `issues.*` filters (Py bug B3), and gives its
//! PRs to the PR table and the rest to the Issues table. Here each table requests its own first page with its own
//! filters: the PR table from `/pulls` or a search (Py bug B180, #384) and the Issues table from the issues endpoint.
//! Each table fills when its own answer lands. Every page belongs to its table's [`ListSection`]; this pairs the two
//! for a selection, a reload and response routing.

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{Issue, PartialPullRequest, Repository};

use super::issues::IssuesSource;
use super::list::ListSection;
use super::pull_requests::PullRequestsSource;
use crate::component::Cx;
use crate::components::table::SearchableTable;
use crate::effect::{GithubResponse, RequestId};
use crate::options::AppOptions;

/// The command-log line for a failed page whose body never decoded.
const UNEXPECTED_ERROR: &str = "Unexpected error fetching issues and PRs from Github API";
/// The command-log line for any other failed page.
const API_ERROR: &str = "API Error fetching issues and PRs from Github API";

/// Logs a failed PR or issues page as Python does: one ERROR line, no toast, then the error itself at debug.
pub(super) fn log_fetch_failure(error: &GhError) {
    if matches!(error, GhError::Decode(_)) {
        tracing::error!("{UNEXPECTED_ERROR}");
    } else {
        tracing::error!("{API_ERROR}");
    }
    tracing::debug!("Fetching issues and PRs failed: {error}");
}

/// The PR and Issues sections' state.
#[derive(Debug)]
pub(super) struct IssueLists {
    prs: ListSection<PullRequestsSource>,
    issues: ListSection<IssuesSource>,
}

impl IssueLists {
    pub(super) fn new(options: AppOptions) -> Self {
        Self {
            prs: ListSection::new(options),
            issues: ListSection::new(options),
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
    /// each requests its own first page.
    pub(super) fn load_repo(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        self.prs.begin(repo, cx);
        self.issues.begin(repo, cx);
        self.prs.request_first_page(cx);
        self.issues.request_first_page(cx);
    }

    /// The palette showed the PR or the Issues section again with `repo` selected: Python's bare
    /// `fetch_issues_and_pull_requests`. Each first page is requested as on selection, but nothing is read from the
    /// cache and the tables aren't cleared: their rows stay until the page lands and replaces them.
    pub(super) fn reload(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        // The rows may be for an older repo, if it was selected while both sections were hidden; the pages and the
        // caches they write are for this one.
        self.prs.retarget(repo);
        self.issues.retarget(repo);
        self.prs.request_first_page(cx);
        self.issues.request_first_page(cx);
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

    /// Takes a response to either table's first or later page, and hands back any other. A page that was cancelled
    /// by a newer selection is handed back, so the app drops it (Py bug B45).
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let response = self.prs.on_response(id, response, cx)?;
        self.issues.on_response(id, response, cx)
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
