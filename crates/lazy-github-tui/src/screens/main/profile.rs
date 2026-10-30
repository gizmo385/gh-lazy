//! Fetching a user's profile for the User profile dialog: the profile and their recent repos, asked for together,
//! and the dialog once both have answered. Python's `open_user_profile`.

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{FullUser, Repository};

use super::MainAction;
use crate::action::Action;
use crate::component::Cx;
use crate::effect::{GithubRequest, GithubResponse, RequestId};
use crate::modal::UserProfileDialog;
use crate::toast::Toast;

/// Which table's `u` (or which link) asked for a profile, which decides what a picked repo does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileOrigin {
    /// The repos table: the owner of the highlighted repo.
    Repos,
    /// The Pull Requests table: the author of the highlighted PR.
    PullRequests,
    /// The Issues table: the author of the highlighted issue.
    Issues,
    /// A user link, pasted or followed: the user it points at.
    Link,
}

/// The profile fetches in flight. Each `u` makes its own pair of requests and opens its own dialog.
#[derive(Debug, Default)]
pub(super) struct ProfileFetch {
    pending: Vec<Pending>,
}

/// One fetch: both requests, and what each answered so far.
#[derive(Debug)]
struct Pending {
    login: String,
    origin: ProfileOrigin,
    user_id: RequestId,
    repos_id: RequestId,
    user: Option<Result<Box<FullUser>, GhError>>,
    repos: Option<Result<Vec<Repository>, GhError>>,
}

impl ProfileFetch {
    /// Asks for `login`'s profile and repos at once. There's no indicator, and the app stays usable meanwhile.
    pub(super) fn open(&mut self, login: String, origin: ProfileOrigin, cx: &mut Cx<'_>) {
        let user_id = cx.request(GithubRequest::User(login.clone()));
        let repos_id = cx.request(GithubRequest::UserRepos(login.clone()));
        self.pending.push(Pending {
            login,
            origin,
            user_id,
            repos_id,
            user: None,
            repos: None,
        });
    }

    /// Takes the answers to the fetches' requests, and hands back any other. Once a fetch has both, it opens the
    /// dialog or toasts why it can't.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let Some(index) = self.pending.iter().position(|p| p.user_id == id || p.repos_id == id) else {
            return Some(response);
        };
        let pending = &mut self.pending[index];
        match response {
            GithubResponse::User(result) if pending.user_id == id && pending.user.is_none() => {
                pending.user = Some(result);
            }
            GithubResponse::UserRepos(result) if pending.repos_id == id && pending.repos.is_none() => {
                pending.repos = Some(result);
            }
            other => return Some(other),
        }
        if pending.user.is_some() && pending.repos.is_some() {
            self.pending.remove(index).finish(cx);
        }
        None
    }
}

impl Pending {
    /// Both answers are in. A body that doesn't decode fails the whole fetch; a profile GitHub doesn't give is an
    /// unknown user; a repos list it doesn't give is an empty one.
    fn finish(self, cx: &mut Cx<'_>) {
        let (Some(user), Some(repos)) = (self.user, self.repos) else {
            return;
        };
        let login = self.login;
        let decode_error = [user.as_ref().err(), repos.as_ref().err()]
            .into_iter()
            .flatten()
            .find(|error| matches!(error, GhError::Decode(_)));
        if let Some(error) = decode_error {
            tracing::error!("Error fetching user profile");
            tracing::debug!("Fetching the profile of {login} failed: {error}");
            cx.notify(Toast::error(format!("Failed to load profile for @{login}")));
            return;
        }
        let user = match user {
            Ok(user) => user,
            Err(error) => {
                tracing::debug!("Fetching the profile of {login} failed: {error}");
                cx.notify(Toast::error(format!("Could not find user @{login}")));
                return;
            }
        };
        let repos = repos.unwrap_or_else(|error| {
            tracing::debug!("Fetching the repos of {login} failed: {error}");
            Vec::new()
        });
        let origin = self.origin;
        cx.open_dialog(UserProfileDialog::new(*user, repos), move |picked| {
            Action::Main(MainAction::ProfilePicked(origin, picked.map(Box::new)))
        });
    }
}

#[cfg(test)]
mod tests;
