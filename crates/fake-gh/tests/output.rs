//! The exact bytes fake-gh prints for `gh api -i`: the status line, canonical-case headers sorted by name, a blank
//! line and the body. The Rust client is built against this output (`README.md`, "Known differences from real gh").

use std::collections::BTreeMap;

use fake_gh::fixture::Response;
use fake_gh::output::{Rendered, write_api_response};
use serde_json::json;

fn render(resp: &Response) -> String {
    let rendered = Rendered::from_response(resp, None).unwrap();
    let mut out = Vec::new();
    write_api_response(&mut out, &rendered, true).unwrap();
    String::from_utf8(out).unwrap()
}

/// The exact bytes, escaped, with a line break after each `\n` so the snapshot stays readable.
fn escaped(s: &str) -> String {
    s.as_bytes().escape_ascii().to_string().replace("\\n", "\\n\n")
}

fn response(status: u16, headers: &[(&str, &str)]) -> Response {
    Response {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect::<BTreeMap<_, _>>(),
        body: None,
        body_text: None,
        body_file: None,
        delay_ms: 0,
    }
}

#[test]
fn json_200() {
    let mut resp = response(
        200,
        &[
            ("x-ratelimit-remaining", "4999"),
            (
                "link",
                r#"<https://api.github.com/user/repos?page=2&per_page=50>; rel="next""#,
            ),
            ("etag", "W/\"abc\""),
        ],
    );
    resp.body = Some(
        json!([{"name": "lazy-demo", "full_name": "parity-bot/lazy-demo", "private": false, "description": "a: b"}]),
    );
    insta::assert_snapshot!("json_200", escaped(&render(&resp)));
}

#[test]
fn not_found_404() {
    let rendered = Rendered::not_found();
    let mut out = Vec::new();
    write_api_response(&mut out, &rendered, true).unwrap();
    insta::assert_snapshot!("not_found_404", escaped(&String::from_utf8(out).unwrap()));
}

#[test]
fn text_200() {
    let mut resp = response(200, &[("Content-Type", "application/vnd.github.raw; charset=utf-8")]);
    resp.body_text = Some("# parity-bot/lazy-demo\nDemo data for the parity harness.\n- one\n- two\n".into());
    insta::assert_snapshot!("text_200", escaped(&render(&resp)));
}
