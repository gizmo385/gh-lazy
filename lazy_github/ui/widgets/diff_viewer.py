from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Vertical, VerticalScroll
from textual.css.query import NoMatches
from textual.message import Message
from textual.types import NoSelection
from textual.widgets import Button, Collapsible, Input, Label, Rule, Select, TextArea
from textual.widgets.text_area import Selection
from unidiff import Hunk, PatchSet, UnidiffParseError

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.pull_requests import create_new_review
from lazy_github.lib.logging import lg
from lazy_github.models.github import PartialPullRequest, ReviewState


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

    def __init__(
        self, hunk: Hunk, filename: str, selection_start: int, selection_end: int, diff_to_comment_on: str
    ) -> None:
        super().__init__()
        # This field is displayed the user knows what they're commenting on
        self.diff_to_comment_on = diff_to_comment_on
        # These fields are used for constructing the API request body later
        self.hunk = hunk
        self.filename = filename
        self.selection_start = selection_start
        self.selection_end = selection_end
        self.new_comment = TextArea(id="new_comment")

    def compose(self) -> ComposeResult:
        yield Label("Commenting on:")
        responding_to = TextArea(self.diff_to_comment_on, read_only=True)
        responding_to.can_focus = False
        yield responding_to
        yield Label("Pending comment")
        yield self.new_comment
        yield Button("Remove comment", variant="warning", id="remove_comment")

    @property
    def text(self) -> str:
        return self.new_comment.text

    @on(Button.Pressed, "#remove_comment")
    async def remove_comment(self, _: Button.Pressed) -> None:
        self.post_message(CommentRemoved(self))
        await self.remove()


class TriggerNewComment(Message):
    """Message sent to trigger the addition of a new comment block into the UI"""

    def __init__(self, hunk: Hunk, filename: str, selection_start: int, selection_end: int) -> None:
        super().__init__()
        self.hunk = hunk
        self.filename = filename
        self.selection_start = selection_start
        self.selection_end = selection_end


class TriggerReviewSubmission(Message):
    """Message sent to trigger the sending of the in-progress review to Github"""

    pass


class CommentRemoved(Message):
    """Message sent to trigger removal of a comment from the list of comments to be submitted in a review"""

    def __init__(self, comment: AddCommentContainer) -> None:
        super().__init__()
        self.comment = comment


class DiffHunkViewer(TextArea):
    BINDINGS = [
        LazyGithubBindings.DIFF_SELECT_LINE,
        LazyGithubBindings.DIFF_CURSOR_DOWN,
        LazyGithubBindings.DIFF_CURSOR_UP,
        LazyGithubBindings.DIFF_CLEAR_SELECTION,
        LazyGithubBindings.DIFF_ADD_COMMENT,
    ]

    def __init__(self, hunk: Hunk, filename: str, id: str | None = None) -> None:
        super().__init__(
            id=id,
            read_only=True,
            show_line_numbers=True,
            line_number_start=hunk.source_start,
            soft_wrap=False,
            text="",
        )
        self.theme = "vscode_dark"
        self.text = "".join([str(s) for s in hunk.source_lines()])
        self.filename = filename
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
        self.post_message(TriggerNewComment(self.hunk, self.filename, self.selection.start[0], self.selection.end[0]))


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
        submit_review_label = "Add Comments"
        if not self.can_only_comment:
            yield Label("Review Status:")
            yield Select(
                options=[(s.title().replace("_", " "), s) for s in ReviewState if s != ReviewState.DISMISSED],
                id="review_status",
                value=ReviewState.COMMENTED,
            )
            submit_review_label = "Submit Review"
        yield Input(placeholder="Review summary", id="review_summary")
        yield Button(submit_review_label, id="submit_review", variant="success")

    @on(Button.Pressed, "#submit_review")
    def trigger_review_submission(self, _: Button.Pressed) -> None:
        self.post_message(TriggerReviewSubmission())


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

    def __init__(self, pr: PartialPullRequest, reviewer_is_author: bool, diff: str, id: str | None = None) -> None:
        super().__init__(id=id)
        self.pr = pr
        self.reviewer_is_author = reviewer_is_author
        self._raw_diff = diff
        self._hunk_container_map: dict[str, Container] = {}
        self._added_review_comments: list[AddCommentContainer] = []

    def action_previous_hunk(self) -> None:
        self.screen.focus_previous()

    def action_next_hunk(self) -> None:
        self.screen.focus_next()

    async def handle_comment_removed(self, message: CommentRemoved) -> None:
        if message.comment in self._added_review_comments:
            self._added_review_comments.remove(message.comment)

    @on(TriggerReviewSubmission)
    async def submit_review(self, _: TriggerReviewSubmission) -> None:
        # Retrieve the current state of the review
        try:
            review_state: ReviewState | NoSelection = self.query_one("#review_status", Select).value
        except NoMatches:
            review_state = ReviewState.COMMENTED

        # Ensure that *something* has been selected
        if isinstance(review_state, NoSelection):
            self.notify("Please select a status for the new review!", severity="error")
            return

        # Construct the review body and submit it to Github
        review_body = self.query_one("#review_summary", Input).value
        comments: list[dict[str, str | int]] = []
        for comment_field in self._added_review_comments:
            if not comment_field.text or not comment_field.is_mounted:
                continue

            comments.append(
                {
                    "path": comment_field.filename,
                    "body": comment_field.text,
                    # The comment positions are one-indexed
                    "position": comment_field.hunk.source_start + comment_field.selection_start + 1,
                }
            )
        new_review = await create_new_review(self.pr, review_state, review_body, comments)
        if new_review is not None:
            lg.debug(f"New review: {new_review}")
            self.notify("New review created!")

    @on(TriggerNewComment)
    async def show_comment_for_hunk(self, message: TriggerNewComment) -> None:
        # Create a new inline container for commenting on the selected diff.
        text = "".join([str(s) for s in message.hunk][message.selection_start : message.selection_end + 1])
        hunk_container = self._hunk_container_map[str(message.hunk)]
        new_comment_container = AddCommentContainer(
            message.hunk, message.filename, message.selection_start, message.selection_end, text
        )
        await hunk_container.mount(new_comment_container)
        new_comment_container.new_comment.focus()
        hunk_container.scroll_to_center(new_comment_container)

        # Keep track of this so we can construct the actual review object later on
        self._added_review_comments.append(new_comment_container)

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
                    if patch_file.is_binary_file:
                        yield Label("Cannot display binary files")
                    else:
                        for hunk in patch_file:
                            with Container() as c:
                                yield Label(str(hunk).splitlines()[0])
                                yield DiffHunkViewer(hunk, patch_file.path)
                                # Add the container for this hunk to a map that can be used to add inline comments later
                                self._hunk_container_map[str(hunk)] = c
                yield Rule()
            yield SubmitReview(can_only_comment=self.reviewer_is_author)
