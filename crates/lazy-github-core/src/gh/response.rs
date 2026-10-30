//! Parsing `gh api` output and mapping a finished process to a response or a [`GhError`].

use serde::de::DeserializeOwned;

use super::GhError;
use super::error::find_header;

/// `gh api` exits with this code when no credentials are available.
const EXIT_AUTH_REQUIRED: i32 = 4;

/// A successful HTTP response from `gh api`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhResponse {
    /// The HTTP status code, below 300.
    pub status: u16,
    headers: Vec<(String, String)>,
    /// The response body, verbatim.
    pub body: Vec<u8>,
}

impl GhResponse {
    /// The value of the first header named `name`, compared case-insensitively, with surrounding whitespace trimmed.
    pub fn header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }

    /// Decodes the body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, GhError> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// The body as text, replacing invalid UTF-8.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Whether the `Link` header points at a next page.
    pub fn has_next_page(&self) -> bool {
        self.header("link").is_some_and(|link| {
            link.split(',')
                .any(|part| part.split(';').skip(1).any(|p| p.trim() == "rel=\"next\""))
        })
    }
}

/// An HTTP response as printed by `gh api -i`, before deciding whether it is a success.
struct RawResponse {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Removes the first line from `rest` and returns it without its LF or CRLF ending.
fn take_line<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
    if rest.is_empty() {
        return None;
    }
    let (line, tail) = match rest.iter().position(|&b| b == b'\n') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (*rest, &rest[rest.len()..]),
    };
    *rest = tail;
    Some(line.strip_suffix(b"\r").unwrap_or(line))
}

/// Splits `gh api -i` output into status line, headers and body.
///
/// The output is `HTTP/<ver> <code> <reason>`, `Name: value` header lines, an empty line, then the body verbatim.
/// Lines may end in LF or CRLF. Returns `None` if the output doesn't start with a status line.
fn parse_include(stdout: &[u8]) -> Option<RawResponse> {
    let mut rest = stdout;

    let status_line = std::str::from_utf8(take_line(&mut rest)?).ok()?;
    let mut parts = status_line.splitn(3, ' ');
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    let status = parts.next()?.parse().ok()?;
    let reason = parts.next().unwrap_or_default().trim().to_owned();

    let mut headers = Vec::new();
    while let Some(line) = take_line(&mut rest) {
        if line.is_empty() {
            break;
        }
        let line = String::from_utf8_lossy(line);
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    Some(RawResponse {
        status,
        reason,
        headers,
        body: rest.to_vec(),
    })
}

/// The status and message from `gh`'s HTTP error line: `gh: <message> (HTTP <code>)`, or bare `gh: HTTP <code>`
/// (empty message). `gh` may print more lines after it, such as a missing-scope hint; those are skipped.
fn stderr_http_error(stderr: &str) -> Option<(u16, String)> {
    stderr.lines().find_map(|line| {
        let line = line.trim();
        if let Some(code) = line.strip_prefix("gh: HTTP ") {
            return Some((code.trim().parse().ok()?, String::new()));
        }
        let text = line.strip_prefix("gh: ").unwrap_or(line);
        let inner = text.strip_suffix(')')?;
        let start = inner.rfind("(HTTP ")?;
        let status = inner[start + "(HTTP ".len()..].parse().ok()?;
        Some((status, inner[..start].trim().to_owned()))
    })
}

/// The `message` field of a JSON error body.
fn json_message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.get("message")?.as_str().map(str::to_owned)
}

/// The last JSON value in `stdout`. A paginated request that fails on a later page has already printed the earlier
/// pages, so the error body is the trailing object. Returns all of `stdout` if it isn't a clean JSON stream.
fn trailing_json_value(stdout: &[u8]) -> &[u8] {
    let mut stream = serde_json::Deserializer::from_slice(stdout).into_iter::<serde::de::IgnoredAny>();
    let mut start = 0;
    let mut end = 0;
    while let Some(value) = stream.next() {
        if value.is_err() {
            return stdout;
        }
        start = end;
        end = stream.byte_offset();
    }
    stdout[start..end].trim_ascii()
}

fn http_error(status: u16, message: String, body: Vec<u8>, headers: Vec<(String, String)>, fallback: &str) -> GhError {
    let message = json_message(&body)
        .or_else(|| Some(message).filter(|m| !m.is_empty()))
        .unwrap_or_else(|| fallback.to_owned());
    GhError::Http {
        status,
        message,
        body,
        headers,
    }
}

fn command_error(exit: Option<i32>, stderr: &str) -> GhError {
    GhError::Command {
        code: exit,
        stderr: stderr.trim().to_owned(),
    }
}

