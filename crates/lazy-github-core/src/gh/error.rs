//! The errors a `gh` invocation can produce.

use std::time::Duration;

use jiff::Timestamp;

/// Why a `gh` invocation failed. Each variant's `Display` is a sentence fit to show the user.
#[derive(Debug, thiserror::Error)]
pub enum GhError {
    /// `gh` could not be spawned because it isn't installed or isn't on `PATH`.
    #[error(
        "The GitHub CLI (gh) is not installed or not on your PATH. Install it from https://cli.github.com, then run `gh auth login`."
    )]
    NotInstalled,
    /// `gh` is not logged in: `gh api` exited with code 4, or `gh auth status` failed.
    #[error(
        "To proceed, please log into the Github CLI via `gh auth login{}`",
        hostname.as_ref().map(|h| format!(" --hostname {h}")).unwrap_or_default()
    )]
    NotAuthenticated {
        /// The GitHub Enterprise host the login is for, when it isn't github.com.
        hostname: Option<String>,
        /// What `gh` printed, for the log.
        detail: String,
    },
    /// GitHub answered with an HTTP error status.
    #[error("GitHub API request failed (HTTP {status}){}", detail_suffix(message))]
    Http {
        /// The HTTP status code, 300 or above.
        status: u16,
        /// The `message` field of the JSON error body, or `gh`'s error line; may be empty.
        message: String,
        /// The raw response body.
        body: Vec<u8>,
        /// The response headers as `(name, value)` pairs, in order. Empty when the request was paginated, as
        /// `gh api --paginate` doesn't print them.
        headers: Vec<(String, String)>,
    },
    /// `gh` exited non-zero without an HTTP response (network down, bad flag).
    #[error(
        "gh failed{}{}",
        code.map(|c| format!(" with exit code {c}")).unwrap_or_default(),
        detail_suffix(stderr)
    )]
    Command {
        /// The exit code, or `None` if `gh` was killed by a signal.
        code: Option<i32>,
        /// `gh`'s full error output, trimmed. May span several lines; `Display` folds it onto one.
        stderr: String,
    },
    /// `gh api -i` output had no HTTP status line.
    #[error("could not parse gh output: {0}")]
    Parse(String),
    /// The response body was not the JSON shape expected.
    #[error("unexpected response from GitHub: {0}")]
    Decode(#[from] serde_json::Error),
    /// Spawning or talking to the `gh` process failed.
    #[error("failed to run gh: {0}")]
    Io(#[from] std::io::Error),
    /// `gh` ran past its time limit and was killed.
    #[error("gh did not respond within {} seconds", after.as_secs())]
    Timeout {
        /// The limit it ran past.
        after: Duration,
    },
}

/// `": "` and `detail` folded onto one line (non-empty lines joined with `"; "`), or nothing if `detail` is blank.
fn detail_suffix(detail: &str) -> String {
    let lines: Vec<&str> = detail.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!(": {}", lines.join("; "))
    }
}

/// How long a rate limit is waited out when the response doesn't say: a minute, as GitHub's docs advise for a
/// secondary limit that sends no `Retry-After`.
pub const RATE_LIMIT_FALLBACK: Duration = Duration::from_mins(1);

/// GitHub's messages for a rate-limited request, lowercase: the primary limit's "API rate limit exceeded for ..." and
/// the secondary limit's "You have exceeded a secondary rate limit ...".
const RATE_LIMIT_MESSAGES: [&str; 2] = ["api rate limit exceeded", "secondary rate limit"];

/// When a GitHub rate limit lifts, read from the headers of the response that hit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimit {
    /// The primary limit: `X-RateLimit-Remaining: 0`, lifting at `X-RateLimit-Reset`.
    Until(Timestamp),
    /// A secondary limit: `Retry-After`, this long after the response.
    After(Duration),
}

impl RateLimit {
    /// The moment the limit lifts, for a response that arrived at `now`.
    pub fn reset(self, now: Timestamp) -> Timestamp {
        match self {
            RateLimit::Until(at) => at,
            // Past jiff's range only for an absurd header; treat that as lifting at once.
            RateLimit::After(wait) => now.checked_add(wait).unwrap_or(now),
        }
    }
}

