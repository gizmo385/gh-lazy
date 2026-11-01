//! GitHub's issue and pull request search, `GET /search/issues`: the request and the page it answers with.
//!
//! The PR table's `mine` filter searches with [`pull_request_search_q`] (#384). The module is kept free of anything
//! else that's specific to one caller, so a search with any `q` (#386) can build on [`SearchIssuesQuery`] and decode
//! [`SearchIssuesPage::items`] itself: items from other repos carry a `repository_url`, not a repo.

use serde::Deserialize;

use super::GhCli;
use crate::config::IssueStateFilter;
use crate::gh::{GhError, GhRequest, GhResponse};
use crate::models::Repository;

/// The most results GitHub's search returns for one query, whatever the page size. A page past it answers 422.
pub const SEARCH_RESULT_CAP: u32 = 1000;

/// The parameters of one `GET /search/issues` page.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchIssuesQuery {
    /// The search, in GitHub's query syntax, such as `is:pr repo:cli/cli author:@me`. Sent as typed; the request
    /// encodes it.
    pub q: String,
    /// The field to sort by, such as `created`, or `None` for best match.
    pub sort: Option<&'static str>,
    /// `asc` or `desc`, or `None` for GitHub's default. Only read with a `sort`.
    pub order: Option<&'static str>,
    /// The page, from 1.
    pub page: u32,
    /// Results per page.
    pub per_page: u32,
}

impl SearchIssuesQuery {
    /// Whether this page reaches [`SEARCH_RESULT_CAP`], so no later page can hold a result.
    pub fn reaches_result_cap(&self) -> bool {
        self.page.saturating_mul(self.per_page) >= SEARCH_RESULT_CAP
    }
}

/// One page of search results, its items left as JSON for the caller to decode.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SearchIssuesPage {
    /// How many results match over every page (GitHub's count, which may pass [`SEARCH_RESULT_CAP`]).
    pub total_count: u64,
    /// Whether GitHub gave up before finding every match. The items are still usable.
    #[serde(default)]
    pub incomplete_results: bool,
    /// The page's results, issue-shaped: an issue, or a pull request with `draft` and `pull_request`.
    pub items: Vec<serde_json::Value>,
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

impl GhCli {
    /// `GET /search/issues` for one page, with the query in the order `q`, `sort`, `order`, `page`, `per_page` (an
    /// unset `sort` or `order` is left out). Cached for `cache.default_ttl`, as the issues list is.
    pub(super) fn search_issues_request(&self, query: &SearchIssuesQuery) -> GhRequest {
        let mut request = GhRequest::get("/search/issues");
        request.query.push(("q".to_owned(), query.q.clone()));
        if let Some(sort) = query.sort {
            request.query.push(("sort".to_owned(), sort.to_owned()));
        }
        if let Some(order) = query.order {
            request.query.push(("order".to_owned(), order.to_owned()));
        }
        request.query.push(("page".to_owned(), query.page.to_string()));
        request.query.push(("per_page".to_owned(), query.per_page.to_string()));
        request.cache = Some(self.ttls().default);
        request
    }
}

