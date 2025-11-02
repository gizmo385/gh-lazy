import re
from pathlib import Path

from rich.segment import Segment
from rich.syntax import Syntax
from rich.text import Text
from textual import on
from textual.app import ComposeResult
from textual.containers import Container, Horizontal, Vertical
from textual.css.query import NoMatches
from textual.events import Key
from textual.message import Message
from textual.reactive import reactive
from textual.screen import ModalScreen
from textual.types import NoSelection
from textual.widget import Widget
from textual.widgets import Button, Collapsible, Input, Label, RichLog, Select, Static, TextArea

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.diff_parser import Hunk, InvalidDiffFormat, parse_diff_from_str
from lazy_github.lib.github.pull_requests import create_new_review
from lazy_github.lib.messages import PullRequestSelected
from lazy_github.models.github import FullPullRequest, ReviewState

DISALLOWED_REVIEW_STATES = [ReviewState.DISMISSED, ReviewState.PENDING]


class FileFilterInput(Input):
    """custom input that clears on ESC"""

    def on_key(self, event: Key) -> None:
        """handle ESC to clear and blur"""
        if event.key == "escape":
            self.value = ""
            self.blur()
            event.stop()


def _count_changes_in_hunk(hunk: Hunk) -> tuple[int, int]:
    """count additions and deletions in a hunk

    returns (additions, deletions)
    """
    additions = 0
    deletions = 0
    for line in hunk.lines:
        if line.startswith("+"):
            additions += 1
        elif line.startswith("-"):
            deletions += 1
    return additions, deletions


def _format_file_header(filepath: str, deleted: bool, hunks: list[Hunk]) -> Text:
    """format file path into readable header with colored change counts"""
    path = Path(filepath)

    if deleted:
        return Text(f"🗑️  {path.name} (deleted)")

    # count total additions/deletions across all hunks
    total_additions = 0
    total_deletions = 0
    for hunk in hunks:
        adds, dels = _count_changes_in_hunk(hunk)
        total_additions += adds
        total_deletions += dels

    # build header with colored counts
    result = Text(f"📄 {path.name}")

    if total_additions > 0 or total_deletions > 0:
        result.append(" (")
        if total_additions > 0:
            result.append(f"+{total_additions}", style="green")
        if total_additions > 0 and total_deletions > 0:
            result.append(" ")
        if total_deletions > 0:
            result.append(f"-{total_deletions}", style="red")
        result.append(")")

    return result


def _format_hunk_header(hunk: Hunk) -> Text:
    """format hunk header into readable description with colored change counts

    converts "@@ -10,5 +12,7 @@ function_name" into something like:
    "Lines 12-19 (+3 -2) • function_name"
    """
    # parse hunk header - format is: @@ -old_start,old_count +new_start,new_count @@ context
    match = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@\s*(.*)", hunk.header)
    if not match:
        return Text(hunk.header)  # fallback to original if parsing fails

    _, old_count, new_start, new_count, context = match.groups()

    old_count = int(old_count) if old_count else 1
    new_count = int(new_count) if new_count else 1
    new_start = int(new_start)
    new_end = new_start + new_count - 1

    if new_count == 1:
        location = f"Line {new_start}"
    else:
        location = f"Lines {new_start}-{new_end}"

    additions, deletions = _count_changes_in_hunk(hunk)
    result = Text(location)

    if additions > 0 or deletions > 0:
        result.append(" (")
        if additions > 0:
            result.append(f"+{additions}", style="green")
        if additions > 0 and deletions > 0:
            result.append(" ")
        if deletions > 0:
            result.append(f"-{deletions}", style="red")
        result.append(")")

    if context.strip():
        result.append(f" • {context.strip()}")

    return result


class TriggerReviewSubmission(Message):
    """message sent to trigger review submission"""

    pass


