from lazy_github.lib.context import LazyGithubContext
from lazy_github.models.github import User


async def get_user_by_username(username: str) -> User | None:
    response = await LazyGithubContext.client.get(f"/users/{username}")
    return User(**response.json()) if response.is_success() else None
