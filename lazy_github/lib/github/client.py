from typing import Any

from lazy_github.lib.config import Config
from lazy_github.lib.constants import JSON_CONTENT_ACCEPT_TYPE
from lazy_github.lib.github.backends.cli import GithubCliBackend
from lazy_github.lib.github.backends.hishel import HishelGithubApiBackend
from lazy_github.lib.github.backends.protocol import GithubApiBackend, Headers, QueryParams
from lazy_github.models.github import User


class GithubClient(GithubApiBackend):
    def __init__(self, config: Config, backend: GithubApiBackend) -> None:
        self.config = config
        self.backend = backend
        self._user: User | None = None

    @classmethod
    def cli(cls, config: Config) -> "GithubClient":
        backend = GithubCliBackend(config)
        return GithubClient(config, backend)

    @classmethod
    def hishel(cls, config: Config, access_token: str) -> "GithubClient":
        backend = HishelGithubApiBackend(config, access_token)
        return GithubClient(config, backend)

    async def user(self) -> User:
        """Returns the authed user for this client"""
        if self._user is None:
            self._user = await self.get_user()
        return self._user

    def github_headers(self, accept: str = JSON_CONTENT_ACCEPT_TYPE, cache_duration: int | None = None) -> Headers:
        """Helper function to build a request with specific headers"""
        return self.backend.github_headers(accept=accept, cache_duration=cache_duration)

    async def get(self, url: str, headers: Headers | None = None, params: QueryParams | None = None) -> Any:
        return await self.backend.get(url, headers, params)

    async def post(self, url: str, headers: Headers | None = None, json: dict[str, Any] | None = None) -> Any:
        return await self.backend.post(url, headers, json)

    async def patch(self, url: str, headers: Headers | None = None, json: dict[str, str] | None = None) -> Any:
        return await self.backend.patch(url, headers, json)

    async def put(self, url: str, headers: Headers | None = None, json: dict[str, str] | None = None) -> Any:
        return await self.backend.put(url, headers, json)

    async def get_user(self) -> User:
        return await self.backend.get_user()
