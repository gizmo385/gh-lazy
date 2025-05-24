from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal
from textual.content import Content
from textual.screen import ModalScreen
from textual.widgets import Button, Input, Label, Markdown, Rule

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github.backends.protocol import GithubApiRequestFailed
from lazy_github.lib.github.pull_requests import get_full_pull_request
from lazy_github.models.github import FullPullRequest
from lazy_github.ui.widgets.common import LazyGithubFooter


class LookupPullRequestButtons(Horizontal):
    DEFAULT_CSS = """
    LookupPullRequestButtons {
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


class LookupPullRequestContainer(Container):
    DEFAULT_CSS = """
    LookupPullRequestContainer {
        align: center middle;
    }
    """

    def compose(self) -> ComposeResult:
        yield Markdown("# Search for a pull request by number:")
        yield Label(Content.from_markup("[bold]Pull Request Number:[/bold]"))
        yield Input(
            id="pull_request_number",
            placeholder="Pull request number",
            type="number",
        )
        yield Rule()
        yield LookupPullRequestButtons()


class LookupPullRequestModal(ModalScreen[FullPullRequest | None]):
    DEFAULT_CSS = """
    LookupPullRequestModal {
        align: center middle;
        content-align: center middle;
    }

    LookupPullRequestContainer {
        width: 60;
        max-height: 25;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield LookupPullRequestContainer()
        yield LazyGithubFooter()

    @on(Button.Pressed, "#lookup")
    async def action_submit(self) -> None:
        assert LazyGithubContext.current_repo is not None, "Current repo is missing!"

        try:
            pr_number = int(self.query_one("#pull_request_number", Input).value)
            pull_request = await get_full_pull_request(LazyGithubContext.current_repo, pr_number)
        except ValueError:
            self.notify("Must enter a valid pull request number!", title="Invalid PR Number", severity="error")
        except GithubApiRequestFailed:
            self.notify("Could not find pull request!", title="Unknown PR", severity="error")
        else:
            self.dismiss(pull_request)

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(None)
