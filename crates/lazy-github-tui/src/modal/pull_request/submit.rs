//! Submitting Create PR: the check of the required fields, the create request, the review request for the listed
//! reviewers, and, after a refused create, the remote branch check and the push prompt. Python's `_create_pr`,
//! `ensure_branch_exists_on_remote` and `check_for_branch_creation`.

use lazy_github_core::gh::GhError;
use lazy_github_core::github::NewPullRequest;
use lazy_github_core::models::FullPullRequest;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{PrField, PullRequestAction, PullRequestDialog};
use crate::action::Action;
use crate::component::Cx;
use crate::effect::{Effect, GitRequest, GitResponse, GithubRequest, GithubResponse, RequestId};
use crate::modal::{ConfirmDialog, DialogAction};
use crate::theme::Accent;
use crate::toast::Toast;

/// A submit in flight. While there is one, another submit does nothing.
#[derive(Debug)]
pub(super) enum Submit {
    /// The create request.
    Creating(RequestId),
    /// The pull request was created; the review request for the listed reviewers is out.
    RequestingReviews {
        id: RequestId,
        pr: Box<FullPullRequest>,
        reviewers: Vec<String>,
    },
}

impl Submit {
    fn id(&self) -> RequestId {
        match self {
            Submit::Creating(id) | Submit::RequestingReviews { id, .. } => *id,
        }
    }
}

/// The work that follows a refused create, which runs while the form stays usable.
#[derive(Debug, Default)]
pub(super) struct BranchFollowUp {
    /// `git ls-remote origin <branch>`, with the branch.
    check: Option<(RequestId, String)>,
    /// The branch the push prompt asks about, while it's open.
    prompt: Option<String>,
    /// The push, with the branch. Never cancelled: a closed dialog just drops its answer.
    push: Option<(RequestId, String)>,
}

