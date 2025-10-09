from difflib import SequenceMatcher
from typing import Dict, List, Set, Tuple, Optional
from collections import defaultdict

from rich.color import blend_rgb, Color
from rich.color_triplet import ColorTriplet
from rich.segment import Segment
from rich.style import Style
from rich.syntax import Syntax
from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Container, Horizontal, Vertical, VerticalScroll
from textual.content import Content
from textual.widget import Widget
from textual.widgets import Static, RichLog, Collapsible
from textual.message import Message
from textual.reactive import reactive

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.diff_parser import Hunk, ChangedFile, parse_diff_from_str, InvalidDiffFormat


# color constants from dunk
MONOKAI_BACKGROUND = Color.from_rgb(red=39, green=40, blue=34)
DUNK_BG_HEX = "#0d0f0b"
MONOKAI_BG_HEX = MONOKAI_BACKGROUND.triplet.hex


class ScrollSync(Message):
    """message to sync scroll between panes"""
    def __init__(self, scroll_y: float) -> None:
        super().__init__()
        self.scroll_y = scroll_y


class SyncedScrollPane(VerticalScroll):
    """scrollable pane that can sync vertical scroll with sibling"""

    DEFAULT_CSS = """
    SyncedScrollPane {
        width: 1fr;
        height: auto;
        max-height: 30;
        border: solid $primary-lighten-1;
        overflow-y: auto;
        overflow-x: auto;
    }

    SyncedScrollPane > Static {
        width: auto;
    }
    """

    _syncing = False  # prevent scroll loop

    def __init__(self, content: Text, classes: str | None = None) -> None:
        super().__init__(classes=classes)
        self.content = content

    def compose(self) -> ComposeResult:
        # create static with shrink=False so it doesn't wrap
        static = Static(self.content, markup=False, shrink=False)
        yield static

    def on_mount(self) -> None:
        """watch scroll position after mount"""
        self.watch(self, "scroll_y", self._on_scroll_y_change)

    def _on_scroll_y_change(self, old_value: float, new_value: float) -> None:
        """when scroll changes, tell sibling to sync"""
        if not self._syncing and old_value != new_value:
            self.post_message(ScrollSync(new_value))

    def sync_scroll(self, scroll_y: float) -> None:
        """sync scroll from sibling without triggering another sync"""
        self._syncing = True
        self.scroll_y = scroll_y
        self._syncing = False