/// Maps a finished `gh api` process to a response or an error. Pure, so it is tested without spawning `gh`.
///
/// `exit` is the exit code (`None` if killed by a signal). With `paginated`, `stdout` is the concatenated page bodies
/// of `gh api --paginate` (no `-i`), and the returned response carries them with status 200 and no headers.
///
/// A [`GhError::NotAuthenticated`] returned here has no hostname; the caller fills it in.
pub fn interpret(exit: Option<i32>, stdout: &[u8], stderr: &[u8], paginated: bool) -> Result<GhResponse, GhError> {
    let stderr = String::from_utf8_lossy(stderr);
    if exit == Some(EXIT_AUTH_REQUIRED) {
        return Err(GhError::NotAuthenticated {
            hostname: None,
            detail: stderr.trim().to_owned(),
        });
    }

    if paginated {
        if exit == Some(0) {
            return Ok(GhResponse {
                status: 200,
                headers: Vec::new(),
                body: stdout.to_vec(),
            });
        }
        return Err(match stderr_http_error(&stderr) {
            Some((status, message)) => {
                http_error(status, message, trailing_json_value(stdout).to_vec(), Vec::new(), "")
            }
            None => command_error(exit, &stderr),
        });
    }

    match parse_include(stdout) {
        Some(raw) if raw.status >= 300 => {
            let message = stderr_http_error(&stderr)
                .map(|(_, message)| message)
                .unwrap_or_default();
            Err(http_error(raw.status, message, raw.body, raw.headers, &raw.reason))
        }
        Some(raw) if exit == Some(0) => Ok(GhResponse {
            status: raw.status,
            headers: raw.headers,
            body: raw.body,
        }),
        // A success status but a failing exit code: `gh` itself failed after the response.
        Some(_) => Err(command_error(exit, &stderr)),
        None if exit == Some(0) => Err(GhError::Parse(if stdout.is_empty() {
            "gh printed no response".to_owned()
        } else {
            "gh output did not start with an HTTP status line".to_owned()
        })),
        None => Err(match stderr_http_error(&stderr) {
            Some((status, message)) => http_error(status, message, Vec::new(), Vec::new(), ""),
            None => command_error(exit, &stderr),
        }),
    }
}

/// Decodes `gh api --paginate` output: one JSON array per page, concatenated, flattened into one list.
pub(crate) fn decode_pages<T: DeserializeOwned>(body: &[u8]) -> Result<Vec<T>, GhError> {
    let mut items = Vec::new();
    for page in serde_json::Deserializer::from_slice(body).into_iter::<Vec<T>>() {
        items.extend(page?);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_parsing() {
        let error = stderr_http_error;
        assert_eq!(error("gh: Not Found (HTTP 404)\n"), Some((404, "Not Found".to_owned())));
        assert_eq!(
            error(
                "gh: Resource not accessible by integration (HTTP 403)\n\
                 This API operation needs the \"admin:org\" scope. To request it, run:  gh auth refresh -s admin:org\n"
            ),
            Some((403, "Resource not accessible by integration".to_owned()))
        );
        assert_eq!(error("gh: HTTP 502\n"), Some((502, String::new())));
        assert_eq!(error("error connecting to api.github.com"), None);
        assert_eq!(error("gh: HTTP nope"), None);
    }

    #[test]
    fn trailing_json_value_picks_the_last_object() {
        assert_eq!(
            trailing_json_value(b"[1,2][3]\n{\"message\":\"x\"}\n"),
            b"{\"message\":\"x\"}"
        );
        assert_eq!(trailing_json_value(b"{\"a\":1}"), b"{\"a\":1}");
        assert_eq!(trailing_json_value(b"[1] not json"), b"[1] not json");
        assert_eq!(trailing_json_value(b""), b"");
    }

    #[test]
    fn bare_http_status_line_on_stderr() {
        let error = interpret(Some(1), b"", b"gh: HTTP 502\n", true).unwrap_err();
        assert!(matches!(&error, GhError::Http { status: 502, message, .. } if message.is_empty()));
        assert_eq!(error.to_string(), "GitHub API request failed (HTTP 502)");
    }

    #[test]
    fn scope_hint_does_not_leak_into_message() {
        let stderr = b"gh: Resource not accessible by integration (HTTP 403)\nThis API operation needs a scope.\n";
        let error = interpret(Some(1), b"", stderr, false).unwrap_err();
        assert!(
            matches!(&error, GhError::Http { status: 403, message, .. } if message == "Resource not accessible by integration")
        );
    }

    #[test]
    fn decode_pages_handles_empty_output() {
        assert!(decode_pages::<u32>(b"").unwrap().is_empty());
        assert_eq!(decode_pages::<u32>(b"[1,2]\n[3]").unwrap(), [1, 2, 3]);
        assert!(matches!(decode_pages::<u32>(b"[1,\"x\"]"), Err(GhError::Decode(_))));
    }

    #[test]
    fn status_line_without_reason() {
        let raw = parse_include(b"HTTP/2.0 204\r\n\r\n").unwrap();
        assert_eq!(raw.status, 204);
        assert!(raw.body.is_empty());
    }

    #[test]
    fn output_without_blank_line_has_empty_body() {
        let raw = parse_include(b"HTTP/1.1 204 No Content\nX-A: b").unwrap();
        assert_eq!(raw.headers, [("X-A".to_owned(), "b".to_owned())]);
        assert!(raw.body.is_empty());
    }
}
