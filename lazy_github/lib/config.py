import json
from contextlib import contextmanager
from datetime import datetime, timedelta
from enum import StrEnum
from pathlib import Path
from typing import Any, Generator, Optional

from pydantic import BaseModel, field_serializer, field_validator
from textual.theme import BUILTIN_THEMES, Theme

from lazy_github.lib.constants import CONFIG_FOLDER, IssueOwnerFilter, IssueStateFilter
from lazy_github.lib.github.backends.protocol import BackendType

_CONFIG_FILE_LOCATION = CONFIG_FOLDER / "config.json"


def _serialize_string_list(cls, string_list: str | list[str]) -> list[str]:
    if isinstance(string_list, list):
        return string_list
    elif isinstance(string_list, str):
        return list(set(s.strip() for s in string_list.split(",") if s.strip()))


def _parse_string_list(cls, v) -> list[str]:
    if isinstance(v, str):
        return list(set(s.strip() for s in v.split(",") if s.strip()))
    return v


class AppearanceSettings(BaseModel):
    """Settings focused on altering the appearance of LazyGithub, including hiding or showing different sections."""

    theme: Theme = BUILTIN_THEMES["textual-dark"]
    """Controls the theme used in LazyGithub, which changes the colors of elements in the UI"""

    show_command_log: bool = False
    """Controls if the command log, which records actions taken in LazyGithub, will be displayed on startup"""

    show_workflows: bool = True
    """Controls if information about Github workflows will be displayed on startup"""

    show_issues: bool = True
    """Controls if information about Github issues will be displayed on startup"""

    show_pull_requests: bool = True
    """Controls if information about Github pull requests will be displayed on startup"""

    @field_serializer("theme")
    @classmethod
    def serialize_theme(cls, theme: Theme | str) -> str:
        return theme.name if isinstance(theme, Theme) else theme

    @field_validator("theme", mode="before")
    @classmethod
    def validate_theme(cls, theme_name: Any) -> Theme:
        return BUILTIN_THEMES.get(theme_name, BUILTIN_THEMES["textual-dark"])


class NotificationSettings(BaseModel):
    """Controls the settings for the optional notification feature, which relies on the standard GitHub CLI."""

    enabled: bool = False
    """Controls if LazyGithub should try to load and display notification information"""

    mark_notification_as_read_when_selected: bool = True
    """Controls if LazyGithub will mark a notification as read when selecting an unread notification in the UI"""


class BindingsSettings(BaseModel):
    """Custom keybinding overrides for LazyGithub. When rebinding, pressing ESCAPE will reset to the default binding."""

    overrides: dict[str, str] = {}
    """Overrides for any specific keybindings in LazyGithub"""


class RepositorySettings(BaseModel):
    """Repository-specific settings"""

    @field_serializer("additional_repos_to_track", "favorites")
    @classmethod
    def serialize_string_list(cls, string_list: str | list[str]) -> list[str]:
        return _serialize_string_list(cls, string_list)

    @field_validator("additional_repos_to_track", "favorites", mode="before")
    @classmethod
    def parse_string_list(cls, v) -> list[str]:
        return _parse_string_list(cls, v)

    additional_repos_to_track: list[str] = []
    """Records repositories the user is not an owner of but would like to show in the UI and keep track of"""

    favorites: list[str] = []
    """Records the repositories the user would like pinned at the top of the repositories table"""


class MergeMethod(StrEnum):
    """Different ways that a pull request can be merged by Github"""

    MERGE = "merge"
    SQUASH = "squash"
    REBASE = "rebase"


class PullRequestSettings(BaseModel):
    """Changes how pull requests are retrieved from the Github API"""

    @field_serializer("additional_suggested_pr_reviewers")
    @classmethod
    def serialize_string_list(cls, string_list: str | list[str]) -> list[str]:
        return _serialize_string_list(cls, string_list)

    @field_validator("additional_suggested_pr_reviewers", mode="before")
    @classmethod
    def parse_string_list(cls, v) -> list[str]:
        return _parse_string_list(cls, v)

    state_filter: IssueStateFilter = IssueStateFilter.ALL
    """Controls if we're only listing pull requests in a particular state (ex: Open)"""

    owner_filter: IssueOwnerFilter = IssueOwnerFilter.ALL
    """Controls if we're only listing pull requests owned by the current authenticated user"""

    preferred_merge_method: MergeMethod = MergeMethod.SQUASH
    """How we will request that Github merge pull requests"""

    additional_suggested_pr_reviewers: list[str] = []
    """An list of additional usernames to suggest as reviewers on PRs"""

    preload_pull_request_for_current_commit: bool = False
    """Attempts to identify the github PR associated with your commit git sha and display it on startup"""

    pull_request_template: Path | None = None
    """The path to a file that is used as the default template for new pull requests"""


