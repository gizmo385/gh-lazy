from logging.handlers import RotatingFileHandler
from typing import Optional

from lazy_github.lib.config import Config
from lazy_github.lib.constants import JSON_CONTENT_ACCEPT_TYPE
from lazy_github.lib.git_cli import current_local_branch_name, current_local_commit, current_local_repo_full_name
from lazy_github.lib.github.backends.protocol import BackendType
from lazy_github.lib.github.client import GithubClient
from lazy_github.lib.logging import LazyGithubLogFormatter, lg
from lazy_github.models.github import Repository


class _LazyGithubContext:
    """Globally accessible wrapper class that centralizes access to the configuration and the Github API client"""

    # Attributes exposed via properties
    _config: Config | None = None
    _client: GithubClient | None = None
    _current_directory_repo: str | None = None
    _current_directory_branch: str | None = None
    _current_commit: str | None = None

    # Directly assigned attributes
    current_repo: Repository | None = None

    @classmethod
    def _setup_logging_handler(cls, config: Config) -> None:
        """Setup the file logger for LazyGithub"""
        try:
            config.core.logfile_path.parent.mkdir(parents=True, exist_ok=True)
            lg_file_handler = RotatingFileHandler(
                filename=config.core.logfile_path,
                maxBytes=config.core.logfile_max_bytes,
                backupCount=config.core.logfile_count,
            )
            lg_file_handler.setFormatter(LazyGithubLogFormatter())
            lg.addHandler(lg_file_handler)
        except Exception:
            lg.exception("Failed to setup file logger for LazyGithub")

    @property
    def config(cls) -> Config:
        if cls._config is None:
            cls._config = Config.load_config()
            cls._setup_logging_handler(cls._config)
        return cls._config

    @property
    def client_type(cls) -> BackendType:
        return cls.config.api.client_type

    @property
    def client(cls) -> GithubClient:
        # Ideally this is would just be a none check but that doesn't properly type check for some reason
        if not isinstance(cls._client, GithubClient):
            match cls.client_type:
                case BackendType.GITHUB_CLI:
                    cls._client = GithubClient.cli(cls.config)
                case BackendType.RAW_HTTP:
                    from lazy_github.lib.github.auth import get_api_token

                    cls._client = GithubClient.hishel(cls.config, get_api_token())
                case _:
                    raise TypeError(f"Invalid client type in config: {cls.client_type}")
        return cls._client

    @property
    def current_directory_repo(cls) -> str | None:
        """The owner/name of the repo associated with the current working directory (if one exists)"""
        if not cls._current_directory_repo:
            cls._current_directory_repo = current_local_repo_full_name()
        return cls._current_directory_repo

    @property
    def current_directory_branch(cls) -> str | None:
        """The owner/name of the repo associated with the current working directory (if one exists)"""
        if not cls._current_directory_branch:
            cls._current_directory_branch = current_local_branch_name()
        return cls._current_directory_branch

    @property
    def current_local_commit(cls) -> str | None:
        """The commit sha of the current local directory repo"""
        if not cls._current_commit:
            cls._current_commit = current_local_commit()
        return cls._current_commit


LazyGithubContext = _LazyGithubContext()


def github_headers(accept: str = JSON_CONTENT_ACCEPT_TYPE, cache_duration: Optional[int] = None) -> dict[str, str]:
    """Helper function to build headers for Github API requests"""
    return LazyGithubContext.client.github_headers(accept, cache_duration)
