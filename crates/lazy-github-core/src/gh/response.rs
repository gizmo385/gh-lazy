//! Parsing `gh api` output and mapping a finished process to a response or a [`GhError`].

use serde::de::DeserializeOwned;

use super::GhError;

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
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
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

/// Finds the status in a `gh` error line ending in `(HTTP <code>)`.
fn stderr_http_status(stderr: &str) -> Option<u16> {
    stderr.lines().find_map(|line| {
        let inner = line.trim_end().strip_suffix(')')?;
        let start = inner.rfind("(HTTP ")?;
        inner[start + "(HTTP ".len()..].parse().ok()
    })
}

/// `gh`'s error output without the `gh: ` prefix and ` (HTTP n)` suffix.
fn stderr_message(stderr: &str) -> String {
    let trimmed = stderr.trim();
    let trimmed = trimmed.strip_prefix("gh: ").unwrap_or(trimmed);
    let trimmed = match trimmed.rfind(" (HTTP ") {
        Some(i) if trimmed.ends_with(')') => &trimmed[..i],
        _ => trimmed,
    };
    trimmed.trim().to_owned()
}

/// The `message` field of a JSON error body.
fn json_message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.get("message")?.as_str().map(str::to_owned)
}

fn http_error(status: u16, body: Vec<u8>, stderr: &str, fallback: &str) -> GhError {
    let message = json_message(&body)
        .or_else(|| Some(stderr_message(stderr)).filter(|m| !m.is_empty()))
        .unwrap_or_else(|| fallback.to_owned());
    GhError::Http { status, message, body }
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
        return Err(match stderr_http_status(&stderr) {
            Some(status) => http_error(status, stdout.to_vec(), &stderr, ""),
            None => GhError::Command {
                code: exit,
                stderr: stderr.trim().to_owned(),
            },
        });
    }

    match parse_include(stdout) {
        Some(raw) if raw.status >= 300 => Err(http_error(raw.status, raw.body, &stderr, &raw.reason)),
        Some(raw) if exit == Some(0) => Ok(GhResponse {
            status: raw.status,
            headers: raw.headers,
            body: raw.body,
        }),
        // A success status but a failing exit code: `gh` itself failed after the response.
        Some(_) => Err(GhError::Command {
            code: exit,
            stderr: stderr.trim().to_owned(),
        }),
        None if exit == Some(0) => Err(GhError::Parse(if stdout.is_empty() {
            "gh printed no response".to_owned()
        } else {
            "gh output did not start with an HTTP status line".to_owned()
        })),
        None => Err(match stderr_http_status(&stderr) {
            Some(status) => http_error(status, Vec::new(), &stderr, ""),
            None => GhError::Command {
                code: exit,
                stderr: stderr.trim().to_owned(),
            },
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
        assert_eq!(stderr_http_status("gh: Not Found (HTTP 404)\n"), Some(404));
        assert_eq!(
            stderr_http_status("gh: Validation Failed (HTTP 422)\nmore detail"),
            Some(422)
        );
        assert_eq!(stderr_http_status("error connecting to api.github.com"), None);
        assert_eq!(stderr_message("gh: Not Found (HTTP 404)\n"), "Not Found");
        assert_eq!(stderr_message("plain"), "plain");
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
