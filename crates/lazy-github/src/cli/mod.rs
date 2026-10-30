//! The subcommands other than `run`. Each writes to the `stdout` and `stderr` it's given and returns the exit code,
//! so the tests can run them without a process. None of them starts logging or creates the config folder.

mod check;
mod clear;
mod json;

use std::fmt::Display;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use lazy_github_core::config::{Config, ConfigError, ConfigPaths};
use lazy_github_core::debug::{DebugEnv, debug_markdown};
use serde_json::Value;

pub use check::check_config;
pub use clear::{aborted, clear_cache, clear_config};
pub use json::JsonStyle;

/// The token file the Python app's own login flow stored, in the config folder.
const PYTHON_AUTH_FILE: &str = "auth.text";

const GH_LOGOUT_POINTER: &str =
    "LazyGithub signs in through the GitHub CLI and stores no credentials of its own. To log out, run: gh auth logout";

/// `lazy-github version`.
pub fn version(stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    let result = writeln!(stdout, "LazyGithub version {}", env!("CARGO_PKG_VERSION")).map(|()| ExitCode::SUCCESS);
    finish(result, stderr)
}

/// `lazy-github dump-config`: where the config is, then the config as the app would save it, as Rich's `print_json`
/// prints it.
pub fn dump_config(
    paths: Result<ConfigPaths, ConfigError>,
    style: JsonStyle,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let paths = match paths {
        Ok(paths) => paths,
        Err(e) => return error(stderr, e),
    };
    let result = (|| {
        let exists = if paths.file.exists() { "True" } else { "False" };
        writeln!(
            stdout,
            "Config file location: {} (exists => {exists})",
            paths.file.display()
        )?;
        // The location line comes first, as in Python, even when the config then fails to load.
        stdout.flush()?;
        let config = match Config::load(&paths) {
            Ok(config) => config,
            Err(e) => return Ok(error(stderr, e)),
        };
        // Parsed back from the saved text so the order is the file's (`preserve_order`), unknown keys included.
        let value: Value = match serde_json::from_str(&config.to_json_pretty()) {
            Ok(value) => value,
            Err(e) => return Ok(error(stderr, e)),
        };
        stdout.write_all(json::to_rich_json(&value, style).as_bytes())?;
        Ok(ExitCode::SUCCESS)
    })();
    finish(result, stderr)
}

/// `lazy-github clear-auth`: deletes a token the Python app left behind, then says how to log out of `gh`, which
/// holds the only login this app uses.
pub fn clear_auth(
    paths: Result<ConfigPaths, ConfigError>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let paths = match paths {
        Ok(paths) => paths,
        Err(e) => return error(stderr, e),
    };
    let result = (|| {
        let token = paths.dir.join(PYTHON_AUTH_FILE);
        match remove_if_present(&token) {
            Ok(false) => {}
            Ok(true) => writeln!(
                stdout,
                "Removed the token the Python app stored in {}.",
                token.display()
            )?,
            Err(e) => {
                return Ok(error(stderr, format_args!("failed to delete {}: {e}", token.display())));
            }
        }
        writeln!(stdout, "{GH_LOGOUT_POINTER}")?;
        Ok(ExitCode::SUCCESS)
    })();
    finish(result, stderr)
}

/// `lazy-github debug`: the debug info (`lazy_github_core::debug`) and a newline. `env` reads an environment
/// variable; `gh_version` asks gh for its version, once the config has loaded.
pub fn debug(
    paths: Result<ConfigPaths, ConfigError>,
    env: impl Fn(&str) -> Option<String>,
    gh_version: impl FnOnce() -> Option<String>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let config = match paths.and_then(|paths| Config::load(&paths)) {
        Ok(config) => config,
        Err(e) => return error(stderr, e),
    };
    let env = DebugEnv::detect(env, gh_version());
    let result = writeln!(stdout, "{}", debug_markdown(&env, &config)).map(|()| ExitCode::SUCCESS);
    finish(result, stderr)
}

