//! Opening Edit PR (`pull_request.edit`) from the PR table and the PR Overview: Python's
//! `PullRequestsContainer.action_edit_pull_request` and `PrOverviewTabPane.action_edit_pull_request`.

use lazy_github_core::models::{FullPullRequest, PartialPullRequest};

use super::{EditOrigin, MainAction, pull_request_failed};
use crate::action::Action;
use crate::component::Cx;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::modal::PullRequestDialog;
use crate::toast::Toast;

/// The table's fetch of the full pull request to edit, while it's in flight. Only the latest `E` opens a dialog.
#[derive(Debug, Default)]
pub(super) struct PrEdit {
    pending: Option<RequestId>,
}

impl PrEdit {
    /// `E` on the PR table: the full pull request of the row under the cursor is fetched (Python's
    /// `get_full_pull_request`), replacing any fetch still out. An empty table toasts (Py bug B13's recommendation).
    pub(super) fn fetch_row(&mut self, row: Option<&PartialPullRequest>, cx: &mut Cx<'_>) {
        let Some(row) = row else {
            cx.notify(Toast::error("No pull request currently selected"));
            return;
        };
        let id = cx.request(GithubRequest::PullRequest {
            repo: Box::new(row.issue.repo.clone()),
            number: row.issue.number,
        });
        if let Some(old) = self.pending.replace(id) {
            cx.effect(Effect::Cancel(old));
        }
    }

    /// Takes the answer to the table's fetch, and hands back any other response. A failure toasts (Py bug B75's
    /// recommendation: Python crashes); a pull request opens Edit PR.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let GithubResponse::PullRequest(result) = response else {
            return Some(response);
        };
        self.pending = None;
        match result {
            Ok(pr) => open(pr, EditOrigin::Table, cx),
            Err(error) => pull_request_failed(&error, cx),
        }
        None
    }
}

/// Edit PR for `pr`, or the warning when it's merged (a closed, unmerged one opens). Once it's updated, the main
/// screen shows it, from `origin`.
pub(super) fn open(pr: Box<FullPullRequest>, origin: EditOrigin, cx: &mut Cx<'_>) {
    if pr.merged_at.is_some() {
        cx.notify(Toast::warning("This PR has already been merged!"));
        return;
    }
    let dialog = PullRequestDialog::edit(pr, cx);
    cx.open_dialog(dialog, move |pr| {
        pr.map_or(Action::Consumed, |pr| {
            Action::Main(MainAction::PullRequestUpdated(origin, pr))
        })
    });
}
