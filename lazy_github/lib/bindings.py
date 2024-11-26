from textual.binding import Binding

from lazy_github.lib.utils import classproperty


class LazyGithubBindings:
    # Global App Bindings
    QUIT_APP = Binding("q", "quit", "Quit", id="app.quit")
    OPEN_COMMAND_PALLETE = Binding("ctrl+p", "command_palette", "Open command pallete", id="app.command_palette")
    MAXIMIZE_WIDGET = Binding("ctrl+m", "maximize", "Maximize", id="app.maximize_widget")

    # Triggering creation flows
    OPEN_ISSUE = Binding("I", "open_issue", "Open new issue", id="issue.new")
    EDIT_ISSUE = Binding("E", "edit_issue", "Edit issue", id="issue.edit")
    OPEN_PULL_REQUEST = Binding("P", "open_pull_request", "Open new pull request", id="pull_request.new")
    NEW_COMMENT = Binding("n", "new_comment", "New comment", id="conversation.comment.new")
    REPLY_TO_REVIEW = Binding("r", "reply_to_review", "Reply to review", id="conversation.review.reply")
    REPLY_TO_COMMENT = Binding("r", "reply_to_individual_comment", "Reply to comment", id="conversation.comment.reply")

    # Repository actions
    TOGGLE_FAVORITE_REPO = Binding("ctrl+f", "toggle_favorite_repo", "Toggle favorite", id="repositories.favorite")

    # Common widget bindings
    SELECT_ENTRY = Binding("enter,space", "select_cursor", "Select table entry", id="common.table.select", show=False)
    SEARCH_TABLE = Binding("/", "focus_search", "Search", id="common.table.search")
    TABLE_DOWN = Binding("j", "cursor_down", "Table cursor down", show=False, id="common.table.cursor_down")
    TABLE_PAGE_DOWN = Binding("J", "page_down", "Table page down", show=False, id="common.table.page_down")
    TABLE_CURSOR_UP = Binding("k", "cursor_up", "Table cursor up", show=False, id="common.table.cursor_up")
    TABLE_PAGE_UP = Binding("K", "page_up", "Table page up", show=False, id="common.table.page_up")
    TABLE_SCROLL_RIGHT = Binding("l", "scroll_right", "Table scroll right", show=False, id="common.table.scroll_right")
    TABLE_PAGE_RIGHT = Binding("L", "page_right", "Table page right", show=False, id="common.table.page_right")
    TABLE_SCROLL_LEFT = Binding("h", "scroll_left", "Table scroll left", show=False, id="common.table.scroll_left")
    TABLE_PAGE_LEFT = Binding("H", "page_left", "Table page left", show=False, id="common.table.page_left")
    TABLE_SCROLL_TOP = Binding("g", "scroll_top", "Table scroll to top", show=False, id="common.table.scroll_top")
    TABLE_SCROLL_BOTTOM = Binding(
        "G", "scroll_bottom", "Table scroll to bottom", show=False, id="common.table.scroll_bottom"
    )
    TABLE_PAGE_LEFT = Binding("^", "page_left", "Table page left", show=False, id="common.table.page_left")
    TABLE_PAGE_RIGHT = Binding("$", "page_right", "Table page right", show=False, id="common.table.page_right")

    # Dialog bindings
    SUBMIT_DIALOG = Binding("shift+enter", "submit", "Submit", id="modal.submit")
    CANCEL_DIALOG = Binding("q, ESC", "cancel", "Cancel", id="modal.cancel")
    SEARCH_DIALOG = Binding("/", "search", "Search", id="modal.search")

    # Focusing different UI elements
    FOCUS_REPOSITORY_TABLE = Binding(
        "1", "focus_section('#repos_table')", "Focus repos table", show=False, id="main.repos.focus"
    )
    FOCUS_PULL_REQUEST_TABLE = Binding(
        "2", "focus_section('#pull_requests_table')", "Focus PRs table", show=False, id="main.pull_requests.focus"
    )
    FOCUS_ISSUE_TABLE = Binding(
        "3", "focus_section('#issues_table')", "Focus issues table", show=False, id="main.issues.focus"
    )
    FOCUS_WORKFLOW_TABS = Binding(
        "4", "focus_workflow_tabs", "Focus workflows table", show=False, id="main.workflows.focus"
    )
    FOCUS_DETAIL_TABS = Binding("5", "focus_tabs", "Focus details tabs", show=False, id="main.details.focus")
    FOCUS_COMMAND_LOG = Binding(
        "6", "focus_section('LazyGithubCommandLog')", "Focus command log", show=False, id="main.command_log.focus"
    )

    @classproperty
    def all(cls) -> list[Binding]:
        """Returns all bindings which can be rebound"""
        return [v for v in cls.__dict__.values() if isinstance(v, Binding) if v.id]

    @classproperty
    def all_by_id(cls) -> dict[str, Binding]:
        """Returns a dictionary of all bindings which can be rebound, with the key being their ID"""
        return {v.id: v for v in cls.__dict__.values() if isinstance(v, Binding) and v.id}