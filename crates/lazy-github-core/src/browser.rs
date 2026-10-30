//! Opening a URL in the user's browser, honoring `$BROWSER` as Python's `webbrowser` does, then the platform's
//! opener (PLAN §10, decision 9). The parity harness points `$BROWSER` at a recorder, so a run can say what an app
//! opened.
//!
//! This is the one place the app runs a program other than `gh` and `git`. Commands are argument vectors, never a
//! shell line.

use std::ffi::{OsStr, OsString};
use std::process::Stdio;

/// One way to open a URL: a program and its arguments, the URL already in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCommand {
    /// The program, then its arguments.
    pub argv: Vec<OsString>,
    /// Started and not waited for: a `$BROWSER` entry ending in a `&` argument (Python's `BackgroundBrowser`). It
    /// counts as opened once it starts.
    pub background: bool,
}

/// Why nothing opened a URL.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// Every command failed to start or exited non-zero.
    #[error("no browser opened it (tried {tried})")]
    NothingOpened {
        /// The programs tried, comma-separated.
        tried: String,
    },
}

/// The separator between `$BROWSER` entries: Python's `os.pathsep`.
const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// The commands to try for `url`, in order: `$BROWSER`'s entries (`browser_env`) as Python's `webbrowser` reads
/// them, then the platform opener.
///
/// `$BROWSER` is split on [`SEPARATOR`] (`:`, or `;` on Windows) and empty entries are skipped. An entry containing
/// `%s` is split like a shell line (`shlex`), with `%s` replaced by the URL in each argument; a last argument `&`
/// marks it [background](BrowserCommand::background). Any other entry is the whole program name or path, spaces
/// and all, with the URL as its only argument. An entry that doesn't split (an unclosed quote) is skipped. A
/// `$BROWSER` that isn't Unicode is read lossily.
pub fn commands(browser_env: Option<&OsStr>, url: &str) -> Vec<BrowserCommand> {
    let env = browser_env.map(OsStr::to_string_lossy);
    commands_split(env.as_deref(), SEPARATOR, url)
}

fn commands_split(browser_env: Option<&str>, separator: char, url: &str) -> Vec<BrowserCommand> {
    let entries = browser_env
        .into_iter()
        .flat_map(|env| env.split(separator))
        .filter(|entry| !entry.is_empty());
    let mut out: Vec<BrowserCommand> = entries.filter_map(|entry| entry_command(entry, url)).collect();
    out.push(platform_opener(url));
    out
}

/// One `$BROWSER` entry's command for `url`.
fn entry_command(entry: &str, url: &str) -> Option<BrowserCommand> {
    if !entry.contains("%s") {
        return Some(BrowserCommand {
            argv: vec![entry.into(), url.into()],
            background: false,
        });
    }
    let mut args = shlex::split(entry)?;
    let background = args.last().is_some_and(|last| last == "&");
    if background {
        args.pop();
    }
    if args.is_empty() {
        return None;
    }
    Some(BrowserCommand {
        argv: args.iter().map(|arg| arg.replace("%s", url).into()).collect(),
        background,
    })
}

/// The platform's own way to open a URL: `open` on macOS, `explorer` on Windows, `xdg-open` elsewhere.
fn platform_opener(url: &str) -> BrowserCommand {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    BrowserCommand {
        argv: vec![program.into(), url.into()],
        background: false,
    }
}

/// Opens `url` with the first of [`commands`] (read from the `BROWSER` environment variable) that works: one that
/// exits 0, or a background one that starts. Standard input, output and error are null, so nothing writes over the
/// terminal UI.
///
/// # Errors
///
/// [`OpenError::NothingOpened`] when every command failed to start or exited non-zero.
pub async fn open(url: &str) -> Result<(), OpenError> {
    let browser = std::env::var_os("BROWSER");
    try_commands(&commands(browser.as_deref(), url)).await
}

/// Runs `commands` in order until one works.
async fn try_commands(commands: &[BrowserCommand]) -> Result<(), OpenError> {
    for command in commands {
        let Some((program, args)) = command.argv.split_first() else {
            continue;
        };
        let mut child = tokio::process::Command::new(program);
        child
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let opened = if command.background {
            // Dropping the handle leaves it running; tokio reaps it once it exits.
            child.spawn().is_ok()
        } else {
            child.status().await.is_ok_and(|status| status.success())
        };
        if opened {
            return Ok(());
        }
        tracing::debug!("{} didn't open the URL", program.to_string_lossy());
    }
    let tried = commands
        .iter()
        .filter_map(|command| command.argv.first())
        .map(|program| program.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(", ");
    Err(OpenError::NothingOpened { tried })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const URL: &str = "https://github.com/parity-bot";

    fn command(argv: &[&str], background: bool) -> BrowserCommand {
        BrowserCommand {
            argv: argv.iter().map(OsString::from).collect(),
            background,
        }
    }

    fn opener() -> BrowserCommand {
        platform_opener(URL)
    }

    #[rstest]
    #[case::unset(None, vec![])]
    #[case::empty(Some(""), vec![])]
    #[case::program(Some("firefox"), vec![command(&["firefox", URL], false)])]
    #[case::two_entries(
        Some("a --x %s:b"),
        vec![command(&["a", "--x", URL], false), command(&["b", URL], false)]
    )]
    #[case::path_with_a_space(Some("/opt/my browser"), vec![command(&["/opt/my browser", URL], false)])]
    #[case::quoted_program(
        Some("\"my browser\" --x=%s"),
        vec![command(&["my browser", &format!("--x={URL}")], false)]
    )]
    #[case::empty_entries_skipped(Some("::firefox:"), vec![command(&["firefox", URL], false)])]
    #[case::background(Some("firefox %s &"), vec![command(&["firefox", URL], true)])]
    #[case::unclosed_quote_skipped(Some("\"firefox %s:lynx"), vec![command(&["lynx", URL], false)])]
    fn browser_entries_come_before_the_platform_opener(
        #[case] env: Option<&str>,
        #[case] mut expected: Vec<BrowserCommand>,
    ) {
        expected.push(opener());
        assert_eq!(commands_split(env, ':', URL), expected);
    }

    #[test]
    fn windows_separates_entries_with_semicolons() {
        assert_eq!(
            commands_split(Some("a %s;b"), ';', URL),
            [command(&["a", URL], false), command(&["b", URL], false), opener()]
        );
        // A `:` doesn't separate there, so a drive letter stays whole.
        assert_eq!(
            commands_split(Some("C:\\browser.exe"), ';', URL),
            [command(&["C:\\browser.exe", URL], false), opener()]
        );
    }

    #[test]
    fn the_platform_opener_is_the_os_one() {
        let program = opener().argv[0].clone();
        if cfg!(target_os = "macos") {
            assert_eq!(program, "open");
        } else if cfg!(windows) {
            assert_eq!(program, "explorer");
        } else {
            assert_eq!(program, "xdg-open");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_next_command_runs_when_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("opened");
        let touch = BrowserCommand {
            argv: vec!["touch".into(), marker.clone().into()],
            background: false,
        };
        let commands = [command(&["false"], false), command(&["/no/such/browser"], false), touch];
        try_commands(&commands).await.unwrap();
        assert!(marker.exists(), "touch ran after false and a missing program failed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn every_command_failing_is_an_error() {
        let commands = [command(&["false"], false), command(&["/no/such/browser"], false)];
        let error = try_commands(&commands).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "no browser opened it (tried false, /no/such/browser)"
        );
        // A background command counts once it starts, whatever it exits with.
        try_commands(&[command(&["false"], true)]).await.unwrap();
    }
}
