//! A workflow run's details: the Overview, Jobs and Logs tabs, and the run's jobs, requested once and shared by all
//! three. Python's `WorkflowRunOverviewTabPane`, `WorkflowRunJobsTabPane` and `WorkflowRunLogsTabPane`, which
//! `MainViewPane.load_workflow_run` puts in the details pane.
//!
//! Python asks for the jobs once per tab; here [`RunDetails`] asks once per selection and hands the answer to each
//! tab as [`RunDetails::jobs`]. The Logs tab (#244) is empty until it lands: it adds its body to [`RunDetails`], takes
//! the jobs in [`RunDetails::on_jobs`], and its own requests through [`DetailsBody::on_response`] and
//! [`DetailsBody::on_tab_shown`]. The job logs fetched for the selection, [`RunDetails::job_logs`], are shared.

mod jobs;
mod overview;
/// The state glyphs and durations, which the Jobs tab (#243) shares.
pub(in crate::screens::main) mod status;

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::MouseEvent;

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{WorkflowJob, WorkflowRun};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::button::BUTTON_BINDINGS;
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::tree::TREE_BINDINGS;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::theme::Theme;

use super::MainAction;
use super::details::DetailsBody;
pub(super) use jobs::JobsTab;
pub(super) use overview::RunOverview;

/// The Jobs tab's own bindings (Python's `WorkflowRunJobsTabPane.BINDINGS`), with focus on its body or its tree.
const JOBS_BINDINGS: &[BindingId] = &[BindingId::WorkflowsViewJobLogs];

/// The three tabs, in strip order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunTab {
    Overview,
    Jobs,
    Logs,
}

impl RunTab {
    const NAMES: &'static [&'static str] = &["Overview", "Jobs", "Logs"];

    fn at(index: usize) -> RunTab {
        match index {
            0 => RunTab::Overview,
            1 => RunTab::Jobs,
            _ => RunTab::Logs,
        }
    }
}

/// The run's jobs, as far as they've come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::screens::main) enum Jobs {
    /// Requested, with this id.
    Loading(RequestId),
    /// Arrived, in API order. A request that failed leaves none, as Python's fetcher returns `[]`.
    Loaded(Arc<[WorkflowJob]>),
}

/// One run's tabs.
#[derive(Debug)]
pub(in crate::screens::main) struct RunDetails {
    run: Arc<WorkflowRun>,
    jobs: Jobs,
    overview: RunOverview,
    jobs_tab: JobsTab,
    /// The logs fetched for the selection's jobs, by job id: `None` for a job that had none or whose logs failed to
    /// load. The Jobs tab shows logs from here without asking again; it asks again for a `None`.
    job_logs: HashMap<u64, Option<String>>,
}

impl RunDetails {
    /// The tabs of `run`, its jobs requested now.
    pub(super) fn new(run: WorkflowRun, cx: &mut Cx<'_>) -> Self {
        let id = cx.request(GithubRequest::WorkflowRunJobs {
            repo: Box::new(run.repository.clone()),
            run_id: run.id,
        });
        let jobs_tab = JobsTab::new(run.run_number, cx.keymap());
        let run = Arc::new(run);
        Self {
            overview: RunOverview::new(Arc::clone(&run)),
            run,
            jobs: Jobs::Loading(id),
            jobs_tab,
            job_logs: HashMap::new(),
        }
    }

    /// The run shown.
    pub(in crate::screens::main) fn run(&self) -> &WorkflowRun {
        &self.run
    }

    /// The run's jobs, `None` while they load. Empty when the run has none, or when they failed to load.
    #[cfg_attr(not(test), expect(dead_code, reason = "the Logs tab (#244) reads the shared jobs"))]
    pub(in crate::screens::main) fn jobs(&self) -> Option<&Arc<[WorkflowJob]>> {
        match &self.jobs {
            Jobs::Loading(_) => None,
            Jobs::Loaded(jobs) => Some(jobs),
        }
    }

    /// The Overview tab.
    #[cfg(test)]
    pub(in crate::screens::main) fn overview(&self) -> &RunOverview {
        &self.overview
    }

    /// The Jobs tab.
    #[cfg(test)]
    pub(in crate::screens::main) fn jobs_tab(&self) -> &JobsTab {
        &self.jobs_tab
    }

    /// The logs fetched for the selection's jobs, by job id.
    #[cfg_attr(not(test), expect(dead_code, reason = "the Logs tab (#244) reads the shared logs"))]
    pub(in crate::screens::main) fn job_logs(&self) -> &HashMap<u64, Option<String>> {
        &self.job_logs
    }

    /// `workflows.view_job_logs`: the logs of the job under the tree's cursor (or of its step's job), from those
    /// already fetched or else asked for. Nothing without a cursor or on `No jobs found`. Any request still on its way
    /// is cancelled.
    fn view_job_logs(&mut self, cx: &mut Cx<'_>) {
        let Some(index) = self.jobs_tab.cursor_job() else {
            return;
        };
        let Some(job_id) = self.jobs_tab.job(index).map(|job| job.id) else {
            return;
        };
        if let Some(request) = self.jobs_tab.loading() {
            cx.effect(Effect::Cancel(request));
        }
        if let Some(Some(text)) = self.job_logs.get(&job_id) {
            self.jobs_tab.set_logs_text(index, Some(text));
            return;
        }
        let request = cx.request(GithubRequest::JobLogs {
            repo: Box::new(self.run.repository.clone()),
            job_id,
        });
        self.jobs_tab.set_loading(index, request);
    }

