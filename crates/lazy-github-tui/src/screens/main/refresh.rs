//! "Refresh from GitHub" on the main screen (#407, decision `cache-ttl-defaults-and-refresh`): every pane fetches what
//! it shows again, the way it already reloads, and keeps showing it until the answer lands. `App` marks the `gh` cache
//! stale first, so each of these reads goes past it.
//!
//! The tables reload through their own paths. The details pane's subject is fetched again here, from its own repo,
//! and the pane then rebuilds its tabs in place. That fetch is a selection-like request: a newer refresh or any new
//! selection cancels it, and its late answer is dropped.

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{FullPullRequest, IssueListItem, Repository, WorkflowRun};

use super::{MainScreen, SectionId, pull_request_failed};
use crate::component::Cx;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};

/// The details pane's subject being fetched again.
#[derive(Debug)]
pub(super) struct DetailsRefresh {
    id: RequestId,
    subject: Subject,
}

/// What [`DetailsRefresh`] fetches, by the request [`Subject::request`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    /// The Repo Overview's repo.
    Repo,
    /// The pull request shown.
    PullRequest,
    /// The issue shown.
    Issue,
    /// The workflow run shown.
    Run,
}

impl MainScreen {
    /// [`Action::Refresh`](crate::action::Action::Refresh) on the main screen, after `App` marked the cache stale: the
    /// repos table, the Pull Requests and Issues tables and the Workflows section when shown with a repo selected, and
    /// the details pane's subject each fetch what they show again. Focus, the active details tab and every cursor
    /// stay where they are. A refresh still running is started over.
    pub fn refresh(&mut self, cx: &mut Cx<'_>) {
        self.repos.refresh(cx);
        if let Some(repo) = cx.current_repo().cloned() {
            if self.section(SectionId::PullRequests).shown() || self.section(SectionId::Issues).shown() {
                self.issue_lists.refresh(&repo, cx);
            }
            if self.section(SectionId::Workflows).shown() {
                self.workflows.refresh(&repo, cx);
            }
        }
        self.refresh_details(cx);
    }

    /// Fetches the details pane's subject again, replacing any such fetch still running. The Introduction has none.
    fn refresh_details(&mut self, cx: &mut Cx<'_>) {
        self.cancel_details_refresh(cx);
        let (subject, request) = if let Some(repo) = self.details.repo() {
            (Subject::Repo, GithubRequest::Repository(repo.full_name.clone()))
        } else if let Some(details) = self.details.pull_request() {
            let issue = &details.pr().pr.issue;
            let request = GithubRequest::PullRequest {
                repo: Box::new(issue.repo.clone()),
                number: issue.number,
            };
            (Subject::PullRequest, request)
        } else if let Some(details) = self.details.issue() {
            let issue = details.issue();
            let request = GithubRequest::Issue {
                repo: Box::new(issue.repo.clone()),
                number: issue.number,
            };
            (Subject::Issue, request)
        } else if let Some(details) = self.details.run() {
            let run = details.run();
            let request = GithubRequest::WorkflowRun {
                repo: Box::new(run.repository.clone()),
                run_id: run.id,
            };
            (Subject::Run, request)
        } else {
            return;
        };
        let id = cx.request(request);
        self.details_refresh = Some(DetailsRefresh { id, subject });
    }

    /// Cancels the details subject's fetch, if one is running: a newer refresh or a new selection replaces it.
    pub(super) fn cancel_details_refresh(&mut self, cx: &mut Cx<'_>) {
        if let Some(refresh) = self.details_refresh.take() {
            cx.effect(Effect::Cancel(refresh.id));
        }
    }

    /// Takes the answer to the details subject's fetch, and hands back any other response.
    pub(super) fn on_details_refresh(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let Some(refresh) = self.details_refresh.take_if(|refresh| refresh.id == id) else {
            return Some(response);
        };
        let shown = match (refresh.subject, response) {
            (Subject::Repo, GithubResponse::Repository(result)) => self.refreshed_repo(result, cx),
            (Subject::PullRequest, GithubResponse::PullRequest(result)) => self.refreshed_pull_request(result, cx),
            (Subject::Issue, GithubResponse::Issue(result)) => self.refreshed_issue(result, cx),
            (Subject::Run, GithubResponse::WorkflowRun(result)) => self.refreshed_run(result, cx),
            // A request's response always has its kind, so this doesn't happen.
            (subject, response) => {
                tracing::debug!("Dropping a {response:?} that answered the refresh of a {subject:?}");
                true
            }
        };
        if !shown {
            tracing::debug!(
                "Dropping the refreshed {:?}: the details show something else",
                refresh.subject
            );
        }
        None
    }

    /// The repo arrived: the overview is rebuilt from it. A repo that's gone, or a failed fetch, still reloads the
    /// README, of the repo as it was, since that's what the overview shows on a pick.
    fn refreshed_repo(&mut self, result: Result<Option<Box<Repository>>, GhError>, cx: &mut Cx<'_>) -> bool {
        let repo = match result {
            Ok(Some(repo)) => *repo,
            other => {
                if let Err(error) = other {
                    tracing::debug!("Refreshing the overview's repo failed: {error}");
                }
                let Some(repo) = self.details.repo().cloned() else {
                    return false;
                };
                repo
            }
        };
        self.details.refresh_repo(&repo, cx)
    }

    /// The pull request arrived: its tabs load again. A failure keeps them, with the toast a failed pick shows.
    fn refreshed_pull_request(&mut self, result: Result<Box<FullPullRequest>, GhError>, cx: &mut Cx<'_>) -> bool {
        match result {
            Ok(pr) => self.details.refresh_pull_request(*pr, cx),
            Err(error) => {
                pull_request_failed(&error, cx);
                true
            }
        }
    }

    /// The issue arrived: its tabs load again. A failure keeps them, logged, as a failed issue read is elsewhere.
    fn refreshed_issue(&mut self, result: Result<Box<IssueListItem>, GhError>, cx: &mut Cx<'_>) -> bool {
        match result.map(|item| *item) {
            Ok(IssueListItem::Issue(issue)) => self.details.refresh_issue(issue, cx),
            // GitHub numbers issues and pull requests together, so a shown issue's number stays an issue's.
            Ok(IssueListItem::PullRequest(pr)) => {
                tracing::debug!(
                    "Issue #{} came back as a pull request; keeping the details",
                    pr.issue.number
                );
                true
            }
            Err(error) => {
                tracing::error!("Error refreshing the issue from the Github API");
                tracing::debug!("Refreshing the issue failed: {error}");
                true
            }
        }
    }

    /// The run arrived: its jobs load again. A failure keeps the tabs, logged.
    fn refreshed_run(&mut self, result: Result<Box<WorkflowRun>, GhError>, cx: &mut Cx<'_>) -> bool {
        match result {
            Ok(run) => self.details.refresh_run(*run, cx),
            Err(error) => {
                tracing::error!("Error refreshing the workflow run from the Github API");
                tracing::debug!("Refreshing the workflow run failed: {error}");
                true
            }
        }
    }
}

#[cfg(test)]
mod tests;
