//! Lookup issue (`issue.lookup`, `O` on the Issues table): a number, looked up with
//! `GET /repos/<owner>/<repo>/issues/<number>` in the selected repo. Python's `LookupIssueModal`.

use lazy_github_core::models::IssueListItem;

use super::pr_lookup::select_repository_first;
use crate::component::Cx;
use crate::effect::{GithubRequest, GithubResponse};
use crate::modal::{LookupKind, LookupTarget, number};
use crate::toast::Toast;

/// The issue lookup. It finds an issue or, since the issues endpoint answers for them too, a pull request.
#[derive(Debug)]
pub(super) enum IssueLookup {}

impl LookupKind for IssueLookup {
    type Found = IssueListItem;
    const TARGET: LookupTarget = LookupTarget::Issue;
    const HEADING: &'static str = "Search for an issue by number:";
    const LABEL: &'static str = "Issue Number:";
    const PLACEHOLDER: &'static str = "Issue number";
    const FILTER: Option<fn(&str) -> bool> = Some(number::can_become_number);

    fn is_valid(text: &str) -> bool {
        number::is_number(text)
    }

    /// Python's `int()` reads the number, so `+5` and `1_0` are numbers and `1.5` isn't. A negative number can't be an
    /// issue: Python sends it and gets a 404, here it's "Unknown Issue" without the round trip.
    fn request(text: &str, cx: &Cx<'_>) -> Result<GithubRequest, Toast> {
        // The opener checks this first, so the dialog never opens without a repo.
        let repo = cx.current_repo().ok_or_else(select_repository_first)?;
        let number = number::parse_int(text)
            .ok_or_else(|| Toast::error("Must enter a valid issue number!").with_title("Invalid Issue Number"))?;
        let number = u64::try_from(number).map_err(|_| unknown_issue())?;
        Ok(GithubRequest::Issue {
            repo: Box::new(repo.clone()),
            number,
        })
    }

    fn found(response: GithubResponse) -> Result<Result<IssueListItem, Toast>, GithubResponse> {
        match response {
            GithubResponse::Issue(Ok(item)) => Ok(Ok(*item)),
            GithubResponse::Issue(Err(error)) => {
                tracing::debug!("Looking up an issue failed: {error}");
                Ok(Err(unknown_issue()))
            }
            other => Err(other),
        }
    }
}

fn unknown_issue() -> Toast {
    Toast::error("Could not find issue!").with_title("Unknown Issue")
}

#[cfg(test)]
mod tests;
