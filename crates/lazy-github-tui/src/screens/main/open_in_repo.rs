//! Opening something in its repo (#405): the repo is selected, then the pull request or issue opens in Details. A
//! chosen notification opens this way, and so do #394's search hits, so both behave the same. Followed and pasted
//! links keep their own path ([`MainScreen::open_link_subject`]): they resolve before selecting anything, and fall
//! back to the browser.
//!
//! Every stage that waits on GitHub (the repo lookup, the issue, the pull request) shares the screen's one pending
//! selection, so a newer selection cancels whichever stage is in flight and drops its late answer.

use lazy_github_core::gh::GhError;
use lazy_github_core::github::{HitKind, SearchHit};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Notification, RepoName, Repository, SubjectNumber};

use super::{MainAction, MainScreen, pull_request_failed};
use crate::action::Action;
use crate::component::Cx;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::toast::Toast;

/// The toasts after a chosen notification opens: Python's `action_view_notifications`, and "Opening issue for
/// notification" for the issue subjects it doesn't open (decision `notification-opens-issue`).
const OPENING_PULL_REQUEST: &str = "Opening pull request for notification";
const OPENING_ISSUE: &str = "Opening issue for notification";
const OPENING_REPOSITORY: &str = "Opening repository for notification";

/// A request to select a repo, then open an item in it: [`MainAction::OpenInRepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenInRepo {
    /// The repo to select.
    pub repo: RepoRef,
    /// What opens once it's selected.
    pub item: RepoItem,
    /// What asked, which decides the toasts.
    pub origin: OpenOrigin,
}

impl OpenInRepo {
    /// A chosen notification: its repository, and the pull request or issue its subject URL numbers, else the repo.
    pub fn notification(notification: &Notification) -> Self {
        let item = match notification.subject.number() {
            Some(SubjectNumber::PullRequest(number)) => RepoItem::PullRequest(number),
            Some(SubjectNumber::Issue(number)) => RepoItem::Issue(number),
            None => RepoItem::Repo,
        };
        Self {
            repo: RepoRef::Known(Box::new(notification.repository.clone())),
            item,
            origin: OpenOrigin::Notification,
        }
    }

    /// A hit chosen in the Search modal: its repo by name, and its pull request or issue.
    pub fn search_hit(hit: &SearchHit) -> Self {
        let item = match hit.kind {
            HitKind::PullRequest { .. } => RepoItem::PullRequest(hit.number),
            HitKind::Issue => RepoItem::Issue(hit.number),
        };
        Self {
            repo: RepoRef::Named(hit.repo.clone()),
            item,
            origin: OpenOrigin::SearchHit,
        }
    }
}

/// The repo an [`OpenInRepo`] selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoRef {
    /// The whole repository, as a notification carries it: selected at once.
    Known(Box<Repository>),
    /// Only its name, as a search hit carries it. The selected repo is used when it has that name (ignoring case);
    /// any other is looked up first, and nothing changes until it arrives.
    Named(RepoName),
}

/// What opens in the selected repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoItem {
    /// Nothing more: the repo's overview stays.
    Repo,
    /// The pull request with this number.
    PullRequest(u64),
    /// The issue with this number. GitHub numbers pull requests and issues together, so one that turns out to be a
    /// pull request opens as one.
    Issue(u64),
}

impl RepoItem {
    fn number(self) -> Option<u64> {
        match self {
            RepoItem::Repo => None,
            RepoItem::PullRequest(number) | RepoItem::Issue(number) => Some(number),
        }
    }
}

/// What asked to open something in its repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenOrigin {
    /// A notification chosen in the Notifications modal: every outcome toasts what opened, as information, and a
    /// failure quietly leaves the repo open.
    Notification,
    /// A hit in the Search modal (#394): nothing toasts when it opens, and a failure is an error.
    SearchHit,
}

/// What showed in Details, for [`OpenOrigin::opened`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opened {
    PullRequest,
    Issue,
}

impl OpenOrigin {
    /// The toast once the item shows, if any.
    fn opened(self, opened: Opened) -> Option<Toast> {
        match (self, opened) {
            (OpenOrigin::Notification, Opened::PullRequest) => Some(Toast::info(OPENING_PULL_REQUEST)),
            (OpenOrigin::Notification, Opened::Issue) => Some(Toast::info(OPENING_ISSUE)),
            (OpenOrigin::SearchHit, _) => None,
        }
    }

    /// The toast once only the repo opened, as asked, if any.
    fn repo_only(self) -> Option<Toast> {
        match self {
            OpenOrigin::Notification => Some(Toast::info(OPENING_REPOSITORY)),
            OpenOrigin::SearchHit => None,
        }
    }

