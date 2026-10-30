//! Side effects as data. `App::update` returns them and only the [`Runtime`](crate::runtime::Runtime) performs them.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::git::GitCli;
use lazy_github_core::github::{
    DiffFetchError, GhCliOptions, Github, IssueListQuery, IssueUpdate, MergeRequest, MergeResult, NewIssue,
    NewPullRequest, NewReview, NotificationQuery, PageQuery, PullRequestUpdate, RepoListQuery,
};
use lazy_github_core::models::{
    Branch, CheckRun, CombinedStatus, CommentRef, FullPullRequest, FullUser, Issue, IssueComment, IssueListItem,
    Notification, ReactionKind, Reactions, RepoName, Repository, Review, ReviewComment, User, Workflow, WorkflowJob,
    WorkflowRun,
};

/// Something the runtime does on the app's behalf.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Run `request` in the background. The result comes back as `Event::Github` with the same `id`.
    Github {
        /// Which request this is, so a stale response can be told apart from the one being waited for.
        id: RequestId,
        /// What to ask GitHub.
        request: GithubRequest,
    },
    /// Run `request` in the user's working directory, in the background. The result comes back as `Event::Git` with
    /// the same `id`.
    Git {
        /// Which request this is, from the same counter as GitHub requests.
        id: RequestId,
        /// What to ask git.
        request: GitRequest,
    },
    /// Abort the GitHub or git request with this id, killing its `gh` or `git` child. Its response never arrives. A
    /// no-op once it finished.
    Cancel(RequestId),
    /// Save this snapshot of the config. Saves happen in order, and a newer pending save replaces an older one.
    SaveConfig(Box<Config>),
    /// Run every later GitHub request with these options ([`Github::reconfigure`]): Settings saved new cache TTLs
    /// or a new `api.base_url`. Requests already running keep the old ones.
    ReconfigureGithub(GhCliOptions),
    /// Read a table cache file (`lazy_github_core::cache::read`) off the async threads. The text comes back as
    /// `Event::CacheRead` with the same `id`, `None` when there's no file.
    ReadCache {
        /// Which read this is, from the same counter as GitHub requests.
        id: RequestId,
        /// The file.
        path: PathBuf,
    },
    /// Write a table cache file (`lazy_github_core::cache::write`), creating its directory. Writes to one path happen
    /// in order, and a newer pending write replaces an older one. A failed write is logged as a warning.
    WriteCache {
        /// The file.
        path: PathBuf,
        /// Its new text, from `lazy_github_core::cache::to_json` (or `drafts::to_json` for a PR draft).
        text: String,
    },
    /// Delete a cache file, on the same writer as [`Effect::WriteCache`], so a write and a delete of one path happen
    /// in the order issued and the newest pending one wins. A missing file is fine; a failed delete is logged as a
    /// warning.
    RemoveCache {
        /// The file.
        path: PathBuf,
    },
    /// Send `Event::Timer(id)` every `every`, first at `+every` (Textual's `set_interval`). A no-op if `id` is
    /// already running.
    StartInterval {
        /// The timer.
        id: TimerId,
        /// The period, which is also the delay before the first tick.
        every: Duration,
    },
    /// Send `Event::Timer(id)` once, after `after`. Replaces a running timer with the same id.
    ///
    /// A tick the replaced timer already sent may still be queued and arrive after this.
    StartTimeout {
        /// The timer.
        id: TimerId,
        /// The delay.
        after: Duration,
    },
    /// Stop the timer with this id. A no-op if it isn't running.
    ///
    /// A tick already sent may still be queued and arrive after this; owners that care check their own state.
    CancelTimer(TimerId),
    /// Ring the terminal bell (Textual's `App.bell`). Terminals may beep, flash, or do nothing.
    Bell,
    /// Ask the terminal to put this text on the system clipboard with OSC 52, as Textual's `copy_to_clipboard` does.
    /// Terminals without OSC 52 ignore it.
    SetClipboard(String),
    /// Stop the event loop after the current batch of events.
    Quit,
}

/// Identifies one GitHub request. Allocated by `App`, increasing monotonically, so ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(u64);

