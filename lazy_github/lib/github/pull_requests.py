from lazy_github.lib.config import MergeMethod
from lazy_github.lib.constants import DIFF_CONTENT_ACCEPT_TYPE
from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.lib.github.backends.cli import run_gh_cli_command
from lazy_github.lib.github.backends.protocol import BackendType, GithubApiRequestFailed
from lazy_github.lib.github.issues import list_issues
from lazy_github.lib.logging import lg
from lazy_github.models.github import (
    FullPullRequest,
    Issue,
    PartialPullRequest,
    PullRequestMergeResult,
    Repository,
    Review,
    ReviewComment,
    ReviewState,
)


async def list_for_repo(repo: Repository) -> list[PartialPullRequest]:
    """Lists the pull requests associated with the specified repo"""
    state_filter = LazyGithubContext.config.pull_requests.state_filter
    owner_filter = LazyGithubContext.config.pull_requests.owner_filter
    issues = await list_issues(repo, state_filter, owner_filter)
    return [i for i in issues if isinstance(i, PartialPullRequest)]


async def create_pull_request(
    repo: Repository, title: str, body: str, base_ref: str, head_ref: str, draft: bool = False
) -> FullPullRequest:
    url = f"/repos/{repo.owner.login}/{repo.name}/pulls"
    request_body = {
        "title": title,
        "draft": draft,
        "base": base_ref,
        # TODO: This prevents it from working with forks, but means it'll work for same-repo PRs. Issue
        "head": f"{repo.owner.login}:{head_ref}",
    }
    if body:
        request_body["body"] = body
    response = await LazyGithubContext.client.post(url, headers=github_headers(), json=request_body)
    response.raise_for_status()
    return FullPullRequest(**response.json(), repo=repo)


async def update_pull_request(
    repo: Repository, existing_pr: FullPullRequest, updated_title: str, updated_body: str
) -> FullPullRequest:
    """Update the title or body of an existing PR"""
    if existing_pr.title == updated_title and existing_pr.body == updated_body:
        return existing_pr
    body = {"title": updated_title, "body": updated_body, "base": existing_pr.base.ref}
    url = f"/repos/{repo.full_name}/pulls/{existing_pr.number}"
    response = await LazyGithubContext.client.patch(url, json=body)
    response.raise_for_status()
    existing_pr.title = updated_title
    existing_pr.body = updated_body
    return existing_pr


async def get_full_pull_request(repo: Repository, pr_number: int) -> FullPullRequest:
    """Converts a partial pull request into a full pull request"""
    url = f"/repos/{repo.owner.login}/{repo.name}/pulls/{pr_number}"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    response.raise_for_status()
    return FullPullRequest(**response.json(), repo=repo)


async def get_diff(pr: FullPullRequest) -> str:
    """Fetches the raw diff for an individual pull request"""
    match LazyGithubContext.client_type:
        case BackendType.GITHUB_CLI:
            response = await run_gh_cli_command(["pr", "diff", "-R", pr.repo.full_name, str(pr.number)])
        case BackendType.RAW_HTTP:
            headers = github_headers(DIFF_CONTENT_ACCEPT_TYPE)
            response = await LazyGithubContext.client.get(pr.diff_url, headers=headers)
        case _:
            raise TypeError("Unexpected github client: How did you even get here")

    response.raise_for_status()
    return response.text


async def merge_pull_request(pr: FullPullRequest, merge_method: MergeMethod) -> PullRequestMergeResult:
    """
    Attempts to merge the PR via the Github API. The head sha of the PR must match for the merge to be successful.
    """
    url = f"/repos/{pr.repo.owner.login}/{pr.repo.name}/pulls/{pr.number}/merge"
    body = {"merge_method": merge_method, "sha": pr.head.sha}
    response = await LazyGithubContext.client.put(url, headers=github_headers(), json=body)
    response.raise_for_status()
    return PullRequestMergeResult(**response.json())


async def get_review_comments(pr: FullPullRequest, review: Review) -> list[ReviewComment]:
    url = f"/repos/{pr.repo.owner.login}/{pr.repo.name}/pulls/{pr.number}/reviews/{review.id}/comments"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    response.raise_for_status()
    return [ReviewComment(**c) for c in response.json()]


async def get_reviews(pr: FullPullRequest, with_comments: bool = True, with_pending: bool = False) -> list[Review]:
    url = url = f"/repos/{pr.repo.owner.login}/{pr.repo.name}/pulls/{pr.number}/reviews"
    response = await LazyGithubContext.client.get(url, headers=github_headers())
    response.raise_for_status()
    reviews: list[Review] = []
    for raw_review in response.json():
        review = Review(**raw_review)
        if review.state == ReviewState.PENDING and not with_pending:
            continue
        if with_comments:
            review.comments = await get_review_comments(pr, review)
        reviews.append(review)
    return reviews


