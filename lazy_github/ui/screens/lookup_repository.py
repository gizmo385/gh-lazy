from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal
from textual.content import Content
from textual.screen import ModalScreen
from textual.validation import Regex
from textual.widgets import Button, Input, Label, Markdown, Rule, Switch

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github.repositories import get_repository_by_name
from lazy_github.models.github import Repository
from lazy_github.ui.widgets.common import LazyGithubFooter


class LookupRepositoryButtons(Horizontal):
    DEFAULT_CSS = """
    LookupRepositoryButtons {
        align: center middle;
        height: auto;
        width: 100%;
    }
    Button {
        margin: 1;
    }
    """

    def compose(self) -> ComposeResult:
        yield Button("Open", id="lookup", variant="success")
        yield Button("Cancel", id="cancel", variant="error")


class LookupRepositoryContainer(Container):
    DEFAULT_CSS = """
    LookupRepositoryContainer {
        align: center middle;
    }
    """

    def compose(self) -> ComposeResult:
        yield Markdown("# Search for a repository by name:")
        yield Label(Content.from_markup("[bold]Repository (owner/name):[/bold]"))
        yield Input(
            id="repo_to_lookup",
            placeholder="Repository name",
            validators=Regex(r"^[^/]+/[^/]+$"),
        )
        yield Rule()
        yield Label("Continue tracking this repo?")
        yield Switch(id="continue_tracking")
        yield Rule()
        yield LookupRepositoryButtons()


class LookupRepositoryModal(ModalScreen[Repository | None]):
    DEFAULT_CSS = """
    LookupRepositoryModal {
        align: center middle;
        content-align: center middle;
    }

    LookupRepositoryContainer {
        width: 60;
        max-height: 25;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield LookupRepositoryContainer()
        yield LazyGithubFooter()

    def _continue_tracking_repo(self, repo_name: str) -> None:
        if repo_name not in LazyGithubContext.config.repositories.additional_repos_to_track:
            with LazyGithubContext.config.to_edit() as config:
                # If we haven't tracked this repo already, we will do so
                config.repositories.additional_repos_to_track.append(repo_name)

    @on(Button.Pressed, "#lookup")
    async def action_submit(self) -> None:
        repo_input = self.query_one("#repo_to_lookup", Input)
        continue_tracking_input = self.query_one("#continue_tracking", Switch)
        if repo_input.is_valid:
            repo_name = repo_input.value
            if continue_tracking_input.value:
                self._continue_tracking_repo(repo_name)

            # Load the repo from from the Github API and return it to the caller
            if repo := await get_repository_by_name(repo_input.value):
                self.dismiss(repo)
            else:
                self.notify("Could not find repo!", title="Validation Error", severity="error")
        else:
            self.notify("You must enter a repo name!", title="Validation Error", severity="error")

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(None)
