from pathlib import Path

from textual import on, suggester, validation, work
from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.content import Content
from textual.screen import ModalScreen
from textual.types import DuplicateID
from textual.widgets import Button, Input, Label, Markdown, Rule, SelectionList, Switch, TextArea
from textual.widgets.selection_list import Selection

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.git_cli import (
    does_branch_exist_on_remote,
    does_branch_have_configured_upstream,
    push_branch_to_remote,
)
from lazy_github.lib.github.branches import list_branches
from lazy_github.lib.github.pull_requests import (
    create_pull_request,
    list_requested_reviewers,
    request_reviews,
    update_pull_request,
)
from lazy_github.lib.github.repositories import get_collaborators
from lazy_github.lib.github.users import get_user_by_username
from lazy_github.lib.logging import lg
from lazy_github.lib.messages import BranchesLoaded, PullRequestCreatedOrUpdated
from lazy_github.models.github import Branch, FullPullRequest, PartialPullRequest
from lazy_github.ui.screens.confirm import ConfirmDialog


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

    def __init__(self, existing_pull_request: FullPullRequest | None) -> None:
        super().__init__()
        self.existing_pull_request = existing_pull_request
        self.branches: dict[str, Branch] = {}

    def compose(self) -> ComposeResult:
        assert LazyGithubContext.current_repo is not None, "Unexpectedly missing current repo in new PR modal"
        non_empty_validator = validation.Length(minimum=1)
        base_ref_value = (
            self.existing_pull_request.base.ref
            if self.existing_pull_request
            else LazyGithubContext.current_repo.default_branch
        )
        head_ref_value = self.existing_pull_request.head.ref if self.existing_pull_request else None
        yield Label(Content.from_markup("[bold]Base[/bold]"))
        yield Input(
            id="base_ref",
            placeholder="Choose a base ref",
            value=base_ref_value,
            validators=[non_empty_validator],
            disabled=bool(self.existing_pull_request),
        )
        yield Label(Content.from_markup("← [bold]Compare[/bold]"))
        yield Input(
            id="head_ref",
            placeholder="Choose a head ref",
            validators=[non_empty_validator],
            value=head_ref_value,
            disabled=bool(self.existing_pull_request),
        )
        yield Label("Draft")
        yield Switch(
            id="pr_is_draft",
            value=self.existing_pull_request.draft if self.existing_pull_request else False,
            disabled=bool(self.existing_pull_request),
        )

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
        if not self.existing_pull_request:
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

    def __init__(self, existing_pull_request: PartialPullRequest | None) -> None:
        super().__init__()
        self.existing_pull_request = existing_pull_request

    def compose(self) -> ComposeResult:
        submit_text = "Update" if self.existing_pull_request else "Create"
        yield Button(submit_text, id="submit_new_pr", variant="success")
        yield Button("Cancel", id="cancel_new_pr", variant="error")


class ReviewerSelectionContainer(Vertical):
    DEFAULT_CSS = """
    ReviewerSelectionContainer {
        height: auto;
        width: 100%;
    }
    """

    def __init__(self, existing_pull_request: FullPullRequest | None) -> None:
        super().__init__()
        self.existing_pull_request = existing_pull_request
        self.reviewers: set[str] = set()
        self.new_reviewer = Input(id="new_reviewer", placeholder="Reviewer to add")
        self.current_reviewers_label = Label(
            "Current Reviewers - Click/Select them to remove", id="current_reviewers_label"
        )
        self.current_reviewers_label.display = False
        self.reviewers_selection_list = SelectionList(id="current_reviewers")
        self.reviewers_selection_list.display = False

    def compose(self) -> ComposeResult:
        yield Label(Content.from_markup("[bold]Reviewers[/bold]"))
        yield self.new_reviewer
        yield self.current_reviewers_label
        yield self.reviewers_selection_list

    async def on_mount(self) -> None:
        reviewer_suggester = suggester.SuggestFromList(
            LazyGithubContext.config.pull_requests.additional_suggested_pr_reviewers
        )
        self.new_reviewer.suggester = reviewer_suggester
        self._fetch_collaborators()

        if self.existing_pull_request:
            self._fetch_existing_review_requests()

    @work
    async def _fetch_existing_review_requests(self) -> None:
        if not self.existing_pull_request:
            return

        existing_reviewers = await list_requested_reviewers(self.existing_pull_request)
        lg.debug(f"Found existing reviewers: {existing_reviewers}")
        self.reviewers.update(u for u in existing_reviewers)
        for reviewer in existing_reviewers:
            try:
                self.reviewers_selection_list.add_option(Selection(reviewer, reviewer, id=reviewer, initial_state=True))
            except DuplicateID:
                # Already exists, just keep going
                continue

        if self.reviewers:
            self.current_reviewers_label.display = True
            self.reviewers_selection_list.display = True

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
            try:
                self.reviewers_selection_list.add_option(Selection(reviewer, reviewer, id=reviewer, initial_state=True))
            except DuplicateID:
                pass
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


