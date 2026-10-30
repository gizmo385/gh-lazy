//! Whether `gh` itself works mid-session: installed, logged in and answering in time.
//!
//! Each request still fails with its own error, as in Python. On top of that, the first request to find `gh`
//! missing, logged out or hung shows one toast saying so, and while `gh` is missing or logged out the notifications
//! poll skips its ticks, until any request succeeds again (decision `gh-fails-mid-session`). Python has none of
//! this: a missing `gh` crashes the request's worker, and the poll toasts "please login" every minute.

use lazy_github_core::gh::GhError;
use lazy_github_core::github::hostname_for_base_url;

use crate::component::Cx;
use crate::effect::GithubResponse;
use crate::toast::Toast;

/// The toasts' title: the generic one for a `gh` failure with no toast of its own.
pub const TOAST_TITLE: &str = "API Error";

/// What's wrong with `gh`, as the last failed request found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outage {
    /// `gh` couldn't be run ([`GhError::NotInstalled`]).
    NotInstalled,
    /// `gh` isn't logged in ([`GhError::is_unauthorized`]).
    LoggedOut,
    /// `gh` ran past its time limit ([`GhError::Timeout`]).
    TimedOut,
}

impl Outage {
    fn of(error: &GhError) -> Option<Self> {
        match error {
            GhError::NotInstalled => Some(Self::NotInstalled),
            GhError::Timeout { .. } => Some(Self::TimedOut),
            error if error.is_unauthorized() => Some(Self::LoggedOut),
            _ => None,
        }
    }
}

/// The app's view of `gh`. `App` owns one for the life of the app.
#[derive(Debug, Default)]
pub struct GhHealth {
    outage: Option<Outage>,
}

impl GhHealth {
    /// A response arrived on the main screen. A failure that says `gh` is missing, logged out or hung toasts its
    /// message, unless the app already knew; a success ends the outage.
    ///
    /// The notifications poll's requests aren't toasted here: the poll reports its own failures, as in Python.
    pub fn observe(&mut self, response: &GithubResponse, cx: &mut Cx<'_>) {
        let Some(error) = response.error() else {
            // `gh --version` works logged out, so it only shows that `gh` runs.
            let runs_only = matches!(response, GithubResponse::GhVersion(_));
            if !(runs_only && self.outage == Some(Outage::LoggedOut)) {
                self.recover();
            }
            return;
        };
        let Some(outage) = Outage::of(error) else {
            return;
        };
        if self.outage == Some(outage) {
            return;
        }
        self.outage = Some(outage);
        if !matches!(
            response,
            GithubResponse::CheckAuth(_) | GithubResponse::UnreadNotificationCount(_)
        ) {
            cx.notify(Toast::error(message(error, cx)).with_title(TOAST_TITLE));
        }
    }

    fn recover(&mut self) {
        if let Some(outage) = self.outage.take() {
            tracing::debug!("gh works again after {outage:?}");
        }
    }

    /// Whether `gh` is missing or logged out, as far as the app knows, so the notifications poll should wait. A
    /// timeout doesn't pause it: the next tick may well get through.
    pub fn is_unusable(&self) -> bool {
        matches!(self.outage, Some(Outage::NotInstalled | Outage::LoggedOut))
    }
}

/// The toast's text for `error`: its own, except that an HTTP 401 says to log in, as `gh` exiting 4 does.
fn message(error: &GhError, cx: &Cx<'_>) -> String {
    match error {
        GhError::Http { status: 401, .. } => GhError::NotAuthenticated {
            hostname: hostname_for_base_url(&cx.config().api.base_url),
            detail: String::new(),
        }
        .to_string(),
        error => error.to_string(),
    }
}

/// Reports a failure that Python has no toast for because it crashes on it (#323): logs `what` with the error, and
/// toasts error / "API Error" with the error's text. A missing, logged-out or hung `gh` gets no second toast here, as
/// [`GhHealth`] toasts that once.
pub fn report_unhandled(what: &str, error: &GhError, cx: &mut Cx<'_>) {
    tracing::error!("{what}: {error}");
    if Outage::of(error).is_none() {
        cx.notify(Toast::error(error.to_string()).with_title(TOAST_TITLE));
    }
}

#[cfg(test)]
mod tests;
