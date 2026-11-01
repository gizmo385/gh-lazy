//! GitHub's issue and pull request search, `GET /search/issues`: the query, the request, the page it answers with,
//! and the ways it fails.
//!
//! One builder serves every search: the Search modal's (#386), with a [`SearchScope`] and the text the user typed, and
//! the PR table's `mine` filter (#384), whose `q` is [`pull_request_search_q`]. Results can come from any repository,
//! so each [`SearchHit`] carries the repo its `repository_url` names. API behavior is in the decision file
//! `search-issues-and-prs`.

use jiff::Timestamp;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

use super::GhCli;
use crate::config::IssueStateFilter;
use crate::gh::{GhError, GhRequest, GhResponse};
use crate::models::{IssueListItem, IssueState, RepoName, Repository};

/// The most results GitHub's search returns for one query, whatever the page size. A page past it answers 422
/// ([`SearchError::PastLimit`]).
pub const SEARCH_RESULT_CAP: u32 = 1000;

/// Results per page for a search the user submits. With it, [`SEARCH_RESULT_CAP`] is exactly 20 pages.
pub const SEARCH_PER_PAGE: u32 = 50;

/// The most repositories a [`SearchScope::Repos`] holds. GitHub answers 25 or more OR'ed `repo:` terms with an empty
/// page and no error, so callers truncate to this.
pub const SEARCH_MAX_REPOS: usize = 20;

/// The qualifiers that scope a search themselves. Text with one of them overrides the [`SearchScope`].
const SCOPE_QUALIFIERS: [&str; 3] = ["repo:", "org:", "user:"];

/// GitHub's 422 message for a page past [`SEARCH_RESULT_CAP`]: "Only the first 1000 search results are available".
const PAST_LIMIT_MESSAGE: &str = "search results are available";

/// Where a search looks, unless its text names a scope itself ([`SearchQuery::scope_from_text`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SearchScope {
    /// One repository, `owner/name`: `repo:owner/name`.
    Repo(String),
    /// Several repositories, each `owner/name`: `repo:a` for one, `(repo:a OR repo:b ...)` for more. At most
    /// [`SEARCH_MAX_REPOS`]. An empty set adds no scope.
    Repos(Vec<String>),
    /// Every repository of a user or organization: `user:owner` (which GitHub applies to organizations too).
    Owner(String),
    /// All of GitHub: no scope terms.
    All,
}

impl SearchScope {
    /// The scope's terms in `q`, or `None` when it adds none.
    fn terms(&self) -> Option<String> {
        match self {
            SearchScope::Repo(repo) => Some(format!("repo:{repo}")),
            SearchScope::Repos(repos) => {
                debug_assert!(
                    repos.len() <= SEARCH_MAX_REPOS,
                    "{} repos in a search scope; GitHub finds nothing past {SEARCH_MAX_REPOS}",
                    repos.len()
                );
                match repos.as_slice() {
                    [] => None,
                    [repo] => Some(format!("repo:{repo}")),
                    repos => {
                        let terms: Vec<String> = repos.iter().map(|repo| format!("repo:{repo}")).collect();
                        Some(format!("({})", terms.join(" OR ")))
                    }
                }
            }
            SearchScope::Owner(owner) => Some(format!("user:{owner}")),
            SearchScope::All => None,
        }
    }
}

/// One page of a search.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchQuery {
    /// What to search for, in GitHub's query syntax, as typed. Trimmed when sent.
    pub text: String,
    /// Where to look, unless [`Self::text`] names a scope itself.
    pub scope: SearchScope,
    /// The `sort` parameter, such as `created`, or `None` for best match. The Search modal leaves it unset: a
    /// `sort:` term in the text does the same.
    pub sort: Option<&'static str>,
    /// The `order` parameter, `asc` or `desc`. Only read by GitHub with a `sort`.
    pub order: Option<&'static str>,
    /// The page, from 1.
    pub page: u32,
    /// Results per page.
    pub per_page: u32,
    /// Whether `gh` may answer from its cache, for `cache.default_ttl`. Off for a search the user submits, so it's
    /// always fresh.
    pub cached: bool,
}

/// Why a [`SearchQuery`] has no `q` to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SearchQueryError {
    /// No text and no scope: there is nothing to search for.
    #[error("Type a query to search")]
    Empty,
}

