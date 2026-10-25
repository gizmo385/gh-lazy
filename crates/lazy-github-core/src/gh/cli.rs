//! Spawning `gh` and feeding it requests.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::debug;

use super::response::{decode_pages, interpret};
use super::{GhError, GhRequest, GhResponse};

/// Runs requests through the `gh` CLI. Cheap to clone; clones share configuration.
#[derive(Debug, Clone)]
pub struct GhCli {
    inner: Arc<GhCliInner>,
}

#[derive(Debug)]
struct GhCliInner {
    program: PathBuf,
    hostname: Option<String>,
}

impl Default for GhCli {
    /// Runs `gh` from `PATH` against github.com (or whatever `GH_HOST` selects).
    fn default() -> Self {
        Self::new("gh", None)
    }
}

impl GhCli {
    /// A client that runs `program` and, when `hostname` is set, passes `--hostname` for a GitHub Enterprise host.
    pub fn new(program: impl Into<PathBuf>, hostname: Option<String>) -> Self {
        Self {
            inner: Arc::new(GhCliInner {
                program: program.into(),
                hostname,
            }),
        }
    }

    /// Runs one request with `gh api -i`.
    pub async fn request(&self, req: GhRequest) -> Result<GhResponse, GhError> {
        let args = req.argv(self.hostname(), false);
        let output = self.run(&args, req.body.as_ref()).await?;
        self.interpret(&output, false)
    }

    /// Runs a `GET` with `gh api --paginate` and returns the items of every page.
    ///
    /// Only for endpoints whose every page is wanted at once; lazily loaded tables page explicitly with `request`.
    pub async fn paginate<T: DeserializeOwned>(&self, req: GhRequest) -> Result<Vec<T>, GhError> {
        let args = req.argv(self.hostname(), true);
        let output = self.run(&args, req.body.as_ref()).await?;
        let response = self.interpret(&output, true)?;
        decode_pages(&response.body)
    }

    /// Checks `gh auth status [--hostname h]`: `Ok` when logged in, otherwise [`GhError::NotAuthenticated`].
    pub async fn auth_status(&self) -> Result<(), GhError> {
        let output = self.run(&self.auth_status_argv(), None).await?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Err(GhError::NotAuthenticated {
            hostname: self.inner.hostname.clone(),
            detail,
        })
    }

    /// The GitHub Enterprise host passed as `--hostname`, or `None` for gh's default host.
    pub(crate) fn hostname(&self) -> Option<&str> {
        self.inner.hostname.as_deref()
    }

    /// The arguments [`Self::auth_status`] passes to `gh` (not including the program name).
    pub(crate) fn auth_status_argv(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["auth".into(), "status".into()];
        if let Some(hostname) = self.hostname() {
            args.push("--hostname".into());
            args.push(hostname.into());
        }
        args
    }

    fn interpret(&self, output: &Output, paginated: bool) -> Result<GhResponse, GhError> {
        interpret(output.status.code(), &output.stdout, &output.stderr, paginated).map_err(|error| match error {
            GhError::NotAuthenticated { detail, .. } => GhError::NotAuthenticated {
                hostname: self.inner.hostname.clone(),
                detail,
            },
            other => other,
        })
    }

    /// The `gh` command for `args`, with every stream piped (stdin only when `has_input`) and an environment that keeps
    /// its output machine-readable.
    fn command(&self, args: &[OsString], has_input: bool) -> Command {
        let mut command = Command::new(&self.inner.program);
        command
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            // Inherited, these make gh colorize `-i` headers and pretty-print JSON even into a pipe, which breaks
            // header parsing and decoding.
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GH_FORCE_TTY")
            .env("NO_COLOR", "1")
            .env("GH_PAGER", "")
            .stdin(if has_input { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    /// Spawns `gh` with every stream piped (stdin only when there is a body) and waits for it.
    ///
    /// Dropping the returned future kills the child.
    async fn run(&self, args: &[OsString], body: Option<&serde_json::Value>) -> Result<Output, GhError> {
        debug!(
            "Running Github CLI command: gh {}",
            args.iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let input = body.map(serde_json::to_vec).transpose()?;

        let mut child = self
            .command(args, input.is_some())
            .spawn()
            .map_err(|error| match error.kind() {
                io::ErrorKind::NotFound => GhError::NotInstalled,
                _ => GhError::Io(error),
            })?;

        // Write the body while the output is being read, so a large response can't block on a full pipe.
        let stdin = child.stdin.take();
        let write = async move {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                stdin.write_all(&input).await?;
                stdin.shutdown().await?;
                // Dropping `stdin` here closes it, so gh sees end of input.
            }
            Ok::<(), io::Error>(())
        };
        let (written, output) = tokio::join!(write, child.wait_with_output());
        let output = output?;
        match written {
            // gh exited without reading all its input (a bad flag, say); its exit status explains why.
            Err(error) if error.kind() != io::ErrorKind::BrokenPipe => return Err(error.into()),
            _ => {}
        }

        if !output.stderr.is_empty() {
            debug!(
                "Error output from Github CLI: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn command_environment_keeps_output_plain() {
        let gh = GhCli::default();
        let command = gh.command(&["api".into(), "/user".into()], false);
        let envs: Vec<(&OsStr, Option<&OsStr>)> = command.as_std().get_envs().collect();
        let env = |name: &str| envs.iter().find(|(key, _)| *key == name).map(|(_, value)| *value);
        assert_eq!(env("GH_PROMPT_DISABLED"), Some(Some(OsStr::new("1"))));
        assert_eq!(env("GH_NO_UPDATE_NOTIFIER"), Some(Some(OsStr::new("1"))));
        assert_eq!(env("NO_COLOR"), Some(Some(OsStr::new("1"))));
        assert_eq!(env("GH_PAGER"), Some(Some(OsStr::new(""))));
        // `None` means removed from the inherited environment.
        assert_eq!(env("CLICOLOR_FORCE"), Some(None));
        assert_eq!(env("GH_FORCE_TTY"), Some(None));
        assert_eq!(command.as_std().get_program(), "gh");
        assert_eq!(command.as_std().get_args().collect::<Vec<_>>(), ["api", "/user"]);
    }

    #[tokio::test]
    async fn missing_program_is_not_installed() {
        let gh = GhCli::new("/nonexistent/gh", None);
        assert!(matches!(
            gh.request(GhRequest::get("/user")).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(
            gh.paginate::<serde_json::Value>(GhRequest::get("/user/repos")).await,
            Err(GhError::NotInstalled)
        ));
        assert!(matches!(gh.auth_status().await, Err(GhError::NotInstalled)));
    }
}
