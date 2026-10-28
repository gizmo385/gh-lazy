//! Side effects as data. `App::update` returns them and only the [`Runtime`](crate::runtime::Runtime) performs them.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::git::GitCli;
use lazy_github_core::github::{
    DiffFetchError, Github, IssueListQuery, IssueUpdate, MergeRequest, MergeResult, NewIssue, NewReview, PageQuery,
    RepoListQuery,
};
use lazy_github_core::models::{
    Branch, CheckRun, CombinedStatus, CommentRef, FullPullRequest, FullUser, Issue, IssueComment, IssueListItem,
    ReactionKind, Reactions, RepoName, Repository, Review, ReviewComment, User, Workflow, WorkflowJob, WorkflowRun,
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
        /// Its new text, from `lazy_github_core::cache::to_json`.
        text: String,
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
}

/// One [`Github`] call, as data. One variant per trait method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubRequest {
    /// [`Github::check_auth`].
    CheckAuth,
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
    /// [`Github::current_user`].
    CurrentUser(Result<User, GhError>),
    /// [`Github::list_repositories`].
    ListRepositories(Result<Vec<Repository>, GhError>),
    /// [`Github::list_issues`].
    ListIssues(Result<Vec<IssueListItem>, GhError>),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount(Result<usize, GhError>),
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
    /// [`Github::rerun_workflow_run`] or [`Github::rerun_failed_jobs`].
    RerunWorkflow(Result<(), GhError>),
    /// [`Github::workflow_run`], boxed to keep this enum small.
    WorkflowRun(Result<Box<WorkflowRun>, GhError>),
}

impl GithubRequest {
    /// Performs the request against `gh`.
    pub async fn run<G: Github>(self, gh: &G) -> GithubResponse {
        match self {
            GithubRequest::CheckAuth => GithubResponse::CheckAuth(gh.check_auth().await),
            GithubRequest::CurrentUser => GithubResponse::CurrentUser(gh.current_user().await),
            GithubRequest::ListRepositories(query) => {
                GithubResponse::ListRepositories(gh.list_repositories(query).await)
            }
            GithubRequest::ListIssues { repo, query } => GithubResponse::ListIssues(gh.list_issues(&repo, query).await),
            GithubRequest::UnreadNotificationCount => {
                GithubResponse::UnreadNotificationCount(gh.unread_notification_count().await)
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
        }
    }
}

/// One `git` call in the user's working directory, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRequest {
    /// [`GitCli::origin_repo`]: the repo LazyGithub was launched in.
    OriginRepo,
}

/// The result of a [`GitRequest`], in the variant of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitResponse {
    /// [`GitCli::origin_repo`]: `None` when there's no current-directory repo, whatever the reason.
    OriginRepo(Option<RepoName>),
}

impl GitRequest {
    /// Performs the request against `git`.
    pub async fn run(self, git: &GitCli) -> GitResponse {
        match self {
            GitRequest::OriginRepo => GitResponse::OriginRepo(git.origin_repo().await),
        }
    }
}
