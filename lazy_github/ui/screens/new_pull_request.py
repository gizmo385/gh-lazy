from textual import on, suggester, validation, work
from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import Button, Input, Label, Markdown, Rule, SelectionList, Switch, TextArea
from textual.widgets.selection_list import Selection

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github.branches import list_branches
from lazy_github.lib.github.pull_requests import create_pull_request, request_reviews
from lazy_github.lib.github.repositories import get_collaborators
from lazy_github.lib.github.users import get_user_by_username
from lazy_github.lib.logging import lg
from lazy_github.lib.messages import BranchesLoaded, PullRequestCreated
from lazy_github.models.github import Branch, FullPullRequest


class BranchSelection(Horizontal):
    DEFAULT_CSS = """
    BranchSelection {
        width: 100%;
        height: auto;
    }
    
    Label {
        padding-top: 1;
    }

    Input {
        width: 30%;
    }
    """

    def __init__(self) -> None:
        super().__init__()
        self.branches: dict[str, Branch] = {}

    def compose(self) -> ComposeResult:
        assert LazyGithubContext.current_repo is not None, "Unexpectedly missing current repo in new PR modal"
        non_empty_validator = validation.Length(minimum=1)
        yield Label("[bold]Base[/bold]")
        yield Input(
            id="base_ref",
            placeholder="Choose a base ref",
            value=LazyGithubContext.current_repo.default_branch,
            validators=[non_empty_validator],
        )
        yield Label(":left_arrow: [bold]Compare[/bold]")
        yield Input(id="head_ref", placeholder="Choose a head ref", validators=[non_empty_validator])
        yield Label("Draft")
        yield Switch(id="pr_is_draft", value=False)

    @property
    def _head_ref_input(self) -> Input:
        return self.query_one("#head_ref", Input)

    @property
    def _base_ref_input(self) -> Input:
        return self.query_one("#base_ref", Input)

    @property
    def head_ref(self) -> str:
        return self._head_ref_input.value

    @property
    def base_ref(self) -> str:
        return self._base_ref_input.value

    @work
    async def set_default_branch_value(self) -> None:
        if (
            LazyGithubContext.current_directory_repo
            and LazyGithubContext.current_repo
            and LazyGithubContext.current_directory_repo == LazyGithubContext.current_repo.full_name
        ):
            self.query_one("#head_ref", Input).value = LazyGithubContext.current_directory_branch or ""

    async def on_mount(self) -> None:
        self.fetch_branches()
        self.set_default_branch_value()

    @on(BranchesLoaded)
    def handle_loaded_branches(self, message: BranchesLoaded) -> None:
        self.branches = {b.name: b for b in message.branches}
        branch_suggester = suggester.SuggestFromList(self.branches.keys())
        self._head_ref_input.suggester = branch_suggester
        self._base_ref_input.suggester = branch_suggester

    @work
    async def fetch_branches(self) -> None:
        # This shouldn't happen since the current repo needs to be set to open this modal, but we'll validate it to
        # make sure
        assert LazyGithubContext.current_repo is not None, "Current repo unexpectedly missing in new PR modal"

        branches = await list_branches(LazyGithubContext.current_repo)
        self.post_message(BranchesLoaded(branches))


class NewPullRequestButtons(Horizontal):
    DEFAULT_CSS = """
    NewPullRequestButtons {
        align: center middle;
        height: auto;
        width: 100%;
    }
    Button {
        margin: 1;
    }
    """

    def compose(self) -> ComposeResult:
        yield Button("Create", id="submit_new_pr", variant="success")
        yield Button("Cancel", id="cancel_new_pr", variant="error")


