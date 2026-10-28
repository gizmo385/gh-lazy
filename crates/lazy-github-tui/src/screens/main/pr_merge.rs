//! Merging the pull request the details show (`pull_request.merge`): Python's
//! `PrOverviewTabPane.action_merge_pull_request`, with a refetch once it's merged (Py bug B15's recommendation).

use lazy_github_core::gh::GhError;
use lazy_github_core::github::{MergeRequest, MergeResult};
use lazy_github_core::models::{FullPullRequest, PartialPullRequest};

use super::{MainAction, PullRequestTarget};
use crate::action::Action;
use crate::component::Cx;
use crate::effect::{GithubRequest, GithubResponse, RequestId};
use crate::toast::Toast;

/// The merges in flight. Nothing stops a second `ctrl+m` from sending a second request, as in Python.
#[derive(Debug, Default)]
pub(super) struct PrMerges {
    pending: Vec<PendingMerge>,
}

/// One merge request, with the pull request it's for, which is fetched again once it's merged.
#[derive(Debug)]
struct PendingMerge {
    id: RequestId,
    pr: Box<PartialPullRequest>,
}

impl PrMerges {
    /// Merges `pr` with `pull_requests.preferred_merge_method` and its head commit, or warns that it's already merged.
    /// There's no confirmation (Py bug B30's recommendation).
    pub(super) fn start(&mut self, pr: &FullPullRequest, cx: &mut Cx<'_>) {
        if pr.merged_at.is_some() {
            cx.notify(Toast::warning("PR has already been merged!").with_title("Already Merged"));
            return;
        }
        let request = MergeRequest {
            merge_method: cx.config().pull_requests.preferred_merge_method,
            sha: pr.head.sha.clone(),
        };
        let id = cx.request(GithubRequest::MergePullRequest {
            repo: Box::new(pr.pr.issue.repo.clone()),
            number: pr.number(),
            request,
        });
        self.pending.push(PendingMerge {
            id,
            pr: Box::new(pr.pr.clone()),
        });
    }

    /// Takes the answer to a merge, and hands back any other response. A merge toasts and fetches the pull request
    /// again, shown as a fresh selection; a refusal or a failure toasts and changes nothing.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let GithubResponse::MergePullRequest(result) = response else {
            return Some(response);
        };
        let Some(index) = self.pending.iter().position(|pending| pending.id == id) else {
            return Some(GithubResponse::MergePullRequest(result));
        };
        let PendingMerge { pr, .. } = self.pending.remove(index);
        merged(pr, result, cx);
        None
    }
}

fn merged(pr: Box<PartialPullRequest>, result: Result<MergeResult, GhError>, cx: &mut Cx<'_>) {
    let number = pr.issue.number;
    let full_name = &pr.issue.repo.full_name;
    match result {
        Ok(result) if result.merged => {
            tracing::info!("Merged PR {number} in repo {full_name}");
            cx.notify(
                Toast::info(format!(
                    "Pull request {number} merged. Note some cached information on the UI may be out of date."
                ))
                .with_title("PR Merged"),
            );
            cx.dispatch(Action::Main(MainAction::OpenPullRequest {
                target: PullRequestTarget::Partial(pr),
                focus: true,
            }));
        }
        Ok(_) => {
            tracing::warn!("Failed to merge PR {number} in repo {full_name}");
            merge_failed(number, cx);
        }
        Err(error) => {
            tracing::error!("Failed to merge PR {number} in repo {full_name}");
            tracing::debug!("The merge request failed: {error}");
            merge_failed(number, cx);
        }
    }
}

fn merge_failed(number: u64, cx: &mut Cx<'_>) {
    cx.notify(Toast::error(format!("Pull request {number} could not be merged")).with_title("Error Merging PR"));
}
