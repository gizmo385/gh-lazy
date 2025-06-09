from httpx import HTTPStatusError
from textual import on, work
from textual.app import ComposeResult
from textual.containers import Container, ScrollableContainer, VerticalScroll
from textual.content import Content
from textual.coordinate import Coordinate
from textual.widgets import Collapsible, DataTable, Label, ListItem, ListView, Markdown, Rule, TabPane

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.constants import CHECKMARK, X_MARK
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github.backends.protocol import GithubApiRequestFailed
from lazy_github.lib.github.checks import combined_check_status_for_ref
from lazy_github.lib.github.issues import get_comments, list_issues
from lazy_github.lib.github.pull_requests import (
    get_diff,
    get_full_pull_request,
    get_reviews,
    list_pull_requests_for_commit,
    merge_pull_request,
    reconstruct_review_conversation_hierarchy,
)
from lazy_github.lib.logging import lg
from lazy_github.lib.messages import IssuesAndPullRequestsFetched, PullRequestSelected
from lazy_github.models.github import (
    CheckStatus,
    CheckStatusState,
    FullPullRequest,
    PartialPullRequest,
    Repository,
)
from lazy_github.ui.screens.create_or_edit_pull_request import CreateOrEditPullRequestModal
from lazy_github.ui.screens.lookup_pull_request import LookupPullRequestModal
from lazy_github.ui.screens.new_comment import NewCommentModal
from lazy_github.ui.widgets.common import (
    LazilyLoadedDataTable,
    LazyGithubContainer,
    TableRow,
)
from lazy_github.ui.widgets.conversations import IssueCommentContainer, ReviewContainer
from lazy_github.ui.widgets.diff_viewer import DiffViewerContainer


def pull_request_to_cell(pr: PartialPullRequest) -> TableRow:
    return (pr.number, str(pr.state), pr.user.login, pr.title)


class PullRequestsContainer(LazyGithubContainer):
    """
    This container includes the primary datatable for viewing pull requests on the UI.
    """

    BINDINGS = [LazyGithubBindings.LOOKUP_PULL_REQUEST, LazyGithubBindings.EDIT_PULL_REQUEST]

    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self.status_column_index = -1
        self.number_column_index = -1
        self.title_column_index = -1
        self._table = LazilyLoadedDataTable(
            id="searchable_prs",
            table_id="pull_requests_table",
            search_input_id="pr_search_query",
            sort_key="number",
            load_function=None,
            batch_size=30,
            item_to_row=pull_request_to_cell,
            item_to_key=lambda p: str(p.number),
            cache_name="pull_requests",
            reverse_sort=True,
        )

    def compose(self) -> ComposeResult:
        self.border_title = Content.from_markup("\\[2] Pull Requests")
        yield self._table

    @work
    async def action_edit_pull_request(self) -> None:
        pr = await self.get_selected_pr()
        full_pr = await get_full_pull_request(pr.repo, pr.number)
        if full_pr.merged_at:
            self.notify("This PR has already been merged!", severity="warning")
        elif updated_pr := await self.app.push_screen_wait(CreateOrEditPullRequestModal(full_pr)):
            self.searchable_table.add_item(updated_pr)
            self.post_message(PullRequestSelected(updated_pr))

    @work
    async def action_lookup_pull_request(self) -> None:
        if pr := await self.app.push_screen_wait(LookupPullRequestModal()):
            if not self.searchable_table.item_in_table(pr):
                self.searchable_table.add_item(pr)

            self.post_message(PullRequestSelected(pr))
            lg.info(f"Looked up PR #{pr.number}")

    async def fetch_more_pull_requests(self, batch_size: int, batch_to_fetch: int) -> list[PartialPullRequest]:
        if not LazyGithubContext.current_repo:
            return []

        next_page = await list_issues(
            LazyGithubContext.current_repo,
            LazyGithubContext.config.pull_requests.state_filter,
            LazyGithubContext.config.pull_requests.owner_filter,
            page=batch_to_fetch,
            per_page=batch_size,
        )

        return [i for i in next_page if isinstance(i, PartialPullRequest)]

    def load_cached_pull_requests_for_repo(self, repo: Repository) -> None:
        self.searchable_table.initialize_from_cache(repo, PartialPullRequest)

    @property
    def searchable_table(self) -> LazilyLoadedDataTable[PartialPullRequest]:
        return self.query_one("#searchable_prs", LazilyLoadedDataTable)

    @property
    def table(self) -> DataTable:
        return self.searchable_table.table

    def on_mount(self) -> None:
        self.table.cursor_type = "row"
        self.table.add_column("#", key="number")
        self.table.add_column("Status", key="status")
        self.table.add_column("Author", key="author")
        self.table.add_column("Title", key="title")

        self.status_column_index = self.table.get_column_index("status")
        self.number_column_index = self.table.get_column_index("number")
        self.title_column_index = self.table.get_column_index("title")

    async def on_issues_and_pull_requests_fetched(self, message: IssuesAndPullRequestsFetched) -> None:
        message.stop()

        self.searchable_table.add_items(message.pull_requests)
        self.searchable_table.change_load_function(self.fetch_more_pull_requests)
        self.searchable_table.can_load_more = True
        self.searchable_table.current_batch = 1

    @work
    async def load_pull_request_for_current_commit(self) -> None:
        if LazyGithubContext.current_local_commit:
            associated_prs = await list_pull_requests_for_commit(LazyGithubContext.current_local_commit)
            if len(associated_prs) == 1:
                lg.info("Loading PR for your current commit")
                self.post_message(PullRequestSelected(associated_prs[0], False))

    async def get_selected_pr(self) -> PartialPullRequest:
        pr_number_coord = Coordinate(self.table.cursor_row, self.number_column_index)
        number = self.table.get_cell_at(pr_number_coord)
        return self.searchable_table.items[str(number)]

    @on(DataTable.RowSelected, "#pull_requests_table")
    async def pr_selected(self) -> None:
        pr = await self.get_selected_pr()
        lg.info(f"Selected PR: #{pr.number}")
        self.post_message(PullRequestSelected(pr))