class ReviewerSelectionContainer(Vertical):
    DEFAULT_CSS = """
    ReviewerSelectionContainer {
        height: auto;
        width: 100%;
    }
    """

    def __init__(self) -> None:
        super().__init__()
        self.reviewers: set[str] = set()
        self.new_reviewer = Input(id="new_reviewer", placeholder="Reviewer to add")
        self.current_reviewers_label = Label(
            "Current Reviewers - Click/Select them to remove", id="current_reviewers_label"
        )
        self.current_reviewers_label.display = False
        self.reviewers_selection_list = SelectionList(id="current_reviewers")
        self.reviewers_selection_list.display = False

    def compose(self) -> ComposeResult:
        yield Label("[bold]Reviewers[/bold]")
        yield self.new_reviewer
        yield self.current_reviewers_label
        yield self.reviewers_selection_list

    async def on_mount(self) -> None:
        reviewer_suggester = suggester.SuggestFromList(
            LazyGithubContext.config.pull_requests.additional_suggested_pr_reviewers
        )
        self.new_reviewer.suggester = reviewer_suggester
        self._fetch_collaborators()

    @work
    async def _fetch_collaborators(self) -> None:
        if LazyGithubContext.current_repo:
            collaborators = await get_collaborators(LazyGithubContext.current_repo.full_name)
            collaborators.extend(LazyGithubContext.config.pull_requests.additional_suggested_pr_reviewers)
            reviewer_suggester = suggester.SuggestFromList(collaborators)
            self.new_reviewer.suggester = reviewer_suggester

    @work
    async def _validate_new_reviewer(self, reviewer: str) -> None:
        if reviewer in self.reviewers:
            self.notify("Already a reviewer!", severity="error")
        elif not await get_user_by_username(reviewer):
            self.notify(f"Could not find user [yellow]{reviewer}[/yellow] - are you sure they exist?", severity="error")
        else:
            lg.info(f"Adding {self.new_reviewer.value} to PR reviewer list")
            self.reviewers.add(reviewer)
            self.reviewers_selection_list.add_option(Selection(reviewer, reviewer, id=reviewer, initial_state=True))
            self.new_reviewer.value = ""
            self.current_reviewers_label.display = True
            self.reviewers_selection_list.display = True

    @on(Input.Submitted)
    async def handle_new_reviewer_submitted(self, _: Input.Submitted) -> None:
        self._validate_new_reviewer(self.new_reviewer.value.strip().lower())

    @on(SelectionList.SelectionToggled)
    async def handle_reviewer_deselected(self, toggled: SelectionList.SelectionToggled) -> None:
        reviewer_to_remove = toggled.selection.value
        self.notify(f"Removing {reviewer_to_remove}")
        self.reviewers.remove(reviewer_to_remove)
        self.reviewers_selection_list.remove_option(reviewer_to_remove)

        # If we've removed all of the reviewers, hide the display
        if not self.reviewers:
            self.new_reviewer.focus()
            self.current_reviewers_label.display = False
            self.reviewers_selection_list.display = False


class NewPullRequestContainer(VerticalScroll):
    DEFAULT_CSS = """
    Markdown {
        margin-bottom: 1;
    }
    NewPullRequestContainer {
        padding: 1;
    }
    Horizontal {
        content-align: center middle;
    }

    #pr_description {
        height: 10;
        width: 100%;
        margin-bottom: 1;
    }

    #pr_title {
        margin-bottom: 1;
    }
    """
    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG]

    def compose(self) -> ComposeResult:
        yield Markdown("# New Pull Request")
        yield BranchSelection()
        yield Rule()
        yield Label("[bold]Title[/bold]")
        yield Input(id="pr_title", placeholder="Title", validators=[validation.Length(minimum=1)])
        yield Label("[bold]Description[/bold]")
        yield TextArea.code_editor(id="pr_description", soft_wrap=True, tab_behavior="focus")
        yield Rule()
        yield ReviewerSelectionContainer()
        yield NewPullRequestButtons()

    def on_mount(self) -> None:
        self.query_one("#pr_title", Input).focus()

    @on(Button.Pressed, "#cancel_new_pr")
    def cancel_pull_request(self, _: Button.Pressed):
        self.app.pop_screen()

    async def _create_pr(self) -> None:
        assert LazyGithubContext.current_repo is not None, "Unexpectedly missing current repo in new PR modal"
        title_field = self.query_one("#pr_title", Input)
        title_field.validate(title_field.value)
        description_field = self.query_one("#pr_description", TextArea)
        head_ref_field = self.query_one("#head_ref", Input)
        head_ref_field.validate(head_ref_field.value)
        base_ref_field = self.query_one("#base_ref", Input)
        base_ref_field.validate(base_ref_field.value)
        draft_field = self.query_one("#pr_is_draft", Switch)

        if not (title_field.is_valid and head_ref_field.is_valid and base_ref_field.is_valid):
            self.notify("Missing required fields!", title="Invalid PR!", severity="error")
            return

        try:
            created_pr = await create_pull_request(
                LazyGithubContext.current_repo,
                title_field.value,
                description_field.text,
                base_ref_field.value,
                head_ref_field.value,
                draft=draft_field.value,
            )
        except Exception:
            lg.exception("Error while attempting to create new pull request")
            self.notify(
                "Check that your branches are valid and that a PR does not already exist",
                title="Error creating pull request",
                severity="error",
            )
            return

        reviewer_container = self.query_one(ReviewerSelectionContainer)
        reviewers = list(reviewer_container.reviewers)
        if reviewers:
            lg.info(f"Requesting PR reviews from: {', '.join(reviewers)}")
            await request_reviews(created_pr, reviewers)

        self.notify("Successfully created PR!")
        self.post_message(PullRequestCreated(created_pr))

    async def action_submit(self) -> None:
        await self._create_pr()

    @on(Button.Pressed, "#submit_new_pr")
    async def submit_new_pr(self, _: Button.Pressed):
        await self._create_pr()


class NewPullRequestModal(ModalScreen[FullPullRequest | None]):
    DEFAULT_CSS = """
    NewPullRequestModal {
        align: center middle;
        content-align: center middle;
    }

    NewPullRequestContainer {
        width: 100;
        height: auto;
        border: thick $background 80%;
        background: $surface-lighten-3;
        margin: 1;
    }
    """

    BINDINGS = [LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield NewPullRequestContainer()

    def action_close(self) -> None:
        self.dismiss(None)

    @on(PullRequestCreated)
    def on_pull_request_created(self, message: PullRequestCreated) -> None:
        self.dismiss(message.pull_request)
