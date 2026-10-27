//! Lookup pull request (`pull_request.lookup`, `O` on the Pull Requests table): a number, looked up with
//! `GET /repos/<owner>/<repo>/pulls/<number>` in the selected repo. Python's `LookupPullRequestModal`.

use lazy_github_core::models::FullPullRequest;

use crate::component::Cx;
use crate::effect::{GithubRequest, GithubResponse};
use crate::modal::{LookupKind, LookupTarget, number};
use crate::toast::Toast;

/// The toast for a lookup opened before any repo was selected (Py bug B12: Python fails an `assert` instead).
pub(super) fn select_repository_first() -> Toast {
    Toast::error("Please select a repository first!")
}

/// The pull request lookup.
#[derive(Debug)]
pub(super) enum PrLookup {}

impl LookupKind for PrLookup {
    type Found = FullPullRequest;
    const TARGET: LookupTarget = LookupTarget::PullRequest;
    const HEADING: &'static str = "Search for a pull request by number:";
    const LABEL: &'static str = "Pull Request Number:";
    const PLACEHOLDER: &'static str = "Pull request number";
    const FILTER: Option<fn(&str) -> bool> = Some(number::can_become_number);

    fn is_valid(text: &str) -> bool {
        number::is_number(text)
    }

    /// Python's `int()` reads the number, so `+5` and `1_0` are numbers and `1.5` isn't. A negative number can't be a
    /// pull request: Python sends it and gets a 404, here it's "Unknown PR" without the round trip.
    fn request(text: &str, cx: &Cx<'_>) -> Result<GithubRequest, Toast> {
        // The opener checks this first, so the dialog never opens without a repo.
        let repo = cx.current_repo().ok_or_else(select_repository_first)?;
        let number = number::parse_int(text)
            .ok_or_else(|| Toast::error("Must enter a valid pull request number!").with_title("Invalid PR Number"))?;
        let number = u64::try_from(number).map_err(|_| unknown_pr())?;
        Ok(GithubRequest::PullRequest {
            repo: Box::new(repo.clone()),
            number,
        })
    }

    fn found(response: GithubResponse) -> Result<Result<FullPullRequest, Toast>, GithubResponse> {
        match response {
            GithubResponse::PullRequest(Ok(pr)) => Ok(Ok(*pr)),
            GithubResponse::PullRequest(Err(error)) => {
                tracing::debug!("Looking up a pull request failed: {error}");
                Ok(Err(unknown_pr()))
            }
            other => Err(other),
        }
    }
}

fn unknown_pr() -> Toast {
    Toast::error("Could not find pull request!").with_title("Unknown PR")
}

#[cfg(test)]
mod tests;
