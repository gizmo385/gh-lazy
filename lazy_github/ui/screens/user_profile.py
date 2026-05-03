import asyncio

from textual import on
from textual.app import ComposeResult
from textual.containers import Container, ScrollableContainer
from textual.content import Content
from textual.coordinate import Coordinate
from textual.screen import ModalScreen
from textual.widgets import Label, Markdown, Rule

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.constants import private_string
from lazy_github.lib.github.repositories import list_repos_for_user
from lazy_github.lib.github.users import get_full_user_by_username
from lazy_github.lib.logging import lg
from lazy_github.models.github import FullUser, Repository
from lazy_github.ui.widgets.common import LazyGithubFooter, _VimLikeDataTable


def _format_field(label: str, value: str | int | None) -> str | None:
    if value is None or value == "":
        return None
    return f"[bold]{label}:[/bold] {value}"


class UserReposTable(_VimLikeDataTable):
    """A vim-friendly DataTable for the user's repositories shown inside the profile modal."""

    DEFAULT_CSS = """
    UserReposTable {
        height: auto;
        max-height: 20;
        margin-top: 1;
    }
    """


class UserProfileContainer(Container):
    DEFAULT_CSS = """
    UserProfileContainer {
        align: center top;
        padding: 1 2;
        height: auto;
    }

    UserProfileContainer Label {
        margin-bottom: 1;
    }
    """

    def __init__(self, user: FullUser, repos: list[Repository]) -> None:
        super().__init__()
        self.user = user
        self.repos = repos
        self.repos_by_full_name: dict[str, Repository] = {r.full_name: r for r in repos}
        self.repos_table = UserReposTable(id="user_repos_table")

    def compose(self) -> ComposeResult:
        u = self.user
        header_name = u.name or u.login
        profile_link = f'[link="{u.html_url}"]@{u.login}[/link]'
        kind = (u.type or "User").lower()

        yield Label(Content.from_markup(f"[b]{header_name}[/b] ({profile_link}) — {kind}"))
        yield Rule()

        if u.bio:
            yield Markdown(u.bio)
            yield Rule()

        info_lines = [
            line
            for line in (
                _format_field("Company", u.company),
                _format_field("Location", u.location),
                _format_field("Email", u.email),
                _format_field("Blog", u.blog),
                _format_field("Twitter", f"@{u.twitter_username}" if u.twitter_username else None),
            )
            if line
        ]
        if info_lines:
            for line in info_lines:
                yield Label(Content.from_markup(line))
            yield Rule()

        joined = u.created_at.strftime("%Y-%m-%d") if u.created_at else "unknown"
        yield Label(
            Content.from_markup(
                f"[bold]Public repos:[/bold] {u.public_repos}    "
                f"[bold]Gists:[/bold] {u.public_gists}    "
                f"[bold]Followers:[/bold] {u.followers}    "
                f"[bold]Following:[/bold] {u.following}"
            )
        )
        yield Label(Content.from_markup(f"[bold]Joined:[/bold] {joined}"))

        yield Rule()
        repo_count = len(self.repos)
        if repo_count == 0:
            yield Label(Content.from_markup("[bold]Recent repositories[/bold]"))
            yield Label(Content.from_markup("[dim]No public repositories.[/dim]"))
        else:
            heading = (
                f"[bold]Recent repositories[/bold] "
                f"[dim](showing {repo_count} of {u.public_repos}, press enter to open)[/dim]"
            )
            yield Label(Content.from_markup(heading))
            yield self.repos_table

    def on_mount(self) -> None:
        if not self.repos:
            return
        table = self.repos_table
        table.cursor_type = "row"
        table.add_column("Owner", key="owner")
        table.add_column("Name", key="name")
        table.add_column("Private", key="private")
        table.add_column("Description", key="description")
        for repo in self.repos:
            description = repo.description or ""
            table.add_row(
                repo.owner.login,
                repo.name,
                private_string(repo.private),
                description,
                key=repo.full_name,
            )

    def get_selected_repo(self) -> Repository | None:
        if not self.repos:
            return None
        try:
            owner = self.repos_table.get_cell_at(Coordinate(self.repos_table.cursor_row, 0))
            name = self.repos_table.get_cell_at(Coordinate(self.repos_table.cursor_row, 1))
        except Exception:
            return None
        return self.repos_by_full_name.get(f"{owner}/{name}")


class UserProfileModal(ModalScreen[Repository | None]):
    DEFAULT_CSS = """
    UserProfileModal {
        align: center middle;
        content-align: center middle;
    }

    UserProfileModal > ScrollableContainer {
        width: 90;
        max-height: 90%;
        border: thick $background 80%;
        background: $surface-lighten-1;
    }
    """

    BINDINGS = [LazyGithubBindings.CLOSE_DIALOG]

    def __init__(self, user: FullUser, repos: list[Repository]) -> None:
        super().__init__()
        self.user = user
        self.repos = repos

    def compose(self) -> ComposeResult:
        with ScrollableContainer():
            yield UserProfileContainer(self.user, self.repos)
        yield LazyGithubFooter()

    @property
    def profile_container(self) -> UserProfileContainer:
        return self.query_one(UserProfileContainer)

    @on(_VimLikeDataTable.RowSelected, "#user_repos_table")
    async def repo_selected(self) -> None:
        repo = self.profile_container.get_selected_repo()
        if repo is not None:
            self.dismiss(repo)

    async def action_close(self) -> None:
        self.dismiss(None)


async def open_user_profile(app, username: str) -> Repository | None:
    """Fetch the user's profile and recent repos, push the modal, and return the selected repo (if any)."""
    try:
        user, repos = await asyncio.gather(
            get_full_user_by_username(username),
            list_repos_for_user(username),
        )
    except Exception:
        lg.exception("Error fetching user profile")
        app.notify(f"Failed to load profile for @{username}", severity="error")
        return None

    if user is None:
        app.notify(f"Could not find user @{username}", severity="error")
        return None

    return await app.push_screen_wait(UserProfileModal(user, repos))
