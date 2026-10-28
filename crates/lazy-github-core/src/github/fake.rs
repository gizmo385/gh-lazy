//! An in-memory [`Github`] for tests.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::de::Error as _;

use super::{
    DiffFetchError, Github, IssueListQuery, IssueUpdate, MergeRequest, MergeResult, NewIssue, NewPullRequest,
    NewReview, PageQuery, RepoListQuery,
};
use crate::gh::GhError;
use crate::models::{
    Branch, CheckRun, CheckState, CombinedStatus, CommentRef, FullPullRequest, FullUser, Issue, IssueComment,
    IssueListItem, ReactionKind, Reactions, Repository, Review, ReviewComment, User, Workflow, WorkflowJob,
    WorkflowRun,
};

/// A [`Github`] that returns canned results and records every call.
///
/// By default `check_auth` succeeds, `list_repositories` returns no repositories, `unread_notification_count` returns
/// 0, `list_issues`, `list_workflows` and `list_workflow_runs` return an empty page, `repo_readme` finds no README, `repository` finds no repository, `pull_request_diff` finds no diff, `pull_request` and `pull_request_uncached` fail with a 404, `user` finds no user (a 404), `user_repos` returns none, `workflow_run_jobs` returns no jobs, `commit_status`, `check_runs`, `issue_reactions`, `issue_comments`, `comment_reactions` and `pull_request_reviews` find none, `issue` fails with a 404, `job_logs` finds no logs (a 404), `create_issue`, `update_issue`, `create_issue_comment`, `reply_to_review_comment`, `create_review`, `add_issue_reaction`, `merge_pull_request`, `dispatch_workflow`, `rerun_workflow_run` and `rerun_failed_jobs` fail with a 422, `list_branches` and `list_branches_uncached` return none, `create_pull_request` fails with a 422, `workflow_run` fails with a 404, and `current_user` fails until [`Self::with_user`] sets one. Clones share the canned results and the call log.
#[derive(Debug, Clone, Default)]
pub struct FakeGithub {
    state: Arc<Mutex<State>>,
}

/// One call made to a [`FakeGithub`], with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubCall {
    /// [`Github::check_auth`].
    CheckAuth,
    /// [`Github::current_user`].
    CurrentUser,
    /// [`Github::list_repositories`].
    ListRepositories(RepoListQuery),
    /// [`Github::unread_notification_count`].
    UnreadNotificationCount,
    /// [`Github::list_issues`].
    ListIssues {
        /// The repository listed.
        repo: Repository,
        /// The filters and page.
        query: IssueListQuery,
    },
    /// [`Github::repo_readme`], with the repo's full name.
    RepoReadme(String),
    /// [`Github::repository`], with the full name asked for.
    Repository(String),
    /// [`Github::list_workflows`].
    ListWorkflows {
        /// The repository listed.
        repo: Repository,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::list_workflow_runs`].
    ListWorkflowRuns {
        /// The repository listed.
        repo: Repository,
        /// The page.
        page: PageQuery,
    },
    /// [`Github::pull_request_diff`].
    PullRequestDiff {
        /// The pull request's repository.
        repo: Repository,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::user`], with the login asked for.
    User(String),
    /// [`Github::user_repos`], with the login asked for.
    UserRepos(String),
    /// [`Github::pull_request`].
    PullRequest {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::pull_request_uncached`].
    PullRequestUncached {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::workflow_run_jobs`], with the run's id.
    WorkflowRunJobs(u64),
    /// [`Github::commit_status`].
    CommitStatus {
        /// The repository's full name.
        full_name: String,
        /// The commit.
        sha: String,
    },
    /// [`Github::check_runs`].
    CheckRuns {
        /// The repository's full name.
        full_name: String,
        /// The commit.
        sha: String,
    },
    /// [`Github::issue_reactions`].
    IssueReactions {
        /// The repository's full name.
        full_name: String,
        /// The issue's or pull request's number.
        number: u64,
    },
    /// [`Github::issue`], with the number asked for.
    Issue(u64),
    /// [`Github::issue_comments`].
    IssueComments {
        /// The repository's full name.
        full_name: String,
        /// The issue's or pull request's number.
        number: u64,
    },
    /// [`Github::comment_reactions`].
    CommentReactions {
        /// The repository's full name.
        full_name: String,
        /// The comment.
        comment: CommentRef,
    },
    /// [`Github::job_logs`], with the job's id.
    JobLogs(u64),
    /// [`Github::pull_request_reviews`].
    PullRequestReviews {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        number: u64,
    },
    /// [`Github::create_issue`].
    CreateIssue {
        /// The repository's full name.
        full_name: String,
        /// What was sent.
        issue: NewIssue,
    },
    /// [`Github::create_issue_comment`].
    CreateIssueComment {
        /// The repository's full name.
        full_name: String,
        /// The issue's or pull request's number.
        number: u64,
        /// What was sent.
        body: String,
    },
    /// [`Github::reply_to_review_comment`].
    ReplyToReviewComment {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        pr_number: u64,
        /// The review comment replied to.
        comment_id: u64,
        /// What was sent.
        body: String,
    },
    /// [`Github::create_review`].
    CreateReview {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        number: u64,
        /// What was sent.
        review: NewReview,
    },
    /// [`Github::add_issue_reaction`].
    AddIssueReaction {
        /// The repository's full name.
        full_name: String,
        /// The issue's or pull request's number.
        number: u64,
        /// The reaction sent.
        kind: ReactionKind,
    },
    /// [`Github::merge_pull_request`].
    MergePullRequest {
        /// The repository's full name.
        full_name: String,
        /// The pull request's number.
        number: u64,
        /// What was sent.
        request: MergeRequest,
    },
    /// [`Github::list_branches`], with the repository's full name.
    ListBranches(String),
    /// [`Github::list_branches_uncached`], with the repository's full name.
    ListBranchesUncached(String),
    /// [`Github::create_pull_request`].
    CreatePullRequest {
        /// The repository's full name.
        full_name: String,
        /// What was sent.
        pr: NewPullRequest,
    },
    /// [`Github::dispatch_workflow`].
    DispatchWorkflow {
        /// The repository's full name.
        full_name: String,
        /// The workflow's id.
        workflow_id: u64,
        /// The branch it was asked to run on.
        git_ref: String,
    },
    /// [`Github::rerun_workflow_run`], with the run's id.
    RerunWorkflowRun(u64),
    /// [`Github::rerun_failed_jobs`], with the run's id.
    RerunFailedJobs(u64),
    /// [`Github::workflow_run`], with the run's id.
    WorkflowRun(u64),
    /// [`Github::update_issue`].
    UpdateIssue {
        /// The repository's full name.
        full_name: String,
        /// The issue's number.
        number: u64,
        /// What was sent.
        update: IssueUpdate,
    },
}