class CreateOrEditPullRequestContainer(VerticalScroll):
    DEFAULT_CSS = """
    Markdown {
        margin-bottom: 1;
    }
    CreateOrEditPullRequestContainer {
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

    #branch_missing {
        align: center middle;
        content-align: center middle;
        width: 100%;
        margin-bottom: 1;
    }
    """
    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG]

    def __init__(self, existing_pull_request: FullPullRequest | None) -> None:
        super().__init__()
        self.existing_pull_request = existing_pull_request
        self.branch_missing_label = Label(
            "[yellow]Warning:[/yellow] Current local branch has no configured upstream", id="branch_missing"
        )
        self.branch_missing_label.display = False

    def compose(self) -> ComposeResult:
        pr_template = LazyGithubContext.config.pull_requests.pull_request_template
        pr_title = ""
        pr_description = ""
        modal_heading = "New Pull Request"
        if self.existing_pull_request:
            modal_heading = f"Editing PR {self.existing_pull_request.number}"
            pr_title = self.existing_pull_request.title
            pr_description = self.existing_pull_request.body or ""
        elif isinstance(pr_template, Path) and pr_template.exists():
            pr_description = pr_template.read_text()

        yield Markdown(f"# {modal_heading}")
        yield self.branch_missing_label
        yield BranchSelection(self.existing_pull_request)
        yield Rule()
        yield Label(Content.from_markup("[bold]Title[/bold]"))
        yield Input(id="pr_title", placeholder="Title", validators=[validation.Length(minimum=1)], value=pr_title)
        yield Label(Content.from_markup("[bold]Description[/bold]"))
        yield TextArea.code_editor(id="pr_description", soft_wrap=True, tab_behavior="focus", text=pr_description)
        yield Rule()
        yield ReviewerSelectionContainer(self.existing_pull_request)
        yield NewPullRequestButtons(self.existing_pull_request)

    @work
    async def check_for_branch_creation(self, branch) -> None:
        branch_warning = f"[yellow]Warning:[/yellow] Branch [yellow]{branch}[/yellow] may not exist on remote. Do you want to create it?"
        if await self.app.push_screen_wait(ConfirmDialog(branch_warning)):
            if push_branch_to_remote(branch):
                self.notify(f"Pushed branch [greenyellow]{branch}[/] to remote")
            else:
                self.notify(f"Error while pushing branch [red]{branch}[/red] to remote", severity="error")

    @work
    async def ensure_branch_exists_on_remote(self, branch: str) -> None:
        if not does_branch_exist_on_remote(branch):
            self.check_for_branch_creation(branch)

    @work
    async def ensure_directory_branch_has_configured_upstream(self) -> None:
        if not LazyGithubContext.current_directory_branch:
            return
        if not does_branch_have_configured_upstream(LazyGithubContext.current_directory_branch):
            self.branch_missing_label.display = True

    async def on_mount(self) -> None:
        self.query_one("#pr_title", Input).focus()
        self.ensure_directory_branch_has_configured_upstream()

    @on(Button.Pressed, "#cancel_new_pr")
    def cancel_pull_request(self, _: Button.Pressed):
        self.app.pop_screen()

    async def _edit_pr(self) -> None:
        if not self.existing_pull_request or not LazyGithubContext.current_repo:
            return

        title_field = self.query_one("#pr_title", Input)
        title_field.validate(title_field.value)
        description_field = self.query_one("#pr_description", TextArea)

        # Update pull request fields
        try:
            self.notify("Updating PR...")
            updated_pr = await update_pull_request(
                LazyGithubContext.current_repo, self.existing_pull_request, title_field.value, description_field.text
            )
        except Exception:
            lg.exception("Error while attempting to update pull request")
            self.notify(
                "Unexpected error while attempting to update PR",
                title="Error updating pull request",
                severity="error",
            )
            return

        # Request any new reviewers
        existing_reviewers = await list_requested_reviewers(self.existing_pull_request)
        reviewer_container = self.query_one(ReviewerSelectionContainer)
        reviewers = list(reviewer_container.reviewers)
        if reviewers:
            lg.info(f"Requesting PR reviews from: {', '.join(reviewers)}")
            await request_reviews(updated_pr, [r for r in reviewers if r not in existing_reviewers])

        self.notify("Successfully updated PR!")
        self.post_message(PullRequestCreatedOrUpdated(updated_pr))

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
            self.notify("Creating PR...")
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
            self.ensure_branch_exists_on_remote(head_ref_field.value)
            return

        reviewer_container = self.query_one(ReviewerSelectionContainer)
        reviewers = list(reviewer_container.reviewers)
        if reviewers:
            lg.info(f"Requesting PR reviews from: {', '.join(reviewers)}")
            await request_reviews(created_pr, reviewers)

        self.notify("Successfully created PR!")
        self.post_message(PullRequestCreatedOrUpdated(created_pr))

    async def action_submit(self) -> None:
        if self.existing_pull_request:
            await self._edit_pr()
        else:
            await self._create_pr()

    @on(Button.Pressed, "#submit_new_pr")
    async def submit_new_pr(self, _: Button.Pressed):
        if self.existing_pull_request:
            await self._edit_pr()
        else:
            await self._create_pr()


class CreateOrEditPullRequestModal(ModalScreen[FullPullRequest | None]):
    DEFAULT_CSS = """
    CreateOrEditPullRequestModal {
        align: center middle;
        content-align: center middle;
    }

    CreateOrEditPullRequestContainer {
        width: 100;
        height: auto;
        border: thick $background 80%;
        background: $surface-lighten-3;
        margin: 1;
    }
    """

    def __init__(self, existing_pull_request: FullPullRequest | None = None) -> None:
        super().__init__()
        self.existing_pull_request = existing_pull_request

    BINDINGS = [LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield CreateOrEditPullRequestContainer(self.existing_pull_request)

    def action_close(self) -> None:
        self.dismiss(None)

    @on(PullRequestCreatedOrUpdated)
    def on_pull_request_created(self, message: PullRequestCreatedOrUpdated) -> None:
        self.dismiss(message.pull_request)