/// A search response as its page. Incomplete results are logged at debug and used anyway.
///
/// Fails with [`GhError::Decode`] when the body isn't a search page.
pub(super) fn search_issues_page(response: &GhResponse) -> Result<SearchIssuesPage, GhError> {
    let page: SearchIssuesPage = response.json()?;
    if page.incomplete_results {
        tracing::debug!(
            total_count = page.total_count,
            items = page.items.len(),
            "The search's results are incomplete"
        );
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::github::GhCliOptions;

    fn strings(args: Vec<std::ffi::OsString>) -> Vec<String> {
        args.into_iter().map(|arg| arg.into_string().unwrap()).collect()
    }

    fn query(q: &str, page: u32) -> SearchIssuesQuery {
        SearchIssuesQuery {
            q: q.to_owned(),
            sort: Some("created"),
            order: Some("desc"),
            page,
            per_page: 30,
        }
    }

    fn argv(config: &Config, query: &SearchIssuesQuery) -> Vec<String> {
        let gh = GhCli::new(GhCliOptions::from(config));
        let request = gh.search_issues_request(query);
        strings(request.argv(gh.runner().hostname(), false))
    }

    #[test]
    fn the_request_encodes_q_and_keeps_the_query_order() {
        assert_eq!(
            argv(
                &Config::default(),
                &query("is:issue repo:cli/cli label:\"good first\"", 2)
            ),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/search/issues?q=is%3Aissue+repo%3Acli%2Fcli+label%3A%22good+first%22&sort=created&order=desc&page=2&per_page=30"
            ]
        );
    }

    #[test]
    fn an_unset_sort_and_order_are_left_out_and_a_zero_ttl_is_uncached() {
        let mut config = Config::default();
        config.cache.default_ttl = 0;
        config.api.base_url = "https://ghe.example.com/api/v3".into();
        let query = SearchIssuesQuery {
            sort: None,
            order: None,
            ..query("cli", 1)
        };
        assert_eq!(
            argv(&config, &query),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--hostname",
                "ghe.example.com",
                "-H",
                "Accept: application/vnd.github+json",
                "/search/issues?q=cli&page=1&per_page=30"
            ]
        );
    }

    #[test]
    fn the_result_cap_is_reached_on_page_34_of_30() {
        assert!(!query("x", 33).reaches_result_cap());
        assert!(query("x", 34).reaches_result_cap());
        assert!(
            SearchIssuesQuery {
                per_page: 100,
                ..query("x", 10)
            }
            .reaches_result_cap()
        );
        assert!(query("x", u32::MAX).reaches_result_cap());
    }

    fn response(status: &str, body: &str) -> Result<GhResponse, GhError> {
        let stdout = format!("HTTP/2.0 {status}\r\nContent-Type: application/json\r\n\r\n{body}");
        crate::gh::interpret(Some(0), stdout.as_bytes(), b"", false)
    }

    #[test]
    fn a_page_decodes_with_its_items_as_json() {
        let body = r#"{"total_count": 71, "incomplete_results": false, "items": [{"number": 3, "repository_url": "https://api.github.com/repos/a/b"}, {"number": 1}]}"#;
        let page = search_issues_page(&response("200 OK", body).unwrap()).unwrap();
        assert_eq!(page.total_count, 71);
        assert!(!page.incomplete_results);
        let numbers: Vec<u64> = page.items.iter().map(|item| item["number"].as_u64().unwrap()).collect();
        assert_eq!(numbers, [3, 1]);
        assert_eq!(page.items[0]["repository_url"], "https://api.github.com/repos/a/b");
    }

    #[test]
    fn incomplete_results_are_logged_and_kept() {
        let body = r#"{"total_count": 5, "incomplete_results": true, "items": [{"number": 3}]}"#;
        let (page, out) = crate::test_log::logs(|| search_issues_page(&response("200 OK", body).unwrap()));
        let page = page.unwrap();
        assert!(page.incomplete_results);
        assert_eq!(page.items.len(), 1);
        assert!(out.contains("incomplete"), "{out}");
    }

    #[test]
    fn a_body_that_isnt_a_page_is_a_decode_error() {
        assert!(matches!(
            search_issues_page(&response("200 OK", "[]").unwrap()),
            Err(GhError::Decode(_))
        ));
        assert!(matches!(
            search_issues_page(&response("200 OK", r#"{"total_count": 1}"#).unwrap()),
            Err(GhError::Decode(_))
        ));
    }

    #[test]
    fn the_pull_request_q_per_state() {
        let repo: Repository = serde_json::from_value(serde_json::json!({
            "name": "gh-lazy",
            "full_name": "gizmo385/gh-lazy",
            "private": false,
            "owner": {"login": "gizmo385", "id": 1, "html_url": "https://github.com/gizmo385"},
        }))
        .unwrap();
        assert_eq!(
            pull_request_search_q(&repo, IssueStateFilter::All),
            "is:pr repo:gizmo385/gh-lazy author:@me"
        );
        assert_eq!(
            pull_request_search_q(&repo, IssueStateFilter::Open),
            "is:pr repo:gizmo385/gh-lazy author:@me is:open"
        );
        assert_eq!(
            pull_request_search_q(&repo, IssueStateFilter::Closed),
            "is:pr repo:gizmo385/gh-lazy author:@me is:closed"
        );
    }
}