    /// The toast when the item, or the repo it's in, couldn't be fetched. A notification's says only the repo opened,
    /// as Python's `extract_notification_subject` swallows the failure; the error goes to the debug log either way.
    fn failed(self, full_name: &str, item: RepoItem, error: &dyn std::fmt::Display) -> Toast {
        tracing::debug!("Opening {item:?} in {full_name} failed: {error}");
        match self {
            OpenOrigin::Notification => Toast::info(OPENING_REPOSITORY),
            OpenOrigin::SearchHit => match item.number() {
                Some(number) => Toast::error(format!("Could not open {full_name}#{number}")),
                None => Toast::error(format!("Could not open {full_name}")),
            },
        }
    }
}

/// The selection still loading, which a newer one replaces: only the latest is ever shown.
#[derive(Debug)]
pub(super) struct PendingSelection {
    id: RequestId,
    kind: PendingKind,
}

/// What a [`PendingSelection`] waits for.
#[derive(Debug)]
enum PendingKind {
    /// A full pull request. `focus` says whether the tab strip takes focus once it shows.
    PullRequest { focus: bool, origin: PullRequestOrigin },
    /// The repo an [`OpenInRepo`] named, which is selected once it arrives.
    Repo(Box<OpenInRepo>),
    /// An issue an [`OpenInRepo`] opens, from `repo`.
    Issue {
        repo: Box<Repository>,
        number: u64,
        origin: OpenOrigin,
    },
}

/// What asked for a pending pull request, which decides what its answer toasts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PullRequestOrigin {
    /// A table row, or a just-merged pull request: a failure is "Error fetching pull request!".
    Selection,
    /// An [`OpenInRepo`], with the repo's full name and the number, which its failure toast may name.
    Picked {
        origin: OpenOrigin,
        full_name: String,
        number: u64,
    },
}

impl PendingSelection {
    /// The request's id, to cancel it.
    pub(super) fn id(&self) -> RequestId {
        self.id
    }
}

impl MainScreen {
    /// `MainAction::OpenInRepo`: a new selection, so whatever is still loading is cancelled first. The repo is then
    /// selected and the item asked for, in that order, since selecting a repo cancels any pending selection: at once
    /// for a known repo or the selected one, else once its lookup answers.
    pub(super) fn open_in_repo(&mut self, request: OpenInRepo, cx: &mut Cx<'_>) {
        self.cancel_pending_selection(cx);
        let OpenInRepo { repo, item, origin } = request;
        let name = match repo {
            RepoRef::Known(repo) => return select_then_open(repo, item, origin, cx),
            RepoRef::Named(name) => name,
        };
        let current = cx
            .current_repo()
            .filter(|current| name.matches(&current.full_name))
            .cloned();
        if let Some(current) = current {
            select_then_open(Box::new(current), item, origin, cx);
            return;
        }
        let id = cx.request(GithubRequest::Repository(name.to_string()));
        let kind = PendingKind::Repo(Box::new(OpenInRepo {
            repo: RepoRef::Named(name),
            item,
            origin,
        }));
        self.pending = Some(PendingSelection { id, kind });
    }

    /// `MainAction::OpenRepoItem`, once `repo` is selected: a pull request or an issue is fetched as a selection,
    /// which cancels the commit preload the repo selection just started; the repo alone only toasts.
    pub(super) fn open_repo_item(
        &mut self,
        repo: Box<Repository>,
        item: RepoItem,
        origin: OpenOrigin,
        cx: &mut Cx<'_>,
    ) {
        match item {
            RepoItem::Repo => {
                if let Some(toast) = origin.repo_only() {
                    cx.notify(toast);
                }
            }
            RepoItem::PullRequest(number) => {
                self.cancel_pending_selection(cx);
                let full_name = repo.full_name.clone();
                let origin = PullRequestOrigin::Picked {
                    origin,
                    full_name,
                    number,
                };
                self.request_pull_request(repo, number, true, origin, cx);
            }
            RepoItem::Issue(number) => {
                self.cancel_pending_selection(cx);
                let id = cx.request(GithubRequest::Issue {
                    repo: repo.clone(),
                    number,
                });
                let kind = PendingKind::Issue { repo, number, origin };
                self.pending = Some(PendingSelection { id, kind });
            }
        }
    }

    /// Fetches pull request `number` of `repo` as the pending selection.
    pub(super) fn request_pull_request(
        &mut self,
        repo: Box<Repository>,
        number: u64,
        focus: bool,
        origin: PullRequestOrigin,
        cx: &mut Cx<'_>,
    ) {
        let id = cx.request(GithubRequest::PullRequest { repo, number });
        self.pending = Some(PendingSelection {
            id,
            kind: PendingKind::PullRequest { focus, origin },
        });
    }

