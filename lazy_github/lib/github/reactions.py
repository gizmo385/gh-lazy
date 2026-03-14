from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.models.github import Issue, IssueComment, ReactionSet, ReactionType, Repository, User


async def reactions_on_comment(repo: Repository, comment: IssueComment) -> dict[ReactionType, int]:
    return {}


async def add_reaction_on_comment(repo: Repository, comment: IssueComment, reaction: ReactionType) -> bool:
    return True


async def list_reactions_on_issue(repo: Repository, issue: Issue) -> ReactionSet:
    url = f"/repos/{repo.owner.login}/{repo.name}/issues/{issue.number}/reactions"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    response.raise_for_status()
    reaction_users: dict[ReactionType, list[User]] = {}
    reaction_counts: dict[ReactionType, int] = {}

    for raw_reaction in response.json():
        reaction_type = ReactionType[raw_reaction["content"].upper()]
        reaction_users.setdefault(reaction_type, [])
        reaction_counts.setdefault(reaction_type, 0)

        user = User(**raw_reaction["user"])
        reaction_users[reaction_type].append(user)
        reaction_counts[reaction_type] += 1

    return ReactionSet(reaction_users=reaction_users, reaction_counts=reaction_counts)
