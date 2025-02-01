from textual.app import App, ComposeResult
from textual.containers import VerticalScroll
from textual.widgets import Collapsible, Rule, TextArea
from textual.widgets.text_area import Selection
from unidiff import Hunk, PatchSet

from lazy_github.lib.bindings import LazyGithubBindings

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


class DiffHunkViewer(TextArea):
    BINDINGS = [
        LazyGithubBindings.DIFF_SELECT_LINE,
        LazyGithubBindings.DIFF_CURSOR_DOWN,
        LazyGithubBindings.DIFF_CURSOR_UP,
        LazyGithubBindings.DIFF_CLEAR_SELECTION,
        LazyGithubBindings.DIFF_ADD_COMMENT,
    ]

    def __init__(self, hunk: Hunk, id: str | None = None) -> None:
        super().__init__(id=id, read_only=True, show_line_numbers=True, text=example_patch)
        self.text = str(hunk)
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
        start_position = self.selection.start[0] + self.hunk.source_start - 1
        end_position = self.selection.end[0] + self.hunk.source_start - 1
        if start_position < self.hunk.source_start:
            return
        self.notify(f"Commenting from {start_position} -> {end_position}")


class DiffViewerContainer(VerticalScroll):
    DEFAULT_CSS = """
    DiffHunkViewer {
        height: auto;
    }
    """

    BINDINGS = [LazyGithubBindings.DIFF_NEXT_HUNK, LazyGithubBindings.DIFF_PREVIOUS_HUNK]

    def __init__(self, diff: str, id: str | None = None) -> None:
        super().__init__(id=id)
        self._raw_diff = diff
        self.diff = PatchSet(diff)

    def action_previous_hunk(self) -> None:
        self.screen.focus_previous()

    def action_next_hunk(self) -> None:
        self.screen.focus_next()

    def compose(self) -> ComposeResult:
        for f in self.diff:
            with Collapsible(title=f.path, collapsed=False):
                for hunk in f:
                    yield DiffHunkViewer(hunk)
            yield Rule()


if __name__ == "__main__":
    from lazy_github.ui.widgets.common import LazyGithubFooter

    class DiffViewerApp(App):
        BINDINGS = [LazyGithubBindings.QUIT_APP]

        def compose(self) -> ComposeResult:
            yield DiffViewerContainer(example_patch)
            yield LazyGithubFooter()

    DiffViewerApp().run()
