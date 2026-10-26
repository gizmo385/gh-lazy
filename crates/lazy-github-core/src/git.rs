//! Running `git` in the user's working directory, to find the repo LazyGithub was launched in.
//!
//! The directory is passed in (from `LAZY_GITHUB_ORIGINAL_PWD` or the process's current directory, resolved once by
//! the binary) rather than read from the environment here.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tracing::debug;

use crate::models::RepoName;

/// How long one `git` call may run before it's killed.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a `git` call failed. Each variant's `Display` is one sentence, for the log.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// `git` could not be spawned because it isn't installed or isn't on `PATH`.
    #[error("git is not installed or not on your PATH")]
    NotInstalled,
    /// The directory git should run in doesn't exist.
    #[error("the directory {} doesn't exist", .0.display())]
    NoDirectory(PathBuf),
    /// `git` exited non-zero: not a repo, no such remote, and so on.
    #[error(
        "git failed{}{}",
        code.map(|c| format!(" with exit code {c}")).unwrap_or_default(),
        detail_suffix(stderr)
    )]
    Failed {
        /// The exit code, or `None` if `git` was killed by a signal.
        code: Option<i32>,
        /// `git`'s error output, trimmed.
        stderr: String,
    },
    /// `git` ran past its time limit and was killed.
    #[error("git did not respond within {} seconds", after.as_secs())]
    Timeout {
        /// The limit it ran past.
        after: Duration,
    },
    /// Spawning or talking to the `git` process failed.
    #[error("failed to run git: {0}")]
    Io(#[from] io::Error),
}

/// `": "` and `detail` folded onto one line, or nothing if `detail` is blank.
fn detail_suffix(detail: &str) -> String {
    let lines: Vec<&str> = detail.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!(": {}", lines.join("; "))
    }
}

/// Runs `git` in the user's working directory. Cheap to clone (an `Arc` inside), like
/// [`GhCli`](crate::github::GhCli).
#[derive(Debug, Clone)]
pub struct GitCli {
    inner: Arc<GitCliInner>,
}

#[derive(Debug, Clone)]
struct GitCliInner {
    program: PathBuf,
    /// Where `git` runs; `None` runs it in the process's current directory.
    dir: Option<PathBuf>,
    timeout: Duration,
    /// Extra environment for the child. Empty outside tests.
    envs: Vec<(OsString, OsString)>,
}

