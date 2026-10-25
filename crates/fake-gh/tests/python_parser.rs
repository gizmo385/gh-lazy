//! fake-gh's `-i` output fed through a Rust port of the Python app's parser
//! (`_parse_cli_api_response` in `lazy_github/lib/github/backends/cli.py`), plus snapshots of the exact bytes.

use std::collections::BTreeMap;

use fake_gh::fixture::Response;
use fake_gh::output::{Rendered, write_api_response};
use serde_json::json;

/// What the Python parser extracts from gh's stdout.
#[derive(Debug, PartialEq)]
struct Parsed {
    status: u16,
    /// Insertion-ordered like a Python dict; a repeated name keeps its first position and the last value.
    headers: Vec<(String, String)>,
    body: String,
}

/// Python's `str.splitlines()` boundaries.
fn python_splitlines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let is_break = matches!(
            c,
            '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
        );
        if is_break {
            lines.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

/// `^([a-zA-Z-]+)\:(.+)$`
fn python_header(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let (name, value) = (&line[..colon], &line[colon + 1..]);
    let name_ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-');
    (name_ok && !value.is_empty()).then_some((name, value))
}

fn python_parse(stdout: &str) -> Parsed {
    let mut parsed = Parsed {
        status: 0,
        headers: Vec::new(),
        body: String::new(),
    };
    let mut content = Vec::new();
    for line in python_splitlines(stdout) {
        if line.is_empty() {
            continue;
        }
        if line.to_lowercase().starts_with("http/2.0") {
            parsed.status = line.split(' ').nth(1).unwrap().parse().unwrap();
        } else if let Some((name, value)) = python_header(line) {
            match parsed.headers.iter_mut().find(|(n, _)| n == name) {
                Some(slot) => value.clone_into(&mut slot.1),
                None => parsed.headers.push((name.to_owned(), value.to_owned())),
            }
        } else {
            content.push(line);
        }
    }
    parsed.body = content.join("\n");
    parsed
}

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
fn json_200_round_trips() {
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
    let body =
        json!([{"name": "lazy-demo", "full_name": "parity-bot/lazy-demo", "private": false, "description": "a: b"}]);
    resp.body = Some(body.clone());
    let out = render(&resp);
    insta::assert_snapshot!("json_200", escaped(&out));

    let parsed = python_parse(&out);
    assert_eq!(parsed.status, 200);
    let names: Vec<&str> = parsed.headers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["Content-Type", "Etag", "Link", "X-Ratelimit-Remaining"],
        "canonical case, sorted"
    );
    // Python keeps the space after the colon; `Link` (not `link`) is why it only loads page 1 under gh.
    assert_eq!(
        parsed.headers[2].1,
        r#" <https://api.github.com/user/repos?page=2&per_page=50>; rel="next""#
    );
    assert!(!parsed.headers.iter().any(|(n, _)| n == "link"));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&parsed.body).unwrap(), body);
}

#[test]
fn not_found_round_trips() {
    let rendered = Rendered::not_found();
    let mut out = Vec::new();
    write_api_response(&mut out, &rendered, true).unwrap();
    let out = String::from_utf8(out).unwrap();
    insta::assert_snapshot!("not_found_404", escaped(&out));

    let parsed = python_parse(&out);
    assert_eq!(parsed.status, 404);
    assert_eq!(
        parsed.headers,
        [("Content-Type".to_owned(), " application/json; charset=utf-8".to_owned())]
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&parsed.body).unwrap(),
        json!({"message": "Not Found", "documentation_url": "https://docs.github.com/rest", "status": "404"})
    );
}

#[test]
fn text_body_round_trips() {
    let mut resp = response(200, &[("Content-Type", "application/vnd.github.raw; charset=utf-8")]);
    let text = "# parity-bot/lazy-demo\nDemo data for the parity harness.\n- one\n- two\n";
    resp.body_text = Some(text.into());
    let out = render(&resp);
    insta::assert_snapshot!("text_200", escaped(&out));

    let parsed = python_parse(&out);
    assert_eq!(parsed.status, 200);
    assert_eq!(
        parsed.headers,
        [(
            "Content-Type".to_owned(),
            " application/vnd.github.raw; charset=utf-8".to_owned()
        )]
    );
    // splitlines + join drops the final newline; nothing else changes for text without blank or `Word:` lines.
    assert_eq!(parsed.body, text.trim_end_matches('\n'));
}

#[test]
fn python_quirks_are_reproduced_not_fixed() {
    // Blank lines and `Word: text` lines vanish from text bodies under the Python parser. fake-gh prints the body
    // byte for byte, exactly as real gh does, so the quirk shows up the same way.
    let mut resp = response(200, &[]);
    resp.body_text = Some("Title\n\nNote: kept by gh\nend\n".into());
    let parsed = python_parse(&render(&resp));
    assert_eq!(parsed.body, "Title\nend");
    assert!(parsed.headers.iter().any(|(n, v)| n == "Note" && v == " kept by gh"));
}

#[test]
fn an_http_1_1_status_line_would_not_be_recognized() {
    // Why fake-gh always prints HTTP/2.0.
    assert_eq!(python_parse("HTTP/1.1 200 OK\n\r\n[]").status, 0);
    assert_eq!(python_parse("HTTP/2.0 201 Created\n\r\n[]").status, 201);
}
