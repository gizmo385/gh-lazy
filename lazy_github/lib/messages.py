from functools import cached_property

from textual.message import Message

from lazy_github.models.github import Branch, FullPullRequest, Issue, IssueComment, PartialPullRequest, Repository


class RepoSelected(Message):
    """
    A message indicating that a particular user repo has been selected.

    This message is used to trigger follow-up contextual actions based on the selected repo, such as loading pull
    requests, issues, actions, etc.
    """

    def __init__(self, repo: Repository) -> None:
        self.repo = repo
        super().__init__()


class PullRequestSelected(Message):
    """
    A message indicating that the user is looking for additional information on a particular pull request.
    """

    def __init__(self, pr: PartialPullRequest) -> None:
        self.pr = pr
        super().__init__()


class IssueSelected(Message):
    """
    A message indicating that the user is looking for additional information on a particular issue.
    """

    def __init__(self, issue: Issue) -> None:
        self.issue = issue
        super().__init__()


class IssuesAndPullRequestsFetched(Message):
    """
    Since issues and pull requests are both represented on the Github API as issues, we want to pull issues once and
    then send that message to both sections of the UI.
    """

    def __init__(self, issues_and_pull_requests: list[Issue]) -> None:
        self.issues_and_pull_requests = issues_and_pull_requests
        super().__init__()

    @cached_property
    def pull_requests(self) -> list[PartialPullRequest]:
        return [pr for pr in self.issues_and_pull_requests if isinstance(pr, PartialPullRequest)]

    @cached_property
    def issues(self) -> list[Issue]:
        return [
            issue
            for issue in self.issues_and_pull_requests
            if isinstance(issue, Issue) and not isinstance(issue, PartialPullRequest)
        ]


class PullRequestCreated(Message):
    def __init__(self, pull_request: FullPullRequest) -> None:
        super().__init__()
        self.pull_request = pull_request


class IssueCreated(Message):
    def __init__(self, issue: Issue) -> None:
        super().__init__()
        self.issue = issue


class NewCommentCreated(Message):
    def __init__(self, comment: IssueComment) -> None:
        super().__init__()
        self.comment = comment


class SettingsModalDismissed(Message):
    def __init__(self, changed: bool) -> None:
        super().__init__()
        self.changed = changed


class BranchesLoaded(Message):
    def __init__(self, branches: list[Branch]) -> None:
        super().__init__()
        self.branches = branches
