//! GitHub web links: telling a user, pull request, issue or repository URL apart, and looking up what it points at.
//!
//! The paste-link dialog and link clicks share this. Python's `ui/screens/link_paste.py` (`REPO_PATTERN`,
//! `ISSUE_PATTERN`, `PR_PATTERN`, `USER_PATTERN`, `resolve_github_url`). The patterns are matched by hand: each is a
//! few `/`-separated segments, anchored at the start and, except for a user link, not at the end, as Python's
//! `re.match` is.

use crate::gh::GhError;
use crate::github::Github;
use crate::models::{FullPullRequest, FullUser, Issue, IssueListItem, Repository};

/// What a GitHub web link points at, by its URL alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubLink {
    /// `https://github.com/<login>`, with or without a trailing `/`.
    User {
        /// The user's or organization's login.
        login: String,
    },
    /// `<host>/<owner>/<repo>/pull/<number>...`.
    PullRequest {
        /// The repository's owner, verbatim.
        owner: String,
        /// The repository's name, verbatim.
        repo: String,
        /// The number, saturated at `u64::MAX` when it's longer than that.
        number: u64,
    },
    /// `<host>/<owner>/<repo>/issues/<number>...`.
    Issue {
        /// The repository's owner, verbatim.
        owner: String,
        /// The repository's name, verbatim.
        repo: String,
        /// The number, saturated at `u64::MAX` when it's longer than that.
        number: u64,
    },
    /// `<host>/<owner>/<repo>...`: anything else with an owner and a repo.
    Repository {
        /// The repository's owner, verbatim.
        owner: String,
        /// The repository's name, verbatim: whatever is up to the next `/`, a `?tab=readme` included.
        repo: String,
    },
}

/// What a resolved [`GithubLink`] is, fetched whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSubject {
    /// A pull request, from a pull request link or an issue link whose number is a pull request.
    PullRequest(Box<FullPullRequest>),
    /// An issue.
    Issue(Box<Issue>),
    /// A repository.
    Repository(Box<Repository>),
    /// A user's or organization's profile.
    User(Box<FullUser>),
}

/// The optional scheme every pattern starts with. `http://` isn't accepted (Py bug B105, kept as Python has it).
const HTTPS: &str = "https://";

/// What `text` links to, trying Python's patterns in its order: user, pull request, issue, then a repository only
/// when `accept_repo` (link clicks don't open bare repository links in the app). `None` when no pattern fits.
///
/// The host isn't checked, except for a user link's `github.com`, so a GitHub Enterprise link works and a gitlab.com
/// one is taken as GitHub's (Py bug B105, kept as Python has it). Numbers are ASCII digits; Python's `\d` also takes
/// other Unicode decimal digits, which no web link has.
pub fn parse_github_url(text: &str, accept_repo: bool) -> Option<GithubLink> {
    if let Some(login) = user_login(text) {
        return Some(GithubLink::User {
            login: login.to_owned(),
        });
    }
    let (owner, repo, rest) = repo_segments(text)?;
    let (owner, repo) = (owner.to_owned(), repo.to_owned());
    if let Some(number) = numbered(rest, "/pull/") {
        return Some(GithubLink::PullRequest { owner, repo, number });
    }
    if let Some(number) = numbered(rest, "/issues/") {
        return Some(GithubLink::Issue { owner, repo, number });
    }
    accept_repo.then_some(GithubLink::Repository { owner, repo })
}

/// Whether `text` fits any of the four patterns, a repository link included: Python's `GithubLinkValidator`, behind
/// the paste-link input's red border.
pub fn looks_like_github_url(text: &str) -> bool {
    parse_github_url(text, true).is_some()
}

/// `USER_PATTERN`: `(https://)?github\.com/<login>/?$`, where `$` also matches before a final newline.
fn user_login(text: &str) -> Option<&str> {
    let text = text.strip_prefix(HTTPS).unwrap_or(text);
    let rest = text.strip_prefix("github.com/")?;
    let rest = rest.strip_suffix('\n').unwrap_or(rest);
    let login = rest.strip_suffix('/').unwrap_or(rest);
    is_login(login).then_some(login)
}

/// `[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?`: ASCII letters, digits and inner hyphens.
fn is_login(login: &str) -> bool {
    let bytes = login.as_bytes();
    let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `REPO_PATTERN`: `(https://)?<host>/<owner>/<repo>`, each segment non-empty and without `/`. Returns the owner, the
/// repo and the rest of the text after the repo.
///
/// Without the scheme matched, a text starting with `https://` would need an empty owner, so trying only the
/// stripped text is the regex's answer too.
fn repo_segments(text: &str) -> Option<(&str, &str, &str)> {
    let text = text.strip_prefix(HTTPS).unwrap_or(text);
    let (host, rest) = text.split_once('/')?;
    let (owner, rest) = rest.split_once('/')?;
    let end = rest.find('/').unwrap_or(rest.len());
    let (repo, rest) = rest.split_at(end);
    (!host.is_empty() && !owner.is_empty() && !repo.is_empty()).then_some((owner, repo, rest))
}

/// `<marker>(\d+).*` at the start of `rest`: the number, saturated at `u64::MAX`.
fn numbered(rest: &str, marker: &str) -> Option<u64> {
    let rest = rest.strip_prefix(marker)?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    // A number too long for `u64` is sent as `u64::MAX`, which GitHub answers with a 404, as it does Python's.
    Some(rest[..digits].parse().unwrap_or(u64::MAX))
}

/// Looks up what `link` points at, in Python's order: a user's profile; or the repository, then the pull request, or
/// the issue and, when that's a pull request, the pull request from its own endpoint.
///
/// `Ok(None)` when the repository isn't there (a 404).
///
/// # Errors
///
/// Whatever any lookup fails with: a 404 for an unknown user, pull request or issue, any other HTTP error, a `gh`
/// failure, or a body that doesn't decode.
pub async fn resolve_github_link<G: Github>(gh: &G, link: &GithubLink) -> Result<Option<LinkSubject>, GhError> {
    let subject = match link {
        GithubLink::User { login } => LinkSubject::User(Box::new(gh.user(login).await?)),
        GithubLink::PullRequest { owner, repo, number } => {
            let Some(repository) = gh.repository(&format!("{owner}/{repo}")).await? else {
                return Ok(None);
            };
            LinkSubject::PullRequest(Box::new(gh.pull_request(&repository, *number).await?))
        }
        GithubLink::Issue { owner, repo, number } => {
            let Some(repository) = gh.repository(&format!("{owner}/{repo}")).await? else {
                return Ok(None);
            };
            match gh.issue(&repository, *number).await? {
                IssueListItem::Issue(issue) => LinkSubject::Issue(Box::new(issue)),
                IssueListItem::PullRequest(_) => {
                    LinkSubject::PullRequest(Box::new(gh.pull_request(&repository, *number).await?))
                }
            }
        }
        GithubLink::Repository { owner, repo } => match gh.repository(&format!("{owner}/{repo}")).await? {
            Some(repository) => LinkSubject::Repository(Box::new(repository)),
            None => return Ok(None),
        },
    };
    Ok(Some(subject))
}

#[cfg(test)]
mod tests;
