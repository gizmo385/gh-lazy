from textual.app import ComposeResult
from textual.containers import VerticalScroll

from lazy_github.models.github import FullPullRequest
from lazy_github.ui.widgets.split_diff_viewer import SplitDiffViewer


class DiffViewerContainer(VerticalScroll):
    """container for diff viewer - delegates to SplitDiffViewer"""

    def __init__(
        self,
        pr: FullPullRequest,
        reviewer_is_author: bool,
        diff: str,
        id: str | None = None,
    ) -> None:
        super().__init__(id=id)
        self.pr = pr
        self.reviewer_is_author = reviewer_is_author
        self._raw_diff = diff

    def compose(self) -> ComposeResult:
        yield SplitDiffViewer(self._raw_diff, self.pr, self.reviewer_is_author)
