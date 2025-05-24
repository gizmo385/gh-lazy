from enum import Enum

from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Vertical, VerticalScroll
from textual.content import Content
from textual.css.query import NoMatches
from textual.message import Message
from textual.types import NoSelection
from textual.widgets import Button, Collapsible, Input, Label, Rule, Select, TextArea

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.diff_parser import Hunk, InvalidDiffFormat, parse_diff_from_str
from lazy_github.lib.github.pull_requests import create_new_review
from lazy_github.lib.messages import PullRequestSelected
from lazy_github.models.github import FullPullRequest, ReviewState

DISALLOWED_REVIEW_STATES = [ReviewState.DISMISSED, ReviewState.PENDING]


class HunkSide(Enum):
    BEFORE = "source"
    AFTER = "target"


class AddCommentContainer(Vertical):
    DEFAULT_CSS = """
    AddCommentContainer {
        border: $secondary dashed;
        width: 100%;
        height: auto;
    }
    TextArea {
        height: auto;
        width: 100%;
    }
    #new_comment {
        height: auto;
    }
    Button {
        margin: 1;
        content-align: center middle;
    }
    """

    def __init__(
        self,
        hunk: Hunk,
        filename: str,
        selection_start: int,
        diff_to_comment_on: str,
    ) -> None:
        super().__init__()
        # This field is displayed the user knows what they're commenting on
        self.diff_to_comment_on = diff_to_comment_on
        # These fields are used for constructing the API request body later
        self.hunk = hunk
        self.filename = filename
        self.selection_start = selection_start
        self.new_comment = TextArea(id="new_comment")

    def compose(self) -> ComposeResult:
        yield Label("Commenting on:")
        responding_to = Input(self.diff_to_comment_on, disabled=True)
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

    def __init__(self, hunk: Hunk, filename: str, selection_start: int) -> None:
        super().__init__()
        self.hunk = hunk
        self.filename = filename
        self.selection_start = selection_start


class TriggerReviewSubmission(Message):
    """Message sent to trigger the sending of the in-progress review to Github"""

    pass


class CommentRemoved(Message):
    """Message sent to trigger removal of a comment from the list of comments to be submitted in a review"""

    def __init__(self, comment: AddCommentContainer) -> None:
        super().__init__()
        self.comment = comment


class DiffHunkViewer(TextArea):
    DEFAULT_CSS = """
    DiffHunkViewer {
        max-height: 25;
    }
    """
    BINDINGS = [
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
            line_number_start=hunk.file_start_line,
            soft_wrap=False,
            compact=True,
            text="",
        )
        self.theme = "vscode_dark"
        self.filename = filename
        self.hunk = hunk

        self.text = "\n".join(hunk.lines)

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

    def action_add_comment(self) -> None:
        self.post_message(TriggerNewComment(self.hunk, self.filename, self.cursor_location[0]))


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
                options=[(s.title().replace("_", " "), s) for s in ReviewState if s not in DISALLOWED_REVIEW_STATES],
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

    def __init__(self, pr: FullPullRequest, reviewer_is_author: bool, diff: str, id: str | None = None) -> None:
        super().__init__(id=id)
        self.pr = pr
        self.reviewer_is_author = reviewer_is_author
        self._raw_diff = diff
        self._hunk_container_map: dict[str, Collapsible] = {}
        self._added_review_comments: list[AddCommentContainer] = []

    def action_previous_hunk(self) -> None:
        self.screen.focus_previous()

    def action_next_hunk(self) -> None:
        self.screen.focus_next()

    @on(CommentRemoved)
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
            position = comment_field.hunk.diff_position + comment_field.selection_start + 1
            comments.append(
                {
                    "path": comment_field.filename,
                    "body": comment_field.text,
                    "position": position,
                }
            )

        new_review = await create_new_review(self.pr, review_state, review_body, comments)
        if new_review is not None:
            self.notify("New review created!")
            self.post_message(PullRequestSelected(self.pr))

    @on(TriggerNewComment)
    async def show_comment_for_hunk(self, message: TriggerNewComment) -> None:
        # Create a new inline container for commenting on the selected diff.
        lines = message.hunk.lines
        if lines:
            text = str(lines[message.selection_start]).strip().replace("\n", "")
        else:
            text = ""
        hunk_container = self._hunk_container_map[str(message.hunk)]
        new_comment_container = AddCommentContainer(message.hunk, message.filename, message.selection_start, text)
        await hunk_container.mount(new_comment_container)
        new_comment_container.new_comment.focus()
        hunk_container.scroll_to_center(new_comment_container)

        # Keep track of this so we can construct the actual review object later on
        self._added_review_comments.append(new_comment_container)

    def compose(self) -> ComposeResult:
        try:
            diff = parse_diff_from_str(self._raw_diff)
        except InvalidDiffFormat:
            yield Label("Error parsing diff - please view on Github")
            return

        files_handled = set()
        for path, changed_file in diff.files.items():
            if path in files_handled:
                continue
            files_handled.add(path)

            # These Content types don't properly type check but are necessary to actually use markup in the Collapsible
            # widgets being used for diffs
            changed_file_header = Content.from_markup(
                f"[red]File Deleted:[/red] {path}" if changed_file.deleted else path
            )
            with Collapsible(title=changed_file_header, collapsed=changed_file.deleted):  # type: ignore
                for hunk in changed_file.hunks:
                    with Collapsible(title=Content.from_text(hunk.header, markup=False)) as c:  # type: ignore
                        yield DiffHunkViewer(hunk, path)
                        # Add the container for this hunk to a map that can be used to add inline comments later
                        self._hunk_container_map[str(hunk)] = c
            yield Rule()
        yield SubmitReview(can_only_comment=self.reviewer_is_author)