impl SearchQuery {
    /// The first page of a search for `text` in `scope`: [`SEARCH_PER_PAGE`] results, unsorted (best match) and
    /// uncached.
    pub fn new(text: impl Into<String>, scope: SearchScope) -> Self {
        Self {
            text: text.into(),
            scope,
            sort: None,
            order: None,
            page: 1,
            per_page: SEARCH_PER_PAGE,
            cached: false,
        }
    }

    /// The `q` parameter: `<scope terms> (<text>)`, the parentheses keeping a top-level `OR` in the text inside the
    /// scope. The text alone when there's no scope to add (or [`Self::scope_from_text`]), and the scope terms alone
    /// when the text is blank.
    ///
    /// Fails with [`SearchQueryError::Empty`] when there's neither.
    pub fn q(&self) -> Result<String, SearchQueryError> {
        let text = self.text.trim();
        let scope = if self.scope_from_text() {
            None
        } else {
            self.scope.terms()
        };
        match (scope, text.is_empty()) {
            (None, true) => Err(SearchQueryError::Empty),
            (None, false) => Ok(text.to_owned()),
            (Some(scope), true) => Ok(scope),
            (Some(scope), false) => Ok(format!("{scope} ({text})")),
        }
    }

    /// Whether the text scopes the search itself, so [`Self::scope`] isn't added: it has a term starting `repo:`,
    /// `org:` or `user:` (in any case, optionally negated with `-` or opened with `(`) outside double quotes.
    pub fn scope_from_text(&self) -> bool {
        let text = self.text.as_str();
        let mut in_quotes = false;
        let mut at_term_start = true;
        for (at, c) in text.char_indices() {
            match c {
                '"' => {
                    in_quotes = !in_quotes;
                    at_term_start = false;
                }
                _ if in_quotes => {}
                c if c.is_whitespace() || c == '(' => at_term_start = true,
                _ => {
                    if at_term_start && starts_with_scope_qualifier(&text[at..]) {
                        return true;
                    }
                    at_term_start = false;
                }
            }
        }
        false
    }
}

/// Whether `term` starts with a scope qualifier, in any case, optionally negated with `-`.
fn starts_with_scope_qualifier(term: &str) -> bool {
    let term = term.strip_prefix('-').unwrap_or(term);
    SCOPE_QUALIFIERS.iter().any(|qualifier| {
        term.get(..qualifier.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(qualifier))
    })
}

/// Whether page `page` of `per_page` results reaches [`SEARCH_RESULT_CAP`], so no later page can hold a result.
pub fn reaches_result_cap(page: u32, per_page: u32) -> bool {
    page.saturating_mul(per_page) >= SEARCH_RESULT_CAP
}

/// The `q` that finds the logged-in user's pull requests in `repo`, in `state`:
/// `is:pr repo:<owner>/<name> author:@me`, plus ` is:open` or ` is:closed` unless `state` is `all`. `is:closed`
/// includes merged pull requests, as `state=closed` does.
pub fn pull_request_search_q(repo: &Repository, state: IssueStateFilter) -> String {
    let mut q = format!("is:pr repo:{}/{} author:@me", repo.owner.login, repo.name);
    match state {
        IssueStateFilter::All => {}
        IssueStateFilter::Open => q.push_str(" is:open"),
        IssueStateFilter::Closed => q.push_str(" is:closed"),
    }
    q
}

/// Whether a hit is an issue or a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HitKind {
    /// An issue.
    Issue,
    /// A pull request.
    PullRequest {
        /// Whether it's a draft.
        draft: bool,
        /// Whether it was merged (`pull_request.merged_at` is set).
        merged: bool,
    },
}

/// One search result: an issue or a pull request in any repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// The repository it's in, from its `repository_url`.
    pub repo: RepoName,
    /// The number within the repository.
    pub number: u64,
    /// The title.
    pub title: String,
    /// Open or closed (a merged pull request is closed).
    pub state: IssueState,
    /// Who opened it: `user.login`.
    pub author: String,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// Its page on GitHub.
    pub html_url: String,
    /// Issue or pull request.
    pub kind: HitKind,
    /// The item as GitHub sent it, for [`Self::to_issue_list_item`].
    item: Value,
}

