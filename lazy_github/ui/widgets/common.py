from asyncio import Lock
from datetime import datetime
from typing import Awaitable, Callable, Generic, TypeVar

from pydantic import BaseModel
from textual import on, work
from textual.app import ComposeResult
from textual.containers import Container, Horizontal, Vertical
from textual.events import Blur
from textual.widgets import Button, DataTable, Footer, Input
from textual.widgets.data_table import RowDoesNotExist

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.cache import load_models_from_cache, save_models_to_cache
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.logging import lg
from lazy_github.models.github import Repository

# Some handy type defs
T = TypeVar("T", bound=BaseModel)
TablePopulationFunction = Callable[[int, int], Awaitable[list[T]]]
TableCellType = str | int | datetime
TableRow = tuple[TableCellType, ...]
TableRowMap = dict[str, tuple[TableCellType, ...]]


class LazyGithubFooter(Footer):
    def __init__(self) -> None:
        super().__init__(show_command_palette=False)


class _VimLikeDataTable(DataTable[TableCellType]):
    "An data table for LazyGithub that provides some more vim-like bindings"

    BINDINGS = [
        LazyGithubBindings.SELECT_ENTRY,
        LazyGithubBindings.TABLE_DOWN,
        LazyGithubBindings.TABLE_PAGE_DOWN,
        LazyGithubBindings.TABLE_CURSOR_UP,
        LazyGithubBindings.TABLE_PAGE_UP,
        LazyGithubBindings.TABLE_SCROLL_RIGHT,
        LazyGithubBindings.TABLE_PAGE_RIGHT,
        LazyGithubBindings.TABLE_SCROLL_LEFT,
        LazyGithubBindings.TABLE_PAGE_LEFT,
        LazyGithubBindings.TABLE_SCROLL_TOP,
        LazyGithubBindings.TABLE_SCROLL_BOTTOM,
        LazyGithubBindings.TABLE_PAGE_LEFT,
        LazyGithubBindings.TABLE_PAGE_RIGHT,
    ]


class ToggleableSearchInput(Input):
    def _on_blur(self, event: Blur) -> None:
        if not self.value.strip():
            # If we lose focus and the content is empty, hide it
            self.can_focus = False
            self.display = False
        return super()._on_blur(event)


