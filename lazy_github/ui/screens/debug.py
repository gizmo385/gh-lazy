from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal
from textual.screen import ModalScreen
from textual.widgets import Button, Markdown, Rule

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.debug import collect_debug_info
from lazy_github.ui.widgets.common import LazyGithubFooter


class DebugButtons(Horizontal):
    DEFAULT_CSS = """
    DebugButtons {
        align: center middle;
        height: auto;
        width: 100%;
        dock: bottom;
    }
    Button {
        margin: 1;
    }
    """

    def compose(self) -> ComposeResult:
        yield Button("Copy to Clipboard", id="copy", variant="success")
        yield Button("Close", id="cancel", variant="error")


class DebugContainer(Container):
    DEFAULT_CSS = """
    DebugContainer {
        align: center middle;
    }
    """

    def compose(self) -> ComposeResult:
        yield Markdown(collect_debug_info())
        yield Rule()
        yield DebugButtons()


class DebugModal(ModalScreen[None]):
    DEFAULT_CSS = """
    DebugModal {
        align: center middle;
        content-align: center middle;
    }

    DebugContainer {
        max-width: 80%;
        max-height: 80%;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    def compose(self) -> ComposeResult:
        yield DebugContainer()
        yield LazyGithubFooter()

    @on(Button.Pressed, "#copy")
    async def action_copy(self) -> None:
        self.query_one(Markdown).text_select_all()
        self.action_copy_text()
        self.notify("Debug info copied to clipboard", title="Copied", severity="information")

    @on(Button.Pressed, "#cancel")
    async def action_close(self) -> None:
        self.dismiss(None)


if __name__ == "__main__":
    print(collect_debug_info())
