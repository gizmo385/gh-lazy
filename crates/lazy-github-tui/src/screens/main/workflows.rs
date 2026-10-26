//! The Workflows section: the selected repo's workflow runs, newest first, and its workflows alongside for the Trigger
//! workflow modal (#188). Python's `WorkflowsContainer` and `WorkflowRunsContainer`.

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::PageQuery;
use lazy_github_core::models::{Repository, Workflow, WorkflowRun};

use super::list::{ListSection, ListSource, PER_PAGE, PageFailure};
use crate::component::Cx;
use crate::components::table::{Cell, Column, Page, SearchableTable, SortSpec, TableItem};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::options::AppOptions;

/// The command-log line for a failed workflows request.
const WORKFLOWS_ERROR: &str = "Error retrieving actions from the Github API";
/// The command-log line for a failed runs page.
const RUNS_ERROR: &str = "Error retrieving action runs from the Github API";

/// Logs a failed request as Python's fetchers do: one ERROR line, no toast, then the error itself at debug. Python
/// crashes on a body that doesn't decode (Py bug B43); here that's logged like an HTTP error.
fn log_failure(line: &str, error: &GhError) {
    tracing::error!("{line}");
    tracing::debug!("{line}: {error}");
}

impl TableItem for WorkflowRun {
    /// The run's `id`, unique across workflows: Python keys by `run_number`, which restarts per workflow (Py bug B4).
    type Key = u64;
    type Ctx = ();

    fn key(&self) -> u64 {
        self.id
    }

    /// Python's `workflow_run_to_cell`. The time is UTC (Py bug B26) and a string, so it sorts as one: runs in the
    /// same minute keep API order. Titles are drawn as typed (Py bug B46).
    fn cells(&self, (): &()) -> Vec<Cell> {
        let result = match self.conclusion.as_deref() {
            Some(conclusion) if !conclusion.is_empty() => conclusion,
            _ => &self.status,
        };
        vec![
            Cell::text(self.created_at.strftime("%Y-%m-%d %H:%M").to_string()),
            Cell::text(result),
            Cell::text(self.name.clone()),
            Cell::text(self.display_title.clone()),
            Cell::int(i64::try_from(self.run_number).unwrap_or(i64::MAX)),
        ]
    }
}

/// What fills the Workflows section: the runs.
pub(super) struct RunsSource;

impl ListSource for RunsSource {
    type Item = WorkflowRun;

    fn columns() -> Vec<Column> {
        ["Time", "Result", "Job Name", "Run Name", "Run #"]
            .map(Column::new)
            .into()
    }

    fn sort() -> SortSpec {
        SortSpec::descending(0)
    }

    fn page_request(repo: &Repository, page: u32, per_page: u32, _config: &Config) -> GithubRequest {
        GithubRequest::ListWorkflowRuns {
            repo: Box::new(repo.clone()),
            page: PageQuery { page, per_page },
        }
    }

    fn page_items(response: GithubResponse) -> Result<Result<Page<WorkflowRun>, GhError>, GithubResponse> {
        match response {
            GithubResponse::ListWorkflowRuns(result) => Ok(result.map(Page::from)),
            other => Err(other),
        }
    }

    /// Python's fetcher returns `[]` on an error, which ends paging.
    fn page_failed(error: &GhError) -> PageFailure {
        log_failure(RUNS_ERROR, error);
        PageFailure::End
    }
}

/// The Workflows section's state.
#[derive(Debug)]
pub(super) struct WorkflowsPane {
    runs: ListSection<RunsSource>,
    /// The loaded repo's workflows, for #188's Trigger workflow modal: empty until they arrive, or when they failed.
    workflows: Vec<Workflow>,
    /// The workflows request in flight.
    workflows_request: Option<RequestId>,
    /// The first runs page in flight. Later pages are the [`ListSection`]'s.
    runs_request: Option<RequestId>,
}

impl WorkflowsPane {
    pub(super) fn new(options: AppOptions) -> Self {
        Self {
            runs: ListSection::new(options),
            workflows: Vec::new(),
            workflows_request: None,
            runs_request: None,
        }
    }

    /// The runs table.
    pub(super) fn runs_table(&self) -> &SearchableTable<WorkflowRun> {
        self.runs.table()
    }