impl PullRequestDialog {
    /// `modal.submit` or Create. Title, Compare and Base must each be non-empty as typed (a space passes); the empty
    /// ones are marked. Then the create request goes out.
    pub(super) fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.submit.is_some() {
            tracing::debug!("Ignoring a Create PR submit: one is in flight");
            return;
        }
        // Every field is checked, so each empty one is marked.
        let valid = [PrField::Title, PrField::Compare, PrField::Base]
            .map(|field| self.validate(field))
            .iter()
            .all(|&valid| valid);
        if !valid {
            cx.notify(Toast::error("Missing required fields!").with_title("Invalid PR!"));
            return;
        }
        cx.notify(Toast::info("Creating PR..."));
        let pr = NewPullRequest::new(
            &self.repo,
            self.title.value(),
            self.draft.value(),
            self.base.value(),
            self.compare.value(),
            &self.description.text(),
        );
        self.submit = Some(Submit::Creating(cx.request(GithubRequest::CreatePullRequest {
            repo: Box::new(self.repo.clone()),
            pr,
        })));
    }

    /// Cancels the submit in flight and the remote check, for a dialog that closes. A push already started runs to
    /// the end, so a branch is never half pushed; its answer is dropped with the dialog.
    pub(super) fn cancel_submit(&mut self, cx: &mut Cx<'_>) {
        let check = self.follow_up.check.take().map(|(id, _)| id);
        for id in self.submit.take().map(|submit| submit.id()).into_iter().chain(check) {
            cx.effect(Effect::Cancel(id));
        }
    }

    /// Takes the answer to the create or review request, and hands back any other.
    pub(super) fn on_submit_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        if self.submit.as_ref().map(Submit::id) != Some(id) {
            return Some(response);
        }
        match (self.submit.take(), response) {
            (Some(Submit::Creating(_)), GithubResponse::CreatePullRequest(result)) => self.on_created(result, cx),
            (Some(Submit::RequestingReviews { pr, reviewers, .. }), GithubResponse::RequestReviewers(result)) => {
                self.finish(pr, result.err().map(|error| (reviewers, error)), cx);
            }
            (submit, other) => {
                self.submit = submit;
                return Some(other);
            }
        }
        None
    }

    /// The create answered. Created: the listed reviewers are asked for, if any, then the dialog closes with it.
    /// Refused (any error): the toast, the form kept, and the Compare branch, as typed, checked on the remote.
    fn on_created(&mut self, result: Result<Box<FullPullRequest>, GhError>, cx: &mut Cx<'_>) {
        match result {
            Ok(pr) => {
                let reviewers = self.reviewers.reviewers();
                if reviewers.is_empty() {
                    self.finish(pr, None, cx);
                    return;
                }
                tracing::info!("Requesting PR reviews from: {}", reviewers.join(", "));
                let id = cx.request(GithubRequest::RequestReviewers {
                    repo: Box::new(self.repo.clone()),
                    number: pr.number(),
                    reviewers: reviewers.clone(),
                });
                self.submit = Some(Submit::RequestingReviews { id, pr, reviewers });
            }
            Err(error) => {
                tracing::error!("Error while attempting to create new pull request: {error}");
                cx.notify(
                    Toast::error("Check that your branches are valid and that a PR does not already exist")
                        .with_title("Error creating pull request"),
                );
                let branch = self.compare.value().to_owned();
                let id = cx.git(GitRequest::BranchExistsOnRemote { branch: branch.clone() });
                if let Some((old, _)) = self.follow_up.check.replace((id, branch)) {
                    cx.effect(Effect::Cancel(old));
                }
            }
        }
    }

    /// Done: the success toast, then (Py bug B77's recommendation) one for a refused review request, and the dialog
    /// closes with the pull request. Drafts (#186) are cleared here.
    fn finish(&mut self, pr: Box<FullPullRequest>, refused: Option<(Vec<String>, GhError)>, cx: &mut Cx<'_>) {
        cx.notify(Toast::info("Successfully created PR!"));
        if let Some((reviewers, error)) = refused {
            tracing::error!("Requesting reviews from {} failed: {error}", reviewers.join(", "));
            cx.notify(
                Toast::error(format!("Could not request reviews from {}", reviewers.join(", ")))
                    .with_title("Error requesting reviews"),
            );
        }
        self.output = Some(Some(pr));
    }

    /// Takes the answer to the remote check or the push, and hands back any other.
    pub(super) fn on_branch_git(
        &mut self,
        id: RequestId,
        response: GitResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GitResponse> {
        match response {
            GitResponse::BranchExistsOnRemote(exists) if self.follow_up.check.as_ref().is_some_and(|c| c.0 == id) => {
                let (_, branch) = self.follow_up.check.take()?;
                if !exists {
                    self.ask_to_push(branch, cx);
                }
            }
            GitResponse::PushBranch(result) if self.follow_up.push.as_ref().is_some_and(|p| p.0 == id) => {
                let (_, branch) = self.follow_up.push.take()?;
                match result {
                    Ok(()) => cx.notify(Toast::info(Line::from(vec![
                        Span::raw("Pushed branch "),
                        Span::styled(branch, Style::new().fg(cx.theme().accent(Accent::GreenYellow))),
                        Span::raw(" to remote"),
                    ]))),
                    Err(error) => {
                        tracing::error!("Pushing {branch} failed: {error}");
                        push_failed(&branch, cx);
                    }
                }
            }
            other => return Some(other),
        }
        None
    }

    /// The branch may not be on the remote (or the check failed): the Confirm asks whether to push it. Its answer
    /// comes back as [`PullRequestAction::PushAnswer`].
    fn ask_to_push(&mut self, branch: String, cx: &mut Cx<'_>) {
        let yellow = Style::new().fg(cx.theme().accent(Accent::Yellow));
        let question = Line::from(vec![
            Span::styled("Warning:", yellow),
            Span::raw(" Branch "),
            Span::styled(branch.clone(), yellow),
            Span::raw(" may not exist on remote. Do you want to create it?"),
        ]);
        self.follow_up.prompt = Some(branch);
        cx.open_dialog(ConfirmDialog::new(question), |yes| {
            Action::Dialog(DialogAction::PullRequest(PullRequestAction::PushAnswer(yes)))
        });
    }

    /// The push prompt's answer. Yes pushes the branch, but only from a clone of the selected repo: anywhere else
    /// it's a failed push, and git doesn't run (Python's `push_branch_to_remote`). Nothing is resubmitted.
    pub(super) fn on_push_answer(&mut self, yes: bool, cx: &mut Cx<'_>) {
        let Some(branch) = self.follow_up.prompt.take() else {
            return;
        };
        if !yes {
            return;
        }
        if !cx.local_repo().matches(&self.repo.full_name) {
            tracing::error!(
                "Not pushing {branch}: the current directory isn't a clone of {}",
                self.repo.full_name
            );
            push_failed(&branch, cx);
            return;
        }
        let id = cx.git(GitRequest::PushBranch { branch: branch.clone() });
        self.follow_up.push = Some((id, branch));
    }
}

/// "Error while pushing branch <branch> to remote", the branch in red.
fn push_failed(branch: &str, cx: &mut Cx<'_>) {
    let red = Style::new().fg(cx.theme().accent(Accent::Red));
    cx.notify(Toast::error(Line::from(vec![
        Span::raw("Error while pushing branch "),
        Span::styled(branch.to_owned(), red),
        Span::raw(" to remote"),
    ])));
}