class PrOverviewTabPane(TabPane):
    DEFAULT_CSS = """
    PrOverviewTabPane {
        overflow-y: auto;
    }

    Collapsible {
        height: auto;
    }

    ListView {
        height: auto;
    }
    """

    BINDINGS = [LazyGithubBindings.MERGE_PULL_REQUEST, LazyGithubBindings.EDIT_PULL_REQUEST]

    def __init__(self, pr: FullPullRequest) -> None:
        super().__init__("Overview", id="overview_pane")
        self.pr = pr

    def _status_check_to_label(self, status: CheckStatus) -> str:
        match status.state:
            case CheckStatusState.SUCCESS:
                status_summary = f"[greenyellow]{CHECKMARK} Passed[/]"
            case CheckStatusState.PENDING:
                status_summary = "[yellow]... Pending[/yellow]"
            case CheckStatusState.FAILURE:
                status_summary = f"[red]{X_MARK} Failed[/red]"
            case CheckStatusState.ERROR:
                status_summary = f"[red]{X_MARK} Errored[/red]"

        return f"{status_summary} {status.context} - {status.description}"

    async def action_merge_pull_request(self) -> None:
        if self.pr.merged_at is not None:
            self.notify("PR has already been merged!", title="Already Merged", severity="warning")
            return

        try:
            merge_result = await merge_pull_request(
                self.pr, LazyGithubContext.config.pull_requests.preferred_merge_method
            )
            if merge_result.merged:
                lg.info(f"Merged PR {self.pr.number} in repo {self.pr.repo.full_name}")
                self.notify(
                    f"Pull request {self.pr.number} merged. Note some cached information on the UI may be out of date.",
                    title="PR Merged",
                )

                # This will force refetch the updated information about the PR for the UI
                self.post_message(PullRequestSelected(self.pr))
            else:
                lg.warning(f"Failed to merge PR {self.pr.number} in repo {self.pr.repo.full_name}")
                self.notify(
                    f"Pull request {self.pr.number} could not be merged", title="Error Merging PR", severity="error"
                )
        except GithubApiRequestFailed:
            lg.exception(f"Failed to merge PR {self.pr.number} in repo {self.pr.repo.full_name}")
            self.notify(
                f"Pull request {self.pr.number} could not be merged", title="Error Merging PR", severity="error"
            )

    @work
    async def action_edit_pull_request(self) -> None:
        if self.pr.merged_at:
            self.notify("This PR has already been merged!", severity="warning")
        elif updated_pr := await self.app.push_screen_wait(CreateOrEditPullRequestModal(self.pr)):
            self.post_message(PullRequestSelected(updated_pr))

    def compose(self) -> ComposeResult:
        pr_link = f'[link="{self.pr.html_url}"](#{self.pr.number})[/link]'
        user_link = f'[link="{self.pr.user.html_url}"]{self.pr.user.login}[/link]'
        merge_from = None
        if self.pr.head:
            merge_from = f"[bold]{self.pr.head.user.login}:{self.pr.head.ref}[/bold]"
        merge_to = None
        if self.pr.base:
            merge_to = f"[bold]{self.pr.base.user.login}:{self.pr.base.ref}[/bold]"

        change_summary = " • ".join(
            [
                "1 commit" if self.pr.commits == 1 else f"{self.pr.commits} commits",
                "1 file changed" if self.pr.changed_files == 1 else f"{self.pr.changed_files} files changed",
                f"[greenyellow]+{self.pr.additions}[/]",
                f"[red]-{self.pr.deletions}[/]",
                f"{merge_from} → {merge_to}",
            ]
        )

        if self.pr.merged_at:
            merge_status = "[orchid]Merged[/]"
        elif self.pr.closed_at:
            merge_status = "[red]closed[/]"
        else:
            merge_status = "[greenyellow]Open[/]"

        with ScrollableContainer():
            yield Label(Content.from_markup(f"{merge_status} [b]{self.pr.title}[b] {pr_link} by {user_link}"))
            yield Label(Content.from_markup(change_summary))

            created_date = self.pr.created_at.strftime("%c")
            date_text = f"\nOpened on {created_date}"

            if self.pr.merged_at:
                merged_date = self.pr.merged_at.strftime("%c")
                date_text += f" • Merged on {merged_date}"
            yield Label(Content.from_markup(date_text))
            yield Rule()

            # This is where we'll store information about the status checks being run on the PR
            with Collapsible(title="Status Checks: ...", id="collapsible_status_checks") as c:
                c.loading = True
                # TODO: We should probably make this a table? That would allow follow-up actions to be performed as well
                yield ListView(id="status_checks_list")

            yield Rule()
            yield Markdown(self.pr.body)

    @work
    async def load_checks(self) -> None:
        # TODO: This should probably check normal check runs as well? Unsure if the combined check status includes all
        # of those
        combined_check_status = await combined_check_status_for_ref(self.pr.repo, self.pr.head.sha)
        status_checks_list = self.query_one("#status_checks_list", ListView)
        collapse_container = self.query_one("#collapsible_status_checks", Collapsible)
        if statuses := combined_check_status.statuses:
            status_labels = sorted(self._status_check_to_label(c) for c in statuses)
            status_checks_list.extend(
                ListItem(Label(Content.from_markup(status_label))) for status_label in status_labels
            )
            collapse_container.title = f"Status checks: {combined_check_status.state.value.title()}"
        else:
            collapse_container.title = "No status checks on PR"

        collapse_container.loading = False

    async def on_mount(self) -> None:
        _ = self.load_checks()