async def reply_to_review_comment(
    repo: Repository, issue: Issue, comment: ReviewComment, comment_body: str
) -> ReviewComment:
    url = f"/repos/{repo.owner.login}/{repo.name}/pulls/{issue.number}/comments/{comment.id}/replies"
    response = await LazyGithubContext.client.post(url, headers=github_headers(), json={"body": comment_body})
    response.raise_for_status()
    return ReviewComment(**response.json())


class ReviewCommentNode:
    def __init__(self, comment: ReviewComment) -> None:
        self.children: list["ReviewCommentNode"] = []
        self.comment = comment


def reconstruct_review_conversation_hierarchy(reviews: list[Review]) -> dict[int, ReviewCommentNode]:
    """
    Given a list of PR reviews, this rebuilds a the comment hierarchy as a tree of connected comment nodes. The return
    value of this function is a mapping between the comment IDs and the associated ReviewCommentNode for the top level
    comments ONLY. Any subsequent comments will be included as children in one of the review comment nodes.

    An important disclaimer is that this function does NOT take into account the body associated with the review itself,
    which is present in some reviews. When generating UI from this function, the body of review itself should be
    included prior to printing the review comments themselves.

    Given a variable `hierarchy` generated from a list `reviews` of PR reviews, the output of this can be properly
    unpacked like so:
    ```python
    for review in reviews:
        if review.body:
            # Output the root review body
            print(review.body)

            # Output the review comments that are top level (i.e. their ids are in the hierarchy map)
            for comment in review.comments:
                if comment.id in hierarchy:
                    # Call
                    comment_review_node_handler(hierarchy[comment.id])
    ```
    """
    comment_nodes_by_review_id: dict[int, ReviewCommentNode] = {}
    # Create review nodes for all of the comments in each of the reviews
    for review in reviews:
        for comment in review.comments:
            comment_nodes_by_review_id[comment.id] = ReviewCommentNode(comment)

    # Build a tree that represents the conversational flow between individual comments in the threads
    for review_node in comment_nodes_by_review_id.values():
        in_reply_to_id = review_node.comment.in_reply_to_id
        if in_reply_to_id is not None and in_reply_to_id in comment_nodes_by_review_id:
            comment_nodes_by_review_id[in_reply_to_id].children.append(review_node)

    return {r.comment.id: r for r in comment_nodes_by_review_id.values() if r.comment.in_reply_to_id is None}


async def request_reviews(pr: FullPullRequest, reviewers: list[str]) -> bool:
    """Requests reviews from the specified logins on a PR"""
    body = {"reviewers": reviewers}
    url = f"/repos/{pr.repo.full_name}/pulls/{pr.number}/requested_reviewers"
    response = await LazyGithubContext.client.post(url, json=body)
    try:
        response.raise_for_status()
        return True
    except GithubApiRequestFailed:
        lg.exception("Error creating review requests")
        return False


async def list_requested_reviewers(pr: FullPullRequest) -> list[str]:
    """Lists existing review requests on the specified pull request"""
    response = await LazyGithubContext.client.get(f"/repos/{pr.repo.full_name}/pulls/{pr.number}/requested_reviewers")
    try:
        response.raise_for_status()
        response_body = response.json()
        return [u["login"] for u in response_body["users"]]
    except GithubApiRequestFailed:
        lg.exception("Error creating review requests")
        return []


async def list_pull_requests_for_commit(commit_sha: str) -> list[PartialPullRequest]:
    """Given a commit sha, returns a list of pull requests associated with that commit sha"""
    if not LazyGithubContext.current_repo:
        return []
    repo_full_name = LazyGithubContext.current_repo.full_name
    response = await LazyGithubContext.client.get(f"/repos/{repo_full_name}/commits/{commit_sha}/pulls")
    try:
        response.raise_for_status()
        return [PartialPullRequest(**p, repo=LazyGithubContext.current_repo) for p in response.json()]
    except GithubApiRequestFailed:
        lg.exception("Error loading PRs for current commit")
        return []


async def create_new_review(
    pull_request: FullPullRequest,
    event: ReviewState,
    body: str,
    comments: list[dict[str, str | int]] | None = None,
) -> Review | None:
    """Creates a new Github review for the provided pull request"""
    match event:
        case ReviewState.APPROVED:
            review_event = "APPROVE"
        case ReviewState.COMMENTED:
            review_event = "COMMENT"
        case ReviewState.CHANGES_REQUESTED:
            review_event = "REQUEST_CHANGES"
        case ReviewState.DISMISSED | ReviewState.PENDING:
            lg.error("Cannot create a review with state DISMISSED")
            return None

    review_request = {
        "event": review_event,
        "commit_id": pull_request.head.sha,
        "body": body,
        "comments": comments,
    }
    repo_full_name = pull_request.repo.full_name
    response = await LazyGithubContext.client.post(
        f"/repos/{repo_full_name}/pulls/{pull_request.number}/reviews", json=review_request
    )
    try:
        response.raise_for_status()
        response_body = response.json()
        return Review(**response_body)
    except GithubApiRequestFailed:
        lg.exception("Error loading PRs for current commit")
        return None