    /// The runs table, for the screen's input and drawing paths.
    pub(super) fn runs_table_mut(&mut self) -> &mut SearchableTable<WorkflowRun> {
        self.runs.table_mut()
    }

    /// Whether the section is compact.
    pub(super) fn compact(&self) -> bool {
        self.runs.compact()
    }

    /// The loaded repo's workflows.
    #[cfg(test)]
    pub(super) fn workflows(&self) -> &[Workflow] {
        &self.workflows
    }

    /// A repo was selected while the section is shown: the requests in flight are cancelled, the runs clear under the
    /// overlay, and the workflows and the first runs page are requested together, as Python's `asyncio.gather`.
    pub(super) fn load_repo(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        for id in self
            .workflows_request
            .take()
            .into_iter()
            .chain(self.runs_request.take())
        {
            cx.effect(Effect::Cancel(id));
        }
        self.runs.begin(repo, cx);
        self.workflows.clear();
        self.workflows_request = Some(cx.request(GithubRequest::ListWorkflows {
            repo: Box::new(repo.clone()),
            page: PageQuery::default(),
        }));
        self.runs_request = Some(cx.request(GithubRequest::ListWorkflowRuns {
            repo: Box::new(repo.clone()),
            page: PageQuery {
                page: 1,
                per_page: PER_PAGE,
            },
        }));
    }

    /// Requests the page the runs table asked for.
    pub(super) fn load_more(&mut self, generation: u64, page: u32, per_page: u32, cx: &mut Cx<'_>) {
        self.runs.load_more(generation, page, per_page, cx);
    }

    /// Takes a response to the workflows, the first runs page or a later runs page, and hands back any other. A
    /// request cancelled by a newer selection is handed back, so the app drops it (Py bug B45).
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        match response {
            GithubResponse::ListWorkflows(result) if self.workflows_request == Some(id) => {
                self.workflows_request = None;
                match result {
                    Ok(workflows) => self.workflows = workflows,
                    Err(error) => log_failure(WORKFLOWS_ERROR, &error),
                }
            }
            GithubResponse::ListWorkflowRuns(result) if self.runs_request == Some(id) => {
                self.runs_request = None;
                if let Err(error) = &result {
                    log_failure(RUNS_ERROR, error);
                }
                self.runs.first_page(result.map(Page::from), cx);
            }
            other => return self.runs.on_response(id, other, cx),
        }
        // Python's `set_compact` runs once both requests have answered.
        if self.workflows_request.is_none() && self.runs_request.is_none() {
            self.runs.refresh_compact();
        }
        None
    }
}

/// A run in `repo` for tests: run `id` of the workflow `name`, numbered `run_number`, created at `created_at` (an RFC
/// 3339 timestamp), `completed` with `conclusion` when there is one and `in_progress` otherwise, titled `title`.
#[cfg(test)]
pub(super) fn run(
    repo: &Repository,
    id: u64,
    name: &str,
    run_number: u64,
    created_at: &str,
    conclusion: Option<&str>,
    title: &str,
) -> WorkflowRun {
    let created_at: jiff::Timestamp = created_at.parse().expect("an RFC 3339 timestamp");
    let actor = repo.owner.clone();
    WorkflowRun {
        id,
        name: name.into(),
        display_title: title.into(),
        path: format!(".github/workflows/{}.yml", name.to_lowercase()),
        run_number,
        run_attempt: Some(1),
        head_branch: "main".into(),
        head_sha: "0".repeat(40),
        status: if conclusion.is_some() {
            "completed"
        } else {
            "in_progress"
        }
        .into(),
        conclusion: conclusion.map(Into::into),
        event: "push".into(),
        actor: actor.clone(),
        triggering_actor: actor,
        repository: repo.clone(),
        created_at,
        updated_at: created_at,
        html_url: String::new(),
        jobs_url: String::new(),
        logs_url: String::new(),
        run_started_at: Some(created_at),
    }
}

/// A workflow named `name` for tests.
#[cfg(test)]
pub(super) fn workflow(id: u64, name: &str) -> Workflow {
    Workflow {
        id,
        name: name.into(),
        state: lazy_github_core::models::WorkflowState::Active,
        path: format!(".github/workflows/{}.yml", name.to_lowercase()),
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
        url: String::new(),
    }
}

#[cfg(test)]
mod tests;