class PrDiffTabPane(TabPane):
    def __init__(self, pr: FullPullRequest) -> None:
        super().__init__("Diff", id="diff_pane")
        self.pr = pr
        self.view_container = Container()

    def compose(self) -> ComposeResult:
        yield self.view_container

    @work
    async def fetch_diff(self) -> None:
        try:
            diff = await get_diff(self.pr)
        except HTTPStatusError as hse:
            if hse.response.status_code == 404:
                await self.view_container.mount(Label("No diff contents found"))
            else:
                raise
        else:
            current_user = await LazyGithubContext.client.user()
            reviewer_is_author = self.pr.user.login == current_user.login
            await self.view_container.mount(DiffViewerContainer(self.pr, reviewer_is_author, diff))
        self.loading = False

    async def on_mount(self) -> None:
        self.loading = True
        self.fetch_diff()


class PrConversationTabPane(TabPane):
    BINDINGS = [LazyGithubBindings.NEW_COMMENT]

    def __init__(self, pr: FullPullRequest) -> None:
        super().__init__("Conversation", id="conversation_pane")
        self.pr = pr

    def compose(self) -> ComposeResult:
        yield VerticalScroll(id="pr_comments_and_reviews")

    @property
    def comments_and_reviews(self) -> VerticalScroll:
        return self.query_one("#pr_comments_and_reviews", VerticalScroll)

    @work
    async def fetch_conversation(self) -> None:
        reviews = await get_reviews(self.pr)
        review_hierarchy = reconstruct_review_conversation_hierarchy(reviews)
        comments = await get_comments(self.pr)
        self.comments_and_reviews.remove_children()

        handled_comment_node_ids: list[int] = []
        for review in reviews:
            if review.body:
                handled_comment_node_ids.extend([c.id for c in review.comments])
            review_container = ReviewContainer(self.pr, review, review_hierarchy)
            self.comments_and_reviews.mount(review_container)

        for comment in comments:
            if comment.body and comment.id not in handled_comment_node_ids:
                comment_container = IssueCommentContainer(self.pr, comment)
                self.comments_and_reviews.mount(comment_container)

        if len(self.comments_and_reviews.children) == 0:
            self.comments_and_reviews.mount(Label("No reviews or comments available"))

        self.loading = False

    def on_mount(self) -> None:
        self.loading = True
        self.fetch_conversation()

    @work
    async def action_new_comment(self) -> None:
        await self.app.push_screen_wait(NewCommentModal(self.pr.repo, self.pr, None))
        self.fetch_conversation()
