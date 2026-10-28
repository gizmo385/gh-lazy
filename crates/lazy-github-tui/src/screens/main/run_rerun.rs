//! Re-running the workflow run the details show, from the Run Overview's buttons: Python's
//! `WorkflowRunOverviewTabPane.on_button_pressed`, with the run fetched again once it's queued (Py bug B15's
//! recommendation).

use lazy_github_core::gh::GhError;
use lazy_github_core::models::WorkflowRun;

use super::MainAction;
use crate::action::Action;
use crate::component::Cx;
use crate::effect::{GithubRequest, GithubResponse, RequestId};
use crate::toast::Toast;

/// The re-runs in flight. The buttons don't disable meanwhile, so a second press sends a second request, as in Python.
#[derive(Debug, Default)]
pub(super) struct RunReruns {
    pending: Vec<Pending>,
}

/// One re-run: its request, then the refetch of the run once it's queued.
#[derive(Debug)]
struct Pending {
    id: RequestId,
    /// The run as the details showed it, which shows again if the refetch fails.
    run: Box<WorkflowRun>,
    stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The re-run request, of the failed jobs only or of the whole run.
    Rerun { failed_only: bool },
    /// The run's refetch.
    Refetch,
}

impl RunReruns {
    /// Re-runs `run`'s failed jobs (`failed_only`) or the whole run.
    pub(super) fn start(&mut self, run: &WorkflowRun, failed_only: bool, cx: &mut Cx<'_>) {
        let id = cx.request(GithubRequest::RerunWorkflow {
            repo: Box::new(run.repository.clone()),
            run_id: run.id,
            failed_only,
        });
        self.pending.push(Pending {
            id,
            run: Box::new(run.clone()),
            stage: Stage::Rerun { failed_only },
        });
    }

    /// Takes the answer to a re-run or a refetch, and hands back any other response. A queued re-run toasts and
    /// fetches the run again; a refused one toasts and changes nothing. The refetched run shows as a fresh selection
    /// does, or the old one does if the refetch failed.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let Some(index) = self.pending.iter().position(|pending| pending.id == id) else {
            return Some(response);
        };
        match (self.pending[index].stage, response) {
            (Stage::Rerun { failed_only }, GithubResponse::RerunWorkflow(result)) => {
                let pending = self.pending.remove(index);
                self.rerun_answered(pending.run, failed_only, result, cx);
            }
            (Stage::Refetch, GithubResponse::WorkflowRun(result)) => {
                let pending = self.pending.remove(index);
                let run = result.unwrap_or_else(|error| {
                    tracing::error!("Error fetching workflow run {} after a re-run", pending.run.id);
                    tracing::debug!("Refetching the workflow run failed: {error}");
                    pending.run
                });
                cx.dispatch(Action::Main(MainAction::OpenWorkflowRun(run)));
            }
            (_, other) => return Some(other),
        }
        None
    }

    fn rerun_answered(
        &mut self,
        run: Box<WorkflowRun>,
        failed_only: bool,
        result: Result<(), GhError>,
        cx: &mut Cx<'_>,
    ) {
        let run_id = run.id;
        if let Err(error) = result {
            let (line, message) = if failed_only {
                (
                    format!("Error re-running failed jobs for workflow run {run_id}"),
                    "Failed to re-run failed jobs",
                )
            } else {
                (
                    format!("Error re-running workflow run {run_id}"),
                    "Failed to re-run workflow",
                )
            };
            tracing::error!("{line}");
            tracing::debug!("{line}: {error}");
            cx.notify(Toast::error(message).with_title("Error"));
            return;
        }
        let message = if failed_only {
            "Failed jobs queued for re-run"
        } else {
            "Workflow run queued for re-run"
        };
        cx.notify(Toast::info(message).with_title("Re-run Started"));
        let id = cx.request(GithubRequest::WorkflowRun {
            repo: Box::new(run.repository.clone()),
            run_id,
        });
        self.pending.push(Pending {
            id,
            run,
            stage: Stage::Refetch,
        });
    }
}