#[derive(Debug, Default)]
struct State {
    auth_error: Option<GhError>,
    user: Option<Result<User, GhError>>,
    repositories: Option<Result<Vec<Repository>, GhError>>,
    unread_count: Option<Result<usize, GhError>>,
    /// Keyed by `full_name` and page.
    issue_pages: HashMap<(String, u32), Result<Vec<IssueListItem>, GhError>>,
    /// Keyed by `full_name`.
    readmes: HashMap<String, Result<Option<String>, GhError>>,
    /// Keyed by `full_name`.
    repository: HashMap<String, Result<Option<Repository>, GhError>>,
    /// Keyed by `full_name` and page.
    workflow_pages: HashMap<(String, u32), Result<Vec<Workflow>, GhError>>,
    /// Keyed by `full_name` and page.
    run_pages: HashMap<(String, u32), Result<Vec<WorkflowRun>, GhError>>,
    /// Keyed by `full_name` and pull request number.
    diffs: HashMap<(String, u64), Result<String, DiffFetchError>>,
    /// Keyed by login.
    profiles: HashMap<String, Result<FullUser, GhError>>,
    /// Keyed by login.
    user_repos: HashMap<String, Result<Vec<Repository>, GhError>>,
    /// Keyed by `full_name` and number.
    pull_requests: HashMap<(String, u64), Result<FullPullRequest, GhError>>,
    /// Keyed by `full_name` and number.
    issues: HashMap<(String, u64), Result<IssueListItem, GhError>>,
    /// Keyed by run id.
    run_jobs: HashMap<u64, Result<Vec<WorkflowJob>, GhError>>,
    /// Keyed by `full_name` and sha.
    commit_statuses: HashMap<(String, String), Result<CombinedStatus, GhError>>,
    /// Keyed by `full_name` and sha.
    check_runs: HashMap<(String, String), Result<Vec<CheckRun>, GhError>>,
    /// Keyed by `full_name` and number.
    issue_reactions: HashMap<(String, u64), Result<Reactions, GhError>>,
    /// Keyed by `full_name` and number.
    issue_comments: HashMap<(String, u64), Result<Vec<IssueComment>, GhError>>,
    /// Keyed by `full_name` and comment.
    comment_reactions: HashMap<(String, CommentRef), Result<Reactions, GhError>>,
    /// Keyed by job id.
    job_logs: HashMap<u64, Result<Option<String>, GhError>>,
    /// Keyed by `full_name` and pull request number.
    reviews: HashMap<(String, u64), Result<Vec<Review>, GhError>>,
    /// What `create_issue` returns, whatever the repo; `None` fails with a 422.
    created_issue: Option<Result<Issue, GhError>>,
    /// What `create_issue_comment` returns, by the body sent; bodies not set fail with a 422.
    posted_comments: HashMap<String, Result<IssueComment, GhError>>,
    /// What `reply_to_review_comment` returns, by the body sent; bodies not set fail with a 422.
    review_replies: HashMap<String, Result<ReviewComment, GhError>>,
    /// What `create_review` returns, by the review sent; reviews not set fail with a 422.
    created_reviews: Vec<(NewReview, Result<Review, GhError>)>,
    /// What `update_issue` returns, whatever the issue; `None` fails with a 422.
    updated_issue: Option<Result<Issue, GhError>>,
    /// What `add_issue_reaction` returns, by the kind sent; kinds not set fail with a 422.
    added_reactions: HashMap<ReactionKind, Result<bool, GhError>>,
    /// What `merge_pull_request` returns, by the pull request's number; numbers not set fail with a 422.
    merges: HashMap<u64, Result<MergeResult, GhError>>,
    /// Keyed by `full_name`.
    branches: HashMap<String, Result<Vec<Branch>, GhError>>,
    /// What `create_pull_request` returns, whatever the repo and the request; `None` fails with a 422.
    created_pull_request: Option<Result<FullPullRequest, GhError>>,
    /// What `dispatch_workflow` returns, by workflow id and ref; pairs not set fail with a 422.
    dispatches: HashMap<(u64, String), Result<(), GhError>>,
    /// What the re-runs return, by run id and whether only the failed jobs; runs not set fail with a 422.
    reruns: HashMap<(u64, bool), Result<(), GhError>>,
    /// Keyed by run id.
    workflow_runs: HashMap<u64, Result<WorkflowRun, GhError>>,
    latency: Option<Duration>,
    calls: Vec<GithubCall>,
}

impl FakeGithub {
    /// A fake that is logged in, has no current user and no repositories.
    pub fn new() -> Self {
        Self::default()
    }

    /// `current_user` returns `user`.
    #[must_use]
    pub fn with_user(self, user: User) -> Self {
        self.lock().user = Some(Ok(user));
        self
    }

    /// `current_user` fails with `error`.
    #[must_use]
    pub fn with_user_error(self, error: GhError) -> Self {
        self.lock().user = Some(Err(error));
        self
    }

    /// `list_repositories` returns `repositories`, whatever the query.
    #[must_use]
    pub fn with_repositories(self, repositories: Vec<Repository>) -> Self {
        self.lock().repositories = Some(Ok(repositories));
        self
    }

    /// `list_repositories` fails with `error`.
    #[must_use]
    pub fn with_repositories_error(self, error: GhError) -> Self {
        self.lock().repositories = Some(Err(error));
        self
    }

    /// `unread_notification_count` returns `count`.
    #[must_use]
    pub fn with_unread_count(self, count: Result<usize, GhError>) -> Self {
        self.set_unread_count(count);
        self
    }

    /// From now on, `unread_notification_count` returns `count`. Clones see the change too.
    pub fn set_unread_count(&self, count: Result<usize, GhError>) {
        self.lock().unread_count = Some(count);
    }

    /// `list_issues` returns `page` for page `page_number` of the repository named `full_name`, whatever the
    /// filters. Pages not set are empty.
    #[must_use]
    pub fn with_issue_page(self, full_name: &str, page_number: u32, page: Result<Vec<IssueListItem>, GhError>) -> Self {
        self.lock()
            .issue_pages
            .insert((full_name.to_owned(), page_number), page);
        self
    }

    /// `repo_readme` returns `readme` for the repository named `full_name`. Repos not set have no README.
    #[must_use]
    pub fn with_readme(self, full_name: &str, readme: Result<Option<String>, GhError>) -> Self {
        self.lock().readmes.insert(full_name.to_owned(), readme);
        self
    }

    /// `repository` returns `repository` for the name `full_name`. Names not set find no repository.
    #[must_use]
    pub fn with_repository(self, full_name: &str, repository: Result<Option<Repository>, GhError>) -> Self {
        self.lock().repository.insert(full_name.to_owned(), repository);
        self
    }

    /// `list_workflows` returns `page` for page `page_number` of the repository named `full_name`. Pages not set are
    /// empty.
    #[must_use]
    pub fn with_workflow_page(self, full_name: &str, page_number: u32, page: Result<Vec<Workflow>, GhError>) -> Self {
        self.lock()
            .workflow_pages
            .insert((full_name.to_owned(), page_number), page);
        self
    }

    /// `list_workflow_runs` returns `page` for page `page_number` of the repository named `full_name`. Pages not set
    /// are empty.
    #[must_use]
    pub fn with_run_page(self, full_name: &str, page_number: u32, page: Result<Vec<WorkflowRun>, GhError>) -> Self {
        self.lock().run_pages.insert((full_name.to_owned(), page_number), page);
        self
    }

    /// `pull_request_diff` returns `diff` for pull request `number` of the repository named `full_name`. Pull requests
    /// not set have no diff ([`DiffFetchError::NotFound`]).
    #[must_use]
    pub fn with_pull_request_diff(self, full_name: &str, number: u64, diff: Result<String, DiffFetchError>) -> Self {
        self.lock().diffs.insert((full_name.to_owned(), number), diff);
        self
    }

    /// `user` returns `profile` for `login`. Logins not set get a 404.
    #[must_use]
    pub fn with_profile(self, login: &str, profile: Result<FullUser, GhError>) -> Self {
        self.lock().profiles.insert(login.to_owned(), profile);
        self
    }

    /// `user_repos` returns `repos` for `login`. Logins not set have none.
    #[must_use]
    pub fn with_user_repos(self, login: &str, repos: Result<Vec<Repository>, GhError>) -> Self {
        self.lock().user_repos.insert(login.to_owned(), repos);
        self
    }

    /// `pull_request` returns `pull_request` for number `number` of the repository named `full_name`. Numbers not set
    /// fail with a 404.
    #[must_use]
    pub fn with_pull_request(
        self,
        full_name: &str,
        number: u64,
        pull_request: Result<FullPullRequest, GhError>,
    ) -> Self {
        self.lock()
            .pull_requests
            .insert((full_name.to_owned(), number), pull_request);
        self
    }

    /// `issue` returns `item` for number `number` of the repository named `full_name`. Numbers not set fail with a
    /// 404.
    #[must_use]
    pub fn with_issue(self, full_name: &str, number: u64, item: Result<IssueListItem, GhError>) -> Self {
        self.lock().issues.insert((full_name.to_owned(), number), item);
        self
    }

    /// `workflow_run_jobs` returns `jobs` for run `run_id`, whatever the repository. Runs not set have no jobs.
    #[must_use]
    pub fn with_run_jobs(self, run_id: u64, jobs: Result<Vec<WorkflowJob>, GhError>) -> Self {
        self.lock().run_jobs.insert(run_id, jobs);
        self
    }

    /// `commit_status` returns `status` for commit `sha` of the repository named `full_name`. Commits not set have no
    /// statuses (and GitHub's `pending` state).
    #[must_use]
    pub fn with_commit_status(self, full_name: &str, sha: &str, status: Result<CombinedStatus, GhError>) -> Self {
        self.lock()
            .commit_statuses
            .insert((full_name.to_owned(), sha.to_owned()), status);
        self
    }

    /// `check_runs` returns `runs` for commit `sha` of the repository named `full_name`. Commits not set have none.
    #[must_use]
    pub fn with_check_runs(self, full_name: &str, sha: &str, runs: Result<Vec<CheckRun>, GhError>) -> Self {
        self.lock()
            .check_runs
            .insert((full_name.to_owned(), sha.to_owned()), runs);
        self
    }

    /// `issue_reactions` returns `reactions` for number `number` of the repository named `full_name`. Numbers not set
    /// have none.
    #[must_use]
    pub fn with_issue_reactions(self, full_name: &str, number: u64, reactions: Result<Reactions, GhError>) -> Self {
        self.lock()
            .issue_reactions
            .insert((full_name.to_owned(), number), reactions);
        self
    }

