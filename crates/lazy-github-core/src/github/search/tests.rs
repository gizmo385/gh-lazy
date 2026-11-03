use serde_json::json;

use super::*;
use crate::config::Config;
use crate::gh::RateLimit;
use crate::github::GhCliOptions;

fn strings(args: Vec<std::ffi::OsString>) -> Vec<String> {
    args.into_iter().map(|arg| arg.into_string().unwrap()).collect()
}

fn repo(full_name: &str) -> SearchScope {
    SearchScope::Repo(full_name.to_owned())
}

fn q(text: &str, scope: SearchScope) -> Result<String, SearchQueryError> {
    SearchQuery::new(text, scope).q()
}

// `SearchQuery::q`.

#[test]
fn each_scope_comes_before_the_text_in_parentheses() {
    assert_eq!(q("is:open", repo("cli/cli")).unwrap(), "repo:cli/cli (is:open)");
    assert_eq!(
        q("bug", SearchScope::Owner("gizmo385".into())).unwrap(),
        "user:gizmo385 (bug)"
    );
    assert_eq!(
        q(
            "bug",
            SearchScope::Repos(vec!["a/b".into(), "c/d".into(), "e/f".into()])
        )
        .unwrap(),
        "(repo:a/b OR repo:c/d OR repo:e/f) (bug)"
    );
    assert_eq!(q("is:pr author:@me", SearchScope::All).unwrap(), "is:pr author:@me");
}

#[test]
fn a_top_level_or_stays_inside_the_scope() {
    assert_eq!(q("a OR b", repo("o/r")).unwrap(), "repo:o/r (a OR b)");
}

#[test]
fn one_repo_in_a_set_is_a_plain_repo_term() {
    assert_eq!(q("x", SearchScope::Repos(vec!["o/r".into()])).unwrap(), "repo:o/r (x)");
    // An empty set adds no scope.
    assert_eq!(q("x", SearchScope::Repos(Vec::new())).unwrap(), "x");
}

#[test]
fn twenty_repos_is_the_most_a_set_holds() {
    let repos: Vec<String> = (0..SEARCH_MAX_REPOS).map(|n| format!("o/r{n}")).collect();
    let q = q("", SearchScope::Repos(repos)).unwrap();
    assert_eq!(q.matches("repo:").count(), 20);
    assert_eq!(q.matches(" OR ").count(), 19);
}

#[test]
#[should_panic(expected = "21 repos in a search scope")]
#[cfg(debug_assertions)]
fn more_than_twenty_repos_is_a_bug() {
    let repos: Vec<String> = (0..=SEARCH_MAX_REPOS).map(|n| format!("o/r{n}")).collect();
    let _ = q("x", SearchScope::Repos(repos));
}

#[test]
fn the_scope_alone_lists_everything_in_it() {
    assert_eq!(q("", repo("o/r")).unwrap(), "repo:o/r");
    assert_eq!(q("   ", SearchScope::Owner("o".into())).unwrap(), "user:o");
}

#[test]
fn the_text_is_trimmed() {
    assert_eq!(q("  is:open \n", repo("o/r")).unwrap(), "repo:o/r (is:open)");
    assert_eq!(q("  bug  ", SearchScope::All).unwrap(), "bug");
}

#[test]
fn no_text_and_no_scope_is_empty() {
    assert_eq!(q("", SearchScope::All), Err(SearchQueryError::Empty));
    assert_eq!(q(" \t ", SearchScope::All), Err(SearchQueryError::Empty));
    assert_eq!(SearchQueryError::Empty.to_string(), "Type a query to search");
}

#[test]
fn a_scope_in_the_text_drops_the_selected_one() {
    for text in [
        "repo:cli/cli is:open",
        "is:open org:github",
        "user:octocat bug",
        "-repo:cli/cli bug",
        "REPO:cli/cli",
        "(repo:a/b OR repo:c/d) bug",
        "bug\tUser:x",
    ] {
        let query = SearchQuery::new(text, repo("o/r"));
        assert!(query.scope_from_text(), "{text}");
        assert_eq!(query.q().unwrap(), text.trim(), "{text}");
    }
}

