//! Preloading the pull request for the working directory's commit when a repo loads: Python's
//! `load_pull_request_for_current_commit`, behind `pull_requests.preload_pull_request_for_current_commit`.
//!
//! The preload is a selection like any other: a newer one cancels it, and a late answer is dropped (Py bug B109's
//! answer). The pull requests found belong to the repo asked, not whichever is selected when the answer arrives.

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{PartialPullRequest, Repository};

use super::{MainAction, PullRequestTarget};
use crate::action::Action;
use crate::component::{Cx, LocalHead};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};

/// GitHub's answer when the commit isn't in the repo asked: no pull requests there (Py bug B110's answer).
const UNPROCESSABLE: u16 = 422;

/// A preload in progress, for the repo selected last.
#[derive(Debug)]
enum Preload {
    /// Git hasn't said yet which branch and commit the working directory is on.
    WaitingForGit { repo: Box<Repository> },
    /// The commit's pull requests are being looked up in `repo`.
    LookingUp { id: RequestId },
}

/// The main screen's preload: at most one, for the latest repo selection.
#[derive(Debug, Default)]
pub(super) struct CommitPreload {
    current: Option<Preload>,
}

impl CommitPreload {
    /// `repo` was selected, with the Pull Requests or Issues section shown when `sections_shown`: Python's
    /// `load_repository`. The lookup starts once git has said where the working directory is, unless the setting is
    /// off, the sections are hidden, the repo has no default branch, or that's the branch checked out.
    pub(super) fn start(&mut self, repo: &Repository, sections_shown: bool, cx: &mut Cx<'_>) {
        self.cancel(cx);
        if !cx.config().pull_requests.preload_pull_request_for_current_commit
            || !sections_shown
            || repo.default_branch.is_none()
        {
            return;
        }
        self.current = Some(Preload::WaitingForGit {
            repo: Box::new(repo.clone()),
        });
        self.on_local_head(cx);
    }

    /// Git answered for the branch or the commit: a preload waiting on them looks the commit up, or ends when it
    /// shouldn't.
    pub(super) fn on_local_head(&mut self, cx: &mut Cx<'_>) {
        if !matches!(self.current, Some(Preload::WaitingForGit { .. })) {
            return;
        }
        let LocalHead::Known { branch, commit } = cx.local_head() else {
            return;
        };
        let Some(Preload::WaitingForGit { repo }) = self.current.take() else {
            return;
        };
        // A detached HEAD (`HEAD`) and no branch at all aren't the default branch, as in Python.
        if branch.is_some() && branch == repo.default_branch {
            return;
        }
        let Some(sha) = commit else {
            tracing::debug!("No preload for {}: no current-directory commit", repo.full_name);
            return;
        };
        let id = cx.request(GithubRequest::PullRequestsForCommit { repo, sha });
        self.current = Some(Preload::LookingUp { id });
    }

    /// A newer selection replaces the preload: its lookup is cancelled, and its answer dropped if it comes anyway.
    pub(super) fn cancel(&mut self, cx: &mut Cx<'_>) {
        if let Some(Preload::LookingUp { id }) = self.current.take() {
            cx.effect(Effect::Cancel(id));
        }
    }

    /// Takes the answer to the lookup, and hands back any other response. Exactly one pull request opens without
    /// moving focus; none or several leave the details as they are.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let GithubResponse::PullRequestsForCommit(result) = response else {
            return Some(response);
        };
        if !matches!(self.current, Some(Preload::LookingUp { id: current }) if current == id) {
            tracing::debug!("Dropping the commit's pull requests {id}: a newer selection replaced the preload");
            return None;
        }
        self.current = None;
        match result.map(<[PartialPullRequest; 1]>::try_from) {
            Ok(Ok([pr])) => {
                tracing::info!("Loading PR for your current commit");
                cx.dispatch(Action::Main(MainAction::OpenPullRequest {
                    target: PullRequestTarget::Partial(Box::new(pr)),
                    focus: false,
                }));
            }
            Ok(Err(prs)) => tracing::debug!("No preload: {} pull requests hold the current commit", prs.len()),
            Err(GhError::Http { status, .. }) if status == UNPROCESSABLE => {
                tracing::debug!("No preload: the current commit isn't in the selected repo (HTTP {status})");
            }
            Err(error) => {
                tracing::error!("Error loading PRs for current commit");
                tracing::debug!("Looking up the current commit's pull requests failed: {error}");
            }
        }
        None
    }
}
