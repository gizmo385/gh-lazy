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
        let mut args: Vec<OsString> = vec!["auth".into(), "status".into()];
        if let Some(hostname) = self.hostname() {
            args.push("--hostname".into());
            args.push(hostname.into());
        }
        let output = self.run(&args, None).await?;
        if output.status.success() {
            return Ok(());
        }
        let mut detail = String::from_utf8_lossy(&output.stderr).into_owned();
        detail.push_str(&String::from_utf8_lossy(&output.stdout));
        Err(GhError::NotAuthenticated {
            hostname: self.inner.hostname.clone(),
            detail: detail.trim().to_owned(),
        })
    }

    fn hostname(&self) -> Option<&str> {
        self.inner.hostname.as_deref()
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

        let mut child = Command::new(&self.inner.program)
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
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
