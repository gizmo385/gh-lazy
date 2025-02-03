from textual import on
from textual.app import App, ComposeResult
from textual.containers import Container, Vertical, VerticalScroll
from textual.message import Message
from textual.widgets import Button, Collapsible, Label, Rule, Select, TextArea
from textual.widgets.text_area import Selection
from unidiff import Hunk, PatchSet, UnidiffParseError

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.models.github import ReviewState

example_patch = r"""
diff --git a/lazy_github/lib/messages.py b/lazy_github/lib/messages.py
index f412717..c6fb09a 100644
--- a/lazy_github/lib/messages.py
+++ b/lazy_github/lib/messages.py
@@ -31,10 +31,9 @@ class PullRequestSelected(Message):
     A message indicating that the user is looking for additional information on a particular pull request.
     \"""

-    def __init__(self, pr: PartialPullRequest, focus_pr_details: bool = True) -> None:
-        super().__init__()
+    def __init__(self, pr: PartialPullRequest) -> None:
         self.pr = pr
-        self.focus_pr_details = focus_pr_details
+        super().__init__()


 class IssueSelected(Message):
diff --git a/lazy_github/ui/screens/primary.py b/lazy_github/ui/screens/primary.py
index d88a95e..2b6b9e7 100644
--- a/lazy_github/ui/screens/primary.py
+++ b/lazy_github/ui/screens/primary.py
@@ -287,16 +287,15 @@ class MainViewPane(Container):
     async def load_repository(self, repo: Repository) -> None:
         await self.selections.load_repository(repo)

-    async def load_pull_request(self, pull_request: PartialPullRequest, focus_pr_details: bool = True) -> None:
+    async def load_pull_request(self, pull_request: PartialPullRequest) -> None:
         full_pr = await get_full_pull_request(pull_request.repo, pull_request.number)
         tabbed_content = self.query_one("#selection_detail_tabs", TabbedContent)
         await tabbed_content.clear_panes()
         await tabbed_content.add_pane(PrOverviewTabPane(full_pr))
         await tabbed_content.add_pane(PrDiffTabPane(full_pr))
         await tabbed_content.add_pane(PrConversationTabPane(full_pr))
+        tabbed_content.children[0].focus()
         self.details.border_title = f"[5] PR #{full_pr.number} Details"
-        if focus_pr_details:
-            tabbed_content.children[0].focus()

     async def load_issue(self, issue: Issue) -> None:
         tabbed_content = self.query_one("#selection_detail_tabs", TabbedContent)
@@ -308,7 +307,7 @@ class MainViewPane(Container):

     @on(PullRequestSelected)
     async def handle_pull_request_selection(self, message: PullRequestSelected) -> None:
-        await self.load_pull_request(message.pr, message.focus_pr_details)
+        await self.load_pull_request(message.pr)

     @on(IssueSelected)
     async def handle_issue_selection(self, message: IssueSelected) -> None:
diff --git a/lazy_github/ui/widgets/pull_requests.py b/lazy_github/ui/widgets/pull_requests.py
index 43f7c63..2c39abe 100644
--- a/lazy_github/ui/widgets/pull_requests.py
+++ b/lazy_github/ui/widgets/pull_requests.py
@@ -141,7 +141,7 @@ class PullRequestsContainer(LazyGithubContainer):
             associated_prs = await list_pull_requests_for_commit(LazyGithubContext.current_local_commit)
             if len(associated_prs) == 1:
                 lg.info("Loading PR for your current commit")
-                self.post_message(PullRequestSelected(associated_prs[0], False))
+                self.post_message(PullRequestSelected(associated_prs[0]))

     async def get_selected_pr(self) -> PartialPullRequest:
         pr_number_coord = Coordinate(self.table.cursor_row, self.number_column_index)
"""


class AddCommentContainer(Vertical):
    DEFAULT_CSS = """
    AddCommentContainer {
        border: $secondary dashed;
        width: 100%;
        content-align: center middle;
        height: auto;
    }
    TextArea {
        height: auto;
    }
    Horizontal {
        height: 5;
    }
    Button {
        margin: 1;
        content-align: center middle;
    }
    """

    def __init__(self, diff_to_comment_on: str) -> None:
        super().__init__()
        self.diff_to_comment_on = diff_to_comment_on

    def compose(self) -> ComposeResult:
        yield Label("Commenting on:")
        responding_to = TextArea(self.diff_to_comment_on, read_only=True)
        responding_to.can_focus = False
        yield responding_to
        yield Label("Pending comment")
        yield TextArea(id="new_comment")
        yield Button("Remove comment", variant="warning", id="remove_comment")

    @property
    def new_comment(self) -> TextArea:
        return self.query_one("#new_comment", TextArea)

    @on(Button.Pressed, "#remove_comment")
    async def remove_comment(self, _: Button.Pressed) -> None:
        await self.remove()


