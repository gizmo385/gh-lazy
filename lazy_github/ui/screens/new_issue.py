from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal, ScrollableContainer
from textual.content import Content
from textual.screen import ModalScreen
from textual.widgets import Button, Input, Label, Rule, TextArea

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github import issues
from lazy_github.lib.messages import IssueCreated
from lazy_github.models.github import Issue
from lazy_github.ui.widgets.common import LazyGithubFooter


class NewIssueContainer(Container):
    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG]

    DEFAULT_CSS = """
    #button_holder {
        align: center middle;
    }

    ScrollableContainer {
        height: 80%;
    }

    #new_issue_title {
        height: auto;
    }

    #new_issue_body {
        height: 15;
    }
    """

    def compose(self) -> ComposeResult:
        assert LazyGithubContext.current_repo is not None, "Unexpectedly missing current repo in new PR modal"
        with ScrollableContainer():
            yield Label(Content.from_markup("[bold]Title[/bold]"))
            yield Input(placeholder="Title", id="new_issue_title")

            yield Rule()

            yield Label(Content.from_markup("[bold]Description[/bold]"))
            yield TextArea.code_editor(id="new_issue_body", soft_wrap=True, tab_behavior="focus")

        with Horizontal(id="button_holder"):
            yield Button("Save", id="save_new_issue", variant="success")
            yield Button("Cancel", id="cancel_new_issue", variant="error")

    @on(Button.Pressed, "#cancel_new_issue")
    def cancel_new_issue(self, _: Button) -> None:
        self.app.pop_screen()

    async def create_issue(self) -> None:
        assert LazyGithubContext.current_repo is not None, "Unexpectedly missing current repo from application context!"

        title = self.query_one("#new_issue_title", Input).value
        body = self.query_one("#new_issue_body", TextArea).text
        if not str(title):
            self.notify("Must have a non-empty issue title!", severity="error")
            return
        if not str(body):
            self.notify("Must have a non-empty issue body!", severity="error")
            return

        self.notify("Creating new issue...")
        new_issue = await issues.create_issue(LazyGithubContext.current_repo, title, body)
        self.notify(f"Successfully created issue #{new_issue.number}")
        self.post_message(IssueCreated(new_issue))

    async def action_submit(self) -> None:
        await self.create_issue()

    @on(Button.Pressed, "#save_new_issue")
    async def handle_save_new_issue_button(self, _: Button) -> None:
        await self.create_issue()


class NewIssueModal(ModalScreen[Issue | None]):
    BINDINGS = [LazyGithubBindings.CLOSE_DIALOG]

    DEFAULT_CSS = """
    NewIssueModal {
        align: center middle;
    }

    NewIssueContainer {
        align: center middle;
        height: 30;
        width: 100;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    def compose(self) -> ComposeResult:
        yield NewIssueContainer()
        yield LazyGithubFooter()

    def action_close(self) -> None:
        self.dismiss()

    @on(IssueCreated)
    def on_issue_created(self, message: IssueCreated) -> None:
        self.dismiss(message.issue)