impl RequestId {
    pub(crate) fn new(n: u64) -> Self {
        Self(n)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Identifies a timer. Each id has one owner, which `App` routes `Event::Timer` to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimerId {
    /// Re-checking `gh auth status` while the authentication screen is up (#106).
    AuthPoll,
    /// Polling for notifications (#81).
    Notifications,
    /// Create PR's draft autosave, 2 s after the last edit (#186). Owned by the open Create PR dialog.
    PrDraftSave,
    /// The command palette's "No matches found", 0.5 s after a search that found nothing (#300). Owned by the open
    /// palette.
    PaletteNoMatches,
}

/// One [`Github`] call, as data. One variant per trait method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubRequest {
    /// [`Github::check_auth`].
    CheckAuth,
    /// [`Github::gh_version`].
    GhVersion,
    /// [`Github::current_user`].
    CurrentUser,
    /// [`Github::list_repositories`].
    ListRepositories(RepoListQuery),
    /// [`Github::list_issues`].
    ListIssues {
        /// The repo whose issues and pull requests to list.
        repo: Box<Repository>,
        /// The filters and page.
        query: IssueListQuery,
    },
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount,
    /// [`Github::list_notifications`].
    ListNotifications(NotificationQuery),
    /// [`Github::mark_notification_read`] of the thread with this id.
    MarkNotificationRead(u64),
    /// [`Github::mark_all_notifications_read`].
    MarkAllNotificationsRead,
    /// [`Github::repo_readme`] of the repo with this full name.
    RepoReadme(String),
    /// [`Github::repository`] with this full name.
    Repository(String),
    /// [`Github::list_workflows`].
    ListWorkflows {
        /// The repo whose workflows to list.
        repo: Box<Repository>,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::list_workflow_runs`].
    ListWorkflowRuns {
        /// The repo whose runs to list.
        repo: Box<Repository>,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::pull_request_diff`].
    PullRequestDiff {
        /// The pull request's repo.
        repo: Box<Repository>,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::user`] with this login.
    User(String),
    /// [`Github::user_repos`] with this login.
    UserRepos(String),
    /// [`Github::pull_request`].
    PullRequest {
        /// The repo the pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
    },
    /// [`Github::pull_request_uncached`]. Answered as [`GithubResponse::PullRequest`].
    PullRequestUncached {
        /// The repo the pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
    },
    /// [`Github::workflow_run_jobs`].
    WorkflowRunJobs {
        /// The repo the run is in.
        repo: Box<Repository>,
        /// The run's id.
        run_id: u64,
    },
    /// [`Github::commit_status`].
    CommitStatus {
        /// The commit's repo.
        repo: Box<Repository>,
        /// The commit.
        sha: String,
    },
    /// [`Github::check_runs`].
    CheckRuns {
        /// The commit's repo.
        repo: Box<Repository>,
        /// The commit.
        sha: String,
    },
    /// [`Github::issue_reactions`].
    IssueReactions {
        /// The issue's or pull request's repo.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
    },
    /// [`Github::issue`].
    Issue {
        /// The repo the issue is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
    },
    /// [`Github::issue_comments`].
    IssueComments {
        /// The issue or pull request whose comments are listed.
        issue: Box<Issue>,
    },
    /// [`Github::comment_reactions`].
    CommentReactions {
        /// The comment's repo.
        repo: Box<Repository>,
        /// The comment.
        comment: CommentRef,
    },
    /// [`Github::job_logs`].
    JobLogs {
        /// The repo the job's run is in.
        repo: Box<Repository>,
        /// The job's id.
        job_id: u64,
    },
    /// [`Github::pull_request_reviews`].
    PullRequestReviews {
        /// The pull request whose reviews are listed.
        pr: Box<FullPullRequest>,
    },
    /// [`Github::create_issue`].
    CreateIssue {
        /// The repo the issue opens in.
        repo: Box<Repository>,
        /// Its title and body.
        issue: NewIssue,
    },
    /// [`Github::create_issue_comment`].
    CreateIssueComment {
        /// The repo the issue or pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
        /// The comment's text.
        body: String,
    },
    /// [`Github::reply_to_review_comment`].
    ReplyToReviewComment {
        /// The pull request's repo.
        repo: Box<Repository>,
        /// The pull request's number.
        pr_number: u64,
        /// The review comment replied to.
        comment_id: u64,
        /// The reply's text.
        body: String,
    },
    /// [`Github::create_review`].
    CreateReview {
        /// The pull request's repo.
        repo: Box<Repository>,
        /// The pull request's number.
        number: u64,
        /// The verdict, the commit, the summary and the comments.
        review: NewReview,
    },
    /// [`Github::add_issue_reaction`].
    AddIssueReaction {
        /// The repo the issue or pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
        /// The reaction to add.
        kind: ReactionKind,
    },
    /// [`Github::merge_pull_request`].
    MergePullRequest {
        /// The repo the pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
        /// The method and the head commit.
        request: MergeRequest,
    },
    /// [`Github::list_branches`].
    ListBranches {
        /// The repo whose branches to list.
        repo: Box<Repository>,
    },
    /// [`Github::list_branches_uncached`]. Answered as [`GithubResponse::ListBranches`].
    ListBranchesUncached {
        /// The repo whose branches to list.
        repo: Box<Repository>,
    },
    /// [`Github::create_pull_request`].
    CreatePullRequest {
        /// The repo to open it in.
        repo: Box<Repository>,
        /// The title, draft flag, base, head and description.
        pr: NewPullRequest,
    },
    /// [`Github::dispatch_workflow`].
    DispatchWorkflow {
        /// The repo the workflow is in.
        repo: Box<Repository>,
        /// The workflow's id.
        workflow_id: u64,
        /// The branch to run it on.
        git_ref: String,
    },
    /// [`Github::rerun_workflow_run`], or [`Github::rerun_failed_jobs`] with `failed_only`.
    RerunWorkflow {
        /// The repo the run is in.
        repo: Box<Repository>,
        /// The run's id.
        run_id: u64,
        /// Whether only the failed jobs re-run.
        failed_only: bool,
    },
    /// [`Github::workflow_run`].
    WorkflowRun {
        /// The repo the run is in.
        repo: Box<Repository>,
        /// The run's id.
        run_id: u64,
    },
    /// [`Github::collaborators`], of this repo.
    Collaborators(Box<Repository>),
    /// [`Github::requested_reviewers`].
    RequestedReviewers {
        /// The repo the pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
    },
    /// [`Github::request_reviewers`].
    RequestReviewers {
        /// The repo the pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
        /// The logins asked to review.
        reviewers: Vec<String>,
    },
    /// [`Github::update_pull_request`].
    UpdatePullRequest {
        /// The repo the pull request is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
        /// The new title and body, and its base.
        update: PullRequestUpdate,
    },
    /// [`Github::update_issue`].
    UpdateIssue {
        /// The repo the issue is in.
        repo: Box<Repository>,
        /// Its number.
        number: u64,
        /// The new title, body and state.
        update: IssueUpdate,
    },
}

/// The result of a [`GithubRequest`], in the variant of the same name.
#[derive(Debug)]
pub enum GithubResponse {
    /// [`Github::check_auth`].
    CheckAuth(Result<(), GhError>),
    /// [`Github::gh_version`].
    GhVersion(Result<String, GhError>),
    /// [`Github::current_user`].
    CurrentUser(Result<User, GhError>),
    /// [`Github::list_repositories`].
    ListRepositories(Result<Vec<Repository>, GhError>),
    /// [`Github::list_issues`].
    ListIssues(Result<Vec<IssueListItem>, GhError>),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount(Result<usize, GhError>),
    /// [`Github::list_notifications`].
    ListNotifications(Result<Vec<Notification>, GhError>),
    /// [`Github::mark_notification_read`].
    MarkNotificationRead(Result<(), GhError>),
    /// [`Github::mark_all_notifications_read`].
    MarkAllNotificationsRead(Result<(), GhError>),
    /// [`Github::repo_readme`].
    RepoReadme(Result<Option<String>, GhError>),
    /// [`Github::repository`], boxed to keep this enum small.
    Repository(Result<Option<Box<Repository>>, GhError>),
    /// [`Github::list_workflows`].
    ListWorkflows(Result<Vec<Workflow>, GhError>),
    /// [`Github::list_workflow_runs`].
    ListWorkflowRuns(Result<Vec<WorkflowRun>, GhError>),
    /// [`Github::pull_request_diff`].
    PullRequestDiff(Result<String, DiffFetchError>),
    /// [`Github::user`], boxed to keep this enum small.
    User(Result<Box<FullUser>, GhError>),
    /// [`Github::user_repos`].
    UserRepos(Result<Vec<Repository>, GhError>),
    /// [`Github::pull_request`], boxed to keep this enum small.
    PullRequest(Result<Box<FullPullRequest>, GhError>),
    /// [`Github::workflow_run_jobs`].
    WorkflowRunJobs(Result<Vec<WorkflowJob>, GhError>),
    /// [`Github::commit_status`].
    CommitStatus(Result<CombinedStatus, GhError>),
    /// [`Github::check_runs`].
    CheckRuns(Result<Vec<CheckRun>, GhError>),
    /// [`Github::issue_reactions`].
    IssueReactions(Result<Reactions, GhError>),
    /// [`Github::issue`], boxed to keep this enum small.
    Issue(Result<Box<IssueListItem>, GhError>),
    /// [`Github::issue_comments`].
    IssueComments(Result<Vec<IssueComment>, GhError>),
    /// [`Github::comment_reactions`].
    CommentReactions(Result<Reactions, GhError>),
    /// [`Github::job_logs`].
    JobLogs(Result<Option<String>, GhError>),
    /// [`Github::pull_request_reviews`].
    PullRequestReviews(Result<Vec<Review>, GhError>),
    /// [`Github::create_issue`], boxed to keep this enum small.
    CreateIssue(Result<Box<Issue>, GhError>),
    /// [`Github::create_issue_comment`], boxed to keep this enum small.
    CreateIssueComment(Result<Box<IssueComment>, GhError>),
    /// [`Github::reply_to_review_comment`], boxed to keep this enum small.
    ReplyToReviewComment(Result<Box<ReviewComment>, GhError>),
    /// [`Github::update_issue`], boxed to keep this enum small.
    UpdateIssue(Result<Box<Issue>, GhError>),
    /// [`Github::create_review`], boxed to keep this enum small.
    CreateReview(Result<Box<Review>, GhError>),
    /// [`Github::add_issue_reaction`]: whether the reaction was created.
    AddIssueReaction(Result<bool, GhError>),
    /// [`Github::merge_pull_request`].
    MergePullRequest(Result<MergeResult, GhError>),
    /// [`Github::list_branches`].
    ListBranches(Result<Vec<Branch>, GhError>),
    /// [`Github::dispatch_workflow`].
    DispatchWorkflow(Result<(), GhError>),
    /// [`Github::create_pull_request`], boxed to keep this enum small.
    CreatePullRequest(Result<Box<FullPullRequest>, GhError>),
    /// [`Github::rerun_workflow_run`] or [`Github::rerun_failed_jobs`].
    RerunWorkflow(Result<(), GhError>),
    /// [`Github::workflow_run`], boxed to keep this enum small.
    WorkflowRun(Result<Box<WorkflowRun>, GhError>),
    /// [`Github::collaborators`]: their logins.
    Collaborators(Result<Vec<String>, GhError>),
    /// [`Github::requested_reviewers`]: their logins.
    RequestedReviewers(Result<Vec<String>, GhError>),
    /// [`Github::request_reviewers`].
    RequestReviewers(Result<(), GhError>),
    /// [`Github::update_pull_request`].
    UpdatePullRequest(Result<(), GhError>),
}

impl GithubResponse {
    /// The [`GhError`] the request failed with, if it failed with one. A failed diff fetch has one unless it was a
    /// 404 or a 406 ([`DiffFetchError::gh_error`]).
    #[allow(
        clippy::match_same_arms,
        reason = "a flat table with one arm per variant, in the enum's order; merging the arms of equal result types would scatter it"
    )]
    pub fn error(&self) -> Option<&GhError> {
        match self {
            GithubResponse::CheckAuth(result) => result.as_ref().err(),
            GithubResponse::GhVersion(result) => result.as_ref().err(),
            GithubResponse::CurrentUser(result) => result.as_ref().err(),
            GithubResponse::ListRepositories(result) => result.as_ref().err(),
            GithubResponse::ListIssues(result) => result.as_ref().err(),
            GithubResponse::UnreadNotificationCount(result) => result.as_ref().err(),
            GithubResponse::ListNotifications(result) => result.as_ref().err(),
            GithubResponse::MarkNotificationRead(result) => result.as_ref().err(),
            GithubResponse::MarkAllNotificationsRead(result) => result.as_ref().err(),
            GithubResponse::RepoReadme(result) => result.as_ref().err(),
            GithubResponse::Repository(result) => result.as_ref().err(),
            GithubResponse::ListWorkflows(result) => result.as_ref().err(),
            GithubResponse::ListWorkflowRuns(result) => result.as_ref().err(),
            GithubResponse::User(result) => result.as_ref().err(),
            GithubResponse::UserRepos(result) => result.as_ref().err(),
            GithubResponse::PullRequest(result) => result.as_ref().err(),
            GithubResponse::WorkflowRunJobs(result) => result.as_ref().err(),
            GithubResponse::CommitStatus(result) => result.as_ref().err(),
            GithubResponse::CheckRuns(result) => result.as_ref().err(),
            GithubResponse::IssueReactions(result) => result.as_ref().err(),
            GithubResponse::Issue(result) => result.as_ref().err(),
            GithubResponse::IssueComments(result) => result.as_ref().err(),
            GithubResponse::CommentReactions(result) => result.as_ref().err(),
            GithubResponse::JobLogs(result) => result.as_ref().err(),
            GithubResponse::PullRequestReviews(result) => result.as_ref().err(),
            GithubResponse::CreateIssue(result) => result.as_ref().err(),
            GithubResponse::CreateIssueComment(result) => result.as_ref().err(),
            GithubResponse::ReplyToReviewComment(result) => result.as_ref().err(),
            GithubResponse::UpdateIssue(result) => result.as_ref().err(),
            GithubResponse::CreateReview(result) => result.as_ref().err(),
            GithubResponse::AddIssueReaction(result) => result.as_ref().err(),
            GithubResponse::MergePullRequest(result) => result.as_ref().err(),
            GithubResponse::ListBranches(result) => result.as_ref().err(),
            GithubResponse::DispatchWorkflow(result) => result.as_ref().err(),
            GithubResponse::CreatePullRequest(result) => result.as_ref().err(),
            GithubResponse::RerunWorkflow(result) => result.as_ref().err(),
            GithubResponse::WorkflowRun(result) => result.as_ref().err(),
            GithubResponse::Collaborators(result) => result.as_ref().err(),
            GithubResponse::RequestedReviewers(result) => result.as_ref().err(),
            GithubResponse::RequestReviewers(result) => result.as_ref().err(),
            GithubResponse::UpdatePullRequest(result) => result.as_ref().err(),
            GithubResponse::PullRequestDiff(result) => result.as_ref().err().and_then(DiffFetchError::gh_error),
        }
    }
}

impl GithubRequest {
    /// Performs the request against `gh`.
    #[allow(
        clippy::too_many_lines,
        reason = "a flat table with one arm per Github method; splitting it would need an unreachable arm"
    )]
    pub async fn run<G: Github>(self, gh: &G) -> GithubResponse {
        match self {
            GithubRequest::CheckAuth => GithubResponse::CheckAuth(gh.check_auth().await),
            GithubRequest::GhVersion => GithubResponse::GhVersion(gh.gh_version().await),
            GithubRequest::CurrentUser => GithubResponse::CurrentUser(gh.current_user().await),
            GithubRequest::ListRepositories(query) => {
                GithubResponse::ListRepositories(gh.list_repositories(query).await)
            }
            GithubRequest::ListIssues { repo, query } => GithubResponse::ListIssues(gh.list_issues(&repo, query).await),
            GithubRequest::UnreadNotificationCount => {
                GithubResponse::UnreadNotificationCount(gh.unread_notification_count().await)
            }
            GithubRequest::ListNotifications(query) => {
                GithubResponse::ListNotifications(gh.list_notifications(query).await)
            }
            GithubRequest::MarkNotificationRead(thread_id) => {
                GithubResponse::MarkNotificationRead(gh.mark_notification_read(thread_id).await)
            }
            GithubRequest::MarkAllNotificationsRead => {
                GithubResponse::MarkAllNotificationsRead(gh.mark_all_notifications_read().await)
            }
            GithubRequest::RepoReadme(full_name) => GithubResponse::RepoReadme(gh.repo_readme(&full_name).await),
            GithubRequest::Repository(full_name) => {
                GithubResponse::Repository(gh.repository(&full_name).await.map(|found| found.map(Box::new)))
            }
            GithubRequest::ListWorkflows { repo, page } => {
                GithubResponse::ListWorkflows(gh.list_workflows(&repo, page).await)
            }
            GithubRequest::ListWorkflowRuns { repo, page } => {
                GithubResponse::ListWorkflowRuns(gh.list_workflow_runs(&repo, page).await)
            }
            GithubRequest::PullRequestDiff { repo, number } => {
                GithubResponse::PullRequestDiff(gh.pull_request_diff(&repo, number).await)
            }
            GithubRequest::User(login) => GithubResponse::User(gh.user(&login).await.map(Box::new)),
            GithubRequest::UserRepos(login) => GithubResponse::UserRepos(gh.user_repos(&login).await),
            GithubRequest::PullRequest { repo, number } => {
                GithubResponse::PullRequest(gh.pull_request(&repo, number).await.map(Box::new))
            }
            GithubRequest::PullRequestUncached { repo, number } => {
                GithubResponse::PullRequest(gh.pull_request_uncached(&repo, number).await.map(Box::new))
            }
            GithubRequest::WorkflowRunJobs { repo, run_id } => {
                GithubResponse::WorkflowRunJobs(gh.workflow_run_jobs(&repo, run_id).await)
            }
            GithubRequest::CommitStatus { repo, sha } => {
                GithubResponse::CommitStatus(gh.commit_status(&repo, &sha).await)
            }
            GithubRequest::CheckRuns { repo, sha } => GithubResponse::CheckRuns(gh.check_runs(&repo, &sha).await),
            GithubRequest::IssueReactions { repo, number } => {
                GithubResponse::IssueReactions(gh.issue_reactions(&repo, number).await)
            }
            GithubRequest::Issue { repo, number } => GithubResponse::Issue(gh.issue(&repo, number).await.map(Box::new)),
            GithubRequest::IssueComments { issue } => GithubResponse::IssueComments(gh.issue_comments(&issue).await),
            GithubRequest::CommentReactions { repo, comment } => {
                GithubResponse::CommentReactions(gh.comment_reactions(&repo, comment).await)
            }
            GithubRequest::JobLogs { repo, job_id } => GithubResponse::JobLogs(gh.job_logs(&repo, job_id).await),
            GithubRequest::PullRequestReviews { pr } => {
                GithubResponse::PullRequestReviews(gh.pull_request_reviews(&pr).await)
            }
            GithubRequest::CreateIssue { repo, issue } => {
                GithubResponse::CreateIssue(gh.create_issue(&repo, &issue).await.map(Box::new))
            }
            GithubRequest::CreateIssueComment { repo, number, body } => {
                GithubResponse::CreateIssueComment(gh.create_issue_comment(&repo, number, &body).await.map(Box::new))
            }
            GithubRequest::ReplyToReviewComment {
                repo,
                pr_number,
                comment_id,
                body,
            } => GithubResponse::ReplyToReviewComment(
                gh.reply_to_review_comment(&repo, pr_number, comment_id, &body)
                    .await
                    .map(Box::new),
            ),
            GithubRequest::UpdateIssue { repo, number, update } => {
                GithubResponse::UpdateIssue(gh.update_issue(&repo, number, &update).await.map(Box::new))
            }
            GithubRequest::CreateReview { repo, number, review } => {
                GithubResponse::CreateReview(gh.create_review(&repo, number, &review).await.map(Box::new))
            }
            GithubRequest::AddIssueReaction { repo, number, kind } => {
                GithubResponse::AddIssueReaction(gh.add_issue_reaction(&repo, number, kind).await)
            }
            GithubRequest::MergePullRequest { repo, number, request } => {
                GithubResponse::MergePullRequest(gh.merge_pull_request(&repo, number, &request).await)
            }
            GithubRequest::ListBranches { repo } => GithubResponse::ListBranches(gh.list_branches(&repo).await),
            GithubRequest::ListBranchesUncached { repo } => {
                GithubResponse::ListBranches(gh.list_branches_uncached(&repo).await)
            }
            GithubRequest::CreatePullRequest { repo, pr } => {
                GithubResponse::CreatePullRequest(gh.create_pull_request(&repo, &pr).await.map(Box::new))
            }
            GithubRequest::DispatchWorkflow {
                repo,
                workflow_id,
                git_ref,
            } => GithubResponse::DispatchWorkflow(gh.dispatch_workflow(&repo, workflow_id, &git_ref).await),
            GithubRequest::RerunWorkflow {
                repo,
                run_id,
                failed_only,
            } => GithubResponse::RerunWorkflow(if failed_only {
                gh.rerun_failed_jobs(&repo, run_id).await
            } else {
                gh.rerun_workflow_run(&repo, run_id).await
            }),
            GithubRequest::WorkflowRun { repo, run_id } => {
                GithubResponse::WorkflowRun(gh.workflow_run(&repo, run_id).await.map(Box::new))
            }
            GithubRequest::Collaborators(repo) => GithubResponse::Collaborators(gh.collaborators(&repo).await),
            GithubRequest::RequestedReviewers { repo, number } => {
                GithubResponse::RequestedReviewers(gh.requested_reviewers(&repo, number).await)
            }
            GithubRequest::RequestReviewers {
                repo,
                number,
                reviewers,
            } => GithubResponse::RequestReviewers(gh.request_reviewers(&repo, number, &reviewers).await),
            GithubRequest::UpdatePullRequest { repo, number, update } => {
                GithubResponse::UpdatePullRequest(gh.update_pull_request(&repo, number, &update).await)
            }
        }
    }
}

/// One `git` call in the user's working directory, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRequest {
    /// [`GitCli::origin_repo`]: the repo LazyGithub was launched in.
    OriginRepo,
    /// [`GitCli::current_branch`]: the branch checked out where LazyGithub was launched.
    CurrentBranch,
    /// [`GitCli::has_upstream`].
    HasUpstream {
        /// The local branch.
        branch: String,
    },
    /// [`GitCli::branch_exists_on_remote`].
    BranchExistsOnRemote {
        /// The branch to look for on `origin`.
        branch: String,
    },
    /// [`GitCli::push_branch`].
    PushBranch {
        /// The local branch to push.
        branch: String,
    },
}

/// The result of a [`GitRequest`], in the variant of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitResponse {
    /// [`GitCli::origin_repo`]: `None` when there's no current-directory repo, whatever the reason.
    OriginRepo(Option<RepoName>),
    /// [`GitCli::current_branch`]: `None` when git couldn't say.
    CurrentBranch(Option<String>),
    /// [`GitCli::has_upstream`].
    HasUpstream(bool),
    /// [`GitCli::branch_exists_on_remote`].
    BranchExistsOnRemote(bool),
    /// [`GitCli::push_branch`]: the failure as its log line, so the response stays `Clone`.
    PushBranch(Result<(), String>),
}

impl GitRequest {
    /// Performs the request against `git`.
    pub async fn run(self, git: &GitCli) -> GitResponse {
        match self {
            GitRequest::OriginRepo => GitResponse::OriginRepo(git.origin_repo().await),
            GitRequest::CurrentBranch => GitResponse::CurrentBranch(git.current_branch().await),
            GitRequest::HasUpstream { branch } => GitResponse::HasUpstream(git.has_upstream(&branch).await),
            GitRequest::BranchExistsOnRemote { branch } => {
                GitResponse::BranchExistsOnRemote(git.branch_exists_on_remote(&branch).await)
            }
            GitRequest::PushBranch { branch } => {
                GitResponse::PushBranch(git.push_branch(&branch).await.map_err(|error| error.to_string()))
            }
        }
    }
}