/// The fields of a search item a [`SearchHit`] reads. Anything else (`score`, `labels`, ...) is ignored.
#[derive(Deserialize)]
struct HitFields {
    repository_url: String,
    number: u64,
    title: String,
    state: IssueState,
    user: HitUser,
    updated_at: Timestamp,
    html_url: String,
    #[serde(default)]
    draft: Option<bool>,
    #[serde(default)]
    pull_request: Option<HitPullRequest>,
}

#[derive(Deserialize)]
struct HitUser {
    login: String,
}

#[derive(Deserialize)]
struct HitPullRequest {
    #[serde(default)]
    merged_at: Option<Timestamp>,
}

impl SearchHit {
    /// Decodes one search item.
    ///
    /// # Errors
    ///
    /// When the item lacks a field a hit reads, or its `repository_url` isn't a repository's API URL.
    pub fn from_api_item(item: Value) -> Result<Self, serde_json::Error> {
        let fields = HitFields::deserialize(&item)?;
        let repo = RepoName::from_api_url(&fields.repository_url).ok_or_else(|| {
            serde_json::Error::custom(format!(
                "`repository_url` is not a repository's API URL: {:?}",
                fields.repository_url
            ))
        })?;
        let kind = match fields.pull_request {
            Some(pull_request) => HitKind::PullRequest {
                draft: fields.draft.unwrap_or(false),
                merged: pull_request.merged_at.is_some(),
            },
            None => HitKind::Issue,
        };
        Ok(Self {
            repo,
            number: fields.number,
            title: fields.title,
            state: fields.state,
            author: fields.user.login,
            updated_at: fields.updated_at,
            html_url: fields.html_url,
            kind,
            item,
        })
    }

    /// The hit as an issues-endpoint item in `repo`, through [`IssueListItem::from_api_item`]: for a caller that
    /// knows the hit's repository. Like that, an item with a `draft` key is a pull request.
    ///
    /// # Errors
    ///
    /// When the item doesn't decode as an [`IssueListItem`].
    pub fn to_issue_list_item(&self, repo: &Repository) -> Result<IssueListItem, serde_json::Error> {
        IssueListItem::from_api_item(self.item.clone(), repo)
    }
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    /// How many results match over every page: GitHub's count, which may pass [`SEARCH_RESULT_CAP`].
    pub total_count: u64,
    /// Whether GitHub gave up before finding every match. The hits are still usable.
    pub incomplete_results: bool,
    /// The page's results, in GitHub's order.
    pub items: Vec<SearchHit>,
}

impl SearchPage {
    /// Whether this page, page `page` of `per_page` results, is the last: it's short, or it reaches the total or
    /// [`SEARCH_RESULT_CAP`], whichever is lower.
    pub fn is_last(&self, page: u32, per_page: u32) -> bool {
        let seen = u64::from(page) * u64::from(per_page);
        self.items.len() < per_page as usize || seen >= self.total_count.min(u64::from(SEARCH_RESULT_CAP))
    }
}

/// A search page as GitHub sends it, its items still JSON.
#[derive(Deserialize)]
struct RawSearchPage {
    total_count: u64,
    #[serde(default)]
    incomplete_results: bool,
    items: Vec<Value>,
}

/// Why a search failed. `Display` is a line fit for a status line.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// No text and no scope: nothing was sent.
    #[error("Type a query to search")]
    Empty,
    /// GitHub couldn't read the query (a 422 "Validation Failed"): its reasons, joined with a space.
    #[error("{}", messages.join(" "))]
    Invalid {
        /// Each reason, as GitHub words it.
        messages: Vec<String>,
    },
    /// The page is past [`SEARCH_RESULT_CAP`] (a 422). Paging treats it as the end of the results.
    #[error("Only the first 1000 search results are available")]
    PastLimit,
    /// Any other failure: offline, a 5xx, a rate limit, a body that doesn't decode.
    #[error(transparent)]
    Gh(GhError),
}

impl SearchError {
    /// The [`GhError`] behind a [`SearchError::Gh`], for the rate-limit watch and the unauthorized check.
    pub fn gh_error(&self) -> Option<&GhError> {
        match self {
            SearchError::Gh(error) => Some(error),
            SearchError::Empty | SearchError::Invalid { .. } | SearchError::PastLimit => None,
        }
    }
}

