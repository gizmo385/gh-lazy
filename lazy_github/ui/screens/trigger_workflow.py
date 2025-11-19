from textual import on, suggester, work
from textual.app import ComposeResult
from textual.containers import Container, Horizontal
from textual.screen import ModalScreen
from textual.validation import Length
from textual.widgets import Button, Input, Label, Markdown, Select

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.branches import list_branches
from lazy_github.lib.github.workflows import create_dispatch_event
from lazy_github.lib.messages import BranchesLoaded
from lazy_github.models.github import Repository, Workflow
from lazy_github.ui.widgets.common import LazyGithubFooter


class TriggerWorkflowButtons(Horizontal):
    DEFAULT_CSS = """
    TriggerWorkflowButtons {
        align: center middle;
        height: auto;
        width: 100%;
    }
    Button {
        margin: 1;
    }
    """

    def compose(self) -> ComposeResult:
        yield Button("Trigger", id="trigger", variant="success")
        yield Button("Cancel", id="cancel", variant="error")


class TriggerWorkflowContainer(Container):
    DEFAULT_CSS = """
    TriggerWorkflowContainer {
        align: center middle;
    }
    """

    def __init__(self, workflows: list[Workflow], repo: Repository) -> None:
        super().__init__()
        self.workflows = workflows
        self.repo = repo
        # Create a mapping from workflow ID to workflow object
        self.workflows_by_id = {w.id: w for w in workflows}

    def compose(self) -> ComposeResult:
        yield Markdown("# Trigger Workflow")
        yield Label("[bold]Workflow[/bold]")
        # Create options for the Select widget using workflow ID as the value (hashable)
        workflow_options = [(w.name, w.id) for w in self.workflows]
        yield Select(
            options=workflow_options,
            id="workflow_select",
            prompt="Select a workflow",
        )
        yield Label("[bold]Branch[/bold]")
        yield Input(
            id="branch_to_build",
            placeholder="Choose a branch",
            validators=Length(minimum=1),
            value=self.repo.default_branch or "main",
        )
        yield TriggerWorkflowButtons()

    @on(BranchesLoaded)
    def handle_loaded_branches(self, message: BranchesLoaded) -> None:
        self.branches = {b.name: b for b in message.branches}
        branch_suggester = suggester.SuggestFromList(self.branches.keys())
        self.query_one("#branch_to_build", Input).suggester = branch_suggester

    @work
    async def fetch_branches(self) -> None:
        branches = await list_branches(self.repo)
        self.post_message(BranchesLoaded(branches))

    async def on_mount(self) -> None:
        self.fetch_branches()


class TriggerWorkflowModal(ModalScreen[bool]):
    DEFAULT_CSS = """
    TriggerWorkflowModal {
        align: center middle;
        content-align: center middle;
    }

    TriggerWorkflowContainer {
        width: 60;
        max-height: 25;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    def __init__(self, workflows: list[Workflow], repo: Repository) -> None:
        super().__init__()
        self.workflows = workflows
        self.repo = repo

    def compose(self) -> ComposeResult:
        yield TriggerWorkflowContainer(self.workflows, self.repo)
        yield LazyGithubFooter()

    @on(Button.Pressed, "#trigger")
    async def action_submit(self) -> None:
        container = self.query_one(TriggerWorkflowContainer)
        workflow_select = self.query_one("#workflow_select", Select)
        branch_input = self.query_one("#branch_to_build", Input)

        # Validate workflow is selected
        if workflow_select.value == Select.BLANK:
            self.notify("You must select a workflow!", title="Validation Error", severity="error")
            return

        # Validate branch input
        branch_input.validate(branch_input.value)
        if not branch_input.is_valid:
            self.notify("You must enter a branch!", title="Validation Error", severity="error")
            return

        # Look up the workflow from the selected ID
        workflow_id = workflow_select.value
        # Type guard: we already checked it's not BLANK above
        assert isinstance(workflow_id, int), "Workflow ID should be an int"
        selected_workflow = container.workflows_by_id[workflow_id]

        if await create_dispatch_event(self.repo, selected_workflow, branch_input.value):
            self.dismiss(True)
        else:
            self.notify(
                "Could not trigger build - are you sure this workflow supports dispatch events?",
                title="Error Triggering Build",
                severity="error",
            )

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(False)