class IssueSettings(BaseModel):
    """Changes how issues are retrieved from the Github API"""

    state_filter: IssueStateFilter = IssueStateFilter.ALL
    """Controls if we're only listing issues in a particular state (ex: Open)"""

    owner_filter: IssueOwnerFilter = IssueOwnerFilter.ALL
    """Controls if we're only listing issues owned by the current authenticated user"""


class CacheSettings(BaseModel):
    """Settings that control how long data will be cached from the GitHub API"""

    auth_cache_duration: int = int(timedelta(days=1).total_seconds())
    """Controls how long the application will assume that the authentication is valid after successfully checking"""

    auth_last_checked: datetime | None = None
    """Records when the authentication validity was last checked by LazyGithub"""

    cache_directory: Path = CONFIG_FOLDER / ".cache"
    """Controls where LazyGithub will cache request results and table information"""

    default_ttl: int = int(timedelta(minutes=10).total_seconds())
    """Controls the default HTTP cache control duration header that will be sent on all requests"""

    list_repos_ttl: int = int(timedelta(days=1).total_seconds())
    """Controls the HTTP cache control duration header that is sent for requests to list repositories"""

    list_issues_ttl: int = int(timedelta(hours=1).total_seconds())
    """Controls the HTTP cache control duration header that is sent for requests to list issues"""


class CoreConfig(BaseModel):
    first_start: bool = True
    """Records if this is the first time LazyGithub has been started"""

    logfile_path: Path = CONFIG_FOLDER / "lazy_github.log"
    """Controls where the application logs should be stored"""

    logfile_max_bytes: int = 5000000
    """Controls large the application log can grow before being rotated"""

    logfile_count: int = 5
    """Controls how many rotated application logs to keep"""


class ApiConfig(BaseModel):
    """Controlling how the GitHub API is accessed in LazyGithub"""

    base_url: str = "https://api.github.com"
    """Controls which URL we're going to be sending HTTP requests to"""

    client_type: BackendType = BackendType.RAW_HTTP
    """Controls what mechanism we will be using to send API requests to Github"""


_CONFIG_INSTANCE: Optional["Config"] = None


class Config(BaseModel):
    appearance: AppearanceSettings = AppearanceSettings()
    """Controls the appearance of UI elements in LazyGithub"""

    notifications: NotificationSettings = NotificationSettings()
    """Controls interactions with Github notifications in LazyGithub"""

    bindings: BindingsSettings = BindingsSettings()
    """Controls any keybinding overrides in LazyGithub"""

    repositories: RepositorySettings = RepositorySettings()
    """Customizations for the repository functionalities in LazyGithub"""

    pull_requests: PullRequestSettings = PullRequestSettings()
    """Customizations for the pull request functionalities in LazyGithub"""

    issues: IssueSettings = IssueSettings()
    """Customizations for the issue functionalities in LazyGithub"""

    cache: CacheSettings = CacheSettings()
    """Controlling how LazyGithub will cache information"""

    core: CoreConfig = CoreConfig()
    """Customizing shared core behaviors in LazyGithub, such as logging"""

    api: ApiConfig = ApiConfig()
    """Customizing how LazyGithub will interact with the Github APIs"""

    @classmethod
    def load_config(cls) -> "Config":
        global _CONFIG_INSTANCE
        if _CONFIG_INSTANCE is None:
            if _CONFIG_FILE_LOCATION.exists():
                _CONFIG_INSTANCE = cls(**json.loads(_CONFIG_FILE_LOCATION.read_text()))
            else:
                _CONFIG_INSTANCE = cls()
        return _CONFIG_INSTANCE

    def save(self) -> None:
        CONFIG_FOLDER.mkdir(parents=True, exist_ok=True)
        _CONFIG_FILE_LOCATION.write_text(self.model_dump_json(indent=4))

    @classmethod
    @contextmanager
    def to_edit(cls) -> Generator["Config", None, None]:
        current_config = cls.load_config()
        yield current_config
        current_config.save()


if __name__ == "__main__":
    print(f"Config file location: {_CONFIG_FILE_LOCATION} (exists => {_CONFIG_FILE_LOCATION.exists()})")
    print(Config.load_config().model_dump_json(indent=4))
