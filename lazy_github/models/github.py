from datetime import datetime
from enum import StrEnum

from pydantic import BaseModel, Field


class User(BaseModel):
    login: str
    id: int
    name: str | None = None
    html_url: str


class RepositoryPermission(BaseModel):
    admin: bool
    maintain: bool
    push: bool
    triage: bool
    pull: bool


class Repository(BaseModel):
    name: str
    full_name: str
    default_branch: str | None = None
    private: bool
    archived: bool | None = None
    owner: User
    description: str | None = None
    permissions: RepositoryPermission | None = None


class IssueState(StrEnum):
    OPEN = "open"
    CLOSED = "closed"


class Issue(BaseModel):
    id: int
    number: int
    locked: bool
    state: IssueState
    title: str
    body: str | None = None
    user: User
    created_at: datetime
    updated_at: datetime
    closed_at: datetime | None = None
    closed_by: User | None = None
    assignee: User | None = None
    assignees: list[User] | None
    comments_url: str
    html_url: str

    # This field isn't actually returned from the API, but we will pass it in manually. It's useful for follow-up
    # requests that require access to the original repo
    repo: Repository


class Ref(BaseModel):
    user: User
    ref: str
    sha: str


class PartialPullRequest(Issue):
    """
    A pull request that may be included in the response to a list issues API call and is missing some information
    """

    draft: bool


class FullPullRequest(PartialPullRequest):
    """More comprehensive details on a pull request from the API"""

    additions: int
    deletions: int
    changed_files: int
    commits: int
    head: Ref
    base: Ref
    merged_at: datetime | None
    diff_url: str


class PullRequestMergeResult(BaseModel):
    sha: str
    merged: bool
    message: str


class AuthorAssociation(StrEnum):
    COLLABORATOR = "COLLABORATOR"
    CONTRIBUTOR = "CONTRIBUTOR"
    FIRST_TIMER = "FIRST_TIMER"
    FIRST_TIME_CONTRIBUTOR = "FIRST_TIME_CONTRIBUTOR"
    MANNEQUIN = "MANNEQUIN"
    MEMBER = "MEMBER"
    NONE = "NONE"
    OWNER = "OWNER"


class IssueComment(BaseModel):
    id: int
    body: str
    user: User | None
    created_at: datetime
    updated_at: datetime
    author_association: AuthorAssociation


class ReviewState(StrEnum):
    APPROVED = "APPROVED"
    CHANGES_REQUESTED = "CHANGES_REQUESTED"
    COMMENTED = "COMMENTED"
    DISMISSED = "DISMISSED"
    PENDING = "PENDING"


class ReviewComment(IssueComment):
    pull_request_review_id: int
    path: str
    url: str
    position: int | None
    original_position: int | None
    in_reply_to_id: int | None = None


class Review(BaseModel):
    id: int
    user: User
    body: str
    state: ReviewState
    comments: list[ReviewComment] = []
    submitted_at: datetime | None = None


class Commit(BaseModel):
    sha: str
    url: str


class Branch(BaseModel):
    name: str
    commit: Commit
    protected: bool


class WorkflowState(StrEnum):
    ACTIVE = "active"
    DELETED = "deleted"
    DISABLED_FORK = "disabled_fork"
    DISABLED_INACTIVITY = "disabled_inactivity"
    DISABLED_MANUALLY = "disabled_manually"


class Workflow(BaseModel):
    id: int
    name: str
    state: WorkflowState
    path: str
    created_at: datetime
    updated_at: datetime
    url: str


class WorkflowRun(BaseModel):
    name: str
    display_title: str
    path: str
    run_number: int
    head_branch: str
    status: str
    conclusion: str | None = None
    event: str
    actor: User
    triggering_actor: User
    repository: Repository
    created_at: datetime
    updated_at: datetime


class NotificationSubject(BaseModel):
    title: str
    url: str | None
    latest_comment_url: str | None
    subject_type: str = Field(alias="type")


class Notification(BaseModel):
    id: int
    repository: Repository
    subject: NotificationSubject
    reason: str
    unread: bool
    updated_at: datetime
    last_read_at: datetime | None


class CheckStatusState(StrEnum):
    SUCCESS = "success"
    PENDING = "pending"
    ERROR = "error"
    FAILURE = "failure"


class CheckStatus(BaseModel):
    description: str
    context: str
    state: CheckStatusState
    target_url: str | None
    updated_at: datetime | None
    created_at: datetime | None


class CombinedCheckStatus(BaseModel):
    state: CheckStatusState
    statuses: list[CheckStatus]