/// The value of the first header named `name`, compared case-insensitively, with surrounding whitespace trimmed.
pub(super) fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

/// The headers GitHub describes its rate limits with, in the order the command log lists them: the ones that say when
/// the limit lifts first, as a narrow log cuts the line.
const RATE_LIMIT_HEADERS: [&str; 5] = [
    "Retry-After",
    "X-RateLimit-Remaining",
    "X-RateLimit-Reset",
    "X-RateLimit-Limit",
    "X-RateLimit-Resource",
];

impl GhError {
    /// Whether the failure means the user has to (re)authenticate: `NotAuthenticated` or an HTTP 401.
    pub fn is_unauthorized(&self) -> bool {
        matches!(
            self,
            GhError::NotAuthenticated { .. } | GhError::Http { status: 401, .. }
        )
    }

    /// The rate limit this failure hit, if it's one: an HTTP 403 or 429 with a `Retry-After` header (a secondary
    /// limit), or with `X-RateLimit-Remaining: 0` and an `X-RateLimit-Reset` (the primary limit). `Retry-After` wins
    /// when both are present, as GitHub's docs say. Any other 403, such as a missing permission, is `None`.
    ///
    /// A response with no headers (`gh api --paginate` prints none, and an error read off `gh`'s stderr has none) is
    /// a limit when GitHub's message says so ("API rate limit exceeded", "secondary rate limit"), lifting
    /// [`RATE_LIMIT_FALLBACK`] after it arrived.
    pub fn rate_limit(&self) -> Option<RateLimit> {
        let GhError::Http {
            status: 403 | 429,
            headers,
            message,
            ..
        } = self
        else {
            return None;
        };
        if headers.is_empty() {
            let message = message.to_ascii_lowercase();
            return RATE_LIMIT_MESSAGES
                .iter()
                .any(|text| message.contains(text))
                .then_some(RateLimit::After(RATE_LIMIT_FALLBACK));
        }
        if let Some(seconds) = find_header(headers, "retry-after").and_then(|value| value.parse::<u64>().ok()) {
            return Some(RateLimit::After(Duration::from_secs(seconds)));
        }
        if find_header(headers, "x-ratelimit-remaining")? != "0" {
            return None;
        }
        let reset = find_header(headers, "x-ratelimit-reset")?.parse::<i64>().ok()?;
        Timestamp::from_second(reset).ok().map(RateLimit::Until)
    }

