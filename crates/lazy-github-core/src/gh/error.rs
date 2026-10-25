//! The errors a `gh` invocation can produce.

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

impl GhError {
    /// Whether the failure means the user has to (re)authenticate: `NotAuthenticated` or an HTTP 401.
    pub fn is_unauthorized(&self) -> bool {
        matches!(
            self,
            GhError::NotAuthenticated { .. } | GhError::Http { status: 401, .. }
        )
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
                body: Vec::new()
            }
            .is_unauthorized()
        );
        assert!(
            !GhError::Http {
                status: 403,
                message: String::new(),
                body: Vec::new()
            }
            .is_unauthorized()
        );
        assert!(!GhError::NotInstalled.is_unauthorized());
    }
}
