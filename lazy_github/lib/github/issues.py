from typing import TypedDict, Unpack, cast

from lazy_github.lib.constants import IssueOwnerFilter, IssueStateFilter
from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.models.github import Issue, IssueComment, PartialPullRequest, Repository

DEFAULT_PAGE_SIZE = 30


async def get_issue_by_number(repo: Repository, issue_number: int) -> Issue:
    """Looks up a single issue by number in the given repository"""
    url = f"/repos/{repo.owner.login}/{repo.name}/issues/{issue_number}"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    response.raise_for_status()
    return Issue(**response.json(), repo=repo)


class UpdateIssuePayload(TypedDict):
    title: str | None
    body: str | None
    state: str | None


async def list_issues(
    repo: Repository, state: IssueStateFilter, owner: IssueOwnerFilter, page: int = 1, per_page: int = DEFAULT_PAGE_SIZE
) -> list[Issue]:
    """Fetch issues (included pull requests) from the repo matching the state/owner filters"""
    query_params = {"state": str(state).lower(), "page": page, "per_page": per_page}
    if owner == IssueOwnerFilter.MINE:
        user = await LazyGithubContext.client.user()
        query_params["creator"] = user.login

    response = await LazyGithubContext.client.get(
        f"/repos/{repo.owner.login}/{repo.name}/issues",
        headers=github_headers(),
        params=query_params,
    )
    response.raise_for_status()
    result: list[Issue] = []
    for issue in response.json():
        if "draft" in issue:
            result.append(PartialPullRequest(**issue, repo=repo))
        else:
            result.append(Issue(**issue, repo=repo))
    return result


async def get_comments(issue: Issue) -> list[IssueComment]:
    response = await LazyGithubContext.client.get(issue.comments_url, headers=github_headers())
    response.raise_for_status()
    return [IssueComment(**i) for i in response.json()]


async def create_comment(issue: Issue, comment_body: str) -> IssueComment:
    url = f"/repos/{issue.repo.owner.login}/{issue.repo.name}/issues/{issue.number}/comments"
    body = {"body": comment_body}
    response = await LazyGithubContext.client.post(url, json=body, headers=github_headers())
    response.raise_for_status()
    return IssueComment(**response.json())


async def update_issue(issue_to_update: Issue, **updated_fields: Unpack[UpdateIssuePayload]) -> Issue:
    repo = issue_to_update.repo
    url = f"/repos/{repo.owner.login}/{repo.name}/issues/{issue_to_update.number}"
    response = await LazyGithubContext.client.patch(
        url,
        json=cast(dict[str, str], updated_fields),
        headers=github_headers(),
    )
    response.raise_for_status()
    return Issue(**response.json(), repo=repo)


async def create_issue(repo: Repository, title: str, body: str) -> Issue:
    url = f"/repos/{repo.owner.login}/{repo.name}/issues"
    json_body = {"title": title, "body": body}
    response = await LazyGithubContext.client.post(url, json=json_body, headers=github_headers())
    response.raise_for_status()
    return Issue(**response.json(), repo=repo)
