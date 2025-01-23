from typing import Any

import hishel
from httpx import HTTPStatusError, Response

from lazy_github.lib.config import Config
from lazy_github.lib.constants import JSON_CONTENT_ACCEPT_TYPE
from lazy_github.lib.github.backends.protocol import (
    GithubApiBackend,
    GithubApiRequestFailed,
    GithubApiResponse,
    Headers,
    QueryParams,
)
from lazy_github.models.github import User


class HishelApiResponse(GithubApiResponse):
    def __init__(self, api_response: Response) -> None:
        self.api_response = api_response

    def raise_for_status(self) -> None:
        try:
            self.api_response.raise_for_status()
        except HTTPStatusError as e:
            raise GithubApiRequestFailed(e)

    def is_success(self) -> bool:
        return self.api_response.is_success

    def json(self) -> Any:
        return self.api_response.json()

    @property
    def text(self) -> str:
        return self.api_response.text

    @property
    def headers(self) -> dict[str, str]:
        return dict(self.api_response.headers)


class HishelGithubApiBackend(GithubApiBackend):
    def __init__(self, config: Config, access_token: str) -> None:
        self.config = config
        self.access_token = access_token

        storage = hishel.AsyncFileStorage(base_path=config.cache.cache_directory)
        self.api_client = hishel.AsyncCacheClient(storage=storage, base_url=config.api.base_url)

    async def get(
        self,
        url: str,
        headers: Headers | None = None,
        params: QueryParams | None = None,
    ) -> HishelApiResponse:
        response = await self.api_client.get(url, headers=headers, params=params, follow_redirects=True)
        return HishelApiResponse(response)

    async def post(
        self,
        url: str,
        headers: Headers | None = None,
        json: dict[str, Any] | None = None,
    ) -> HishelApiResponse:
        response = await self.api_client.post(url, headers=headers, json=json)
        return HishelApiResponse(response)

    async def patch(
        self,
        url: str,
        headers: Headers | None = None,
        json: dict[str, str] | None = None,
    ) -> HishelApiResponse:
        response = await self.api_client.patch(url, headers=headers, json=json)
        return HishelApiResponse(response)

    async def put(
        self,
        url: str,
        headers: Headers | None = None,
        json: dict[str, str] | None = None,
    ) -> HishelApiResponse:
        response = await self.api_client.put(url, headers=headers, json=json)
        return HishelApiResponse(response)

    def github_headers(self, accept: str = JSON_CONTENT_ACCEPT_TYPE, cache_duration: int | None = None) -> Headers:
        max_age = cache_duration or self.config.cache.default_ttl
        return {
            "Accept": accept,
            "Authorization": f"Bearer {self.access_token}",
            "Cache-Control": f"max-age={max_age}",
        }

    async def get_user(self) -> User:
        """Returns the authed user for this client"""
        response = await self.api_client.get("/user", headers=self.github_headers())
        return User(**response.json())