class TriggerNewComment(Message):
    def __init__(self, hunk: Hunk, selection_start: int, selection_end: int) -> None:
        super().__init__()
        self.hunk = hunk
        self.selection_start = selection_start
        self.selection_end = selection_end


class DiffHunkViewer(TextArea):
    BINDINGS = [
        LazyGithubBindings.DIFF_SELECT_LINE,
        LazyGithubBindings.DIFF_CURSOR_DOWN,
        LazyGithubBindings.DIFF_CURSOR_UP,
        LazyGithubBindings.DIFF_CLEAR_SELECTION,
        LazyGithubBindings.DIFF_ADD_COMMENT,
    ]

    def __init__(self, hunk: Hunk, id: str | None = None) -> None:
        super().__init__(
            id=id,
            read_only=True,
            show_line_numbers=True,
            line_number_start=hunk.source_start - 1,
            soft_wrap=False,
            text=example_patch,
        )
        self.theme = "vscode_dark"
        self.text = "".join([str(s) for s in hunk])
        self.hunk = hunk

    def action_cursor_left(self, select: bool = False) -> None:
        # We don't want to move the cursor left/right
        return

    def action_cursor_right(self, select: bool = False) -> None:
        # We don't want to move the cursor left/right
        return

    def action_cursor_line_start(self, select: bool = False) -> None:
        # We don't want to move the cursor left/right
        return

    def action_cursor_line_end(self, select: bool = False) -> None:
        # We don't want to move the cursor left/right
        return

    def action_cursor_down(self, select: bool = False) -> None:
        return super().action_cursor_down(select or self.selection.start != self.selection.end)

    def action_cursor_up(self, select: bool = False) -> None:
        return super().action_cursor_up(select or self.selection.start != self.selection.end)

    def action_clear_selection(self) -> None:
        self.selection = Selection.cursor(self.cursor_location)

    def action_add_comment(self) -> None:
        self.post_message(TriggerNewComment(self.hunk, self.selection.start[0], self.selection.end[0]))


class SubmitReview(Container):
    DEFAULT_CSS = """
    Button {
        margin: 1;
        content-align: center middle;
    }
    """

    def __init__(self, can_only_comment: bool = False) -> None:
        super().__init__()
        self.can_only_comment = can_only_comment

    def compose(self) -> ComposeResult:
        yield Label("Review Status:")
        if not self.can_only_comment:
            yield Select(
                options=[(s.title().replace("_", " "), s) for s in ReviewState if s != ReviewState.DISMISSED],
                id="review_status",
                value=ReviewState.COMMENTED,
            )
        yield Button("Submit Review", id="submit_review", variant="success")


class DiffViewerContainer(VerticalScroll):
    DEFAULT_CSS = """
    DiffHunkViewer {
        height: auto;
    }
    Container {
        height: auto;
    }
    Label {
        margin-left: 1;
    }
    """

    BINDINGS = [LazyGithubBindings.DIFF_NEXT_HUNK, LazyGithubBindings.DIFF_PREVIOUS_HUNK]

    def __init__(self, diff: str, id: str | None = None) -> None:
        super().__init__(id=id)
        self._raw_diff = diff
        self._hunk_container_map: dict[str, Container] = {}

    def action_previous_hunk(self) -> None:
        self.screen.focus_previous()

    def action_next_hunk(self) -> None:
        self.screen.focus_next()

    @on(TriggerNewComment)
    async def show_comment_for_hunk(self, message: TriggerNewComment) -> None:
        # Create a new inline container for commenting on the selected diff.
        text = "".join([str(s) for s in message.hunk][message.selection_start : message.selection_end + 1])
        hunk_container = self._hunk_container_map[str(message.hunk)]
        new_comment_container = AddCommentContainer(text)
        await hunk_container.mount(new_comment_container)
        new_comment_container.new_comment.focus()
        hunk_container.scroll_to_center(new_comment_container)

    def compose(self) -> ComposeResult:
        try:
            diff = PatchSet(self._raw_diff)
        except UnidiffParseError:
            yield Label("Error parsing diff - please view on Github")
        else:
            files_handled = set()
            for patch_file in diff:
                if patch_file.path in files_handled:
                    continue
                files_handled.add(patch_file.path)
                with Collapsible(title=patch_file.path, collapsed=False):
                    for hunk in patch_file:
                        with Container() as c:
                            yield Label(str(hunk).splitlines()[0])
                            yield DiffHunkViewer(hunk)
                            # Add the container for this hunk to a map that can be used to add inline comments later
                            self._hunk_container_map[str(hunk)] = c
                yield Rule()
            yield SubmitReview()


if __name__ == "__main__":
    from lazy_github.ui.widgets.common import LazyGithubFooter

    class DiffViewerApp(App):
        BINDINGS = [LazyGithubBindings.QUIT_APP]

        def compose(self) -> ComposeResult:
            yield DiffViewerContainer(example_patch)
            yield LazyGithubFooter()

    DiffViewerApp().run()
