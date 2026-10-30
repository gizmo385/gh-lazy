//! Preloading the pull request for the current commit: when a repo loads and the working directory is on another
//! branch than the repo's default, the pull requests GitHub associates with the local HEAD commit are looked up, and
//! a single match is shown in the details without moving focus. Python's `SelectionsPane.load_repository` (the
//! `preload_pull_request_for_current_commit` branch) and `load_pull_request_for_current_commit`.

use lazy_github_core::models::{PartialPullRequest, Repository};

use crate::component::Cx;
use crate::effect::{GithubRequest, GithubResponse, RequestId};

/// The lookup for the repo that loaded last.
#[derive(Debug, Default)]
pub(super) struct CommitPreload {
    /// A repo that loaded before git said which branch and commit are checked out: checked once it has.
    waiting: Option<Repository>,
    /// The lookup in flight.
    pending: Option<RequestId>,
}

impl CommitPreload {
    /// A repo loaded with its pull request or issues section shown: looks up the current commit's pull requests when
    /// the setting is on, the repo has a default branch and the working directory isn't on it. A lookup for an earlier
    /// repo is dropped. Python doesn't check that the working directory is a clone of `repo`, so neither does this.
    pub(super) fn repo_loaded(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        self.waiting = None;
        self.pending = None;
        if !cx.config().pull_requests.preload_pull_request_for_current_commit {
            return;
        }
        let Some(default_branch) = &repo.default_branch else {
            return;
        };
        if !cx.local_head_known() {
            self.waiting = Some(repo.clone());
            return;
        }
        if cx.current_branch() == Some(default_branch.as_str()) {
            return;
        }
        let Some(sha) = cx.current_commit().map(str::to_owned) else {
            return;
        };
        self.pending = Some(cx.request(GithubRequest::PullRequestsForCommit {
            repo: Box::new(repo.clone()),
            sha,
        }));
    }

    /// Git has said which branch and commit are checked out: a repo that loaded before then is checked now.
    pub(super) fn local_head_known(&mut self, cx: &mut Cx<'_>) {
        if let Some(repo) = self.waiting.take() {
            self.repo_loaded(&repo, cx);
        }
    }

    /// A selection replaced whatever the details showed: a lookup still running is no longer wanted.
    pub(super) fn cancel(&mut self) {
        self.waiting = None;
        self.pending = None;
    }

    /// Takes the answer to the lookup, giving the pull request to show when exactly one matched, and hands back any
    /// other response. A failure is logged and shows nothing, as in Python.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
    ) -> Result<Option<PartialPullRequest>, GithubResponse> {
        let GithubResponse::PullRequestsForCommit(result) = response else {
            return Err(response);
        };
        if self.pending != Some(id) {
            tracing::debug!("Dropping the current commit's pull requests {id}: the lookup was replaced");
            return Ok(None);
        }
        self.pending = None;
        match result {
            Ok(mut prs) if prs.len() == 1 => {
                tracing::info!("Loading PR for your current commit");
                Ok(prs.pop())
            }
            Ok(prs) => {
                tracing::debug!("{} pull requests for the current commit: none preloaded", prs.len());
                Ok(None)
            }
            Err(error) => {
                tracing::error!("Error loading PRs for current commit: {error}");
                Ok(None)
            }
        }
    }
}