    /// `issue_comments` returns `comments` for number `number` of the repository named `full_name`. Numbers not set
    /// have none.
    #[must_use]
    pub fn with_issue_comments(
        self,
        full_name: &str,
        number: u64,
        comments: Result<Vec<IssueComment>, GhError>,
    ) -> Self {
        self.lock()
            .issue_comments
            .insert((full_name.to_owned(), number), comments);
        self
    }

    /// `comment_reactions` returns `reactions` for `comment` in the repository named `full_name`. Comments not set
    /// have none.
    #[must_use]
    pub fn with_comment_reactions(
        self,
        full_name: &str,
        comment: CommentRef,
        reactions: Result<Reactions, GhError>,
    ) -> Self {
        self.lock()
            .comment_reactions
            .insert((full_name.to_owned(), comment), reactions);
        self
    }

    /// `pull_request_reviews` returns `reviews` for pull request `number` of the repository named `full_name`, as
    /// given: the caller leaves pending ones out and fills in the comments. Numbers not set have none.
    #[must_use]
    pub fn with_pull_request_reviews(
        self,
        full_name: &str,
        number: u64,
        reviews: Result<Vec<Review>, GhError>,
    ) -> Self {
        self.lock().reviews.insert((full_name.to_owned(), number), reviews);
        self
    }

    /// `job_logs` returns `logs` for job `job_id`, whatever the repository. Jobs not set have no logs: a 404.
    #[must_use]
    pub fn with_job_logs(self, job_id: u64, logs: Result<Option<String>, GhError>) -> Self {
        self.lock().job_logs.insert(job_id, logs);
        self
    }

    /// `create_issue_comment` returns `posted` for a comment with body `body`, whatever the repo and number. Other
    /// bodies fail with a 422.
    #[must_use]
    pub fn with_posted_comment(self, body: &str, posted: Result<IssueComment, GhError>) -> Self {
        self.lock().posted_comments.insert(body.to_owned(), posted);
        self
    }

    /// `reply_to_review_comment` returns `reply` for a reply with body `body`, whatever it answers. Other bodies fail
    /// with a 422.
    #[must_use]
    pub fn with_review_reply(self, body: &str, reply: Result<ReviewComment, GhError>) -> Self {
        self.lock().review_replies.insert(body.to_owned(), reply);
        self
    }

    /// `create_review` returns `created` for exactly `review`, whatever the pull request. Other reviews fail with a
    /// 422.
    #[must_use]
    pub fn with_created_review(self, review: NewReview, created: Result<Review, GhError>) -> Self {
        self.lock().created_reviews.push((review, created));
        self
    }

    /// `add_issue_reaction` returns `added` for a reaction of `kind`, whatever the issue: `true` as for a 201,
    /// `false` as for a 200. Other kinds fail with a 422.
    #[must_use]
    pub fn with_added_reaction(self, kind: ReactionKind, added: Result<bool, GhError>) -> Self {
        self.lock().added_reactions.insert(kind, added);
        self
    }

    /// `merge_pull_request` returns `result` for pull request `number`, whatever the repo and the request sent. Other
    /// numbers fail with a 422.
    #[must_use]
    pub fn with_merge(self, number: u64, result: Result<MergeResult, GhError>) -> Self {
        self.lock().merges.insert(number, result);
        self
    }

    /// `list_branches` returns `branches` for the repository named `full_name`. Repositories not set have none.
    #[must_use]
    pub fn with_branches(self, full_name: &str, branches: Result<Vec<Branch>, GhError>) -> Self {
        self.lock().branches.insert(full_name.to_owned(), branches);
        self
    }

    /// `create_pull_request` returns `created`, whatever the repo and the request sent.
    #[must_use]
    pub fn with_created_pull_request(self, created: Result<FullPullRequest, GhError>) -> Self {
        self.lock().created_pull_request = Some(created);
        self
    }

    /// `dispatch_workflow` returns `result` for workflow `workflow_id` on `git_ref`, whatever the repo. Other pairs
    /// fail with a 422.
    #[must_use]
    pub fn with_dispatch(self, workflow_id: u64, git_ref: &str, result: Result<(), GhError>) -> Self {
        self.lock().dispatches.insert((workflow_id, git_ref.to_owned()), result);
        self
    }

    /// `rerun_failed_jobs` (with `failed_only`) or `rerun_workflow_run` returns `result` for run `run_id`, whatever
    /// the repo. Runs not set fail with a 422.
    #[must_use]
    pub fn with_rerun(self, run_id: u64, failed_only: bool, result: Result<(), GhError>) -> Self {
        self.lock().reruns.insert((run_id, failed_only), result);
        self
    }

    /// `workflow_run` returns `run` for run `run_id`, whatever the repo. Runs not set fail with a 404.
    #[must_use]
    pub fn with_workflow_run(self, run_id: u64, run: Result<WorkflowRun, GhError>) -> Self {
        self.lock().workflow_runs.insert(run_id, run);
        self
    }

    /// `update_issue` returns `updated`, whatever the repo, the number and the update sent.
    #[must_use]
    pub fn with_updated_issue(self, updated: Result<Issue, GhError>) -> Self {
        self.lock().updated_issue = Some(updated);
        self
    }

    /// `create_issue` returns `created`, whatever the repo and the issue sent.
    #[must_use]
    pub fn with_created_issue(self, created: Result<Issue, GhError>) -> Self {
        self.lock().created_issue = Some(created);
        self
    }

    /// `check_auth` fails with `error`.
    #[must_use]
    pub fn with_auth_error(self, error: GhError) -> Self {
        self.lock().auth_error = Some(error);
        self
    }

    /// From now on, `check_auth` fails with `error`, or succeeds for `None`. Clones see the change too, so a test can
    /// log in while the app holds the fake.
    pub fn set_auth_error(&self, error: Option<GhError>) {
        self.lock().auth_error = error;
    }

    /// Every call is recorded at once but returns only after `latency`, on tokio's clock. With
    /// `tokio::time::pause()` this keeps a request in flight, so a test can cancel it.
    #[must_use]
    pub fn with_latency(self, latency: Duration) -> Self {
        self.lock().latency = Some(latency);
        self
    }

    /// Every call made so far, oldest first.
    pub fn calls(&self) -> Vec<GithubCall> {
        self.lock().calls.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking test thread can't leave `State` half-updated, so a poisoned lock is still usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, call: GithubCall) -> MutexGuard<'_, State> {
        let mut state = self.lock();
        state.calls.push(call);
        state
    }
}

impl FakeGithub {
    /// Records `call`, computes its result from the canned state, then waits out the latency. The lock is released
    /// before waiting, so the future stays `Send`.
    async fn respond<T>(&self, call: GithubCall, result: impl FnOnce(&State) -> T) -> T {
        let (result, latency) = {
            let state = self.record(call);
            (result(&state), state.latency)
        };
        if let Some(latency) = latency {
            tokio::time::sleep(latency).await;
        }
        result
    }
}

impl Github for FakeGithub {
    async fn check_auth(&self) -> Result<(), GhError> {
        self.respond(GithubCall::CheckAuth, |state| match &state.auth_error {
            Some(error) => Err(duplicate(error)),
            None => Ok(()),
        })
        .await
    }

    async fn current_user(&self) -> Result<User, GhError> {
        self.respond(GithubCall::CurrentUser, |state| match &state.user {
            Some(Ok(user)) => Ok(user.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Command {
                code: None,
                stderr: "FakeGithub has no current user; call with_user".to_owned(),
            }),
        })
        .await
    }

