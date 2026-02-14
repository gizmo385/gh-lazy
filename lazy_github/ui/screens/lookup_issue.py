from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal
from textual.content import Content
from textual.screen import ModalScreen
from textual.widgets import Button, Input, Label, Markdown, Rule

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github.backends.protocol import GithubApiRequestFailed
from lazy_github.lib.github.issues import get_issue_by_number
from lazy_github.models.github import Issue
from lazy_github.ui.widgets.common import LazyGithubFooter


class LookupIssueButtons(Horizontal):
    DEFAULT_CSS = """
    LookupIssueButtons {
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


class LookupIssueContainer(Container):
    DEFAULT_CSS = """
    LookupIssueContainer {
        align: center middle;
    }
    """

    def compose(self) -> ComposeResult:
        yield Markdown("# Search for an issue by number:")
        yield Label(Content.from_markup("[bold]Issue Number:[/bold]"))
        yield Input(
            id="issue_number",
            placeholder="Issue number",
            type="number",
        )
        yield Rule()
        yield LookupIssueButtons()


class LookupIssueModal(ModalScreen[Issue | None]):
    DEFAULT_CSS = """
    LookupIssueModal {
        align: center middle;
        content-align: center middle;
    }

    LookupIssueContainer {
        width: 60;
        max-height: 25;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield LookupIssueContainer()
        yield LazyGithubFooter()

    @on(Button.Pressed, "#lookup")
    async def action_submit(self) -> None:
        assert LazyGithubContext.current_repo is not None, "Current repo is missing!"

        try:
            issue_number = int(self.query_one("#issue_number", Input).value)
            issue = await get_issue_by_number(LazyGithubContext.current_repo, issue_number)
        except ValueError:
            self.notify("Must enter a valid issue number!", title="Invalid Issue Number", severity="error")
        except GithubApiRequestFailed:
            self.notify("Could not find issue!", title="Unknown Issue", severity="error")
        else:
            self.dismiss(issue)

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(None)