    /// The logs request answered. A failure or an empty body is logged as Python's `get_job_logs` logs a failure,
    /// without a toast.
    fn on_job_logs(&mut self, result: Result<Option<String>, GhError>) {
        let Some(index) = self.jobs_tab.logs_job() else {
            return;
        };
        let Some(job_id) = self.jobs_tab.job(index).map(|job| job.id) else {
            return;
        };
        let logs = match result {
            Ok(Some(text)) => Some(text),
            Ok(None) => {
                tracing::error!("Error retrieving logs for job {job_id}");
                None
            }
            Err(error) => {
                tracing::error!("Error retrieving logs for job {job_id}");
                tracing::debug!("Error retrieving logs for job {job_id}: {error}");
                None
            }
        };
        self.jobs_tab.set_logs_text(index, logs.as_deref());
        self.job_logs.insert(job_id, logs);
    }

    /// The jobs request answered: every tab gets the jobs. A failure is logged as Python's `get_workflow_jobs` logs
    /// it, without a toast, and leaves no jobs.
    fn on_jobs(&mut self, result: Result<Vec<WorkflowJob>, GhError>, theme: &Theme) {
        let jobs: Arc<[WorkflowJob]> = match result {
            Ok(jobs) => jobs.into(),
            Err(error) => {
                let line = format!(
                    "Error retrieving jobs for workflow run {} from the Github API",
                    self.run.id
                );
                tracing::error!("{line}");
                tracing::debug!("{line}: {error}");
                Arc::from([])
            }
        };
        self.overview.set_jobs(Arc::clone(&jobs));
        self.jobs_tab.set_jobs(Arc::clone(&jobs), theme);
        self.jobs = Jobs::Loaded(jobs);
    }
}

impl DetailsBody for RunDetails {
    fn tab_names(&self) -> &'static [&'static str] {
        RunTab::NAMES
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        match response {
            GithubResponse::WorkflowRunJobs(result) if self.jobs == Jobs::Loading(id) => {
                self.on_jobs(result, cx.theme());
                None
            }
            GithubResponse::JobLogs(result) if self.jobs_tab.loading() == Some(id) => {
                self.on_job_logs(result);
                None
            }
            other => Some(other),
        }
    }

    fn close(&self, cx: &mut Cx<'_>) {
        if let Jobs::Loading(id) = self.jobs {
            cx.effect(Effect::Cancel(id));
        }
        if let Some(id) = self.jobs_tab.loading() {
            cx.effect(Effect::Cancel(id));
        }
    }

    fn scroll(&mut self, tab: usize, scroll: Scroll) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.scroll(scroll),
            RunTab::Jobs => self.jobs_tab.scroll(scroll),
            RunTab::Logs => {}
        }
    }

    fn wheel(&mut self, tab: usize, wheel: Wheel) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.wheel(wheel),
            RunTab::Jobs => self.jobs_tab.wheel(wheel),
            RunTab::Logs => {}
        }
    }

    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.render(frame, area, theme, focused),
            RunTab::Jobs => self.jobs_tab.render(frame, area, theme, focused),
            RunTab::Logs => {}
        }
    }

    fn focus_step(&mut self, tab: usize, forward: bool) -> bool {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.focus_step(forward),
            RunTab::Jobs => self.jobs_tab.focus_step(forward),
            RunTab::Logs => false,
        }
    }

    fn focus_first(&mut self, tab: usize) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.focus_first(),
            RunTab::Jobs => self.jobs_tab.focus_first(),
            RunTab::Logs => {}
        }
    }

    fn focus_last(&mut self, tab: usize) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.focus_last(),
            RunTab::Jobs => self.jobs_tab.focus_last(),
            RunTab::Logs => {}
        }
    }

    fn click(&mut self, tab: usize, position: Position) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.click(position),
            RunTab::Jobs => self.jobs_tab.click(position),
            RunTab::Logs => {}
        }
    }

    fn mouse(&mut self, tab: usize, mouse: MouseEvent) -> bool {
        RunTab::at(tab) == RunTab::Jobs && self.jobs_tab.mouse(mouse)
    }

    fn maximizable(&self, tab: usize) -> bool {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.button_focused(),
            RunTab::Jobs => self.jobs_tab.tree_focused(),
            RunTab::Logs => false,
        }
    }

    fn chain(&self, tab: usize) -> Vec<ChainLink> {
        match RunTab::at(tab) {
            RunTab::Overview if self.overview.button_focused() => {
                vec![ChainLink::new(Owner::DetailsButton, BUTTON_BINDINGS)]
            }
            RunTab::Jobs if self.jobs_tab.tree_focused() => vec![
                ChainLink::new(Owner::Tree, TREE_BINDINGS),
                ChainLink::new(Owner::RunJobs, JOBS_BINDINGS),
            ],
            RunTab::Jobs => vec![ChainLink::new(Owner::RunJobs, JOBS_BINDINGS)],
            RunTab::Overview | RunTab::Logs => Vec::new(),
        }
    }

    /// The tree's keys act at once; `l` asks for logs, so it runs in [`Self::on_command`]. Re-running (#188) isn't
    /// built yet: `enter` on a button is consumed and does nothing.
    fn on_binding(&mut self, _tab: usize, owner: Owner, id: BindingId) -> Option<Action> {
        match owner {
            Owner::Tree => self.jobs_tab.tree_binding(id).then_some(Action::Consumed),
            Owner::RunJobs => Some(Action::Main(MainAction::DetailsCommand(owner, id))),
            _ => Some(Action::Unimplemented(id)),
        }
    }

    fn on_command(&mut self, tab: usize, owner: Owner, id: BindingId, cx: &mut Cx<'_>) {
        if RunTab::at(tab) == RunTab::Jobs && owner == Owner::RunJobs && id == BindingId::WorkflowsViewJobLogs {
            self.view_job_logs(cx);
        }
    }
}

#[cfg(test)]
pub(in crate::screens::main) mod tests;