class SplitDiffHunk(Widget):
    """widget that show single hunk in side-by-side split view with syntax highlighting"""

    # make it focusable so we can tab/navigate to it
    can_focus = True

    DEFAULT_CSS = """
    SplitDiffHunk {
        width: 100%;
        height: auto;
        margin: 1 0;
        border: solid transparent;
    }

    SplitDiffHunk:focus-within {
        border: thick $accent;
        background: $boost;
    }

    SplitDiffHunk .header-row {
        width: 100%;
        height: auto;
    }

    SplitDiffHunk .column-header {
        width: 1fr;
        height: 1;
        text-align: center;
        text-style: bold;
    }

    SplitDiffHunk .removed-header {
        background: $error 30%;
        color: $error;
    }

    SplitDiffHunk .added-header {
        background: $success 30%;
        color: $success;
    }

    SplitDiffHunk:focus-within .removed-header {
        background: $error 50%;
        text-style: bold;
    }

    SplitDiffHunk:focus-within .added-header {
        background: $success 50%;
        text-style: bold;
    }

    SplitDiffHunk .removed-side {
        border: solid $error;
    }

    SplitDiffHunk .added-side {
        border: solid $success;
    }

    SplitDiffHunk Horizontal {
        width: 100%;
        height: auto;
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

    def compose(self) -> ComposeResult:
        """create side-by-side diff view with syntax highlighting"""
        # parse hunk lines to separate source (removed/context) and target (added/context)
        # track which lines are actually changed vs context
        source_lines = []
        target_lines = []
        source_removed_indices = set()
        target_added_indices = set()

        source_idx = 0
        target_idx = 0

        for line in self.hunk.lines:
            if line.startswith('-'):
                # removed line - only in source
                source_lines.append(line[1:])
                source_removed_indices.add(source_idx)
                source_idx += 1
            elif line.startswith('+'):
                # added line - only in target
                target_lines.append(line[1:])
                target_added_indices.add(target_idx)
                target_idx += 1
            else:
                # context line - in both
                clean_line = line[1:] if line.startswith(' ') else line
                source_lines.append(clean_line)
                target_lines.append(clean_line)
                source_idx += 1
                target_idx += 1

        # create syntax highlighted text with diff highlighting
        from rich.console import Console
        from rich.table import Table

        lexer = Syntax.guess_lexer(self.filename)

        # pad shorter side with empty lines so both same height
        max_lines = max(len(source_lines), len(target_lines))
        while len(source_lines) < max_lines:
            source_lines.append("")
        while len(target_lines) < max_lines:
            target_lines.append("")

        # build source side with red background on removed lines
        source_text = Text()
        for idx, line in enumerate(source_lines):
            if idx in source_removed_indices:
                # highlight removed line with red background
                source_text.append(f"{self.hunk.file_start_line + idx:4d} ", style="dim")
                source_text.append(line, style="on #3a0a0a")  # dark red background
            else:
                # context line or padding
                if line:  # only show line number if not padding
                    source_text.append(f"{self.hunk.file_start_line + idx:4d} ", style="dim")
                    source_text.append(line)
                else:
                    # empty padding line
                    source_text.append("     ", style="dim")
            source_text.append("\n")

        # build target side with green background on added lines
        target_text = Text()
        for idx, line in enumerate(target_lines):
            if idx in target_added_indices:
                # highlight added line with green background
                target_text.append(f"{self.hunk.file_start_line + idx:4d} ", style="dim")
                target_text.append(line, style="on #0a3a0a")  # dark green background
            else:
                # context line or padding
                if line:  # only show line number if not padding
                    target_text.append(f"{self.hunk.file_start_line + idx:4d} ", style="dim")
                    target_text.append(line)
                else:
                    # empty padding line
                    target_text.append("     ", style="dim")
            target_text.append("\n")

        # show headers with proper width
        with Horizontal(classes="header-row"):
            yield Static("─ BEFORE", classes="column-header removed-header", markup=False)
            yield Static("+ AFTER", classes="column-header added-header", markup=False)

        # create two synced scroll panes side by side
        with Horizontal():
            self.source_pane = SyncedScrollPane(source_text, classes="removed-side")
            yield self.source_pane

            self.target_pane = SyncedScrollPane(target_text, classes="added-side")
            yield self.target_pane

    def on_scroll_sync(self, message: ScrollSync) -> None:
        """handle scroll sync message from one pane and apply to other"""
        message.stop()  # don't bubble up

        # sync to the pane that didn't send the message
        if message.control == self.source_pane:
            self.target_pane.sync_scroll(message.scroll_y)
        elif message.control == self.target_pane:
            self.source_pane.sync_scroll(message.scroll_y)


class SplitDiffViewer(Vertical):
    """main container for split diff view, similar to dunk"""

    DEFAULT_CSS = """
    SplitDiffViewer {
        width: 100%;
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
    """

    BINDINGS = [
        LazyGithubBindings.DIFF_NEXT_HUNK,
        LazyGithubBindings.DIFF_PREVIOUS_HUNK,
    ]

    def __init__(self, diff: str, id: str | None = None) -> None:
        super().__init__(id=id)
        self._raw_diff = diff

    def action_previous_hunk(self) -> None:
        """jump to previous hunk (J key)"""
        # get all hunk widgets
        hunks = list(self.query(SplitDiffHunk))
        if not hunks:
            return

        # find currently focused hunk
        focused = self.screen.focused
        if focused in hunks:
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
        # get all hunk widgets
        hunks = list(self.query(SplitDiffHunk))
        if not hunks:
            return

        # find currently focused hunk
        focused = self.screen.focused
        if focused in hunks:
            current_idx = hunks.index(focused)
            if current_idx < len(hunks) - 1:
                hunks[current_idx + 1].focus()
                hunks[current_idx + 1].scroll_visible()
        else:
            # nothing focused, focus first hunk
            hunks[0].focus()
            hunks[0].scroll_visible()

    def compose(self) -> ComposeResult:
        """parse diff and create split view widgets"""
        try:
            diff = parse_diff_from_str(self._raw_diff)
        except InvalidDiffFormat as e:
            yield Static(f"error parsing diff - please view on github: {e}", markup=False)
            return
        except Exception as e:
            yield Static(f"unexpected error: {e}", markup=False)
            return

        for path, changed_file in diff.files.items():
            # create file-level collapsible - use Content to avoid markup parsing
            file_title = f"{path} (deleted)" if changed_file.deleted else path
            file_content = Content.from_text(file_title, markup=False)

            with Collapsible(title=file_content, collapsed=changed_file.deleted):  # type: ignore
                if changed_file.deleted:
                    yield Static("file was removed", classes="deleted-file", markup=False)
                    continue

                # create collapsible for each hunk - use Content to avoid markup parsing
                for hunk in changed_file.hunks:
                    hunk_content = Content.from_text(hunk.header, markup=False)
                    with Collapsible(title=hunk_content, collapsed=False):  # type: ignore
                        try:
                            yield SplitDiffHunk(hunk, path)
                        except Exception as e:
                            yield Static(f"error rendering hunk: {e}", markup=False)
