import re

from textual import on
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.screen import ModalScreen
from textual.validation import ValidationResult, Validator
from textual.widgets import Button, Input, Label, Markdown, Rule

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.lib.github.backends.protocol import GithubApiRequestFailed
from lazy_github.lib.github.pull_requests import get_full_pull_request
from lazy_github.lib.github.repositories import get_repository_by_name
from lazy_github.lib.github.users import get_full_user_by_username
from lazy_github.models.github import FullPullRequest, FullUser, Issue, Repository
from lazy_github.ui.widgets.common import LazyGithubFooter, ModalDialogButtons

REPO_PATTERN_STRING = r"(https://)?[^\/]+\/(?P<owner>[^\/]+)\/(?P<repo>[^\/]+)"
REPO_PATTERN = re.compile(REPO_PATTERN_STRING)
ISSUE_PATTERN = re.compile(REPO_PATTERN_STRING + r"\/issues\/(?P<issue>\d+).*")
PR_PATTERN = re.compile(REPO_PATTERN_STRING + r"\/pull\/(?P<pull_request>\d+).*")
USER_PATTERN = re.compile(r"(https://)?github\.com\/(?P<login>[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)\/?$")

PastedLinkSubject = Repository | FullPullRequest | Issue | FullUser


class UnknownGithubLink(Exception):
    """Raised when a URL doesn't match any known GitHub link pattern."""


async def _resolve_issue_or_pr(repo: Repository, number: int) -> PastedLinkSubject | None:
    """Look up `#number` in `repo`, returning a FullPullRequest if it's actually a PR.

    GitHub's `/issues/{n}` endpoint serves both issues and PRs and includes a
    `pull_request` key when the number refers to a PR. We check for that and
    redirect to the PR endpoint so the UI opens the right view.
    """
    url = f"/repos/{repo.owner.login}/{repo.name}/issues/{number}"
    try:
        response = await LazyGithubContext.client.get(url, headers=github_headers())
        response.raise_for_status()
    except GithubApiRequestFailed:
        return None

    payload = response.json()
    if "pull_request" in payload:
        try:
            return await get_full_pull_request(repo, number)
        except GithubApiRequestFailed:
            return None
    return Issue(**payload, repo=repo)


async def resolve_github_url(link: str, *, accept_repo: bool = True) -> PastedLinkSubject | None:
    """Resolve a GitHub URL to a Repository, FullPullRequest, Issue, or FullUser.

    `accept_repo=False` causes plain repo URLs (no /pull/N or /issues/N suffix)
    to raise UnknownGithubLink instead of resolving — useful for the in-document
    click handler, where opening a bare repo URL in-app would be a surprising
    full UI swap. The paste modal keeps the default behavior.

    Raises UnknownGithubLink if the URL doesn't match a known pattern.
    Returns None if the underlying API lookup fails.
    """
    if user_match := USER_PATTERN.match(link):
        try:
            return await get_full_user_by_username(user_match.group("login"))
        except GithubApiRequestFailed:
            return None

    if pr_match := PR_PATTERN.match(link):
        match = pr_match
    elif issue_match := ISSUE_PATTERN.match(link):
        match = issue_match
    elif accept_repo and (repo_match := REPO_PATTERN.match(link)):
        match = repo_match
    else:
        raise UnknownGithubLink(link)

    groups = match.groupdict()
    repo = await get_repository_by_name(f"{groups['owner']}/{groups['repo']}")
    if not repo:
        return None

    if issue_number := groups.get("issue"):
        return await _resolve_issue_or_pr(repo, int(issue_number))
    if pr_number := groups.get("pull_request"):
        try:
            return await get_full_pull_request(repo, int(pr_number))
        except GithubApiRequestFailed:
            return None
    return repo


class GithubLinkValidator(Validator):
    def validate(self, value: str) -> ValidationResult:
        if (
            REPO_PATTERN.match(value)
            or ISSUE_PATTERN.match(value)
            or PR_PATTERN.match(value)
            or USER_PATTERN.match(value)
        ):
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
        self.query_one("#submit", Button).disabled = True
        self.query_one("#cancel", Button).disabled = True
        try:
            try:
                subject = await resolve_github_url(link)
            except UnknownGithubLink:
                self.notify("Unknown link")
                return

            if subject is None:
                self.notify("Could not resolve link!", title="GitHub lookup failed", severity="error")
                return
            self.dismiss(subject)
        finally:
            self.query_one("#submit", Button).disabled = False
            self.query_one("#cancel", Button).disabled = False

    @on(Button.Pressed, "#submit")
    async def action_submit(self) -> None:
        link = self.query_one("#link_to_paste", Input)
        await self._parse_link_and_action(link.value)

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(None)
