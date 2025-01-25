from textual.app import ComposeResult
from textual.containers import Grid
from textual.screen import ModalScreen
from textual.widgets import Button, Label

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.ui.widgets.common import LazyGithubFooter


class ConfirmDialog(ModalScreen[bool]):
    """Simple confirmation dialog"""

    BINDINGS = [LazyGithubBindings.CONFIRM_YES, LazyGithubBindings.CONFIRM_NO, LazyGithubBindings.CLOSE_DIALOG]
    DEFAULT_CSS = """
    ConfirmDialog {
        align: center middle;
    }

    #dialog {
        grid-size: 2;
        grid-gutter: 1 2;
        grid-rows: 1fr 3;
        padding: 0 1;
        width: 50;
        height: 11;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }

    #question {
        column-span: 2;
        height: 1fr;
        width: 1fr;
        content-align: center middle;
    }

    Button {
        width: 100%;
        align: center middle;
    }
    """

    def __init__(self, question: str, yes_text: str = "Yes", no_text: str = "No") -> None:
        super().__init__()
        self.question = question
        self.yes_text = yes_text
        self.no_text = no_text

    def compose(self) -> ComposeResult:
        yield Grid(
            Label(self.question, id="question"),
            Button(self.yes_text, variant="primary", id="yes"),
            Button(self.no_text, variant="error", id="no"),
            id="dialog",
        )
        yield LazyGithubFooter()

    def action_close(self) -> None:
        self.dismiss(False)

    def action_confirm_no(self) -> None:
        self.dismiss(False)

    def action_confirm_yes(self) -> None:
        self.dismiss(True)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        self.dismiss(event.button.id == "yes")
