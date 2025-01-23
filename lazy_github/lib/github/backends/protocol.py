from enum import StrEnum
from typing import Any, Mapping, Protocol

from lazy_github.lib.constants import JSON_CONTENT_ACCEPT_TYPE
from lazy_github.models.github import User

QueryParams = Mapping[str, str | int]
Headers = dict[str, str]


class BackendType(StrEnum):
    RAW_HTTP = "RAW_HTTP"
    GITHUB_CLI = "GITHUB_CLI"


class GithubApiRequestFailed(Exception):
    pass


class GithubApiResponse(Protocol):
    def is_success(self) -> bool: ...
    def json(self) -> Any: ...
    def raise_for_status(self) -> None: ...

    @property
    def text(self) -> str: ...

    @property
    def headers(self) -> Headers: ...


class GithubApiBackend(Protocol):
    async def get(
        self,
        url: str,
        headers: Headers | None = None,
        params: QueryParams | None = None,
    ) -> GithubApiResponse: ...

    async def post(
        self,
        url: str,
        headers: Headers | None = None,
        json: dict[str, Any] | None = None,
    ) -> GithubApiResponse: ...

    async def patch(
        self,
        url: str,
        headers: Headers | None = None,
        json: dict[str, str] | None = None,
    ) -> GithubApiResponse: ...

    async def put(
        self,
        url: str,
        headers: Headers | None = None,
        json: dict[str, str] | None = None,
    ) -> GithubApiResponse: ...

    async def get_user(self) -> User: ...

    def github_headers(self, accept: str = JSON_CONTENT_ACCEPT_TYPE, cache_duration: int | None = None) -> Headers: ...
