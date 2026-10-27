//! A workflow run's details: the Overview, Jobs and Logs tabs, and the run's jobs, requested once and shared by all
//! three. Python's `WorkflowRunOverviewTabPane`, `WorkflowRunJobsTabPane` and `WorkflowRunLogsTabPane`, which
//! `MainViewPane.load_workflow_run` puts in the details pane.
//!
//! Python asks for the jobs once per tab; here [`RunDetails`] asks once per selection and hands the answer to each
//! tab as [`RunDetails::jobs`]. The Jobs tab (#243) and the Logs tab (#244) are empty until they land: each adds its
//! body to [`RunDetails`], takes the jobs in [`RunDetails::on_jobs`], and its own requests through
//! [`DetailsBody::on_response`] and [`DetailsBody::on_tab_shown`].

mod overview;
/// The state glyphs and durations, which the Jobs tab (#243) shares.
pub(in crate::screens::main) mod status;

use std::sync::Arc;

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{WorkflowJob, WorkflowRun};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::button::BUTTON_BINDINGS;
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::theme::Theme;

use super::details::DetailsBody;
pub(super) use overview::RunOverview;

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
}

impl RunDetails {
    /// The tabs of `run`, its jobs requested now.
    pub(super) fn new(run: WorkflowRun, cx: &mut Cx<'_>) -> Self {
        let id = cx.request(GithubRequest::WorkflowRunJobs {
            repo: Box::new(run.repository.clone()),
            run_id: run.id,
        });
        let run = Arc::new(run);
        Self {
            overview: RunOverview::new(Arc::clone(&run)),
            run,
            jobs: Jobs::Loading(id),
        }
    }

    /// The run shown.
    pub(in crate::screens::main) fn run(&self) -> &WorkflowRun {
        &self.run
    }

    /// The run's jobs, `None` while they load. Empty when the run has none, or when they failed to load.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the Jobs (#243) and Logs (#244) tabs read the shared jobs")
    )]
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

    /// The jobs request answered: every tab gets the jobs. A failure is logged as Python's `get_workflow_jobs` logs
    /// it, without a toast, and leaves no jobs.
    fn on_jobs(&mut self, result: Result<Vec<WorkflowJob>, GhError>) {
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
        self.jobs = Jobs::Loaded(jobs);
    }
}

impl DetailsBody for RunDetails {
    fn tab_names(&self) -> &'static [&'static str] {
        RunTab::NAMES
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, _cx: &mut Cx<'_>) -> Option<GithubResponse> {
        match response {
            GithubResponse::WorkflowRunJobs(result) if self.jobs == Jobs::Loading(id) => {
                self.on_jobs(result);
                None
            }
            other => Some(other),
        }
    }

    fn close(&self, cx: &mut Cx<'_>) {
        if let Jobs::Loading(id) = self.jobs {
            cx.effect(Effect::Cancel(id));
        }
    }

    fn scroll(&mut self, tab: usize, scroll: Scroll) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.scroll(scroll),
            RunTab::Jobs | RunTab::Logs => {}
        }
    }

    fn wheel(&mut self, tab: usize, wheel: Wheel) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.wheel(wheel),
            RunTab::Jobs | RunTab::Logs => {}
        }
    }

    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.render(frame, area, theme, focused),
            RunTab::Jobs | RunTab::Logs => {}
        }
    }

    fn focus_step(&mut self, tab: usize, forward: bool) -> bool {
        match RunTab::at(tab) {
            RunTab::Overview => self.overview.focus_step(forward),
            RunTab::Jobs | RunTab::Logs => false,
        }
    }

    fn focus_first(&mut self, tab: usize) {
        if RunTab::at(tab) == RunTab::Overview {
            self.overview.focus_first();
        }
    }

    fn focus_last(&mut self, tab: usize) {
        if RunTab::at(tab) == RunTab::Overview {
            self.overview.focus_last();
        }
    }

    fn click(&mut self, tab: usize, position: Position) {
        if RunTab::at(tab) == RunTab::Overview {
            self.overview.click(position);
        }
    }

    fn maximizable(&self, tab: usize) -> bool {
        RunTab::at(tab) == RunTab::Overview && self.overview.button_focused()
    }

    fn chain(&self, tab: usize) -> Vec<ChainLink> {
        match RunTab::at(tab) {
            RunTab::Overview if self.overview.button_focused() => {
                vec![ChainLink::new(Owner::DetailsButton, BUTTON_BINDINGS)]
            }
            RunTab::Overview | RunTab::Jobs | RunTab::Logs => Vec::new(),
        }
    }

    /// Re-running (#188) isn't built yet: `enter` on a button is consumed and does nothing.
    fn on_binding(&mut self, _tab: usize, _owner: Owner, id: BindingId) -> Option<Action> {
        Some(Action::Unimplemented(id))
    }
}

#[cfg(test)]
pub(in crate::screens::main) mod tests;
