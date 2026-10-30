//! `clear-config` and `clear-cache`, and the `[y/N]` prompt they share, which behaves like Click's `confirm`.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

use lazy_github_core::config::{Config, ConfigError, ConfigPaths};

use super::{error, finish, remove_if_present};

/// Python's wording, missing "of", kept as the golden pins it.
const CLEAR_CONFIG_PROMPT: &str = "Confirm deletion your LazyGithub settings";

/// How the user answered a [`confirm`] prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    /// Input ended before an answer. The caller prints `Aborted!` on stderr and exits 1.
    Aborted,
}

/// Asks `<text> [y/N]: ` on `stdout` until `stdin` gives an answer, as Click's `confirm` does: the answer is trimmed
/// and lowercased, `y`/`yes` and `n`/`no` count, an empty one is no, and anything else prints `Error: invalid input`
/// and asks again. Text followed by end of input without a newline still counts as an answer; end of input before any
/// text aborts. A failed read aborts too, where Python would crash.
pub fn confirm(text: &str, stdin: &mut impl BufRead, stdout: &mut impl Write) -> io::Result<Answer> {
    loop {
        write!(stdout, "{text} [y/N]: ")?;
        stdout.flush()?;
        let mut line = Vec::new();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return Ok(Answer::Aborted),
            Ok(_) => {}
        }
        match String::from_utf8_lossy(&line).trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(Answer::Yes),
            "n" | "no" | "" => return Ok(Answer::No),
            _ => writeln!(stdout, "Error: invalid input")?,
        }
    }
}

/// The answer to the prompt, or yes without asking when `stdin` is `None` (`--no-confirm`).
fn ask(text: &str, stdin: Option<&mut impl BufRead>, stdout: &mut impl Write) -> io::Result<Answer> {
    match stdin {
        Some(stdin) => confirm(text, stdin, stdout),
        None => Ok(Answer::Yes),
    }
}

/// Prints Click's abort line and returns failure.
pub fn aborted(stderr: &mut impl Write) -> ExitCode {
    // Nothing is left to report a failed write to.
    let _ = writeln!(stderr, "Aborted!");
    ExitCode::FAILURE
}

/// `lazy-github clear-config`: deletes `config.json` once the user confirms. It never loads the config, so it works
/// on a broken one, which is what the broken-config error suggests running.
pub fn clear_config(
    paths: Result<ConfigPaths, ConfigError>,
    stdin: Option<&mut impl BufRead>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let paths = match paths {
        Ok(paths) => paths,
        Err(e) => return error(stderr, e),
    };
    let result = (|| {
        match ask(CLEAR_CONFIG_PROMPT, stdin, stdout)? {
            Answer::Aborted => return Ok(aborted(stderr)),
            Answer::No => writeln!(stdout, "Canceling")?,
            Answer::Yes => {
                if let Err(e) = remove_if_present(&paths.file) {
                    return Ok(failed_to_delete(stderr, &paths.file, &e));
                }
                writeln!(stdout, "Your settings have been cleared")?;
            }
        }
        Ok(ExitCode::SUCCESS)
    })();
    finish(result, stderr)
}

/// `lazy-github clear-cache`: removes the configured cache directory, everything in it included, once the user
/// confirms.
pub fn clear_cache(
    paths: Result<ConfigPaths, ConfigError>,
    stdin: Option<&mut impl BufRead>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    // A broken config stops here, before the prompt.
    let config = match paths.and_then(|paths| Config::load(&paths)) {
        Ok(config) => config,
        Err(e) => return error(stderr, e),
    };
    let dir = &config.cache.cache_directory;
    let result = (|| {
        let prompt = format!("Confirm deletion of everything in {}", dir.display());
        match ask(&prompt, stdin, stdout)? {
            Answer::Aborted => return Ok(aborted(stderr)),
            Answer::No => writeln!(stdout, "Canceling cache deletion")?,
            Answer::Yes => {
                if let Err(e) = remove_dir_if_present(dir) {
                    return Ok(failed_to_delete(stderr, dir, &e));
                }
                writeln!(stdout, "Cache cleared")?;
            }
        }
        Ok(ExitCode::SUCCESS)
    })();
    finish(result, stderr)
}