    async fn list_repositories(&self, query: RepoListQuery) -> Result<Vec<Repository>, GhError> {
        self.respond(GithubCall::ListRepositories(query), |state| match &state.repositories {
            Some(Ok(repositories)) => Ok(repositories.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(Vec::new()),
        })
        .await
    }

    async fn unread_notification_count(&self) -> Result<usize, GhError> {
        self.respond(GithubCall::UnreadNotificationCount, |state| match &state.unread_count {
            Some(Ok(count)) => Ok(*count),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(0),
        })
        .await
    }

    async fn list_issues(&self, repo: &Repository, query: IssueListQuery) -> Result<Vec<IssueListItem>, GhError> {
        let key = (repo.full_name.clone(), query.page);
        let call = GithubCall::ListIssues {
            repo: repo.clone(),
            query,
        };
        self.respond(call, |state| match state.issue_pages.get(&key) {
            Some(Ok(items)) => Ok(items.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(Vec::new()),
        })
        .await
    }

    async fn repo_readme(&self, full_name: &str) -> Result<Option<String>, GhError> {
        self.respond(GithubCall::RepoReadme(full_name.to_owned()), |state| {
            match state.readmes.get(full_name) {
                Some(Ok(readme)) => Ok(readme.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Ok(None),
            }
        })
        .await
    }

    async fn repository(&self, full_name: &str) -> Result<Option<Repository>, GhError> {
        self.respond(GithubCall::Repository(full_name.to_owned()), |state| {
            match state.repository.get(full_name) {
                Some(Ok(repository)) => Ok(repository.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Ok(None),
            }
        })
        .await
    }

    async fn list_workflows(&self, repo: &Repository, page: PageQuery) -> Result<Vec<Workflow>, GhError> {
        let key = (repo.full_name.clone(), page.page);
        let call = GithubCall::ListWorkflows {
            repo: repo.clone(),
            page,
        };
        self.respond(call, |state| canned_page(state.workflow_pages.get(&key)))
            .await
    }

    async fn list_workflow_runs(&self, repo: &Repository, page: PageQuery) -> Result<Vec<WorkflowRun>, GhError> {
        let key = (repo.full_name.clone(), page.page);
        let call = GithubCall::ListWorkflowRuns {
            repo: repo.clone(),
            page,
        };
        self.respond(call, |state| canned_page(state.run_pages.get(&key))).await
    }

    async fn pull_request_diff(&self, repo: &Repository, number: u64) -> Result<String, DiffFetchError> {
        let key = (repo.full_name.clone(), number);
        let call = GithubCall::PullRequestDiff {
            repo: repo.clone(),
            number,
        };
        self.respond(call, |state| match state.diffs.get(&key) {
            Some(Ok(diff)) => Ok(diff.clone()),
            Some(Err(error)) => Err(duplicate_diff_error(error)),
            None => Err(DiffFetchError::NotFound),
        })
        .await
    }

    async fn user(&self, login: &str) -> Result<FullUser, GhError> {
        self.respond(GithubCall::User(login.to_owned()), |state| {
            match state.profiles.get(login) {
                Some(Ok(profile)) => Ok(profile.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Err(GhError::Http {
                    status: 404,
                    message: "Not Found".to_owned(),
                    body: Vec::new(),
                }),
            }
        })
        .await
    }

    async fn user_repos(&self, login: &str) -> Result<Vec<Repository>, GhError> {
        self.respond(GithubCall::UserRepos(login.to_owned()), |state| {
            canned_page(state.user_repos.get(login))
        })
        .await
    }

    async fn pull_request(&self, repo: &Repository, number: u64) -> Result<FullPullRequest, GhError> {
        let key = (repo.full_name.clone(), number);
        let call = GithubCall::PullRequest {
            full_name: repo.full_name.clone(),
            number,
        };
        self.respond(call, |state| canned_pull_request(state, &key)).await
    }

    /// Answers from the same canned pull requests as `pull_request`.
    async fn pull_request_uncached(&self, repo: &Repository, number: u64) -> Result<FullPullRequest, GhError> {
        let key = (repo.full_name.clone(), number);
        let call = GithubCall::PullRequestUncached {
            full_name: repo.full_name.clone(),
            number,
        };
        self.respond(call, |state| canned_pull_request(state, &key)).await
    }

    async fn workflow_run_jobs(&self, _repo: &Repository, run_id: u64) -> Result<Vec<WorkflowJob>, GhError> {
        self.respond(GithubCall::WorkflowRunJobs(run_id), |state| {
            canned_page(state.run_jobs.get(&run_id))
        })
        .await
    }

    async fn commit_status(&self, repo: &Repository, sha: &str) -> Result<CombinedStatus, GhError> {
        let key = (repo.full_name.clone(), sha.to_owned());
        let call = GithubCall::CommitStatus {
            full_name: repo.full_name.clone(),
            sha: sha.to_owned(),
        };
        self.respond(call, |state| match state.commit_statuses.get(&key) {
            Some(Ok(status)) => Ok(status.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(CombinedStatus {
                state: CheckState::Pending,
                statuses: Vec::new(),
            }),
        })
        .await
    }

    async fn check_runs(&self, repo: &Repository, sha: &str) -> Result<Vec<CheckRun>, GhError> {
        let key = (repo.full_name.clone(), sha.to_owned());
        let call = GithubCall::CheckRuns {
            full_name: repo.full_name.clone(),
            sha: sha.to_owned(),
        };
        self.respond(call, |state| canned_page(state.check_runs.get(&key)))
            .await
    }

    async fn issue_reactions(&self, repo: &Repository, number: u64) -> Result<Reactions, GhError> {
        let key = (repo.full_name.clone(), number);
        let call = GithubCall::IssueReactions {
            full_name: repo.full_name.clone(),
            number,
        };
        self.respond(call, |state| match state.issue_reactions.get(&key) {
            Some(Ok(reactions)) => Ok(reactions.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(Reactions::default()),
        })
        .await
    }

    async fn issue(&self, repo: &Repository, number: u64) -> Result<IssueListItem, GhError> {
        let key = (repo.full_name.clone(), number);
        self.respond(GithubCall::Issue(number), |state| match state.issues.get(&key) {
            Some(Ok(item)) => Ok(item.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Http {
                status: 404,
                message: "Not Found".to_owned(),
                body: Vec::new(),
            }),
        })
        .await
    }

    async fn issue_comments(&self, issue: &Issue) -> Result<Vec<IssueComment>, GhError> {
        let key = (issue.repo.full_name.clone(), issue.number);
        let call = GithubCall::IssueComments {
            full_name: issue.repo.full_name.clone(),
            number: issue.number,
        };
        self.respond(call, |state| canned_page(state.issue_comments.get(&key)))
            .await
    }

    async fn comment_reactions(&self, repo: &Repository, comment: CommentRef) -> Result<Reactions, GhError> {
        let key = (repo.full_name.clone(), comment);
        let call = GithubCall::CommentReactions {
            full_name: repo.full_name.clone(),
            comment,
        };
        self.respond(call, |state| match state.comment_reactions.get(&key) {
            Some(Ok(reactions)) => Ok(reactions.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Ok(Reactions::default()),
        })
        .await
    }

    async fn pull_request_reviews(&self, pr: &FullPullRequest) -> Result<Vec<Review>, GhError> {
        let full_name = pr.pr.issue.repo.full_name.clone();
        let key = (full_name.clone(), pr.number());
        let call = GithubCall::PullRequestReviews {
            full_name,
            number: pr.number(),
        };
        self.respond(call, |state| canned_page(state.reviews.get(&key))).await
    }

    async fn job_logs(&self, _repo: &Repository, job_id: u64) -> Result<Option<String>, GhError> {
        self.respond(GithubCall::JobLogs(job_id), |state| match state.job_logs.get(&job_id) {
            Some(Ok(logs)) => Ok(logs.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Http {
                status: 404,
                message: "Not Found".to_owned(),
                body: Vec::new(),
            }),
        })
        .await
    }

    async fn create_issue(&self, repo: &Repository, issue: &NewIssue) -> Result<Issue, GhError> {
        let call = GithubCall::CreateIssue {
            full_name: repo.full_name.clone(),
            issue: issue.clone(),
        };
        self.respond(call, |state| match &state.created_issue {
            Some(Ok(created)) => Ok(created.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Http {
                status: 422,
                message: "Unprocessable Entity".to_owned(),
                body: Vec::new(),
            }),
        })
        .await
    }

    async fn create_issue_comment(&self, repo: &Repository, number: u64, body: &str) -> Result<IssueComment, GhError> {
        let call = GithubCall::CreateIssueComment {
            full_name: repo.full_name.clone(),
            number,
            body: body.to_owned(),
        };
        self.respond(call, |state| match state.posted_comments.get(body) {
            Some(Ok(posted)) => Ok(posted.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(unprocessable()),
        })
        .await
    }

    async fn reply_to_review_comment(
        &self,
        repo: &Repository,
        pr_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment, GhError> {
        let call = GithubCall::ReplyToReviewComment {
            full_name: repo.full_name.clone(),
            pr_number,
            comment_id,
            body: body.to_owned(),
        };
        self.respond(call, |state| match state.review_replies.get(body) {
            Some(Ok(reply)) => Ok(reply.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(unprocessable()),
        })
        .await
    }

    async fn create_review(&self, repo: &Repository, number: u64, review: &NewReview) -> Result<Review, GhError> {
        let call = GithubCall::CreateReview {
            full_name: repo.full_name.clone(),
            number,
            review: review.clone(),
        };
        self.respond(call, |state| {
            match state.created_reviews.iter().find(|(sent, _)| sent == review) {
                Some((_, Ok(created))) => Ok(created.clone()),
                Some((_, Err(error))) => Err(duplicate(error)),
                None => Err(unprocessable()),
            }
        })
        .await
    }

    async fn add_issue_reaction(&self, repo: &Repository, number: u64, kind: ReactionKind) -> Result<bool, GhError> {
        let call = GithubCall::AddIssueReaction {
            full_name: repo.full_name.clone(),
            number,
            kind,
        };
        self.respond(call, |state| match state.added_reactions.get(&kind) {
            Some(Ok(added)) => Ok(*added),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(unprocessable()),
        })
        .await
    }

    async fn merge_pull_request(
        &self,
        repo: &Repository,
        number: u64,
        request: &MergeRequest,
    ) -> Result<MergeResult, GhError> {
        let call = GithubCall::MergePullRequest {
            full_name: repo.full_name.clone(),
            number,
            request: request.clone(),
        };
        self.respond(call, |state| match state.merges.get(&number) {
            Some(Ok(result)) => Ok(result.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(unprocessable()),
        })
        .await
    }

    async fn list_branches(&self, repo: &Repository) -> Result<Vec<Branch>, GhError> {
        self.respond(GithubCall::ListBranches(repo.full_name.clone()), |state| {
            canned_page(state.branches.get(&repo.full_name))
        })
        .await
    }

    /// Answers from the same canned branches as `list_branches`.
    async fn list_branches_uncached(&self, repo: &Repository) -> Result<Vec<Branch>, GhError> {
        self.respond(GithubCall::ListBranchesUncached(repo.full_name.clone()), |state| {
            canned_page(state.branches.get(&repo.full_name))
        })
        .await
    }

    async fn create_pull_request(&self, repo: &Repository, pr: &NewPullRequest) -> Result<FullPullRequest, GhError> {
        let call = GithubCall::CreatePullRequest {
            full_name: repo.full_name.clone(),
            pr: pr.clone(),
        };
        self.respond(call, |state| match &state.created_pull_request {
            Some(Ok(created)) => Ok(created.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(unprocessable()),
        })
        .await
    }

    async fn dispatch_workflow(&self, repo: &Repository, workflow_id: u64, git_ref: &str) -> Result<(), GhError> {
        let call = GithubCall::DispatchWorkflow {
            full_name: repo.full_name.clone(),
            workflow_id,
            git_ref: git_ref.to_owned(),
        };
        let key = (workflow_id, git_ref.to_owned());
        self.respond(call, |state| match state.dispatches.get(&key) {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(unprocessable()),
        })
        .await
    }

    async fn rerun_workflow_run(&self, _repo: &Repository, run_id: u64) -> Result<(), GhError> {
        self.respond(GithubCall::RerunWorkflowRun(run_id), |state| {
            canned_rerun(state, run_id, false)
        })
        .await
    }

    async fn rerun_failed_jobs(&self, _repo: &Repository, run_id: u64) -> Result<(), GhError> {
        self.respond(GithubCall::RerunFailedJobs(run_id), |state| {
            canned_rerun(state, run_id, true)
        })
        .await
    }

    async fn workflow_run(&self, _repo: &Repository, run_id: u64) -> Result<WorkflowRun, GhError> {
        self.respond(GithubCall::WorkflowRun(run_id), |state| {
            match state.workflow_runs.get(&run_id) {
                Some(Ok(run)) => Ok(run.clone()),
                Some(Err(error)) => Err(duplicate(error)),
                None => Err(GhError::Http {
                    status: 404,
                    message: "Not Found".to_owned(),
                    body: Vec::new(),
                }),
            }
        })
        .await
    }

    async fn update_issue(&self, repo: &Repository, number: u64, update: &IssueUpdate) -> Result<Issue, GhError> {
        let call = GithubCall::UpdateIssue {
            full_name: repo.full_name.clone(),
            number,
            update: update.clone(),
        };
        self.respond(call, |state| match &state.updated_issue {
            Some(Ok(updated)) => Ok(updated.clone()),
            Some(Err(error)) => Err(duplicate(error)),
            None => Err(GhError::Http {
                status: 422,
                message: "Unprocessable Entity".to_owned(),
                body: Vec::new(),
            }),
        })
        .await
    }
}

/// A copy of `error`, as [`duplicate`] makes for a [`GhError`].
fn duplicate_diff_error(error: &DiffFetchError) -> DiffFetchError {
    match error {
        DiffFetchError::NotFound => DiffFetchError::NotFound,
        DiffFetchError::TooLarge => DiffFetchError::TooLarge,
        DiffFetchError::Http(status) => DiffFetchError::Http(*status),
        DiffFetchError::Gh(error) => DiffFetchError::Gh(duplicate(error)),
    }
}

/// A canned pull request, or a 404 when none was set.
fn canned_pull_request(state: &State, key: &(String, u64)) -> Result<FullPullRequest, GhError> {
    match state.pull_requests.get(key) {
        Some(Ok(pull_request)) => Ok(pull_request.clone()),
        Some(Err(error)) => Err(duplicate(error)),
        None => Err(GhError::Http {
            status: 404,
            message: "Not Found".to_owned(),
            body: Vec::new(),
        }),
    }
}

/// A canned re-run's result, or a 422 when none was set.
fn canned_rerun(state: &State, run_id: u64, failed_only: bool) -> Result<(), GhError> {
    match state.reruns.get(&(run_id, failed_only)) {
        Some(Ok(())) => Ok(()),
        Some(Err(error)) => Err(duplicate(error)),
        None => Err(unprocessable()),
    }
}

/// A canned page, or an empty one when none was set.
fn canned_page<T: Clone>(page: Option<&Result<Vec<T>, GhError>>) -> Result<Vec<T>, GhError> {
    match page {
        Some(Ok(items)) => Ok(items.clone()),
        Some(Err(error)) => Err(duplicate(error)),
        None => Ok(Vec::new()),
    }
}

/// A copy of `error`, so a canned error can be returned on every call. `GhError` isn't `Clone` because its
/// `serde_json` and `io` sources aren't; those are rebuilt from their message (and kind).
/// GitHub's answer to a write it refuses.
fn unprocessable() -> GhError {
    GhError::Http {
        status: 422,
        message: "Unprocessable Entity".to_owned(),
        body: Vec::new(),
    }
}

fn duplicate(error: &GhError) -> GhError {
    match error {
        GhError::NotInstalled => GhError::NotInstalled,
        GhError::NotAuthenticated { hostname, detail } => GhError::NotAuthenticated {
            hostname: hostname.clone(),
            detail: detail.clone(),
        },
        GhError::Http { status, message, body } => GhError::Http {
            status: *status,
            message: message.clone(),
            body: body.clone(),
        },
        GhError::Command { code, stderr } => GhError::Command {
            code: *code,
            stderr: stderr.clone(),
        },
        GhError::Parse(message) => GhError::Parse(message.clone()),
        GhError::Decode(error) => GhError::Decode(serde_json::Error::custom(error)),
        GhError::Io(error) => GhError::Io(io::Error::new(error.kind(), error.to_string())),
        GhError::Timeout { after } => GhError::Timeout { after: *after },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{RepoKind, RepoSort, ReviewEvent, SortDirection};

    fn octocat() -> User {
        User {
            login: "octocat".into(),
            id: 1,
            name: Some("The Octocat".into()),
            html_url: "https://github.com/octocat".into(),
        }
    }

    fn repo(name: &str) -> Repository {
        Repository {
            name: name.into(),
            full_name: format!("octocat/{name}"),
            default_branch: Some("main".into()),
            private: false,
            archived: Some(false),
            owner: octocat(),
            description: None,
            permissions: None,
        }
    }

    #[tokio::test]
    async fn canned_results_and_call_log() {
        let github = FakeGithub::new()
            .with_user(octocat())
            .with_repositories(vec![repo("a"), repo("b")]);
        // Clones share the log, as clones handed to spawned tasks would.
        let clone = github.clone();

        clone.check_auth().await.unwrap();
        assert_eq!(clone.current_user().await.unwrap(), octocat());
        let owned = RepoListQuery {
            kind: RepoKind::Owner,
            sort: RepoSort::Updated,
            direction: SortDirection::Desc,
        };
        let repos = clone.list_repositories(owned).await.unwrap();
        assert_eq!(repos, [repo("a"), repo("b")]);
        github.current_user().await.unwrap();
        assert_eq!(
            github
                .clone()
                .with_unread_count(Ok(7))
                .unread_notification_count()
                .await
                .unwrap(),
            7
        );

        assert_eq!(
            github.calls(),
            [
                GithubCall::CheckAuth,
                GithubCall::CurrentUser,
                GithubCall::ListRepositories(owned),
                GithubCall::CurrentUser,
                GithubCall::UnreadNotificationCount,
            ]
        );
    }

    #[tokio::test]
    async fn defaults() {
        let github = FakeGithub::new();
        github.check_auth().await.unwrap();
        assert!(github.list_repositories(RepoListQuery::all()).await.unwrap().is_empty());
        assert_eq!(github.unread_notification_count().await.unwrap(), 0);
        assert!(matches!(github.current_user().await, Err(GhError::Command { .. })));
    }

    #[tokio::test]
    async fn canned_errors_repeat() {
        let github = FakeGithub::new()
            .with_auth_error(GhError::NotAuthenticated {
                hostname: None,
                detail: "logged out".into(),
            })
            .with_user_error(GhError::Io(io::Error::new(io::ErrorKind::BrokenPipe, "pipe")))
            .with_repositories_error(GhError::Http {
                status: 502,
                message: "Bad Gateway".into(),
                body: Vec::new(),
            })
            .with_unread_count(Err(GhError::Parse("no status line".into())));
        for _ in 0..2 {
            assert!(matches!(
                github.check_auth().await,
                Err(GhError::NotAuthenticated { detail, .. }) if detail == "logged out"
            ));
            assert!(matches!(
                github.current_user().await,
                Err(GhError::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe && e.to_string() == "pipe"
            ));
            assert!(matches!(
                github.list_repositories(RepoListQuery::all()).await,
                Err(GhError::Http { status: 502, .. })
            ));
            assert!(matches!(
                github.unread_notification_count().await,
                Err(GhError::Parse(_))
            ));
        }
        assert_eq!(github.calls().len(), 8);
    }

    fn issue_item(number: u64) -> IssueListItem {
        let item = serde_json::json!({
            "id": number,
            "number": number,
            "locked": false,
            "state": "open",
            "title": "An issue",
            "user": {"login": "octocat", "id": 1, "html_url": "https://github.com/octocat"},
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "comments_url": "https://api.github.com/repos/octocat/a/issues/1/comments",
            "html_url": "https://github.com/octocat/a/issues/1",
        });
        IssueListItem::from_api_item(item, &repo("a")).unwrap()
    }

    #[tokio::test]
    async fn issue_pages_by_repo_and_page() {
        use crate::config::{IssueOwnerFilter, IssueStateFilter};

        let github = FakeGithub::new()
            .with_issue_page("octocat/a", 1, Ok(vec![issue_item(2), issue_item(1)]))
            .with_issue_page(
                "octocat/a",
                2,
                Err(GhError::Http {
                    status: 500,
                    message: "Server Error".into(),
                    body: Vec::new(),
                }),
            );
        let first = IssueListQuery::first_page(IssueStateFilter::Open, IssueOwnerFilter::Mine);
        let second = IssueListQuery { page: 2, ..first };

        let page = github.list_issues(&repo("a"), first).await.unwrap();
        assert_eq!(page, [issue_item(2), issue_item(1)]);
        assert!(matches!(
            github.list_issues(&repo("a"), second).await,
            Err(GhError::Http { status: 500, .. })
        ));
        // Unset pages and repositories are empty.
        assert!(
            github
                .list_issues(&repo("a"), IssueListQuery { page: 3, ..first })
                .await
                .unwrap()
                .is_empty()
        );
        assert!(github.list_issues(&repo("b"), first).await.unwrap().is_empty());

        assert_eq!(
            github.calls()[..2],
            [
                GithubCall::ListIssues {
                    repo: repo("a"),
                    query: first
                },
                GithubCall::ListIssues {
                    repo: repo("a"),
                    query: second
                },
            ]
        );
        assert_eq!(github.calls().len(), 4);
    }

    #[tokio::test]
    async fn readmes_by_repo() {
        let github = FakeGithub::new()
            .with_readme("octocat/a", Ok(Some("# A\n".into())))
            .with_readme(
                "octocat/b",
                Err(GhError::Timeout {
                    after: Duration::from_secs(1),
                }),
            );
        assert_eq!(github.repo_readme("octocat/a").await.unwrap().as_deref(), Some("# A\n"));
        assert!(matches!(
            github.repo_readme("octocat/b").await,
            Err(GhError::Timeout { .. })
        ));
        assert_eq!(github.repo_readme("octocat/c").await.unwrap(), None);
        assert_eq!(github.calls()[0], GithubCall::RepoReadme("octocat/a".into()));
    }

    #[tokio::test]
    async fn pull_requests_by_repo_and_number() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let pr = FullPullRequest::from_api(fixture["response"]["body"].clone(), &repo("a")).unwrap();
        let github = FakeGithub::new()
            .with_pull_request("octocat/a", 5, Ok(pr.clone()))
            .with_pull_request("octocat/a", 2, Err(GhError::NotInstalled));
        assert_eq!(github.pull_request(&repo("a"), 5).await.unwrap(), pr);
        assert!(matches!(
            github.pull_request(&repo("a"), 2).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(
            github.pull_request(&repo("b"), 5).await,
            Err(GhError::Http { status: 404, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::PullRequest {
                full_name: "octocat/a".into(),
                number: 5
            }
        );
        assert_eq!(github.pull_request_uncached(&repo("a"), 5).await.unwrap(), pr);
        assert_eq!(
            github.calls()[3],
            GithubCall::PullRequestUncached {
                full_name: "octocat/a".into(),
                number: 5
            }
        );
    }

    #[tokio::test]
    async fn reviews_by_repo_and_number() {
        use crate::models::ReviewState;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/5/GET.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let pr = FullPullRequest::from_api(fixture["response"]["body"].clone(), &repo("a")).unwrap();
        let other = FullPullRequest::from_api(fixture["response"]["body"].clone(), &repo("b")).unwrap();
        let review = Review {
            id: 1,
            user: octocat(),
            body: "ok".into(),
            state: ReviewState::Approved,
            submitted_at: None,
            comments: Vec::new(),
        };
        let github = FakeGithub::new().with_pull_request_reviews("octocat/a", 5, Ok(vec![review.clone()]));
        assert_eq!(github.pull_request_reviews(&pr).await.unwrap(), [review]);
        assert!(github.pull_request_reviews(&other).await.unwrap().is_empty());
        assert_eq!(
            github.calls()[0],
            GithubCall::PullRequestReviews {
                full_name: "octocat/a".into(),
                number: 5
            }
        );
        let failing = FakeGithub::new().with_pull_request_reviews("octocat/a", 5, Err(GhError::NotInstalled));
        assert!(matches!(
            failing.pull_request_reviews(&pr).await,
            Err(GhError::NotInstalled)
        ));
    }

    #[tokio::test]
    async fn issues_by_repo_and_number() {
        let github = FakeGithub::new()
            .with_issue("octocat/a", 3, Ok(issue_item(3)))
            .with_issue("octocat/a", 2, Err(GhError::NotInstalled));
        assert_eq!(github.issue(&repo("a"), 3).await.unwrap(), issue_item(3));
        assert!(matches!(github.issue(&repo("a"), 2).await, Err(GhError::NotInstalled)));
        assert!(matches!(
            github.issue(&repo("b"), 3).await,
            Err(GhError::Http { status: 404, .. })
        ));
        assert_eq!(github.calls()[0], GithubCall::Issue(3));
    }

    #[tokio::test]
    async fn run_jobs_by_run_id() {
        let job: WorkflowJob = serde_json::from_value(serde_json::json!({
            "id": 7, "run_id": 1, "name": "build", "status": "queued", "html_url": ""
        }))
        .unwrap();
        let github = FakeGithub::new()
            .with_run_jobs(1, Ok(vec![job.clone()]))
            .with_run_jobs(2, Err(GhError::NotInstalled));
        assert_eq!(github.workflow_run_jobs(&repo("a"), 1).await.unwrap(), [job]);
        assert!(matches!(
            github.workflow_run_jobs(&repo("a"), 2).await,
            Err(GhError::NotInstalled)
        ));
        assert!(github.workflow_run_jobs(&repo("a"), 3).await.unwrap().is_empty());
        assert_eq!(github.calls()[0], GithubCall::WorkflowRunJobs(1));
    }

    #[tokio::test]
    async fn create_issue_records_the_call_and_answers_the_canned_issue() {
        let github = FakeGithub::new();
        let new = NewIssue {
            title: "T".into(),
            body: String::new(),
        };
        assert!(matches!(
            github.create_issue(&repo("a"), &new).await,
            Err(GhError::Http { status: 422, .. })
        ));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/issue-write/repos/parity-bot/issue-fixture/issues/POST.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let created = Issue::from_api(fixture["response"]["body"].clone(), &repo("a")).unwrap();
        let github = github.with_created_issue(Ok(created.clone()));
        assert_eq!(github.create_issue(&repo("a"), &new).await.unwrap(), created);
        assert_eq!(
            github.calls(),
            [
                GithubCall::CreateIssue {
                    full_name: "octocat/a".into(),
                    issue: new.clone(),
                },
                GithubCall::CreateIssue {
                    full_name: "octocat/a".into(),
                    issue: new,
                },
            ]
        );
    }

    #[tokio::test]
    async fn update_issue_records_the_call_and_answers_the_canned_issue() {
        let github = FakeGithub::new();
        let update = IssueUpdate {
            title: "T".into(),
            body: String::new(),
            state: None,
        };
        assert!(matches!(
            github.update_issue(&repo("a"), 12, &update).await,
            Err(GhError::Http { status: 422, .. })
        ));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/issue-write/repos/parity-bot/issue-fixture/issues/12/PATCH.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let updated = Issue::from_api(fixture["response"]["body"].clone(), &repo("a")).unwrap();
        let github = github.with_updated_issue(Ok(updated.clone()));
        assert_eq!(github.update_issue(&repo("a"), 12, &update).await.unwrap(), updated);
        assert_eq!(
            github.calls()[1],
            GithubCall::UpdateIssue {
                full_name: "octocat/a".into(),
                number: 12,
                update,
            }
        );
    }

    #[tokio::test]
    async fn create_issue_comment_answers_by_body_and_records_the_call() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/issue-write/repos/parity-bot/issue-fixture/issues/12/comments/POST.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let posted: IssueComment = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
        let github = FakeGithub::new().with_posted_comment("hi", Ok(posted.clone()));
        assert_eq!(github.create_issue_comment(&repo("a"), 12, "hi").await.unwrap(), posted);
        assert!(matches!(
            github.create_issue_comment(&repo("a"), 12, "").await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::CreateIssueComment {
                full_name: "octocat/a".into(),
                number: 12,
                body: "hi".into(),
            }
        );
    }

    #[tokio::test]
    async fn reply_to_review_comment_answers_by_body_and_records_the_call() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/pr-write/repos/parity-bot/pr-details/pulls/5/comments/9001/replies/POST.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let reply: ReviewComment = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
        let github = FakeGithub::new().with_review_reply("yes", Ok(reply.clone()));
        assert_eq!(
            github
                .reply_to_review_comment(&repo("a"), 5, 9001, "yes")
                .await
                .unwrap(),
            reply
        );
        assert!(matches!(
            github.reply_to_review_comment(&repo("a"), 5, 9001, "no").await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::ReplyToReviewComment {
                full_name: "octocat/a".into(),
                pr_number: 5,
                comment_id: 9001,
                body: "yes".into(),
            }
        );
    }

    #[tokio::test]
    async fn create_review_answers_by_review_and_records_the_call() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/pr-write/repos/parity-bot/pr-details/pulls/5/reviews/POST.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let created: Review = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
        let review = NewReview {
            event: ReviewEvent::Approve,
            commit_id: "abc".into(),
            body: "ok".into(),
            comments: Vec::new(),
        };
        let github = FakeGithub::new().with_created_review(review.clone(), Ok(created.clone()));
        assert_eq!(github.create_review(&repo("a"), 5, &review).await.unwrap(), created);
        let other = NewReview {
            event: ReviewEvent::Comment,
            ..review.clone()
        };
        assert!(matches!(
            github.create_review(&repo("a"), 5, &other).await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::CreateReview {
                full_name: "octocat/a".into(),
                number: 5,
                review,
            }
        );
    }

    #[tokio::test]
    async fn add_issue_reaction_answers_by_kind_and_records_the_call() {
        let github = FakeGithub::new()
            .with_added_reaction(ReactionKind::Heart, Ok(true))
            .with_added_reaction(ReactionKind::Rocket, Ok(false));
        assert!(
            github
                .add_issue_reaction(&repo("a"), 5, ReactionKind::Heart)
                .await
                .unwrap()
        );
        assert!(
            !github
                .add_issue_reaction(&repo("a"), 5, ReactionKind::Rocket)
                .await
                .unwrap()
        );
        assert!(matches!(
            github.add_issue_reaction(&repo("a"), 5, ReactionKind::Eyes).await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::AddIssueReaction {
                full_name: "octocat/a".into(),
                number: 5,
                kind: ReactionKind::Heart,
            }
        );
    }

    #[tokio::test]
    async fn merge_pull_request_answers_by_number_and_records_the_call() {
        let merged = MergeResult {
            merged: true,
            message: "Pull Request successfully merged".into(),
            sha: Some("5eed".into()),
        };
        let github = FakeGithub::new().with_merge(5, Ok(merged.clone()));
        let request = MergeRequest {
            merge_method: crate::config::MergeMethod::Rebase,
            sha: "c0ffee".into(),
        };
        assert_eq!(
            github.merge_pull_request(&repo("a"), 5, &request).await.unwrap(),
            merged
        );
        assert!(matches!(
            github.merge_pull_request(&repo("a"), 4, &request).await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::MergePullRequest {
                full_name: "octocat/a".into(),
                number: 5,
                request,
            }
        );
    }

    #[tokio::test]
    async fn branches_by_repo_and_dispatches_by_workflow_and_ref() {
        let main = Branch {
            name: "main".into(),
            commit: crate::models::CommitRef {
                sha: "a1".into(),
                url: String::new(),
            },
            protected: false,
        };
        let github = FakeGithub::new()
            .with_branches("octocat/a", Ok(vec![main.clone()]))
            .with_branches("octocat/b", Err(GhError::NotInstalled))
            .with_dispatch(7, "main", Ok(()));
        assert_eq!(github.list_branches(&repo("a")).await.unwrap(), [main]);
        assert!(github.list_branches(&repo("b")).await.is_err());
        assert!(github.list_branches(&repo("c")).await.unwrap().is_empty());
        assert!(github.dispatch_workflow(&repo("a"), 7, "main").await.is_ok());
        assert!(matches!(
            github.dispatch_workflow(&repo("a"), 7, "release").await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(
            github.calls()[3],
            GithubCall::DispatchWorkflow {
                full_name: "octocat/a".into(),
                workflow_id: 7,
                git_ref: "main".into(),
            }
        );
        assert_eq!(github.calls()[0], GithubCall::ListBranches("octocat/a".into()));
        assert_eq!(github.list_branches_uncached(&repo("a")).await.unwrap().len(), 1);
        assert_eq!(
            github.calls().last(),
            Some(&GithubCall::ListBranchesUncached("octocat/a".into()))
        );
    }

    #[tokio::test]
    async fn create_pull_request_records_the_call_and_answers_the_canned_pull_request() {
        let pr = NewPullRequest::new(&repo("a"), "T", false, "main", "feature", "");
        let github = FakeGithub::new();
        assert!(matches!(
            github.create_pull_request(&repo("a"), &pr).await,
            Err(GhError::Http { status: 422, .. })
        ));
        let github = github.with_created_pull_request(Err(GhError::NotInstalled));
        assert!(matches!(
            github.create_pull_request(&repo("a"), &pr).await,
            Err(GhError::NotInstalled)
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::CreatePullRequest {
                full_name: "octocat/a".into(),
                pr,
            }
        );
    }

    #[tokio::test]
    async fn reruns_and_runs_by_run_id() {
        let run = crate::models::WorkflowRun {
            id: 42,
            ..serde_json::from_value(serde_json::json!({
                "id": 0, "name": "CI", "display_title": "t", "path": "p", "run_number": 1, "head_branch": "main",
                "head_sha": "0", "status": "completed", "event": "push",
                "actor": {"login": "o", "id": 1, "html_url": ""},
                "triggering_actor": {"login": "o", "id": 1, "html_url": ""},
                "repository": serde_json::to_value(repo("a")).unwrap(),
                "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
                "html_url": "", "jobs_url": "", "logs_url": ""
            }))
            .unwrap()
        };
        let github = FakeGithub::new()
            .with_rerun(42, true, Ok(()))
            .with_rerun(42, false, Err(GhError::NotInstalled))
            .with_workflow_run(42, Ok(run.clone()));
        assert!(github.rerun_failed_jobs(&repo("a"), 42).await.is_ok());
        assert!(matches!(
            github.rerun_workflow_run(&repo("a"), 42).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(
            github.rerun_failed_jobs(&repo("a"), 41).await,
            Err(GhError::Http { status: 422, .. })
        ));
        assert_eq!(github.workflow_run(&repo("a"), 42).await.unwrap(), run);
        assert!(matches!(
            github.workflow_run(&repo("a"), 41).await,
            Err(GhError::Http { status: 404, .. })
        ));
        assert_eq!(
            github.calls()[..2],
            [GithubCall::RerunFailedJobs(42), GithubCall::RerunWorkflowRun(42)]
        );
    }

    #[tokio::test]
    async fn job_logs_by_job_id() {
        let github = FakeGithub::new()
            .with_job_logs(1, Ok(Some("line\n".into())))
            .with_job_logs(2, Ok(None))
            .with_job_logs(3, Err(GhError::NotInstalled));
        assert_eq!(github.job_logs(&repo("a"), 1).await.unwrap().as_deref(), Some("line\n"));
        assert_eq!(github.job_logs(&repo("a"), 2).await.unwrap(), None);
        assert!(matches!(
            github.job_logs(&repo("a"), 3).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(
            github.job_logs(&repo("a"), 4).await,
            Err(GhError::Http { status: 404, .. })
        ));
        assert_eq!(github.calls()[0], GithubCall::JobLogs(1));
    }

    #[tokio::test]
    async fn repositories_by_name() {
        let github = FakeGithub::new()
            .with_repository("octocat/a", Ok(Some(repo("a"))))
            .with_repository("octocat/b", Err(GhError::NotInstalled));
        assert_eq!(github.repository("octocat/a").await.unwrap(), Some(repo("a")));
        assert!(matches!(
            github.repository("octocat/b").await,
            Err(GhError::NotInstalled)
        ));
        assert_eq!(github.repository("octocat/c").await.unwrap(), None);
        assert_eq!(github.calls()[0], GithubCall::Repository("octocat/a".into()));
    }

    #[tokio::test]
    async fn workflow_and_run_pages_by_repo_and_page() {
        let github = FakeGithub::new().with_run_page(
            "octocat/a",
            2,
            Err(GhError::Http {
                status: 500,
                message: "Server Error".into(),
                body: Vec::new(),
            }),
        );
        let second = PageQuery {
            page: 2,
            ..PageQuery::default()
        };
        // Unset pages are empty.
        assert!(
            github
                .list_workflows(&repo("a"), PageQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            github
                .list_workflow_runs(&repo("a"), PageQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            github.list_workflow_runs(&repo("a"), second).await,
            Err(GhError::Http { status: 500, .. })
        ));
        assert_eq!(
            github.calls(),
            [
                GithubCall::ListWorkflows {
                    repo: repo("a"),
                    page: PageQuery::default()
                },
                GithubCall::ListWorkflowRuns {
                    repo: repo("a"),
                    page: PageQuery::default()
                },
                GithubCall::ListWorkflowRuns {
                    repo: repo("a"),
                    page: second
                },
            ]
        );
    }

    #[tokio::test]
    async fn pull_request_diffs_by_repo_and_number() {
        let github = FakeGithub::new()
            .with_pull_request_diff("octocat/a", 5, Ok("diff --git a/f b/f\n".into()))
            .with_pull_request_diff("octocat/a", 4, Err(DiffFetchError::TooLarge))
            .with_pull_request_diff("octocat/a", 6, Err(DiffFetchError::Gh(GhError::NotInstalled)));
        for _ in 0..2 {
            assert_eq!(
                github.pull_request_diff(&repo("a"), 5).await.unwrap(),
                "diff --git a/f b/f\n"
            );
            assert!(matches!(
                github.pull_request_diff(&repo("a"), 4).await,
                Err(DiffFetchError::TooLarge)
            ));
            assert!(matches!(
                github.pull_request_diff(&repo("a"), 6).await,
                Err(DiffFetchError::Gh(GhError::NotInstalled))
            ));
        }
        // Unset pull requests have no diff.
        assert!(matches!(
            github.pull_request_diff(&repo("b"), 5).await,
            Err(DiffFetchError::NotFound)
        ));
        assert_eq!(
            github.calls()[0],
            GithubCall::PullRequestDiff {
                repo: repo("a"),
                number: 5
            }
        );
        assert_eq!(github.calls().len(), 7);
    }

    #[test]
    fn duplicate_keeps_every_message() {
        let decode = serde_json::from_str::<u32>("\"x\"").unwrap_err();
        let errors = [
            GhError::NotInstalled,
            GhError::NotAuthenticated {
                hostname: Some("ghe.example.com".into()),
                detail: String::new(),
            },
            GhError::Http {
                status: 404,
                message: "Not Found".into(),
                body: b"{}".to_vec(),
            },
            GhError::Command {
                code: Some(1),
                stderr: "boom".into(),
            },
            GhError::Parse("no status line".into()),
            GhError::Decode(decode),
            GhError::Io(io::Error::other("broken")),
            GhError::Timeout {
                after: Duration::from_secs(30),
            },
        ];
        for error in &errors {
            assert_eq!(duplicate(error).to_string(), error.to_string());
        }
    }

    #[tokio::test]
    async fn checks_and_reactions_by_repo_and_key() {
        use crate::models::{CheckRunStatus, Reaction, ReactionKind};

        let run = CheckRun {
            id: 1,
            name: "build".into(),
            status: CheckRunStatus::Queued,
            conclusion: None,
            html_url: None,
        };
        let status = CombinedStatus {
            state: CheckState::Success,
            statuses: Vec::new(),
        };
        let reactions: Reactions = [Reaction {
            kind: ReactionKind::Eyes,
            user: octocat(),
        }]
        .into_iter()
        .collect();
        let github = FakeGithub::new()
            .with_commit_status("octocat/a", "abc", Ok(status.clone()))
            .with_commit_status("octocat/a", "bad", Err(GhError::NotInstalled))
            .with_check_runs("octocat/a", "abc", Ok(vec![run.clone()]))
            .with_issue_reactions("octocat/a", 5, Ok(reactions.clone()))
            .with_issue_reactions("octocat/a", 6, Err(GhError::NotInstalled));
        assert_eq!(github.commit_status(&repo("a"), "abc").await.unwrap(), status);
        assert!(github.commit_status(&repo("a"), "bad").await.is_err());
        let unset = github.commit_status(&repo("b"), "abc").await.unwrap();
        assert_eq!((unset.state, unset.statuses.len()), (CheckState::Pending, 0));
        assert_eq!(github.check_runs(&repo("a"), "abc").await.unwrap(), [run]);
        assert!(github.check_runs(&repo("a"), "def").await.unwrap().is_empty());
        assert_eq!(github.issue_reactions(&repo("a"), 5).await.unwrap(), reactions);
        assert!(github.issue_reactions(&repo("a"), 6).await.is_err());
        assert!(github.issue_reactions(&repo("a"), 7).await.unwrap().is_empty());
        assert_eq!(
            github.calls()[..3],
            [
                GithubCall::CommitStatus {
                    full_name: "octocat/a".into(),
                    sha: "abc".into()
                },
                GithubCall::CommitStatus {
                    full_name: "octocat/a".into(),
                    sha: "bad".into()
                },
                GithubCall::CommitStatus {
                    full_name: "octocat/b".into(),
                    sha: "abc".into()
                },
            ]
        );
        assert!(github.calls().contains(&GithubCall::IssueReactions {
            full_name: "octocat/a".into(),
            number: 7
        }));
    }

    #[tokio::test]
    async fn comments_and_comment_reactions_by_repo_and_key() {
        use crate::models::{Issue, IssueState, Reaction, ReactionKind};

        let issue = |name: &str, number: u64| Issue {
            id: number,
            number,
            locked: false,
            state: IssueState::Open,
            title: "t".into(),
            body: None,
            user: octocat(),
            created_at: jiff::Timestamp::UNIX_EPOCH,
            updated_at: jiff::Timestamp::UNIX_EPOCH,
            closed_at: None,
            closed_by: None,
            assignee: None,
            assignees: None,
            comments_url: String::new(),
            html_url: String::new(),
            repo: repo(name),
        };
        let comment = IssueComment {
            id: 70,
            body: "hi".into(),
            user: None,
            created_at: jiff::Timestamp::UNIX_EPOCH,
            updated_at: jiff::Timestamp::UNIX_EPOCH,
            author_association: "NONE".into(),
        };
        let reactions: Reactions = [Reaction {
            kind: ReactionKind::Rocket,
            user: octocat(),
        }]
        .into_iter()
        .collect();
        let github = FakeGithub::new()
            .with_issue_comments("octocat/a", 5, Ok(vec![comment.clone()]))
            .with_issue_comments("octocat/a", 6, Err(GhError::NotInstalled))
            .with_comment_reactions("octocat/a", CommentRef::Review(70), Ok(reactions.clone()))
            .with_comment_reactions("octocat/a", CommentRef::Issue(71), Err(GhError::NotInstalled));
        assert_eq!(github.issue_comments(&issue("a", 5)).await.unwrap(), [comment]);
        assert!(github.issue_comments(&issue("a", 6)).await.is_err());
        assert!(github.issue_comments(&issue("b", 5)).await.unwrap().is_empty());
        assert_eq!(
            github
                .comment_reactions(&repo("a"), CommentRef::Review(70))
                .await
                .unwrap(),
            reactions
        );
        // The same id as an issue comment is a different comment.
        assert!(
            github
                .comment_reactions(&repo("a"), CommentRef::Issue(70))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            github
                .comment_reactions(&repo("a"), CommentRef::Issue(71))
                .await
                .is_err()
        );
        assert_eq!(
            github.calls()[..2],
            [
                GithubCall::IssueComments {
                    full_name: "octocat/a".into(),
                    number: 5
                },
                GithubCall::IssueComments {
                    full_name: "octocat/a".into(),
                    number: 6
                },
            ]
        );
        assert!(github.calls().contains(&GithubCall::CommentReactions {
            full_name: "octocat/a".into(),
            comment: CommentRef::Issue(71)
        }));
    }
}
