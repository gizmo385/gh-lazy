from textual.app import ComposeResult
from textual.containers import VerticalScroll
from textual.widgets import Markdown, TabPane

from lazy_github.version import VERSION

LAZY_GITHUB_INFO = f"""
# LazyGithub: A Terminal UI For GitHub ({VERSION})

LazyGithub is a terminal UI for interacting with GitHub. You can get started by selecting one of the repositories listed
in the table on the left. Once you have selected a repository, any issues, pull requests, and action workflows will be
retrieved and displayed below it.


**Issues, pull requests, and action lists can also be interacted with!**

## Pull Requests

You can interact with a PR by viewing its details, viewing the PR diff, and participating in any active conversations
happening on the PR. You can also create a new PR for the currently selected repo.

## Issues

You can interact with an issue by viewing its details, partipcating in any activate conversations happening on the
issue, and by editing the issue itself. This means that you can close an issue from within LazyGithub.

## Actions

Github action and workflow support is pretty limited. It currently only supports listing the workflows and the most
recent runs across all actions on the currently selected repo, as well as kicking off a workflow that supports manual
triggers.
""".strip()


class LazyGithubInfoTabPane(TabPane):
    DEFAULT_CSS = """
    .subtitle {
        align: center top;
    }
    """

    def __init__(self) -> None:
        super().__init__("Introduction")

    def compose(self) -> ComposeResult:
        with VerticalScroll():
            yield Markdown(LAZY_GITHUB_INFO)
