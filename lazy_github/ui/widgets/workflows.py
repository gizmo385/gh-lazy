from functools import partial

from textual import work
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.coordinate import Coordinate
from textual.widgets import DataTable, TabbedContent, TabPane

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.workflows import list_workflow_runs, list_workflows
from lazy_github.lib.logging import lg
from lazy_github.models.github import Repository, Workflow, WorkflowRun
from lazy_github.ui.screens.trigger_workflow import TriggerWorkflowModal
from lazy_github.ui.widgets.common import LazilyLoadedDataTable, LazyGithubContainer, TableRow


def workflow_to_cell(workflow: Workflow) -> TableRow:
    return (workflow.name, workflow.created_at.strftime("%c"), workflow.updated_at.strftime("%c"), str(workflow.path))


def workflow_run_to_cell(run: WorkflowRun) -> TableRow:
    return (run.created_at.strftime("%Y-%m-%d %H:%M"), run.conclusion or run.status, run.name, run.display_title)


class AvailableWorkflowsContainers(Container):
    BINDINGS = [LazyGithubBindings.TRIGGER_WORKFLOW]
    workflows: dict[str, Workflow] = {}

    def compose(self) -> ComposeResult:
        yield LazilyLoadedDataTable(
            id="searchable_workflows_table",
            table_id="workflows_table",
            search_input_id="workflows_search",
            sort_key="name",
            load_function=None,
            batch_size=30,
            item_to_row=workflow_to_cell,
            item_to_key=lambda w: w.path,
            cache_name="workflows",
            reverse_sort=True,
        )

    @property
    def searchable_table(self) -> LazilyLoadedDataTable[Workflow]:
        return self.query_one("#searchable_workflows_table", LazilyLoadedDataTable)

    @property
    def table(self) -> DataTable:
        return self.query_one("#workflows_table", DataTable)

    def on_mount(self) -> None:
        self.table.cursor_type = "row"
        self.table.add_column("Name", key="name")
        self.table.add_column("Created", key="created")
        self.table.add_column("Updated", key="updated")
        self.table.add_column("Path", key="path")

        self.path_column_id = self.table.get_column_index("path")

    def load_cached_workflows(self, repo: Repository) -> None:
        self.searchable_table.initialize_from_cache(repo, Workflow)

    async def fetch_more_workflows(self, repo: Repository, batch_size: int, batch_to_fetch: int) -> list[Workflow]:
        next_page = await list_workflows(repo, page=batch_to_fetch, per_page=batch_size)
        return [w for w in next_page if isinstance(w, Workflow)]

    async def load_repo(self, repo: Repository) -> None:
        workflows = await list_workflows(repo)

        self.searchable_table.add_items(workflows)
        self.searchable_table.change_load_function(partial(self.fetch_more_workflows, repo))
        self.searchable_table.can_load_more = True
        self.searchable_table.current_batch = 1

    def get_selected_workflow(self) -> Workflow:
        workflow_path_coord = Coordinate(self.table.cursor_row, self.path_column_id)
        return self.searchable_table.items[self.table.get_cell_at(workflow_path_coord)]

    @work
    async def action_trigger_workflow(self) -> None:
        workflow = self.get_selected_workflow()
        lg.info(f"Triggering workflow {workflow.name}")
        if await self.app.push_screen_wait(TriggerWorkflowModal(workflow)):
            self.notify("Successfully triggered workflow")


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


class WorkflowsContainer(LazyGithubContainer):
    def compose(self) -> ComposeResult:
        self.border_title = Content.from_markup("\\[4] Workflows")
        with TabbedContent(id="workflow_tabs"):
            with TabPane("Runs", id="runs_tab"):
                yield WorkflowRunsContainer(id="workflow_runs")
            with TabPane("Workflows", id="workflows_tab"):
                yield AvailableWorkflowsContainers(id="workflows")

    @property
    def workflows(self) -> AvailableWorkflowsContainers:
        return self.query_one("#workflows", AvailableWorkflowsContainers)

    @property
    def workflow_runs(self) -> WorkflowRunsContainer:
        return self.query_one("#workflow_runs", WorkflowRunsContainer)

    def initialize_tables_from_cache(self, repo: Repository) -> None:
        self.workflows.load_cached_workflows(repo)
        self.workflow_runs.load_cached_workflow_runs(repo)

    @work
    async def load_repo(self, repo: Repository) -> None:
        await self.workflows.load_repo(repo)
        await self.workflow_runs.load_repo(repo)
