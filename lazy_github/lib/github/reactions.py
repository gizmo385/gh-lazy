from typing import Any

from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.models.github import Issue, IssueComment, Reaction, ReactionSet, ReactionType, Repository, User


def _build_reaction_set(response: Any) -> ReactionSet:
    response.raise_for_status()
    reaction_users: dict[ReactionType, list[User]] = {}
    reaction_counts: dict[ReactionType, int] = {}

    for raw_reaction in response.json():
        reaction_type = ReactionType.from_github(raw_reaction["content"])
        reaction_users.setdefault(reaction_type, [])
        reaction_counts.setdefault(reaction_type, 0)

        user = User(**raw_reaction["user"])
        reaction_users[reaction_type].append(user)
        reaction_counts[reaction_type] += 1

    return ReactionSet(reaction_users=reaction_users, reaction_counts=reaction_counts)


async def list_reactions_on_comment(repo: Repository, comment: IssueComment) -> ReactionSet:
    url = f"/repos/{repo.full_name}/issues/comments/{comment.id}/reactions"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    return _build_reaction_set(response)


async def add_reaction_on_comment(repo: Repository, comment: IssueComment, reaction: ReactionType) -> Reaction:
    url = f"/repos/{repo.full_name}/issues/comments/{comment.id}/reactions"
    body = {"content": reaction.name.lower()}
    response = await LazyGithubContext.client.post(url, headers=github_headers(), json=body)
    response.raise_for_status()
    return Reaction(**response.json())


async def list_reactions_on_issue(repo: Repository, issue: Issue) -> ReactionSet:
    url = f"/repos/{repo.full_name}/issues/{issue.number}/reactions"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    return _build_reaction_set(response)
