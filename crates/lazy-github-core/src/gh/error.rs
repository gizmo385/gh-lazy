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
    #[error("GitHub API request failed (HTTP {status}): {message}")]
    Http {
        /// The HTTP status code, 300 or above.
        status: u16,
        /// The `message` field of the JSON error body, or `gh`'s error output.
        message: String,
        /// The raw response body.
        body: Vec<u8>,
    },
    /// `gh` exited non-zero without an HTTP response (network down, bad flag).
    #[error(
        "gh failed{}: {stderr}",
        code.map(|c| format!(" with exit code {c}")).unwrap_or_default()
    )]
    Command {
        /// The exit code, or `None` if `gh` was killed by a signal.
        code: Option<i32>,
        /// `gh`'s error output, trimmed.
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
                GhError::Command {
                    code: Some(1),
                    stderr: "error connecting to api.github.com".into(),
                },
                "gh failed with exit code 1: error connecting to api.github.com",
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