#[test]
fn a_qualifier_inside_quotes_or_a_word_doesnt_count() {
    for text in [
        r#""repo:cli/cli" bug"#,
        r#"title "see user:x" bug"#,
        "myrepo:x",
        "author:octocat",
        "label:org:x",
    ] {
        let query = SearchQuery::new(text, repo("o/r"));
        assert!(!query.scope_from_text(), "{text}");
        assert_eq!(query.q().unwrap(), format!("repo:o/r ({text})"));
    }
    // After the closing quote, a term counts again.
    assert!(SearchQuery::new(r#""a b" repo:x/y"#, SearchScope::All).scope_from_text());
}

// The request.

fn argv(config: &Config, query: &SearchQuery) -> Vec<String> {
    let gh = GhCli::new(GhCliOptions::from(config));
    let request = gh.search_issues_request(query).unwrap();
    strings(request.argv(gh.runner().hostname(), false))
}

#[test]
fn the_request_is_an_uncached_plain_get_with_q_first() {
    let query = SearchQuery::new("label:\"good first\" OR bug", repo("cli/cli"));
    assert_eq!(
        argv(&Config::default(), &query),
        [
            "api",
            "-i",
            "-X",
            "GET",
            "-H",
            "Accept: application/vnd.github+json",
            "/search/issues?q=repo%3Acli%2Fcli+%28label%3A%22good+first%22+OR+bug%29&advanced_search=true&page=1&per_page=50"
        ]
    );
}

#[test]
fn a_cached_sorted_query_sends_its_ttl_sort_and_order() {
    let mut config = Config::default();
    config.api.base_url = "https://ghe.example.com/api/v3".into();
    let query = SearchQuery {
        sort: Some("created"),
        order: Some("desc"),
        page: 2,
        per_page: 30,
        cached: true,
        ..SearchQuery::new("is:pr repo:o/r author:@me", SearchScope::All)
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
            "--cache",
            "60s",
            "-H",
            "Accept: application/vnd.github+json",
            "/search/issues?q=is%3Apr+repo%3Ao%2Fr+author%3A%40me&advanced_search=true&sort=created&order=desc&page=2&per_page=30"
        ]
    );
    // A zero TTL is uncached even so.
    config.cache.default_ttl = 0;
    assert!(!argv(&config, &query).contains(&"--cache".to_owned()));
}

#[test]
fn an_empty_query_has_no_request() {
    let gh = GhCli::default();
    assert_eq!(
        gh.search_issues_request(&SearchQuery::new(" ", SearchScope::All)).err(),
        Some(SearchQueryError::Empty)
    );
}

#[test]
fn the_result_cap_is_page_20_of_50_and_page_34_of_30() {
    assert!(!reaches_result_cap(19, SEARCH_PER_PAGE));
    assert!(reaches_result_cap(20, SEARCH_PER_PAGE));
    assert!(!reaches_result_cap(33, 30));
    assert!(reaches_result_cap(34, 30));
    assert!(reaches_result_cap(u32::MAX, u32::MAX));
}

// Decoding.

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> Result<GhResponse, GhError> {
    let mut stdout = format!("HTTP/2.0 {status}\r\nContent-Type: application/json\r\n");
    for (name, value) in headers {
        for part in [name, ": ", value, "\r\n"] {
            stdout.push_str(part);
        }
    }
    stdout.push_str("\r\n");
    stdout.push_str(body);
    crate::gh::interpret(Some(0), stdout.as_bytes(), b"", false)
}

/// A search item as GitHub sends it, for issue `number` in `full_name`, with fields a hit ignores.
fn issue_item(full_name: &str, number: u64) -> Value {
    json!({
        "url": format!("https://api.github.com/repos/{full_name}/issues/{number}"),
        "repository_url": format!("https://api.github.com/repos/{full_name}"),
        "html_url": format!("https://github.com/{full_name}/issues/{number}"),
        "comments_url": format!("https://api.github.com/repos/{full_name}/issues/{number}/comments"),
        "id": 7_000_000 + number,
        "number": number,
        "title": format!("Issue {number}"),
        "user": {"login": "parity-bot", "id": 1, "html_url": "https://github.com/parity-bot", "type": "User"},
        "labels": [{"name": "bug"}],
        "state": "open",
        "locked": false,
        "assignee": null,
        "assignees": [],
        "comments": 3,
        "created_at": "2026-10-01T12:00:00Z",
        "updated_at": "2026-10-20T08:30:00Z",
        "closed_at": null,
        "body": "text",
        "score": 1.0,
    })
}

/// A pull request search item: `draft`, and `pull_request` with `merged_at`.
fn pr_item(full_name: &str, number: u64, draft: bool, merged: bool) -> Value {
    let mut item = issue_item(full_name, number);
    item["html_url"] = format!("https://github.com/{full_name}/pull/{number}").into();
    item["draft"] = draft.into();
    item["pull_request"] = json!({
        "url": format!("https://api.github.com/repos/{full_name}/pulls/{number}"),
        "merged_at": if merged { json!("2026-10-21T00:00:00Z") } else { json!(null) },
    });
    if merged {
        item["state"] = "closed".into();
    }
    item
}

fn page_of(items: &[Value], total_count: u64) -> Result<SearchPage, GhError> {
    let body = json!({
        "total_count": total_count,
        "incomplete_results": false,
        "search_type": "lexical",
        "items": items,
    });
    search_page(&response("200 OK", &[], &body.to_string()).unwrap())
}

#[test]
fn hits_decode_issues_and_pull_requests() {
    let page = page_of(
        &[
            pr_item("cli/cli", 3, false, false),
            pr_item("cli/cli", 2, true, false),
            pr_item("o/r", 1, false, true),
            issue_item("o/r", 9),
        ],
        4,
    )
    .unwrap();
    assert_eq!(page.total_count, 4);
    assert!(!page.incomplete_results);
    let kinds: Vec<HitKind> = page.items.iter().map(|hit| hit.kind).collect();
    assert_eq!(
        kinds,
        [
            HitKind::PullRequest {
                draft: false,
                merged: false
            },
            HitKind::PullRequest {
                draft: true,
                merged: false
            },
            HitKind::PullRequest {
                draft: false,
                merged: true
            },
            HitKind::Issue,
        ]
    );
    let first = &page.items[0];
    assert_eq!(first.repo.to_string(), "cli/cli");
    assert_eq!(first.number, 3);
    assert_eq!(first.title, "Issue 3");
    assert_eq!(first.state, IssueState::Open);
    assert_eq!(first.author, "parity-bot");
    assert_eq!(first.updated_at, "2026-10-20T08:30:00Z".parse::<Timestamp>().unwrap());
    assert_eq!(first.html_url, "https://github.com/cli/cli/pull/3");
    assert_eq!(page.items[2].state, IssueState::Closed);
    assert_eq!(page.items[3].repo.to_string(), "o/r");
}

#[test]
fn a_hit_on_enterprise_names_its_repo() {
    let mut item = issue_item("o/r", 1);
    item["repository_url"] = "https://ghe.example.com/api/v3/repos/team/tool".into();
    let hit = SearchHit::from_api_item(item).unwrap();
    assert_eq!((hit.repo.owner(), hit.repo.name()), ("team", "tool"));
}

#[test]
fn a_hit_whose_repository_url_doesnt_parse_fails_the_page() {
    let mut bad = issue_item("o/r", 1);
    bad["repository_url"] = "https://github.com/o/r".into();
    let error = page_of(&[issue_item("o/r", 2), bad], 2).unwrap_err();
    assert!(matches!(&error, GhError::Decode(_)), "{error:?}");
    assert!(error.to_string().contains("repository_url"), "{error}");
    // As does a missing field.
    let mut no_user = issue_item("o/r", 3);
    no_user.as_object_mut().unwrap().remove("user");
    assert!(matches!(page_of(&[no_user], 1), Err(GhError::Decode(_))));
}

#[test]
fn a_body_that_isnt_a_page_is_a_decode_error() {
    assert!(matches!(
        search_page(&response("200 OK", &[], "[]").unwrap()),
        Err(GhError::Decode(_))
    ));
    assert!(matches!(
        search_page(&response("200 OK", &[], r#"{"total_count": 1}"#).unwrap()),
        Err(GhError::Decode(_))
    ));
}

#[test]
fn incomplete_results_are_logged_and_kept() {
    let body = json!({"total_count": 5, "incomplete_results": true, "items": [issue_item("o/r", 3)]}).to_string();
    let (page, out) = crate::test_log::logs(|| search_page(&response("200 OK", &[], &body).unwrap()));
    let page = page.unwrap();
    assert!(page.incomplete_results);
    assert_eq!(page.items.len(), 1);
    assert!(out.contains("incomplete"), "{out}");
}

fn repository(full_name: &str) -> Repository {
    let (owner, name) = full_name.split_once('/').unwrap();
    serde_json::from_value(json!({
        "name": name,
        "full_name": full_name,
        "private": false,
        "owner": {"login": owner, "id": 1, "html_url": format!("https://github.com/{owner}")},
    }))
    .unwrap()
}

#[test]
fn to_issue_list_item_agrees_with_the_issues_endpoint() {
    let repo = repository("cli/cli");
    for item in [pr_item("cli/cli", 3, true, false), issue_item("cli/cli", 9)] {
        let hit = SearchHit::from_api_item(item.clone()).unwrap();
        assert_eq!(
            hit.to_issue_list_item(&repo).unwrap(),
            IssueListItem::from_api_item(item, &repo).unwrap()
        );
    }
    let hit = SearchHit::from_api_item(pr_item("cli/cli", 3, true, false)).unwrap();
    assert!(hit.to_issue_list_item(&repo).unwrap().is_pull_request());
}

fn hits(n: usize) -> Vec<SearchHit> {
    let item = issue_item("o/r", 1);
    vec![SearchHit::from_api_item(item).unwrap(); n]
}

fn page(n: usize, total_count: u64) -> SearchPage {
    SearchPage {
        total_count,
        incomplete_results: false,
        items: hits(n),
    }
}

#[test]
fn is_last_at_a_short_page_the_total_and_the_cap() {
    // A short page.
    assert!(page(49, 885_203).is_last(3, 50));
    // A full page short of the total.
    assert!(!page(50, 120).is_last(2, 50));
    // A full page at an exact multiple of the total: nothing follows.
    assert!(page(50, 100).is_last(2, 50));
    // Page 20 of 50 reaches the cap, whatever the total.
    assert!(!page(50, 885_203).is_last(19, 50));
    assert!(page(50, 885_203).is_last(20, 50));
    // An empty page is the last.
    assert!(page(0, 0).is_last(1, 50));
}

// Errors.

fn search_error(status: &str, headers: &[(&str, &str)], body: &str) -> SearchError {
    SearchError::from(response(status, headers, body).unwrap_err())
}

#[test]
fn a_validation_failure_lists_github_s_messages() {
    let body = json!({
        "message": "Validation Failed",
        "errors": [
            {
                "message": "\"notadate\" is not a recognized date/time format. Please provide an ISO 8601 date/time value, such as YYYY-MM-DD.",
                "resource": "Search",
                "field": "q",
                "code": "invalid",
            },
            {"message": "Second reason.", "code": "invalid"},
        ],
        "documentation_url": "https://docs.github.com/v3/search/",
        "status": "422",
    });
    let error = search_error("422 Unprocessable Entity", &[], &body.to_string());
    assert!(
        matches!(&error, SearchError::Invalid { messages } if messages.len() == 2),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "\"notadate\" is not a recognized date/time format. Please provide an ISO 8601 date/time value, such as YYYY-MM-DD. Second reason."
    );
    assert!(error.gh_error().is_none());
}

#[test]
fn a_validation_failure_with_only_a_code_words_it() {
    let body = json!({
        "message": "Validation Failed",
        "errors": [{"resource": "Search", "field": "q", "code": "missing"}],
    });
    let error = search_error("422 Unprocessable Entity", &[], &body.to_string());
    assert_eq!(error.to_string(), "The query is empty");
    let bare = search_error("422 Unprocessable Entity", &[], r#"{"message": "Validation Failed"}"#);
    assert_eq!(bare.to_string(), "Validation Failed");
}

#[test]
fn a_page_past_the_cap_is_past_the_limit() {
    let body = json!({
        "message": "Only the first 1000 search results are available",
        "documentation_url": "https://docs.github.com/v3/search/",
        "status": "422",
    });
    let error = search_error("422 Unprocessable Entity", &[], &body.to_string());
    assert!(matches!(error, SearchError::PastLimit), "{error:?}");
    assert_eq!(error.to_string(), "Only the first 1000 search results are available");
    assert!(error.gh_error().is_none());
}

#[test]
fn other_failures_keep_their_gh_error() {
    let error = search_error("500 Internal Server Error", &[], r#"{"message": "Server Error"}"#);
    assert!(
        matches!(&error, SearchError::Gh(GhError::Http { status: 500, .. })),
        "{error:?}"
    );
    assert_eq!(error.to_string(), "GitHub API request failed (HTTP 500): Server Error");
    assert!(error.gh_error().is_some());
    // Another 422 is a plain HTTP error.
    let other = search_error("422 Unprocessable Entity", &[], r#"{"message": "Something else"}"#);
    assert!(matches!(other, SearchError::Gh(GhError::Http { status: 422, .. })));
    assert!(matches!(
        SearchError::from(GhError::NotInstalled),
        SearchError::Gh(GhError::NotInstalled)
    ));
    assert_eq!(
        SearchError::from(SearchQueryError::Empty).to_string(),
        "Type a query to search"
    );
}

#[test]
fn a_search_rate_limit_is_a_gh_error_in_the_search_bucket() {
    let error = search_error(
        "403 Forbidden",
        &[
            ("X-RateLimit-Limit", "30"),
            ("X-RateLimit-Remaining", "0"),
            ("X-RateLimit-Reset", "1792933200"),
            ("X-RateLimit-Resource", "search"),
        ],
        r#"{"message": "API rate limit exceeded for user ID 1."}"#,
    );
    let gh = error.gh_error().expect("a gh error");
    assert_eq!(
        gh.rate_limit(),
        Some(RateLimit::Until(Timestamp::from_second(1_792_933_200).unwrap()))
    );
    assert_eq!(gh.rate_limit_resource(), Some("search"));
}

#[test]
fn the_pull_request_q_per_state() {
    let repo = repository("gizmo385/gh-lazy");
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
    // It names its repo, so it goes out as it is whatever the scope.
    let text = pull_request_search_q(&repo, IssueStateFilter::Open);
    assert_eq!(
        SearchQuery::new(text.clone(), SearchScope::Owner("x".into()))
            .q()
            .unwrap(),
        text
    );
}
