import re

from textual import on
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.screen import ModalScreen
from textual.validation import ValidationResult, Validator
from textual.widgets import Button, Input, Label, Markdown, Rule

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.backends.protocol import GithubApiRequestFailed
from lazy_github.lib.github.issues import get_issue_by_number
from lazy_github.lib.github.pull_requests import get_full_pull_request
from lazy_github.lib.github.repositories import get_repository_by_name
from lazy_github.models.github import FullPullRequest, Issue, Repository
from lazy_github.ui.widgets.common import LazyGithubFooter, ModalDialogButtons

REPO_PATTERN_STRING = r"(https://)?[^\/]+\/(?P<owner>[^\/]+)\/(?P<repo>[^\/]+)"
REPO_PATTERN = re.compile(REPO_PATTERN_STRING)
ISSUE_PATTERN = re.compile(REPO_PATTERN_STRING + r"\/issues\/(?P<issue>\d+).*")
PR_PATTERN = re.compile(REPO_PATTERN_STRING + r"\/pull\/(?P<pull_request>\d+).*")

PastedLinkSubject = Repository | FullPullRequest | Issue


class GithubLinkValidator(Validator):
    def validate(self, value: str) -> ValidationResult:
        if REPO_PATTERN.match(value) or ISSUE_PATTERN.match(value) or PR_PATTERN.match(value):
            return self.success()
        else:
            return self.failure()


class PasteLinkContainer(Container):
    DEFAULT_CSS = """
    PasteLinkContainer {
        align: center middle;
    }
    """

    def compose(self) -> ComposeResult:
        yield Markdown("# Paste a web GitHub link:")
        yield Label(Content.from_markup("[bold]Link:[/bold]"))
        yield Input(id="link_to_paste", placeholder="https://github.com/...", validators=GithubLinkValidator())
        yield Rule()
        yield ModalDialogButtons(submit_text="Open")


class PasteLinkModal(ModalScreen[PastedLinkSubject]):
    DEFAULT_CSS = """
    PasteLinkModal {
        align: center middle;
        content-align: center middle;
    }

    PasteLinkContainer {
        width: 80;
        max-height: 20;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield PasteLinkContainer()
        yield LazyGithubFooter()

    async def _parse_link_and_action(self, link: str) -> None:
        match = PR_PATTERN.match(link) or ISSUE_PATTERN.match(link) or REPO_PATTERN.match(link)
        if match is None:
            return
        self.query_one("#submit", Button).disabled = True
        self.query_one("#cancel", Button).disabled = True

        groups = match.groupdict()

        repo = await get_repository_by_name(f"{groups['owner']}/{groups['repo']}")
        if not repo:
            self.notify("Could not find repository!", title="Unknown repo", severity="error")
            return

        if issue_number := groups.get("issue"):
            try:
                issue = await get_issue_by_number(repo, int(issue_number))
                self.dismiss(issue)
            except GithubApiRequestFailed:
                self.notify("Could not find issue!", title="Unknown issue", severity="error")
        elif pr_number := groups.get("pull_request"):
            try:
                pr = await get_full_pull_request(repo, int(pr_number))
                self.dismiss(pr)
            except GithubApiRequestFailed:
                self.notify("Could not find pull request!", title="Unknown PR", severity="error")
        else:
            self.notify("Unknown link")
        self.query_one("#submit", Button).disabled = False
        self.query_one("#cancel", Button).disabled = False

    @on(Button.Pressed, "#submit")
    async def action_submit(self) -> None:
        link = self.query_one("#link_to_paste", Input)
        await self._parse_link_and_action(link.value)

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(None)