impl GitCli {
    /// Runs `git` from `PATH` in `dir`, or in the process's current directory when `dir` is `None`.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self::build(PathBuf::from("git"), dir)
    }

    /// Like [`Self::new`], running `program` instead of `git`. For tests.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_program(program: impl Into<PathBuf>, dir: Option<PathBuf>) -> Self {
        Self::build(program.into(), dir)
    }

    fn build(program: PathBuf, dir: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(GitCliInner {
                program,
                dir,
                timeout: GIT_TIMEOUT,
                envs: Vec::new(),
            }),
        }
    }

    /// This client with `key=value` added to git's environment, such as an isolated `HOME`. For tests.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        Arc::make_mut(&mut self.inner).envs.push((key.into(), value.into()));
        self
    }

    /// `git remote get-url <remote>`, trimmed.
    pub async fn remote_url(&self, remote: &str) -> Result<String, GitError> {
        let stdout = self.run(&["remote", "get-url", remote]).await?;
        Ok(stdout.trim().to_owned())
    }

    /// `origin`'s repo, or `None` when there isn't one: not a git repo, no `origin`, a remote that isn't a GitHub
    /// `owner/name`, or git couldn't run (Py bug B41). Either way the answer is logged at debug.
    pub async fn origin_repo(&self) -> Option<RepoName> {
        match self.remote_url("origin").await {
            Ok(url) => {
                let repo = RepoName::from_remote_url(&url);
                if let Some(repo) = &repo {
                    debug!("Current-directory repo: {repo} (origin is {url})");
                } else {
                    debug!("No current-directory repo: origin is {url}, which isn't a GitHub owner/name");
                }
                repo
            }
            Err(error) => {
                debug!("No current-directory repo: {error}");
                None
            }
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.inner.program);
        command
            .args(args)
            // Never wait on a credential or host-key prompt nobody can see.
            .env("GIT_TERMINAL_PROMPT", "0")
            .envs(self.inner.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // `current_dir` has the effect of Python's `git -C <dir>` without changing the arguments.
        if let Some(dir) = &self.inner.dir {
            command.current_dir(dir);
        }
        command
    }

    /// Spawns `git args` and returns its stdout, for at most the client's time limit.
    ///
    /// Past the limit, `git` is killed and reaped and the result is [`GitError::Timeout`]. Dropping the returned
    /// future kills the child too, through `kill_on_drop`.
    async fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let mut child = match self.command(args).spawn() {
            Ok(child) => child,
            Err(error) => return Err(self.spawn_error(error).await),
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let limit = self.inner.timeout;
        // Borrows `child` only until it finishes or the timeout drops it, so the timeout path can still kill it.
        let finish = async {
            let (stdout, stderr, status) = tokio::join!(read_all(stdout), read_all(stderr), child.wait());
            Ok::<_, io::Error>((status?, stdout?, stderr?))
        };
        let (status, stdout, stderr) = match tokio::time::timeout(limit, finish).await {
            Ok(finished) => finished?,
            Err(_elapsed) => {
                // `kill` waits after killing, so the process is reaped before the caller hears about it.
                if let Err(error) = child.kill().await {
                    debug!("Couldn't kill the timed-out git: {error}");
                }
                return Err(GitError::Timeout { after: limit });
            }
        };
        if !status.success() {
            return Err(GitError::Failed {
                code: status.code(),
                stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }

    /// What a failed spawn means. A missing working directory fails the spawn like a missing program does, so it's
    /// told apart here for the log.
    async fn spawn_error(&self, error: io::Error) -> GitError {
        if let Some(dir) = &self.inner.dir
            && !tokio::fs::metadata(dir).await.is_ok_and(|meta| meta.is_dir())
        {
            return GitError::NoDirectory(dir.clone());
        }
        match error.kind() {
            io::ErrorKind::NotFound => GitError::NotInstalled,
            _ => GitError::Io(error),
        }
    }
}

/// Everything `stream` yields until end of file, or nothing when the stream wasn't piped.
async fn read_all(stream: Option<impl AsyncRead + Unpin>) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if let Some(mut stream) = stream {
        stream.read_to_end(&mut buf).await?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A temp dir holding an isolated `home/` and a `work/` dir, with git configured to ignore the developer's
    /// global and system config.
    struct Sandbox {
        dir: tempfile::TempDir,
    }

    impl Sandbox {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("home")).unwrap();
            std::fs::create_dir_all(dir.path().join("work")).unwrap();
            Self { dir }
        }

        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }

        fn work(&self) -> PathBuf {
            self.dir.path().join("work")
        }

        fn git_in(&self, dir: &Path, args: &[&str]) {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("HOME", self.home())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "git {args:?}: {output:?}");
        }

        /// `work/` as a git repo, with `origin` set to `remote` when there is one.
        fn repo(&self, remote: Option<&str>) {
            self.git_in(&self.work(), &["init", "-q"]);
            if let Some(url) = remote {
                self.git_in(&self.work(), &["remote", "add", "origin", url]);
            }
        }

        /// A client for `dir`, isolated like the setup commands, which never looks for a repo above the sandbox.
        fn client(&self, dir: PathBuf) -> GitCli {
            GitCli::new(Some(dir))
                .with_env("HOME", self.home())
                .with_env("GIT_CONFIG_NOSYSTEM", "1")
                .with_env("GIT_CEILING_DIRECTORIES", self.dir.path())
        }
    }

    fn name(repo: Option<RepoName>) -> Option<String> {
        repo.map(|repo| repo.to_string())
    }

    #[tokio::test]
    async fn an_https_origin_names_the_repo() {
        let sandbox = Sandbox::new();
        sandbox.repo(Some("https://github.com/o/n.git"));
        let git = sandbox.client(sandbox.work());
        assert_eq!(git.remote_url("origin").await.unwrap(), "https://github.com/o/n.git");
        assert_eq!(name(git.origin_repo().await).as_deref(), Some("o/n"));
    }

    #[tokio::test]
    async fn an_ssh_origin_names_the_repo() {
        let sandbox = Sandbox::new();
        sandbox.repo(Some("git@github.com:o/n.git"));
        let git = sandbox.client(sandbox.work());
        assert_eq!(name(git.origin_repo().await).as_deref(), Some("o/n"));
    }

    #[tokio::test]
    async fn a_subdirectory_of_the_repo_finds_it_too() {
        let sandbox = Sandbox::new();
        sandbox.repo(Some("git@github.com:o/n.git"));
        let sub = sandbox.work().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(name(sandbox.client(sub).origin_repo().await).as_deref(), Some("o/n"));
    }

    #[tokio::test]
    async fn no_origin_is_no_repo() {
        let sandbox = Sandbox::new();
        sandbox.repo(None);
        let git = sandbox.client(sandbox.work());
        assert!(matches!(
            git.remote_url("origin").await,
            Err(GitError::Failed { code: Some(c), .. }) if c != 0
        ));
        assert_eq!(git.origin_repo().await, None);
    }

    #[tokio::test]
    async fn a_directory_outside_any_repo_is_no_repo() {
        let sandbox = Sandbox::new();
        let git = sandbox.client(sandbox.work());
        assert!(matches!(git.remote_url("origin").await, Err(GitError::Failed { .. })));
        assert_eq!(git.origin_repo().await, None);
    }

    #[tokio::test]
    async fn a_missing_directory_is_no_repo() {
        let sandbox = Sandbox::new();
        let missing = sandbox.work().join("gone");
        let git = sandbox.client(missing.clone());
        assert!(matches!(git.remote_url("origin").await, Err(GitError::NoDirectory(dir)) if dir == missing));
        assert_eq!(git.origin_repo().await, None);
    }

    #[tokio::test]
    async fn a_missing_program_is_not_installed() {
        let sandbox = Sandbox::new();
        let git = GitCli::with_program("lazy-github-no-such-git", Some(sandbox.work()));
        assert!(matches!(git.remote_url("origin").await, Err(GitError::NotInstalled)));
        assert_eq!(git.origin_repo().await, None);
    }

    #[test]
    fn errors_display_one_line() {
        let cases = [
            (GitError::NotInstalled, "git is not installed or not on your PATH"),
            (
                GitError::NoDirectory(PathBuf::from("gone")),
                "the directory gone doesn't exist",
            ),
            (
                GitError::Failed {
                    code: Some(2),
                    stderr: "error: No such remote 'origin'\n".into(),
                },
                "git failed with exit code 2: error: No such remote 'origin'",
            ),
            (
                GitError::Failed {
                    code: None,
                    stderr: String::new(),
                },
                "git failed",
            ),
            (
                GitError::Timeout {
                    after: Duration::from_secs(5),
                },
                "git did not respond within 5 seconds",
            ),
            (
                GitError::Io(io::Error::other("broken pipe")),
                "failed to run git: broken pipe",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn the_child_never_prompts() {
        let git = GitCli::new(Some(PathBuf::from("somewhere")));
        let command = git.command(&["remote", "get-url", "origin"]);
        let command = command.as_std();
        assert_eq!(command.get_program(), "git");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["remote", "get-url", "origin"]);
        assert_eq!(command.get_current_dir(), Some(Path::new("somewhere")));
        let prompt = command.get_envs().find(|(key, _)| *key == "GIT_TERMINAL_PROMPT");
        assert_eq!(
            prompt,
            Some((
                std::ffi::OsStr::new("GIT_TERMINAL_PROMPT"),
                Some(std::ffi::OsStr::new("0"))
            ))
        );
        assert_eq!(GitCli::new(None).command(&[]).as_std().get_current_dir(), None);
    }
}
