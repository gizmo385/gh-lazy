from textual import work
from textual.app import ComposeResult
from textual.containers import ScrollableContainer
from textual.content import Content
from textual.widgets import Label, Markdown, Rule, TabPane

from lazy_github.lib.github.repositories import get_repo_readme
from lazy_github.models.github import Repository


class RepoOverviewTabPane(TabPane):
    DEFAULT_CSS = """
    RepoOverviewTabPane {
        overflow-y: auto;
    }
    """

    def __init__(self, repo: Repository) -> None:
        super().__init__("Repo Overview", id="repo_overview_pane")
        self.repo = repo

    def compose(self) -> ComposeResult:
        repo_url = f"https://github.com/{self.repo.full_name}"
        owner_url = self.repo.owner.html_url

        title_link = f"[@click=screen.open_gh_link('{repo_url}')]{self.repo.full_name}[/]"
        owner_link = f"[@click=screen.open_gh_link('{owner_url}')]{self.repo.owner.login}[/]"

        flags: list[str] = []
        if self.repo.private:
            flags.append("[red]Private[/]")
        else:
            flags.append("[greenyellow]Public[/]")
        if self.repo.archived:
            flags.append("[orchid]Archived[/]")

        with ScrollableContainer():
            yield Label(Content.from_markup(f"[b]{title_link}[/] • {' • '.join(flags)}"))
            yield Label(Content.from_markup(f"Owned by {owner_link}"))
            if self.repo.default_branch:
                yield Label(Content.from_markup(f"Default branch: [bold]{self.repo.default_branch}[/]"))
            if self.repo.description:
                yield Label(Content.from_markup(f"\n{self.repo.description}"))
            yield Rule()
            yield Markdown(id="repo_readme", open_links=False)

    def on_mount(self) -> None:
        self.load_readme()

    @work
    async def load_readme(self) -> None:
        readme = await get_repo_readme(self.repo)
        markdown = self.query_one("#repo_readme", Markdown)
        if readme:
            await markdown.update(readme)
        else:
            await markdown.update("*No README found.*")