    /// The rate-limit headers of an HTTP failure as `Name: value` pairs joined with `", "`, in GitHub's spelling.
    /// Empty when there are none.
    pub fn rate_limit_headers(&self) -> String {
        let GhError::Http { headers, .. } = self else {
            return String::new();
        };
        RATE_LIMIT_HEADERS
            .iter()
            .filter_map(|name| find_header(headers, name).map(|value| format!("{name}: {value}")))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_one_sentence(text: &str) {
        assert!(!text.is_empty());
        assert!(!text.contains('\n'), "multi-line message: {text:?}");
        assert!(text.chars().next().is_some_and(|c| !c.is_whitespace()), "{text:?}");
    }

    #[test]
    fn every_variant_displays_one_sentence() {
        let network = crate::gh::interpret(
            Some(1),
            b"",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/gh/network_error.stderr"
            )),
            false,
        )
        .unwrap_err();
        assert!(matches!(&network, GhError::Command { stderr, .. } if stderr.lines().count() == 2));
        let decode = serde_json::from_str::<u32>("\"x\"").unwrap_err();
        let cases = [
            (
                GhError::NotInstalled,
                "The GitHub CLI (gh) is not installed or not on your PATH. Install it from https://cli.github.com, then run `gh auth login`.",
            ),
            (
                GhError::NotAuthenticated {
                    hostname: None,
                    detail: "ignored".into(),
                },
                "To proceed, please log into the Github CLI via `gh auth login`",
            ),
            (
                GhError::NotAuthenticated {
                    hostname: Some("ghe.example.com".into()),
                    detail: String::new(),
                },
                "To proceed, please log into the Github CLI via `gh auth login --hostname ghe.example.com`",
            ),
            (
                GhError::Http {
                    status: 404,
                    message: "Not Found".into(),
                    body: Vec::new(),
                    headers: Vec::new(),
                },
                "GitHub API request failed (HTTP 404): Not Found",
            ),
            (
                network,
                "gh failed with exit code 1: error connecting to api.github.com; check your internet connection or https://githubstatus.com",
            ),
            (
                GhError::Command {
                    code: Some(2),
                    stderr: String::new(),
                },
                "gh failed with exit code 2",
            ),
            (
                GhError::Http {
                    status: 502,
                    message: String::new(),
                    body: Vec::new(),
                    headers: Vec::new(),
                },
                "GitHub API request failed (HTTP 502)",
            ),
            (
                GhError::Command {
                    code: None,
                    stderr: "killed".into(),
                },
                "gh failed: killed",
            ),
            (
                GhError::Parse("no HTTP status line".into()),
                "could not parse gh output: no HTTP status line",
            ),
            (
                GhError::Io(std::io::Error::other("broken pipe")),
                "failed to run gh: broken pipe",
            ),
            (
                GhError::Timeout {
                    after: Duration::from_secs(30),
                },
                "gh did not respond within 30 seconds",
            ),
        ];
        for (error, expected) in cases {
            let text = error.to_string();
            assert_eq!(text, expected);
            is_one_sentence(&text);
        }
        let text = GhError::Decode(decode).to_string();
        assert!(text.starts_with("unexpected response from GitHub: invalid type"));
        is_one_sentence(&text);
    }

    fn http(status: u16, headers: &[(&str, &str)]) -> GhError {
        GhError::Http {
            status,
            message: String::new(),
            body: Vec::new(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn a_403_with_no_remaining_requests_is_the_primary_limit() {
        let error = http(
            403,
            &[
                ("x-ratelimit-limit", "5000"),
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1792933200"),
                ("x-ratelimit-resource", "core"),
            ],
        );
        let reset = Timestamp::from_second(1_792_933_200).unwrap();
        assert_eq!(error.rate_limit(), Some(RateLimit::Until(reset)));
        assert_eq!(error.rate_limit().unwrap().reset(Timestamp::UNIX_EPOCH), reset);
        assert_eq!(
            error.rate_limit_headers(),
            "X-RateLimit-Remaining: 0, X-RateLimit-Reset: 1792933200, X-RateLimit-Limit: 5000, X-RateLimit-Resource: core"
        );
        // A 429 with the same headers is the same limit.
        let error = http(
            429,
            &[("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", "1792933200")],
        );
        assert_eq!(error.rate_limit(), Some(RateLimit::Until(reset)));
    }

    #[test]
    fn a_429_with_retry_after_is_a_secondary_limit() {
        let error = http(429, &[("Retry-After", " 60 "), ("X-RateLimit-Remaining", "4000")]);
        assert_eq!(error.rate_limit(), Some(RateLimit::After(Duration::from_mins(1))));
        let now = Timestamp::from_second(1_792_929_600).unwrap();
        assert_eq!(
            error.rate_limit().unwrap().reset(now),
            Timestamp::from_second(1_792_929_660).unwrap()
        );
        assert_eq!(
            error.rate_limit_headers(),
            "Retry-After: 60, X-RateLimit-Remaining: 4000"
        );
        // Retry-After wins over an exhausted primary limit, as GitHub's docs say.
        let both = http(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1792933200"),
                ("retry-after", "30"),
            ],
        );
        assert_eq!(both.rate_limit(), Some(RateLimit::After(Duration::from_secs(30))));
    }

    #[test]
    fn other_failures_are_not_rate_limits() {
        // A permission 403: requests remain.
        let permission = http(
            403,
            &[("X-RateLimit-Remaining", "4999"), ("X-RateLimit-Reset", "1792933200")],
        );
        assert_eq!(permission.rate_limit(), None);
        assert_eq!(http(403, &[]).rate_limit(), None);
        assert_eq!(http(429, &[]).rate_limit(), None);
        // No reset time to wait for.
        assert_eq!(http(403, &[("X-RateLimit-Remaining", "0")]).rate_limit(), None);
        assert_eq!(
            http(403, &[("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", "soon")]).rate_limit(),
            None
        );
        // Other statuses never are, whatever the headers.
        assert_eq!(http(500, &[("Retry-After", "60")]).rate_limit(), None);
        assert_eq!(GhError::NotInstalled.rate_limit(), None);
        assert_eq!(GhError::NotInstalled.rate_limit_headers(), "");
    }

    fn http_message(status: u16, message: &str) -> GhError {
        GhError::Http {
            status,
            message: message.into(),
            body: Vec::new(),
            headers: Vec::new(),
        }
    }

    #[test]
    fn without_headers_github_s_message_says_it_s_a_limit() {
        let fallback = Some(RateLimit::After(Duration::from_mins(1)));
        for (status, message) in [
            (403, "API rate limit exceeded for user ID 1."),
            (429, "API rate limit exceeded for 203.0.113.7."),
            (
                403,
                "You have exceeded a secondary rate limit. Please wait a few minutes before you try again.",
            ),
            (429, "You have exceeded a SECONDARY RATE LIMIT"),
        ] {
            assert_eq!(
                http_message(status, message).rate_limit(),
                fallback,
                "{status} {message}"
            );
        }
        // Another 403, another status, or no message: not a limit.
        assert_eq!(
            http_message(403, "Resource not accessible by integration").rate_limit(),
            None
        );
        assert_eq!(http_message(500, "API rate limit exceeded").rate_limit(), None);
        assert_eq!(http_message(403, "").rate_limit(), None);
    }

    #[test]
    fn with_headers_only_the_headers_count() {
        // A permission 403 that happens to quote the text, with requests left: the headers say it's not a limit.
        let error = GhError::Http {
            status: 403,
            message: "API rate limit exceeded for user ID 1.".into(),
            body: Vec::new(),
            headers: vec![("X-RateLimit-Remaining".into(), "4999".into())],
        };
        assert_eq!(error.rate_limit(), None);
    }

    #[test]
    fn a_paginated_limit_is_recognised_from_gh_s_error_line() {
        let stdout = b"[{\"id\":1}]{\"message\":\"API rate limit exceeded for user ID 1.\"}";
        let error = crate::gh::interpret(
            Some(1),
            stdout,
            b"gh: API rate limit exceeded for user ID 1. (HTTP 403)\n",
            true,
        )
        .unwrap_err();
        assert_eq!(error.rate_limit(), Some(RateLimit::After(Duration::from_mins(1))));
        assert_eq!(error.rate_limit_headers(), "");
    }

    #[test]
    fn interpret_keeps_the_headers_of_an_error_response() {
        let stdout = b"HTTP/2.0 403 Forbidden\r\nX-Ratelimit-Remaining: 0\r\nX-Ratelimit-Reset: 1792933200\r\n\r\n\
            {\"message\":\"API rate limit exceeded for user ID 1.\"}";
        let error = crate::gh::interpret(
            Some(1),
            stdout,
            b"gh: API rate limit exceeded for user ID 1. (HTTP 403)\n",
            false,
        )
        .unwrap_err();
        assert_eq!(
            error.rate_limit(),
            Some(RateLimit::Until(Timestamp::from_second(1_792_933_200).unwrap()))
        );
        assert_eq!(
            error.to_string(),
            "GitHub API request failed (HTTP 403): API rate limit exceeded for user ID 1."
        );
    }

    #[test]
    fn unauthorized() {
        assert!(
            GhError::NotAuthenticated {
                hostname: None,
                detail: String::new()
            }
            .is_unauthorized()
        );
        assert!(
            GhError::Http {
                status: 401,
                message: String::new(),
                body: Vec::new(),
                headers: Vec::new(),
            }
            .is_unauthorized()
        );
        assert!(
            !GhError::Http {
                status: 403,
                message: String::new(),
                body: Vec::new(),
                headers: Vec::new(),
            }
            .is_unauthorized()
        );
        assert!(!GhError::NotInstalled.is_unauthorized());
        assert!(
            !GhError::Timeout {
                after: Duration::from_secs(20)
            }
            .is_unauthorized()
        );
    }
}