class UnifiedDiffPane(Widget):
    """unified diff pane showing all lines with +/- prefixes"""

    can_focus = True

    BINDINGS = [
        ("j", "line_down", "Line down"),
        ("k", "line_up", "Line up"),
        LazyGithubBindings.DIFF_ADD_COMMENT,
    ]

    DEFAULT_CSS = """
    UnifiedDiffPane {
        width: 100%;
        height: auto;
        max-height: 25;
        border: solid $primary-lighten-1;
        overflow-y: auto;
    }

    UnifiedDiffPane:focus {
        border: solid $accent;
    }

    UnifiedDiffPane > RichLog {
        width: 100%;
        height: auto;
    }

    UnifiedDiffPane > .current-line-indicator {
        dock: bottom;
        background: $accent;
        color: $text;
        text-align: right;
        padding: 0 1;
        height: 1;
    }
    """

    current_line = reactive(0)

    def __init__(
        self,
        hunk: Hunk,
        filename: str,
    ) -> None:
        super().__init__()
        self.hunk = hunk
        self.filename = filename
        self.lines = hunk.lines  # original diff lines with +/- prefixes
        self._line_indicator: Static | None = None
        self._rich_log: RichLog | None = None
        self._syntax_lines: list[Text] = []  # cache syntax highlighted lines
        self._syntax_computed = False

    def compose(self) -> ComposeResult:
        # create RichLog for displaying diff
        self._rich_log = RichLog(wrap=False, markup=True)
        yield self._rich_log

        # add current line indicator
        self._line_indicator = Static(
            f"Line: {self.current_line + 1}/{len(self.lines)}", classes="current-line-indicator"
        )
        yield self._line_indicator

    def on_mount(self) -> None:
        """render initial content when mounted"""
        self._compute_syntax_highlighting()
        self._render_lines()

    def _compute_syntax_highlighting(self) -> None:
        """compute syntax highlighting once and cache it"""
        if self._syntax_computed:
            return

        # try to get syntax highlighting
        try:
            # build full text for syntax highlighting
            clean_lines = []
            for line in self.lines:
                # remove +/- prefix for syntax highlighting
                if line.startswith(("+", "-", " ")):
                    clean_lines.append(line[1:])
                else:
                    clean_lines.append(line)

            full_text = "\n".join(clean_lines)

            # create syntax object
            syntax = Syntax(
                full_text,
                lexer=Syntax.guess_lexer(self.filename),
                line_numbers=False,
                theme="monokai",
                word_wrap=False,
            )

            # render to get highlighted text
            from io import StringIO

            from rich.console import Console as RenderConsole

            temp_console = RenderConsole(file=StringIO(), force_terminal=True, width=200, legacy_windows=False)

            # render syntax to segments
            segments = list(temp_console.render(syntax))

            # split into lines
            syntax_lines = []
            current_line_segments = []
            for segment in segments:
                if "\n" in segment.text:
                    parts = segment.text.split("\n")
                    for i, part in enumerate(parts):
                        if i > 0:
                            syntax_lines.append(current_line_segments)
                            current_line_segments = []
                        if part:
                            current_line_segments.append(Segment(part, segment.style))
                else:
                    current_line_segments.append(segment)
            if current_line_segments:
                syntax_lines.append(current_line_segments)

            # convert to Text objects and cache
            for idx, line in enumerate(self.lines):
                if idx < len(syntax_lines):
                    syntax_text = Text.assemble(*[(seg.text, seg.style) for seg in syntax_lines[idx]])
                else:
                    syntax_text = Text(clean_lines[idx] if idx < len(clean_lines) else "")
                self._syntax_lines.append(syntax_text)

        except Exception:
            # fallback to plain text if syntax highlighting fails
            for line in self.lines:
                # remove +/- prefix
                clean = line[1:] if line.startswith(("+", "-", " ")) else line
                self._syntax_lines.append(Text(clean))

        self._syntax_computed = True

    def _render_lines(self) -> None:
        """render all lines with current line highlighted (uses cached syntax)"""
        if not self._rich_log:
            return

        # clear and rebuild
        self._rich_log.clear()

        for idx, line in enumerate(self.lines):
            line_num = self.hunk.file_start_line + idx
            is_current = idx == self.current_line

            # get cached syntax highlighted text and make copy so we don't mutate cache
            if idx < len(self._syntax_lines):
                syntax_text = self._syntax_lines[idx].copy()
            else:
                syntax_text = Text(line)

            # determine background color
            if is_current:
                if line.startswith("-"):
                    bg_color = "#8b0000"  # dark red
                elif line.startswith("+"):
                    bg_color = "#006400"  # dark green
                else:
                    bg_color = "yellow"
            else:
                if line.startswith("-"):
                    bg_color = "#3a0a0a"  # subtle dark red
                elif line.startswith("+"):
                    bg_color = "#0a3a0a"  # subtle dark green
                else:
                    bg_color = None

            # apply background to syntax highlighted text
            if bg_color:
                syntax_text.stylize(f"on {bg_color}")

            # add prefix and line number
            prefix = "► " if is_current else "  "
            final_text = Text(f"{prefix}{line_num:4d} │ ") + syntax_text

            self._rich_log.write(final_text)

    def watch_current_line(self, old_value: int, new_value: int) -> None:
        """update display when current line changes"""
        if self._line_indicator:
            self._line_indicator.update(f"Line: {new_value + 1}/{len(self.lines)}")
        # re-render to show new current line highlight (fast now with cached syntax)
        self._render_lines()

    def action_line_down(self) -> None:
        """move to next line"""
        if self.current_line < len(self.lines) - 1:
            self.current_line += 1
            # only scroll if getting close to bottom of visible area
            self._scroll_to_current_if_needed()

    def action_line_up(self) -> None:
        """move to previous line"""
        if self.current_line > 0:
            self.current_line -= 1
            # only scroll if getting close to top of visible area
            self._scroll_to_current_if_needed()

    def _scroll_to_current_if_needed(self) -> None:
        """scroll to keep current line visible, but only if near edge"""
        if not self._rich_log:
            return

        # get visible region info
        visible_height = self.size.height - 1  # subtract 1 for line indicator
        scroll_y = self.scroll_y

        # estimate which lines are visible (roughly 1 line per scroll unit)
        # add some padding so we scroll before line actually leaves view
        scroll_margin = 3  # lines of padding before we scroll

        visible_top = int(scroll_y)
        visible_bottom = int(scroll_y + visible_height)

        # check if current line is getting close to edges
        if self.current_line < visible_top + scroll_margin:
            # too close to top, scroll up
            self.scroll_to(y=max(0, self.current_line - scroll_margin), animate=False)
        elif self.current_line > visible_bottom - scroll_margin:
            # too close to bottom, scroll down
            self.scroll_to(y=self.current_line - visible_height + scroll_margin, animate=False)

    def action_add_comment(self) -> None:
        """trigger adding comment on current line"""
        line_text = self.lines[self.current_line].strip() if self.current_line < len(self.lines) else ""
        self.post_message(
            TriggerAddComment(
                self.hunk,
                self.filename,
                self.current_line,
                line_text,
            )
        )

    def get_current_line_index(self) -> int:
        """get the current line index"""
        return self.current_line