    /// Takes the answer to the pending selection, and hands back any other response. A pull request answering a
    /// selection since replaced is dropped (nothing else asks for one through the screen); a repo or an issue that
    /// isn't the pending one is handed on.
    pub(super) fn on_pending_selection(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let current = self.pending.as_ref().is_some_and(|pending| pending.id == id);
        match response {
            GithubResponse::PullRequest(_) if !current => {
                tracing::debug!("Dropping the pull request response {id} of a selection no longer current");
                return None;
            }
            GithubResponse::PullRequest(_) | GithubResponse::Issue(_) | GithubResponse::Repository(_) if current => {}
            response => return Some(response),
        }
        let Some(pending) = self.pending.take() else {
            // `current` said there's one.
            return None;
        };
        match (pending.kind, response) {
            (PendingKind::PullRequest { focus, origin }, GithubResponse::PullRequest(result)) => {
                self.on_pull_request(result, focus, origin, cx);
            }
            (PendingKind::Repo(request), GithubResponse::Repository(result)) => on_named_repo(*request, result, cx),
            (PendingKind::Issue { repo, number, origin }, GithubResponse::Issue(result)) => {
                self.on_issue(repo, number, origin, result, cx);
            }
            // A request's response always has its kind, so this doesn't happen.
            (kind, response) => tracing::debug!("Dropping a {response:?} that answered a pending {kind:?}"),
        }
        None
    }

    /// A failure leaves everything as it was, with what its origin toasts.
    fn on_pull_request(
        &mut self,
        result: Result<Box<FullPullRequest>, GhError>,
        focus: bool,
        origin: PullRequestOrigin,
        cx: &mut Cx<'_>,
    ) {
        match (result, origin) {
            (Ok(pr), origin) => {
                self.show_pull_request(*pr, focus, cx);
                if let PullRequestOrigin::Picked { origin, .. } = origin
                    && let Some(toast) = origin.opened(Opened::PullRequest)
                {
                    cx.notify(toast);
                }
            }
            (Err(error), PullRequestOrigin::Selection) => pull_request_failed(&error, cx),
            (
                Err(error),
                PullRequestOrigin::Picked {
                    origin,
                    full_name,
                    number,
                },
            ) => cx.notify(origin.failed(&full_name, RepoItem::PullRequest(number), &error)),
        }
    }

    /// The issue arrived: it shows, or, when the number is a pull request's, that pull request is fetched in its place.
    fn on_issue(
        &mut self,
        repo: Box<Repository>,
        number: u64,
        origin: OpenOrigin,
        result: Result<Box<IssueListItem>, GhError>,
        cx: &mut Cx<'_>,
    ) {
        match result.map(|item| *item) {
            Ok(IssueListItem::Issue(issue)) => {
                self.open_issue(issue, cx);
                if let Some(toast) = origin.opened(Opened::Issue) {
                    cx.notify(toast);
                }
            }
            Ok(IssueListItem::PullRequest(pr)) => {
                let full_name = repo.full_name.clone();
                let number = pr.issue.number;
                let origin = PullRequestOrigin::Picked {
                    origin,
                    full_name,
                    number,
                };
                self.request_pull_request(repo, number, true, origin, cx);
            }
            Err(error) => cx.notify(origin.failed(&repo.full_name, RepoItem::Issue(number), &error)),
        }
    }

    /// A new selection replaces any still loading: the pending stage, the preload's lookup and a refresh's fetch of the
    /// details' subject are cancelled, and their answers, if they come anyway, are dropped. Every selection calls this
    /// first.
    pub(super) fn cancel_pending_selection(&mut self, cx: &mut Cx<'_>) {
        if let Some(pending) = self.pending.take() {
            cx.effect(Effect::Cancel(pending.id()));
        }
        self.preload.cancel(cx);
        self.cancel_details_refresh(cx);
    }
}

/// The named repo arrived: it's selected and its item asked for. A repo that doesn't exist, or a failed lookup,
/// changes nothing.
fn on_named_repo(request: OpenInRepo, result: Result<Option<Box<Repository>>, GhError>, cx: &mut Cx<'_>) {
    let OpenInRepo { repo, item, origin } = request;
    let name = match &repo {
        RepoRef::Named(name) => name.to_string(),
        RepoRef::Known(repo) => repo.full_name.clone(),
    };
    match result {
        Ok(Some(found)) => select_then_open(found, item, origin, cx),
        Ok(None) => cx.notify(origin.failed(&name, item, &"no such repository")),
        Err(error) => cx.notify(origin.failed(&name, item, &error)),
    }
}

/// Selects `repo`, then opens `item` in it: `App` applies the two in order, so the repo selection's cancel comes
/// before the item's request exists.
fn select_then_open(repo: Box<Repository>, item: RepoItem, origin: OpenOrigin, cx: &mut Cx<'_>) {
    cx.dispatch(Action::SelectRepo(repo.clone()));
    cx.dispatch(Action::Main(MainAction::OpenRepoItem { repo, item, origin }));
}
