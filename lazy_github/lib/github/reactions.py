from typing import Any

from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.models.github import Issue, IssueComment, Reaction, ReactionSet, ReactionType, Repository, User


def _build_reaction_set(response: Any) -> ReactionSet:
    response.raise_for_status()
    users_by_reaction_type: dict[ReactionType, set[User]] = {}

    for raw_reaction in response.json():
        reaction_type = ReactionType.from_github(raw_reaction["content"])
        users_by_reaction_type.setdefault(reaction_type, set())

        user = User(**raw_reaction["user"])
        users_by_reaction_type[reaction_type].add(user)

    return ReactionSet(users_by_reaction_type=users_by_reaction_type)


async def list_reactions_on_comment(
    repo: Repository, comment: IssueComment, per_page: int = 100, page: int = 1
) -> ReactionSet:
    url = f"/repos/{repo.full_name}/issues/comments/{comment.id}/reactions"
    params = {"page": page, "per_page": per_page}
    response = await LazyGithubContext.client.get(url, headers=github_headers(), params=params)
    return _build_reaction_set(response)


async def list_reactions_on_issue(repo: Repository, issue: Issue) -> ReactionSet:
    url = f"/repos/{repo.full_name}/issues/{issue.number}/reactions"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    return _build_reaction_set(response)


async def add_reaction_on_comment(repo: Repository, comment: IssueComment, reaction: ReactionType) -> Reaction:
    """Adds a reaction to an issue comment. Returns the number of reactions created."""
    url = f"/repos/{repo.full_name}/issues/comments/{comment.id}/reactions"
    body = {"content": reaction.name.lower()}
    response = await LazyGithubContext.client.post(url, headers=github_headers(), json=body)
    response.raise_for_status()
    return Reaction(**response.json())


async def add_reaction_on_issue(repo: Repository, issue: Issue, reaction: ReactionType) -> int:
    """Adds a reaction to an issue. Returns the number of reactions created."""
    url = f"/repos/{repo.full_name}/issues/{issue.number}/reactions"
    body = {"content": reaction.github_value}

    response = await LazyGithubContext.client.post(url, json=body, headers=github_headers())
    response.raise_for_status()
    return 1 if response.http_status == 201 else 0
