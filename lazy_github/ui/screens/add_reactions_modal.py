from dataclasses import dataclass

from textual import on
from textual.app import ComposeResult
from textual.containers import Container
from textual.screen import ModalScreen
from textual.widgets import Button, Markdown, SelectionList
from textual.widgets.selection_list import Selection

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.models.github import ReactionType
from lazy_github.ui.widgets.common import LazyGithubFooter, ModalDialogButtons


@dataclass
class ReactionDelta:
    added: list[ReactionType]
    removed: list[ReactionType]


class AddReactionsContainer(Container):
    def __init__(self, initial_reactions: list[ReactionType]) -> None:
        super().__init__()
        self.initial_reactions = initial_reactions

    def compose(self) -> ComposeResult:
        yield Markdown("# Add a new reaction")
        selections: list[Selection] = []
        for r in list(ReactionType):
            if r in self.initial_reactions:
                continue
            selections.append(Selection(r.emoji, r, False))
        yield SelectionList[ReactionType](*selections, id="reaction_selection_list")
        yield ModalDialogButtons()


class AddReactionsModal(ModalScreen[ReactionDelta]):
    BINDINGS = [LazyGithubBindings.CLOSE_DIALOG]

    DEFAULT_CSS = """
    AddReactionsModal {
        align: center middle;
    }
    AddReactionsContainer {
        align: center middle;
        height: 20;
        width: 50;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    def __init__(self, initial_reactions: list[ReactionType]) -> None:
        super().__init__()
        self.initial_reactions = initial_reactions

    def compose(self) -> ComposeResult:
        yield AddReactionsContainer(self.initial_reactions)
        yield LazyGithubFooter()

    @on(Button.Pressed, "#submit")
    def action_submit(self) -> None:
        added: list[ReactionType] = []
        removed: list[ReactionType] = []
        selected_reactions = self.query_one("#reaction_selection_list", SelectionList).selected
        container = self.query_one(AddReactionsContainer)

        for reaction in selected_reactions:
            if reaction not in container.initial_reactions:
                added.append(reaction)

        for reaction in container.initial_reactions:
            if reaction not in selected_reactions:
                removed.append(reaction)

        self.dismiss(ReactionDelta(added, removed))

    @on(Button.Pressed, "#cancel")
    def action_close(self) -> None:
        self.dismiss(ReactionDelta([], []))