class SearchableDataTable(Vertical, Generic[T]):
    BINDINGS = [LazyGithubBindings.SEARCH_TABLE]

    DEFAULT_CSS = """
    ToggleableSearchInput {
        margin-bottom: 1;
    }
    """

    def __init__(
        self,
        table_id: str,
        search_input_id: str,
        sort_key: str,
        item_to_row: Callable[[T], TableRow],
        item_to_key: Callable[[T], str],
        *args,
        reverse_sort: bool = False,
        cache_name: str | None = None,
        repo_based_cache: bool = True,
        **kwargs,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.table = _VimLikeDataTable(id=table_id)
        self.search_input = ToggleableSearchInput(placeholder="Search...", id=search_input_id)
        self.search_input.display = False
        self.search_input.can_focus = False
        self.sort_key = sort_key
        self.reverse_sort = reverse_sort
        self.item_to_row = item_to_row
        self.item_to_key = item_to_key
        self.cache_name = cache_name
        self.repo_based_cache = repo_based_cache
        self.items: dict[str, T] = {}

    def item_in_table(self, item: T) -> bool:
        return self.item_to_key(item) in self.items

    def sort_table(self):
        self.table.sort(self.sort_key, reverse=self.reverse_sort)

    def compose(self) -> ComposeResult:
        yield self.search_input
        yield self.table

    async def action_focus_search(self) -> None:
        """Focus on the search"""
        self.search_input.can_focus = True
        self.search_input.display = True
        self.search_input.focus()

    def clear_rows(self):
        """Removes all rows currently displayed and tracked in this table"""
        self.items = {}
        self.table.clear()

    def initialize_from_cache(self, repo: Repository | None, expect_type: type[T]) -> None:
        """Loads values expected to be of the specified type from the cache for this table"""
        self.clear_rows()
        if not self.cache_name:
            return

        lg.debug(f"Reading values of type '{expect_type.__name__}' from '{self.cache_name}' cache")

        cached_models = load_models_from_cache(repo if self.repo_based_cache else None, self.cache_name, expect_type)
        self.add_items(cached_models, write_to_cache=False)

    def save_to_cache(self):
        """Saves the models in the table to the specified cache location, if one is set"""
        if not self.cache_name:
            return

        save_models_to_cache(
            LazyGithubContext.current_repo if self.repo_based_cache else None,
            self.cache_name,
            self.items.values(),
        )

    def add_item(self, item: T, write_to_cache: bool = True) -> None:
        """Add an individual row with the specified key to the table. The table will be sorted after the key is added"""
        item_key = self.item_to_key(item)
        try:
            # Before we add the row, we want to see if the key already exists
            if item_key in self.items:
                self.table.remove_row(item_key)
        except RowDoesNotExist:
            # If the row doesn't exist, then something already removed it and we can move on
            pass

        self.items[item_key] = item
        self.table.add_row(*self.item_to_row(item), key=item_key)
        self.table.sort(self.sort_key, reverse=self.reverse_sort)

        if write_to_cache and self.cache_name:
            self.save_to_cache()

    def add_items(self, new_items: list[T], write_to_cache: bool = True) -> None:
        """Add new rows to the currently displayed table and cache"""
        for item in new_items:
            self.add_item(item, write_to_cache=False)

        if write_to_cache:
            self.save_to_cache()

    @on(Input.Submitted)
    async def handle_submitted_search(self) -> None:
        """When a search is submitted, triggers the filter for the entries in the table"""
        search_query = self.search_input.value.strip().lower()
        filtered_items: list[T] = []
        for item in self.items.values():
            if search_query in str(self.item_to_row(item)).lower() or not search_query:
                filtered_items.append(item)

        self.table.clear()
        self.add_items(filtered_items, write_to_cache=False)
        self.table.focus()


class LazilyLoadedDataTable(SearchableDataTable[T], Generic[T]):
    """A searchable data table that is lazily loaded when you have viewed the currently loaded data"""

    def __init__(
        self,
        table_id: str,
        search_input_id: str,
        sort_key: str,
        load_function: TablePopulationFunction | None,
        batch_size: int,
        item_to_row: Callable[[T], TableRow],
        item_to_key: Callable[[T], str],
        *args,
        load_more_data_buffer: int = 5,
        reverse_sort: bool = False,
        cache_name: str | None = None,
        repo_based_cache: bool = True,
        **kwargs,
    ) -> None:
        super().__init__(
            table_id,
            search_input_id,
            sort_key,
            item_to_row,
            item_to_key,
            *args,
            reverse_sort=reverse_sort,
            cache_name=cache_name,
            repo_based_cache=repo_based_cache,
            **kwargs,
        )
        self.fetch_lock = Lock()
        self.load_function = load_function
        self.batch_size = batch_size
        self.load_more_data_buffer = load_more_data_buffer
        self.current_batch = 0

        # We initialize this to true and set it to false later if we believe we've run out of data to load from the load
        # function.
        self.can_load_more = True

    def change_load_function(self, new_load_function: TablePopulationFunction | None) -> None:
        self.load_function = new_load_function

    def clear_rows(self):
        """Removes all rows currently displayed and tracked in this table"""
        super().clear_rows()
        self.current_batch = 0
        self.can_load_more = True
        self.load_function = None

    @work
    async def load_more_data(self, row_highlighted: DataTable.RowHighlighted) -> None:
        async with self.fetch_lock:
            rows_remaining = len(self.items) - row_highlighted.cursor_row
            if not (self.can_load_more and self.load_function):
                return

            if rows_remaining > self.load_more_data_buffer:
                return

            additional_data = await self.load_function(self.batch_size, self.current_batch + 1)
            self.current_batch += 1
            if len(additional_data) == 0:
                self.can_load_more = False

            self.add_items(additional_data)

    @on(DataTable.RowHighlighted)
    async def check_highlighted_row_boundary(self, row_highlighted: DataTable.RowHighlighted) -> None:
        self.load_more_data(row_highlighted)


class LazyGithubContainer(Container):
    """
    Base container class for focusible containers within the Lazy Github UI
    """

    DEFAULT_CSS = """
    LazyGithubContainer {
        display: block;
        border: solid $primary-lighten-3;
    }

    LazyGithubContainer:focus-within {
        min-height: 40%;
        border: solid $success;
    }
    """


class ModalDialogButtons(Horizontal):
    DEFAULT_CSS = """
    ModalDialogButtons {
        align: center middle;
        height: auto;
        width: 100%;
    }
    Button {
        margin: 1;
    }
    """

    def __init__(self, submit_text: str = "Submit", cancel_text="Cancel") -> None:
        super().__init__()
        self.submit_text = submit_text
        self.cancel_text = cancel_text

    def compose(self) -> ComposeResult:
        yield Button(self.submit_text, id="submit", variant="success")
        yield Button(self.cancel_text, id="cancel", variant="error")
