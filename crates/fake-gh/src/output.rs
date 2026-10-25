//! Printing `gh api` responses the way real gh does.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::fixture::{Response, canonical_header_name};

/// A response ready to print: headers canonicalized and defaulted, body as bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub status: u16,
    /// Go-canonical names, sorted.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

impl Rendered {
    /// Renders a fixture response. `body_file` is the resolved path of `response.body_file`, if any.
    pub fn from_response(resp: &Response, body_file: Option<&Path>) -> io::Result<Self> {
        let body = if let Some(value) = &resp.body {
            serde_json::to_vec(value).map_err(io::Error::other)?
        } else if let Some(text) = &resp.body_text {
            text.as_bytes().to_vec()
        } else if let Some(path) = body_file {
            std::fs::read(path)?
        } else {
            Vec::new()
        };
        let default_type = if resp.body.is_some() {
            JSON_CONTENT_TYPE
        } else {
            TEXT_CONTENT_TYPE
        };
        Ok(Rendered::new(resp.status, &resp.headers, body, default_type))
    }

    fn new(status: u16, headers: &BTreeMap<String, String>, body: Vec<u8>, default_type: &str) -> Self {
        let mut headers: BTreeMap<String, String> = headers
            .iter()
            .map(|(k, v)| (canonical_header_name(k), v.clone()))
            .collect();
        headers
            .entry("Content-Type".into())
            .or_insert_with(|| default_type.into());
        Rendered { status, headers, body }
    }

    /// What GitHub answers for a path it doesn't know. fake-gh serves this on a miss.
    pub fn not_found() -> Self {
        let body =
            json!({"message": "Not Found", "documentation_url": "https://docs.github.com/rest", "status": "404"});
        // Serializing a `Value` built from literals can't fail.
        let body = serde_json::to_vec(&body).unwrap_or_default();
        Rendered::new(404, &BTreeMap::new(), body, JSON_CONTENT_TYPE)
    }

    /// The `message` field of a JSON body, as gh shows it in errors.
    pub fn message(&self) -> Option<String> {
        let value: Value = serde_json::from_slice(&self.body).ok()?;
        value.get("message")?.as_str().map(str::to_owned)
    }

    /// For status ≥ 400, the line real gh writes to stderr.
    pub fn error_line(&self) -> Option<String> {
        (self.status >= 400).then(|| match self.message() {
            Some(m) => format!("gh: {m} (HTTP {})\n", self.status),
            None => format!("gh: HTTP {}\n", self.status),
        })
    }

    /// The `Link: <url>; rel="next"` target, if any.
    pub fn next_link(&self) -> Option<String> {
        let link = self.headers.get("Link")?;
        link.split(',').find_map(|part| {
            let (url, params) = part.split_once(';')?;
            let is_next = params.split(';').any(|p| p.trim().replace(' ', "") == "rel=\"next\"");
            let url = url.trim().strip_prefix('<')?.strip_suffix('>')?;
            is_next.then(|| url.to_owned())
        })
    }
}

/// `HTTP/2.0 <status> <reason>`, as gh prints it (Go's `resp.Proto` and `resp.Status`).
pub fn status_line(status: u16) -> String {
    let reason = http::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason());
    match reason {
        Some(r) => format!("HTTP/2.0 {status} {r}"),
        None => format!("HTTP/2.0 {status}"),
    }
}

/// Prints one response. With `include`, the status line (LF) and headers (CRLF) come first, then a blank CRLF line.
/// The body is printed as-is, with no newline added.
pub fn write_api_response(w: &mut dyn Write, resp: &Rendered, include: bool) -> io::Result<()> {
    if include {
        // LF only, unlike the header lines; real gh does the same.
        writeln!(w, "{}", status_line(resp.status))?;
        for (name, value) in &resp.headers {
            write!(w, "{name}: {value}\r\n")?;
        }
        w.write_all(b"\r\n")?;
    }
    w.write_all(&resp.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, headers: &[(&str, &str)]) -> Response {
        Response {
            status,
            headers: headers.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            body: None,
            body_text: None,
            body_file: None,
            delay_ms: 0,
        }
    }

    #[test]
    fn headers_are_canonical_sorted_and_defaulted() {
        let mut resp = response(
            200,
            &[
                ("x-ratelimit-limit", "5000"),
                ("link", "<u>; rel=\"next\""),
                ("ETAG", "e"),
            ],
        );
        resp.body = Some(json!({"b": 1, "a": [1, 2]}));
        let r = Rendered::from_response(&resp, None).unwrap();
        let names: Vec<&str> = r.headers.keys().map(String::as_str).collect();
        assert_eq!(names, ["Content-Type", "Etag", "Link", "X-Ratelimit-Limit"]);
        assert_eq!(r.headers["Content-Type"], JSON_CONTENT_TYPE);
        assert_eq!(r.body, br#"{"b":1,"a":[1,2]}"#);
        assert_eq!(r.next_link().as_deref(), Some("u"));

        let mut text = response(200, &[("content-type", "text/x-diff")]);
        text.body_text = Some("diff\n".into());
        let r = Rendered::from_response(&text, None).unwrap();
        assert_eq!(r.headers["Content-Type"], "text/x-diff");
        assert_eq!(
            Rendered::from_response(&response(204, &[]), None).unwrap().headers["Content-Type"],
            TEXT_CONTENT_TYPE
        );
    }

    #[test]
    fn status_lines() {
        assert_eq!(status_line(200), "HTTP/2.0 200 OK");
        assert_eq!(status_line(404), "HTTP/2.0 404 Not Found");
        assert_eq!(status_line(422), "HTTP/2.0 422 Unprocessable Entity");
        assert_eq!(status_line(299), "HTTP/2.0 299");
    }

    #[test]
    fn error_lines() {
        assert_eq!(
            Rendered::not_found().error_line().as_deref(),
            Some("gh: Not Found (HTTP 404)\n")
        );
        let mut resp = response(500, &[]);
        resp.body_text = Some("oops".into());
        assert_eq!(
            Rendered::from_response(&resp, None).unwrap().error_line().as_deref(),
            Some("gh: HTTP 500\n")
        );
        assert_eq!(
            Rendered::from_response(&response(304, &[]), None).unwrap().error_line(),
            None
        );
    }

    #[test]
    fn next_link_among_several() {
        let resp = response(
            200,
            &[(
                "Link",
                r#"<https://api.github.com/x?page=1>; rel="prev", <https://api.github.com/x?page=3>; rel="next""#,
            )],
        );
        let r = Rendered::from_response(&resp, None).unwrap();
        assert_eq!(r.next_link().as_deref(), Some("https://api.github.com/x?page=3"));
        let last = response(200, &[("Link", r#"<https://api.github.com/x?page=1>; rel="first""#)]);
        assert_eq!(Rendered::from_response(&last, None).unwrap().next_link(), None);
    }
}