/// Removes the directory `path` and everything in it, if it exists. Like Python's `shutil.rmtree`, it refuses a
/// symlink (rather than removing the link) and anything that isn't a directory.
fn remove_dir_if_present(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if metadata.is_symlink() {
        return Err(io::Error::other("it is a symbolic link"));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a directory"));
    }
    std::fs::remove_dir_all(path)
}

/// Py bug B97's recommendation: one `Error:` line on stderr and exit 1, where Python crashes with a traceback.
fn failed_to_delete(stderr: &mut impl Write, path: &Path, e: &io::Error) -> ExitCode {
    error(stderr, format_args!("failed to delete {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{Answer, clear_cache, clear_config, confirm};
    use lazy_github_core::config::ConfigPaths;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PROMPT: &str = "Sure? [y/N]: ";

    /// The answer and everything printed for `input`.
    fn ask(input: &str) -> (Answer, String) {
        let mut out = Vec::new();
        let answer = confirm("Sure?", &mut Cursor::new(input), &mut out).unwrap();
        (answer, String::from_utf8(out).unwrap())
    }

    #[test]
    fn confirm_accepts_each_spelling_in_any_case_trimmed() {
        for input in ["y\n", "Y\n", "yes\n", "YES\n", " Yes \n", "y\r\n"] {
            assert_eq!(ask(input), (Answer::Yes, PROMPT.to_owned()), "{input:?}");
        }
        for input in ["n\n", "N\n", "no\n", " No \n", "nO\r\n"] {
            assert_eq!(ask(input), (Answer::No, PROMPT.to_owned()), "{input:?}");
        }
    }

    #[test]
    fn an_empty_answer_is_no() {
        assert_eq!(ask("\n"), (Answer::No, PROMPT.to_owned()));
        assert_eq!(ask("   \n"), (Answer::No, PROMPT.to_owned()));
    }

    #[test]
    fn an_invalid_answer_asks_again() {
        assert_eq!(
            ask("maybe\nyeah\ny\n"),
            (
                Answer::Yes,
                format!("{PROMPT}Error: invalid input\n{PROMPT}Error: invalid input\n{PROMPT}")
            )
        );
    }

    #[test]
    fn end_of_input_after_an_invalid_answer_aborts() {
        assert_eq!(
            ask("maybe\n"),
            (Answer::Aborted, format!("{PROMPT}Error: invalid input\n{PROMPT}"))
        );
    }

    #[test]
    fn end_of_input_at_once_aborts() {
        assert_eq!(ask(""), (Answer::Aborted, PROMPT.to_owned()));
    }

    #[test]
    fn text_then_end_of_input_is_an_answer() {
        assert_eq!(ask("y"), (Answer::Yes, PROMPT.to_owned()));
        assert_eq!(ask("no"), (Answer::No, PROMPT.to_owned()));
        assert_eq!(
            ask("x"),
            (Answer::Aborted, format!("{PROMPT}Error: invalid input\n{PROMPT}"))
        );
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("lazy-github-clear-unit-{}-{n}", std::process::id()));
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

    #[test]
    fn no_confirm_skips_the_prompt() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        std::fs::create_dir_all(paths.dir.join(".cache")).unwrap();
        std::fs::write(&paths.file, "{}").unwrap();

        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = clear_cache(Ok(paths.clone()), None::<&mut Cursor<&str>>, &mut out, &mut err);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(String::from_utf8(out).unwrap(), "Cache cleared\n");
        assert!(!paths.dir.join(".cache").exists());

        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = clear_config(Ok(paths.clone()), None::<&mut Cursor<&str>>, &mut out, &mut err);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(String::from_utf8(out).unwrap(), "Your settings have been cleared\n");
        assert!(!paths.file.exists());
        assert!(paths.dir.exists());
        assert_eq!(String::from_utf8(err).unwrap(), "");
    }

    #[test]
    fn an_abort_prints_aborted_on_stderr_and_deletes_nothing() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        std::fs::create_dir_all(&paths.dir).unwrap();
        std::fs::write(&paths.file, "{}").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = clear_config(Ok(paths.clone()), Some(&mut Cursor::new("")), &mut out, &mut err);
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Confirm deletion your LazyGithub settings [y/N]: "
        );
        assert_eq!(String::from_utf8(err).unwrap(), "Aborted!\n");
        assert!(paths.file.exists());
    }
}