class CommentData:
    """data class to hold comment information"""

    def __init__(
        self,
        hunk: Hunk,
        filename: str,
        line_number: int,
        line_text: str,
        comment_text: str,
    ) -> None:
        self.hunk = hunk
        self.filename = filename
        self.line_number = line_number
        self.line_text = line_text
        self.comment_text = comment_text


class TriggerAddComment(Message):
    """message sent when user wants to add comment via modal"""

    def __init__(
        self,
        hunk: Hunk,
        filename: str,
        line_number: int,
        line_text: str,
    ) -> None:
        super().__init__()
        self.hunk = hunk
        self.filename = filename
        self.line_number = line_number
        self.line_text = line_text


class CommentCreated(Message):
    """message sent when comment is created from modal"""

    def __init__(self, comment: CommentData) -> None:
        super().__init__()
        self.comment = comment


class CommentDeleted(Message):
    """message sent when comment is deleted from preview area"""

    def __init__(self, comment: CommentData) -> None:
        super().__init__()
        self.comment = comment


class AddCommentModal(ModalScreen):
    """modal screen for adding comment with preview"""

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG]

    DEFAULT_CSS = """
    AddCommentModal {
        align: center middle;
    }

    AddCommentModal > Vertical {
        width: 80;
        height: auto;
        border: thick $accent;
        background: $surface;
        padding: 1;
    }

    AddCommentModal Label {
        margin: 1 0;
        color: $text;
        text-style: bold;
    }

    AddCommentModal Input {
        margin: 0 0 1 0;
    }

    AddCommentModal TextArea {
        height: 10;
        margin: 0 0 1 0;
    }

    AddCommentModal Horizontal {
        width: 100%;
        height: auto;
        align-horizontal: center;
    }

    AddCommentModal Button {
        margin: 0 1;
    }
    """

    def __init__(
        self,
        hunk: Hunk,
        filename: str,
        line_number: int,
        line_text: str,
    ) -> None:
        super().__init__()
        self.hunk = hunk
        self.filename = filename
        self.line_number = line_number
        self.line_text = line_text

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Add Comment")
            yield Label("Commenting on:")
            line_preview = Input(self.line_text, disabled=True)
            line_preview.can_focus = False
            yield line_preview
            yield Label("Comment:")
            yield TextArea(id="comment_input")
            with Horizontal():
                yield Button("Add Comment", variant="success", id="add_comment")
                yield Button("Cancel", variant="default", id="cancel")

    def _submit_comment(self) -> None:
        """shared logic for submitting comment"""
        comment_text = self.query_one("#comment_input", TextArea).text
        if not comment_text.strip():
            self.notify("Comment cannot be empty!", severity="warning")
            return

        comment = CommentData(
            self.hunk,
            self.filename,
            self.line_number,
            self.line_text,
            comment_text,
        )
        self.dismiss(comment)

    @on(Button.Pressed, "#cancel")
    def cancel_comment(self, _: Button.Pressed) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#add_comment")
    def add_comment(self, _: Button.Pressed) -> None:
        self._submit_comment()

    def action_submit(self) -> None:
        """handle shift+enter to submit comment"""
        self._submit_comment()

    def action_close(self) -> None:
        """handle q/ESC to close modal without adding comment"""
        self.dismiss(None)