impl From<SearchQueryError> for SearchError {
    fn from(error: SearchQueryError) -> Self {
        match error {
            SearchQueryError::Empty => SearchError::Empty,
        }
    }
}

/// A 422's error body: `message`, and for "Validation Failed" the `errors` saying why.
#[derive(Deserialize)]
struct ValidationBody {
    #[serde(default)]
    message: String,
    #[serde(default)]
    errors: Vec<ValidationEntry>,
}

#[derive(Deserialize)]
struct ValidationEntry {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

impl ValidationEntry {
    /// The entry's message, or its code worded.
    fn reason(self) -> Option<String> {
        if let Some(message) = self.message.filter(|message| !message.trim().is_empty()) {
            return Some(message.trim().to_owned());
        }
        Some(match self.code?.as_str() {
            "missing" => "The query is empty".to_owned(),
            "invalid" => "The query is invalid".to_owned(),
            code => format!("GitHub rejected the query ({code})"),
        })
    }
}

impl From<GhError> for SearchError {
    /// Tells apart the 422s a search words itself; any other failure is [`SearchError::Gh`].
    fn from(error: GhError) -> Self {
        let GhError::Http {
            status: 422,
            message,
            body,
            ..
        } = &error
        else {
            return SearchError::Gh(error);
        };
        let parsed: Option<ValidationBody> = serde_json::from_slice(body).ok();
        let top = parsed.as_ref().map_or(message.as_str(), |body| body.message.as_str());
        if top.contains(PAST_LIMIT_MESSAGE) || message.contains(PAST_LIMIT_MESSAGE) {
            return SearchError::PastLimit;
        }
        match parsed {
            Some(body) if body.message == "Validation Failed" => {
                let mut messages: Vec<String> = body.errors.into_iter().filter_map(ValidationEntry::reason).collect();
                if messages.is_empty() {
                    messages.push(body.message);
                }
                SearchError::Invalid { messages }
            }
            _ => SearchError::Gh(error),
        }
    }
}

impl GhCli {
    /// `GET /search/issues` for one page, with the query in the order `q`, `advanced_search=true` (which `OR` and
    /// parentheses need), `sort`, `order`, `page`, `per_page` (an unset `sort` or `order` is left out). A plain
    /// `gh api -i` call, so a rate limit's headers are kept; cached for `cache.default_ttl` only when
    /// [`SearchQuery::cached`].
    ///
    /// Fails with [`SearchQueryError::Empty`] when the query has nothing to search for.
    pub(super) fn search_issues_request(&self, query: &SearchQuery) -> Result<GhRequest, SearchQueryError> {
        let mut request = GhRequest::get("/search/issues");
        request.query.push(("q".to_owned(), query.q()?));
        request.query.push(("advanced_search".to_owned(), "true".to_owned()));
        if let Some(sort) = query.sort {
            request.query.push(("sort".to_owned(), sort.to_owned()));
        }
        if let Some(order) = query.order {
            request.query.push(("order".to_owned(), order.to_owned()));
        }
        request.query.push(("page".to_owned(), query.page.to_string()));
        request.query.push(("per_page".to_owned(), query.per_page.to_string()));
        if query.cached {
            request.cache = Some(self.ttls().default);
        }
        Ok(request)
    }
}

/// A search response as its page. Incomplete results are logged at debug and used anyway.
///
/// Fails with [`GhError::Decode`] when the body isn't a search page or any item isn't a hit (one bad item fails the
/// page, as on the issues endpoint).
pub(super) fn search_page(response: &GhResponse) -> Result<SearchPage, GhError> {
    let page: RawSearchPage = response.json()?;
    if page.incomplete_results {
        tracing::debug!(
            total_count = page.total_count,
            items = page.items.len(),
            "The search's results are incomplete"
        );
    }
    Ok(SearchPage {
        total_count: page.total_count,
        incomplete_results: page.incomplete_results,
        items: page
            .items
            .into_iter()
            .map(SearchHit::from_api_item)
            .collect::<Result<_, _>>()?,
    })
}

#[cfg(test)]
mod tests;
