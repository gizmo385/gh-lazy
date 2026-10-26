//! `interpret` over the captured `gh` outputs in `tests/fixtures/gh/`.
//!
//! Captured from gh 2.101.0 against github.com, with `X-Github-Request-Id` scrubbed: `labels_http2`, `labels_http11`
//! (`GODEBUG=http2client=0`), `no_content`, `not_found`, `readme_raw_real` and `paginated_merged`. Hand-written, for
//! cases github.com can't produce on demand or edge cases the real output doesn't hit: `auth_required`,
//! `network_error`, `paginated` (older gh's page-after-page form), `paginated_forbidden`, `paginated_page2_fails` and
//! `readme_raw` (body lines that look like headers).

use serde::Deserialize;

use super::response::decode_pages;
use super::{GhError, GhResponse, interpret};

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/gh/", $name)).as_slice()
    };
}

#[derive(Debug, Deserialize, PartialEq)]
struct Label {
    id: u64,
    name: String,
}

fn assert_labels_ok(response: &GhResponse) {
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json; charset=utf-8"));
    // Header lookup ignores case; the value is whatever the capture had left.
    assert!(
        response
            .header("X-RATELIMIT-REMAINING")
            .is_some_and(|n| n.parse::<u32>().is_ok())
    );
    assert_eq!(response.header("x-missing"), None);
    assert!(response.has_next_page());
    let labels: Vec<Label> = response.json().unwrap();
    assert_eq!(
        labels,
        [Label {
            id: 2_341_282_801,
            name: "accessibility".into()
        }]
    );
}

#[test]
fn ok_http2() {
    let response = interpret(Some(0), fixture!("labels_http2.stdout"), b"", false).unwrap();
    assert_labels_ok(&response);
}

#[test]
fn ok_http11() {
    let response = interpret(Some(0), fixture!("labels_http11.stdout"), b"", false).unwrap();
    assert_labels_ok(&response);
}

#[test]
fn no_content() {
    let response = interpret(Some(0), fixture!("no_content.stdout"), b"", false).unwrap();
    assert_eq!(response.status, 204);
    assert!(response.body.is_empty());
    assert_eq!(response.header("server"), Some("github.com"));
    assert!(!response.has_next_page());
}

#[test]
fn not_found() {
    let error = interpret(
        Some(1),
        fixture!("not_found.stdout"),
        fixture!("not_found.stderr"),
        false,
    )
    .unwrap_err();
    let GhError::Http { status, message, body } = error else {
        panic!("expected Http, got {error:?}");
    };
    assert_eq!(status, 404);
    assert_eq!(message, "Not Found");
    assert!(body.starts_with(b"{\"message\":\"Not Found\""));
}

#[test]
fn raw_readme_body_is_verbatim() {
    let response = interpret(Some(0), fixture!("readme_raw.stdout"), b"", false).unwrap();
    assert_eq!(
        response.text(),
        "# Project\n\nSome intro text.\n\n\nNote: x\nWarning: y\n\n```\ncode block\n```\n"
    );
    assert_eq!(response.header("note"), None);
}

#[test]
fn raw_readme_from_github() {
    let response = interpret(Some(0), fixture!("readme_raw_real.stdout"), b"", false).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.text(), "Hello World!\n");
    assert_eq!(
        response.header("content-type"),
        Some("application/vnd.github.raw; charset=utf-8")
    );
}

#[test]
fn auth_required() {
    let error = interpret(
        Some(4),
        fixture!("auth_required.stdout"),
        fixture!("auth_required.stderr"),
        false,
    )
    .unwrap_err();
    assert!(error.is_unauthorized());
    let GhError::NotAuthenticated { hostname, detail } = error else {
        panic!("expected NotAuthenticated, got {error:?}");
    };
    assert_eq!(hostname, None);
    assert!(detail.starts_with("To get started with GitHub CLI"));
}

#[test]
fn network_error() {
    let error = interpret(
        Some(1),
        fixture!("network_error.stdout"),
        fixture!("network_error.stderr"),
        false,
    )
    .unwrap_err();
    let GhError::Command { code, stderr } = error else {
        panic!("expected Command, got {error:?}");
    };
    assert_eq!(code, Some(1));
    assert!(stderr.starts_with("error connecting to api.github.com"));
}

#[test]
fn success_exit_without_status_line_is_parse_error() {
    assert!(matches!(interpret(Some(0), b"[]", b"", false), Err(GhError::Parse(_))));
}

#[test]
fn paginated_pages_are_flattened() {
    let response = interpret(Some(0), fixture!("paginated.stdout"), b"", true).unwrap();
    let labels: Vec<Label> = decode_pages(&response.body).unwrap();
    let ids: Vec<u64> = labels.iter().map(|l| l.id).collect();
    assert_eq!(ids, [1, 2, 3]);
}

/// gh 2.101 merges the pages of a JSON array into one array (`repos/octocat/linguist/labels?per_page=3`: 3 pages, 7
/// labels). Older versions print the pages one after another, as `paginated.stdout` has them.
#[test]
fn paginated_pages_merged_by_gh() {
    let response = interpret(Some(0), fixture!("paginated_merged.stdout"), b"", true).unwrap();
    let labels: Vec<Label> = decode_pages(&response.body).unwrap();
    let names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "bug",
            "duplicate",
            "enhancement",
            "help wanted",
            "invalid",
            "question",
            "wontfix"
        ]
    );
}

#[test]
fn paginated_forbidden() {
    let error = interpret(
        Some(1),
        fixture!("paginated_forbidden.stdout"),
        fixture!("paginated_forbidden.stderr"),
        true,
    )
    .unwrap_err();
    let GhError::Http { status, message, .. } = error else {
        panic!("expected Http, got {error:?}");
    };
    assert_eq!(status, 403);
    assert_eq!(message, "Resource not accessible by integration");
}

#[test]
fn paginated_network_error() {
    assert!(matches!(
        interpret(Some(1), b"", fixture!("network_error.stderr"), true),
        Err(GhError::Command { code: Some(1), .. })
    ));
}

#[test]
fn paginated_failure_on_page_two_keeps_only_the_error_body() {
    let error = interpret(
        Some(1),
        fixture!("paginated_page2_fails.stdout"),
        fixture!("paginated_page2_fails.stderr"),
        true,
    )
    .unwrap_err();
    let GhError::Http { status, message, body } = error else {
        panic!("expected Http, got {error:?}");
    };
    assert_eq!(status, 500);
    assert_eq!(message, "Server Error");
    assert_eq!(body, br#"{"message":"Server Error","status":"500"}"#);
}

#[test]
fn realistic_headers() {
    let response = interpret(Some(0), fixture!("labels_http2.stdout"), b"", false).unwrap();
    assert!(response.header("etag").is_some_and(|etag| etag.starts_with("W/\"")));
    assert_eq!(
        response.header("x-oauth-scopes"),
        Some("gist, notifications, read:org, repo, workflow")
    );
}