class SplitDiffHunk(Widget):
    """widget that shows single hunk in unified diff view"""

    # don't make this focusable - let the pane inside be focusable instead
    can_focus = False

    DEFAULT_CSS = """
    SplitDiffHunk {
        width: 100%;
        height: auto;
        margin: 1 0;
    }
    """

    def __init__(
        self,
        hunk: Hunk,
        filename: str,
    ) -> None:
        super().__init__()
        self.hunk = hunk
        self.filename = filename
        self.diff_pane: UnifiedDiffPane | None = None

    def compose(self) -> ComposeResult:
        """create unified diff view"""
        # just create single unified diff pane - much simpler!
        self.diff_pane = UnifiedDiffPane(self.hunk, self.filename)
        yield self.diff_pane


class CommentPreview(Vertical):
    """widget to show preview of single comment that will be submitted"""

    DEFAULT_CSS = """
    CommentPreview {
        border: solid $secondary;
        padding: 1;
        margin: 1 0;
        width: 100%;
        height: auto;
        min-height: 5;
    }

    CommentPreview .comment-line {
        color: $text-muted;
        margin-bottom: 1;
        height: auto;
    }

    CommentPreview .comment-body {
        color: $text;
        margin-bottom: 1;
        padding: 1;
        background: $panel;
        height: auto;
    }

    CommentPreview Button {
        width: auto;
        height: auto;
    }
    """

    def __init__(self, comment: CommentData) -> None:
        super().__init__()
        self.comment = comment

    def compose(self) -> ComposeResult:
        yield Static(
            f"{self.comment.filename}:{self.comment.line_number} - {self.comment.line_text}",
            classes="comment-line",
            markup=False,
        )
        yield Static(self.comment.comment_text, classes="comment-body", markup=False)
        yield Button("Remove", variant="warning", id="remove_comment")

    @on(Button.Pressed, "#remove_comment")
    def remove_comment(self, _: Button.Pressed) -> None:
        self.post_message(CommentDeleted(self.comment))