/// Deletes `path`; `Ok(false)` if there was nothing there.
fn remove_if_present(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Prints `Error: <e>` on stderr and returns failure.
fn error(stderr: &mut impl Write, e: impl Display) -> ExitCode {
    // Nothing is left to report a failed write to.
    let _ = writeln!(stderr, "Error: {e}");
    ExitCode::FAILURE
}

/// The exit code, or an error line if writing to stdout failed.
fn finish(result: io::Result<ExitCode>, stderr: &mut impl Write) -> ExitCode {
    result.unwrap_or_else(|e| error(stderr, format_args!("failed to write to stdout: {e}")))
}

#[cfg(test)]
mod tests {
    use super::{JsonStyle, clear_auth, debug, dump_config, version};
    use lazy_github_core::config::{ConfigError, ConfigPaths};
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("lazy-github-cli-unit-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn paths(&self) -> ConfigPaths {
            ConfigPaths::in_dir(self.0.join("lazy-github"))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn version_prints_the_crate_version() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(version(&mut out, &mut err), ExitCode::SUCCESS);
        assert_eq!(
            text(out),
            concat!("LazyGithub version ", env!("CARGO_PKG_VERSION"), "\n")
        );
        assert_eq!(text(err), "");
    }

    #[test]
    fn dump_config_without_a_file_prints_the_defaults_and_creates_nothing() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            dump_config(Ok(paths.clone()), JsonStyle::Plain, &mut out, &mut err),
            ExitCode::SUCCESS
        );
        let out = text(out);
        let location = format!("Config file location: {} (exists => False)\n{{\n", paths.file.display());
        assert!(out.starts_with(&location), "{out}");
        assert!(out.ends_with("\n}\n"), "{out}");
        assert!(!paths.dir.exists());
    }

    #[test]
    fn a_missing_home_is_one_error_line() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            dump_config(Err(ConfigError::NoHomeDir), JsonStyle::Plain, &mut out, &mut err),
            ExitCode::FAILURE
        );
        assert_eq!(text(err), "Error: could not determine your home directory\n");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            clear_auth(Err(ConfigError::NoHomeDir), &mut out, &mut err),
            ExitCode::FAILURE
        );
        assert_eq!(text(out), "");
    }

    #[test]
    fn clear_auth_removes_the_python_token() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        std::fs::create_dir_all(&paths.dir).unwrap();
        let token = paths.dir.join("auth.text");
        std::fs::write(&token, "ghp_secret").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(clear_auth(Ok(paths), &mut out, &mut err), ExitCode::SUCCESS);
        assert!(!token.exists());
        let out = text(out);
        assert!(
            out.starts_with(&format!(
                "Removed the token the Python app stored in {}.\n",
                token.display()
            )),
            "{out}"
        );
        assert!(out.ends_with("To log out, run: gh auth logout\n"), "{out}");
        assert_eq!(text(err), "");
    }

    #[test]
    fn debug_prints_the_markdown_and_a_newline() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        let env = |name: &str| (name == "TERM").then(|| "xterm-256color".to_owned());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let gh = || Some("2.89.0 (fake-gh)".to_owned());
        assert_eq!(debug(Ok(paths.clone()), env, gh, &mut out, &mut err), ExitCode::SUCCESS);
        let out = text(out);
        assert!(
            out.starts_with(concat!(
                "# Library Versions\n\n- LazyGithub Version: ",
                env!("CARGO_PKG_VERSION"),
                "\n- Rustc Version: "
            )),
            "{out}"
        );
        assert!(out.contains("\n- Gh Version: 2.89.0 (fake-gh)\n- OS: "), "{out}");
        assert!(
            out.contains("\n- Terminal: xterm-256color\n\n# LazyGithub Config\n\n```json\n{\n"),
            "{out}"
        );
        assert!(out.ends_with("\n}\n```\n"), "{out}");
        assert_eq!(text(err), "");
        assert!(!paths.dir.exists());
    }

    #[test]
    fn debug_with_a_broken_config_asks_gh_nothing() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        std::fs::create_dir_all(&paths.dir).unwrap();
        std::fs::write(&paths.file, "{").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let gh = || -> Option<String> { panic!("gh was asked") };
        assert_eq!(
            debug(Ok(paths), |_: &str| None, gh, &mut out, &mut err),
            ExitCode::FAILURE
        );
        assert_eq!(text(out), "");
        let err = text(err);
        assert!(err.starts_with("Error: "), "{err}");
        assert_eq!(err.lines().count(), 1, "{err}");
    }
}
