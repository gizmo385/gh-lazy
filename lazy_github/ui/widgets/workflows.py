from functools import partial

from textual import on, work
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.coordinate import Coordinate
from textual.widgets import DataTable

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.workflows import list_workflow_runs, list_workflows
from lazy_github.lib.logging import lg
from lazy_github.lib.messages import WorkflowRunSelected
from lazy_github.models.github import Repository, Workflow, WorkflowRun
from lazy_github.ui.screens.trigger_workflow import TriggerWorkflowModal
from lazy_github.ui.widgets.common import LazilyLoadedDataTable, LazyGithubContainer, TableRow


def workflow_run_to_cell(run: WorkflowRun) -> TableRow:
    return (run.created_at.strftime("%Y-%m-%d %H:%M"), run.conclusion or run.status, run.name, run.display_title, run.run_number)


class WorkflowRunsContainer(Container):
    def compose(self) -> ComposeResult:
        yield LazilyLoadedDataTable(
            id="searchable_workflow_runs_table",
            table_id="workflow_runs_table",
            search_input_id="workflow_runs_search",
            sort_key="time",
            load_function=None,
            batch_size=30,
            item_to_row=workflow_run_to_cell,
            item_to_key=lambda wr: str(wr.run_number),
            cache_name="workflow_runs",
            reverse_sort=True,
        )

    @property
    def searchable_table(self) -> LazilyLoadedDataTable[WorkflowRun]:
        return self.query_one("#searchable_workflow_runs_table", LazilyLoadedDataTable)

    @property
    def table(self) -> DataTable:
        return self.query_one("#workflow_runs_table", DataTable)

    def on_mount(self) -> None:
        self.table.cursor_type = "row"
        self.table.add_column("Time", key="time")
        self.table.add_column("Result", key="result")
        self.table.add_column("Job Name", key="job_name")
        self.table.add_column("Run Name", key="run_name")
        self.table.add_column("Run #", key="run_number")

        self.run_number_column_id = self.table.get_column_index("run_number")

    def load_cached_workflow_runs(self, repo: Repository) -> None:
        self.searchable_table.initialize_from_cache(repo, WorkflowRun)

    async def fetch_more_workflow_runs(
        self, repo: Repository, batch_size: int, batch_to_fetch: int
    ) -> list[WorkflowRun]:
        next_page = await list_workflow_runs(repo, page=batch_to_fetch, per_page=batch_size)
        return [w for w in next_page if isinstance(w, WorkflowRun)]

    async def load_repo(self, repo: Repository) -> None:
        workflow_runs = await list_workflow_runs(repo)

        self.searchable_table.add_items(workflow_runs)
        self.searchable_table.change_load_function(partial(self.fetch_more_workflow_runs, repo))
        self.searchable_table.can_load_more = True
        self.searchable_table.current_batch = 1

    def get_selected_workflow_run(self) -> WorkflowRun:
        """Get the currently selected workflow run from the table."""
        run_number_coord = Coordinate(self.table.cursor_row, self.run_number_column_id)
        run_number = self.table.get_cell_at(run_number_coord)
        return self.searchable_table.items[str(run_number)]

    @on(DataTable.RowSelected, "#workflow_runs_table")
    async def workflow_run_selected(self) -> None:
        """Handle selection of a workflow run to view details."""
        workflow_run = self.get_selected_workflow_run()
        lg.info(f"Selected workflow run #{workflow_run.run_number}")
        self.post_message(WorkflowRunSelected(workflow_run))


class WorkflowsContainer(LazyGithubContainer):
    BINDINGS = [LazyGithubBindings.TRIGGER_WORKFLOW]

    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self.current_repo: Repository | None = None
        self.workflows: list[Workflow] = []

    def compose(self) -> ComposeResult:
        self.border_title = Content.from_markup("\\[4] Workflows")
        yield WorkflowRunsContainer(id="workflow_runs")

    @property
    def workflow_runs(self) -> WorkflowRunsContainer:
        return self.query_one("#workflow_runs", WorkflowRunsContainer)

    def initialize_tables_from_cache(self, repo: Repository) -> None:
        self.current_repo = repo
        self.workflow_runs.load_cached_workflow_runs(repo)

    @work
    async def load_repo(self, repo: Repository) -> None:
        self.current_repo = repo
        # Load workflows in the background for the trigger modal
        self.workflows = await list_workflows(repo)
        await self.workflow_runs.load_repo(repo)

    @work
    async def action_trigger_workflow(self) -> None:
        """Open a modal to select and trigger a workflow."""
        if not self.current_repo:
            self.notify("No repository selected", severity="warning")
            return

        # Ensure workflows are loaded
        if not self.workflows:
            self.workflows = await list_workflows(self.current_repo)

        if not self.workflows:
            self.notify("No workflows found in this repository", severity="warning")
            return


        if await self.app.push_screen_wait(TriggerWorkflowModal(self.workflows, self.current_repo)):
            self.notify("Successfully triggered workflow")