class SubmitReview(Container):
    """widget for submitting review with comments"""

    DEFAULT_CSS = """
    SubmitReview {
        width: 100%;
        height: auto;
        margin: 2 0;
    }

    SubmitReview Button {
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


class SplitDiffViewer(Vertical):
    """main container for split diff view, similar to dunk"""

    DEFAULT_CSS = """
    SplitDiffViewer {
        width: 100%;
        height: auto;
    }

    SplitDiffViewer Collapsible {
        height: auto;
    }

    SplitDiffViewer Collapsible > Contents {
        height: auto;
    }

    SplitDiffViewer .file-header {
        color: $text;
        text-style: bold;
        margin: 1 0;
    }

    SplitDiffViewer .deleted-file {
        color: $error;
        margin: 1 0;
    }

    SplitDiffViewer .hunk-header {
        color: $text-muted;
        margin: 1 0;
    }

    SplitDiffViewer .pending-comments-header {
        background: $accent;
        color: $text;
        text-style: bold;
        text-align: center;
        width: 100%;
        height: 3;
        margin: 2 0 1 0;
    }

    SplitDiffViewer .pending-comments-container {
        width: 100%;
        height: auto;
        border: solid $accent;
        padding: 1;
    }
    """

    BINDINGS = [
        LazyGithubBindings.DIFF_NEXT_HUNK,
        LazyGithubBindings.DIFF_PREVIOUS_HUNK,
        LazyGithubBindings.DIFF_FILTER_FILES,
    ]

    def __init__(
        self,
        diff: str,
        pr: FullPullRequest,
        reviewer_is_author: bool,
        id: str | None = None,
    ) -> None:
        super().__init__(id=id)
        self._raw_diff = diff
        self.pr = pr
        self.reviewer_is_author = reviewer_is_author
        self._pending_comments: list[CommentData] = []
        self._comments_container: Vertical | None = None
        self._comments_header: Label | None = None
        self._file_filter_input: Input | None = None
        self._file_collapsibles: dict[str, Collapsible] = {}  # filename -> collapsible

    def action_previous_hunk(self) -> None:
        """jump to previous hunk (J key)"""
        # get all hunk widgets
        hunks = list(self.query(SplitDiffHunk))
        if not hunks:
            return

        # find currently focused hunk
        focused = self.screen.focused
        if focused in hunks and isinstance(focused, SplitDiffHunk):
            current_idx = hunks.index(focused)
            if current_idx > 0:
                hunks[current_idx - 1].focus()
                hunks[current_idx - 1].scroll_visible()
        else:
            # nothing focused, focus last hunk
            hunks[-1].focus()
            hunks[-1].scroll_visible()

    def action_next_hunk(self) -> None:
        """jump to next hunk (K key)"""
        hunks = list(self.query(SplitDiffHunk))
        if not hunks:
            return

        focused = self.screen.focused
        if focused in hunks and isinstance(focused, SplitDiffHunk):
            current_idx = hunks.index(focused)
            if current_idx < len(hunks) - 1:
                hunks[current_idx + 1].focus()
                hunks[current_idx + 1].scroll_visible()
        else:
            # nothing focused, focus first hunk
            hunks[0].focus()
            hunks[0].scroll_visible()

    def action_focus_file_filter(self) -> None:
        """focus file filter input (/ key)"""
        if self._file_filter_input:
            self._file_filter_input.focus()

    @on(Input.Changed, "#file_filter")
    def filter_files(self, event: Input.Changed) -> None:
        """filter files based on input"""
        filter_text = event.value.lower()

        for filepath, collapsible in self._file_collapsibles.items():
            # check if filepath matches filter
            if not filter_text or filter_text in filepath.lower():
                collapsible.display = True
            else:
                collapsible.display = False

    @on(Input.Submitted, "#file_filter")
    def on_filter_submitted(self, event: Input.Submitted) -> None:
        """handle enter key - just blur the filter input"""
        if self._file_filter_input:
            self._file_filter_input.blur()
            # focus first visible hunk
            for pane in self.query(UnifiedDiffPane):
                if pane.display:
                    pane.focus()
                    break

    async def on_trigger_add_comment(self, message: TriggerAddComment) -> None:
        """show modal to add comment"""
        message.stop()

        # show modal and wait for result
        if result := await self.app.push_screen_wait(
            AddCommentModal(
                message.hunk,
                message.filename,
                message.line_number,
                message.line_text,
            )
        ):
            # show comments section if first comment
            self._pending_comments.append(result)
            if len(self._pending_comments) == 1:
                if self._comments_header:
                    self._comments_header.display = True
                if self._comments_container:
                    self._comments_container.display = True

            if self._comments_container:
                if self._comments_container.is_mounted:
                    preview = CommentPreview(result)
                    await self._comments_container.mount(preview)
                    self.notify("New comment added")

    async def on_comment_deleted(self, message: CommentDeleted) -> None:
        """handle comment deletion from preview"""
        message.stop()

        if message.comment in self._pending_comments:
            self._pending_comments.remove(message.comment)

        # remove the preview widget
        for preview in self.query(CommentPreview):
            if preview.comment == message.comment:
                await preview.remove()
                break

        # hide comments section if no more comments
        if len(self._pending_comments) == 0:
            if self._comments_header:
                self._comments_header.display = False
            if self._comments_container:
                self._comments_container.display = False

    async def on_trigger_review_submission(self, _: TriggerReviewSubmission) -> None:
        """handle review submission"""
        # find and disable submit button to prevent duplicate submissions
        try:
            submit_button = self.query_one("#submit_review", Button)
            if submit_button.disabled:
                # already submitting, ignore
                return
            submit_button.disabled = True
        except NoMatches:
            submit_button = None

        try:
            try:
                review_state: ReviewState | NoSelection = self.query_one("#review_status", Select).value
            except NoMatches:
                review_state = ReviewState.COMMENTED

            if isinstance(review_state, NoSelection):
                self.notify("Please select a status for the new review!", severity="error")
                return

            # Find all the comments
            comments: list[dict[str, str | int]] = []
            for comment_data in self._pending_comments:
                # calculate position in diff
                position = comment_data.hunk.diff_position + comment_data.line_number + 1

                comments.append(
                    {
                        "path": comment_data.filename,
                        "body": comment_data.comment_text,
                        "position": position,
                    }
                )

            # Submit review
            review_body = self.query_one("#review_summary", Input).value
            new_review = await create_new_review(self.pr, review_state, review_body, comments)
            if new_review is not None:
                self.notify("New review created!")
                self.post_message(PullRequestSelected(self.pr))
        finally:
            # re-enable button when done (success or failure)
            if submit_button:
                submit_button.disabled = False

    def compose(self) -> ComposeResult:
        """parse diff and create split view widgets"""
        # add file filter input at top - get key from binding
        filter_key = LazyGithubBindings.DIFF_FILTER_FILES.key
        self._file_filter_input = FileFilterInput(
            placeholder=f"Filter files (press '{filter_key}' to focus, ESC to clear)", id="file_filter"
        )
        yield self._file_filter_input

        try:
            diff = parse_diff_from_str(self._raw_diff)
        except InvalidDiffFormat as e:
            yield Static(f"error parsing diff - please view on github: {e}", markup=False)
            return
        except Exception as e:
            yield Static(f"unexpected error: {e}", markup=False)
            return

        for path, changed_file in diff.files.items():
            # create file-level collapsible with formatted header (colored)
            file_title = _format_file_header(path, changed_file.deleted, changed_file.hunks)

            with Collapsible(title=file_title, collapsed=changed_file.deleted) as file_collapsible:  # type: ignore
                # track this collapsible for filtering
                self._file_collapsibles[path] = file_collapsible

                if changed_file.deleted:
                    yield Static("file was removed", classes="deleted-file", markup=False)
                    continue

                # create collapsible for each hunk with formatted header (colored)
                for hunk in changed_file.hunks:
                    hunk_title = _format_hunk_header(hunk)
                    with Collapsible(title=hunk_title, collapsed=True):  # type: ignore
                        try:
                            yield SplitDiffHunk(hunk, path)
                        except Exception as e:
                            yield Static(f"error rendering hunk: {e}", markup=False)

        # add pending comments preview section (hidden by default)
        self._comments_header = Label("Pending Comments:", classes="pending-comments-header")
        self._comments_header.display = False
        yield self._comments_header

        self._comments_container = Vertical(classes="pending-comments-container")
        self._comments_container.display = False
        yield self._comments_container

        # add submit review button at bottom
        yield SubmitReview(can_only_comment=self.reviewer_is_author)
