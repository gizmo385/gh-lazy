//! The palette's "Open Bug Report" (Python's `action_open_bug_report`): LazyGithub's own repo is loaded and selected
//! as a table selection would, then New issue opens there with the bug-report template in its description.

use lazy_github_core::config::Config;
use lazy_github_core::debug::{DebugEnv, bug_report_template, debug_markdown};
use lazy_github_core::models::Repository;

use crate::action::Action;
use crate::component::Cx;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::modal::NewIssueDialog;
use crate::screens::main::MainAction;
use crate::toast::Toast;

/// The repo bug reports are filed in.
pub const LAZY_GITHUB_REPO: &str = "gizmo385/gh-lazy";

/// Where a bug report is. `App` owns one and tells it when gh's version becomes known.
#[derive(Debug, Default)]
pub enum BugReport {
    /// None running.
    #[default]
    Idle,
    /// The repo request in flight.
    Loading(RequestId),
    /// The repo loaded and selected; New issue opens once gh has said its version.
    WaitingForGh(Box<Repository>),
}

impl BugReport {
    /// Asks for the repo, replacing a bug report still loading: its request is cancelled.
    pub fn start(&mut self, cx: &mut Cx<'_>) {
        if let Self::Loading(old) = *self {
            cx.effect(Effect::Cancel(old));
        }
        *self = Self::Loading(cx.request(GithubRequest::Repository(LAZY_GITHUB_REPO.to_owned())));
    }

    /// Whether `id` is the repo request this is waiting on.
    pub fn waits_for(&self, id: RequestId) -> bool {
        matches!(*self, Self::Loading(pending) if pending == id)
    }

    /// The repo request's answer ([`Self::waits_for`]). The repo is selected as a table selection selects it, and New
    /// issue opens at once if `env` is known (gh has answered); a missing repo or a failure toasts and changes nothing
    /// else.
    pub fn on_repo(&mut self, response: GithubResponse, env: Option<DebugEnv>, cx: &mut Cx<'_>) {
        *self = Self::Idle;
        match response {
            GithubResponse::Repository(Ok(Some(repo))) => {
                cx.dispatch(Action::SelectRepo(repo.clone()));
                *self = Self::WaitingForGh(repo);
                if let Some(env) = env {
                    self.gh_known(&env, cx);
                }
            }
            GithubResponse::Repository(Ok(None)) => failed(&format!("{LAZY_GITHUB_REPO} wasn't found"), cx),
            GithubResponse::Repository(Err(error)) => failed(&error.to_string(), cx),
            other => failed(&format!("unexpected response {other:?}"), cx),
        }
    }

    /// gh's version is known (`env` holds it): New issue opens if the repo is waiting for it, with the config as it
    /// is now.
    pub fn gh_known(&mut self, env: &DebugEnv, cx: &mut Cx<'_>) {
        if !matches!(self, Self::WaitingForGh(_)) {
            return;
        }
        if let Self::WaitingForGh(repo) = std::mem::take(self) {
            let dialog = dialog(*repo, env, cx.config());
            cx.open_dialog(dialog, |issue| {
                issue.map_or(Action::Consumed, |issue| Action::Main(MainAction::IssueCreated(issue)))
            });
        }
    }
}

/// New issue for `repo` with the bug-report template around the debug info from `env` and `config`.
pub fn dialog(repo: Repository, env: &DebugEnv, config: &Config) -> NewIssueDialog {
    NewIssueDialog::new(repo, Some(&bug_report_template(&debug_markdown(env, config))))
}

fn failed(reason: &str, cx: &mut Cx<'_>) {
    tracing::debug!("Couldn't load {LAZY_GITHUB_REPO} for a bug report: {reason}");
    cx.notify(
        Toast::error("Could not load LazyGithub repo! Please open the bug report on Github")
            .with_title("Error opening bug report"),
    );
}

#[cfg(test)]
mod tests;
