use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lazy_github_core::config::{Config, ConfigPaths};
use lazy_github_core::git::GitCli;
use lazy_github_core::github::{GhCli, GhCliOptions};
use lazy_github_core::logging::{self, LoggingOptions};
use lazy_github_tui::app::App;
use lazy_github_tui::clock::SystemClock;
use lazy_github_tui::options::AppOptions;
use lazy_github_tui::theme::{ColorMode, Theme};

/// The variable the git helpers read the user's working directory from. The `gh-lazy` extension script and Nix
/// wrappers set it before changing directory; otherwise it's the current directory.
const ORIGINAL_PWD_ENV: &str = "LAZY_GITHUB_ORIGINAL_PWD";

/// A terminal UI for GitHub.
#[derive(Debug, Parser)]
#[command(name = "lazy-github", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run LazyGithub (the default)
    Run,
}

fn main() -> ExitCode {
    // Parse first: `--help`, `--version` and usage errors exit here, before anything touches the disk.
    let cli = Cli::try_parse().unwrap_or_else(|err| err.exit());
    let original_pwd = original_pwd(std::env::var_os(ORIGINAL_PWD_ENV), std::env::current_dir);
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(original_pwd),
    }
}

/// The user's working directory: `var` (`LAZY_GITHUB_ORIGINAL_PWD`) if set and non-empty, otherwise `cwd()`, or
/// `None` if that fails too.
///
/// An empty value falls back like an unset one, since Python runs git in the process's directory then. Python
/// exports the value into its own environment for the git helpers. Setting an environment variable is `unsafe` in
/// Rust 2024 and the workspace forbids `unsafe`, so the value is resolved once here and passed down instead.
fn original_pwd(var: Option<OsString>, cwd: impl FnOnce() -> io::Result<PathBuf>) -> Option<PathBuf> {
    var.filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| cwd().ok())
}

/// Loads the config, starts logging and runs the TUI.
fn run(original_pwd: Option<PathBuf>) -> ExitCode {
    // Installed before the TUI's panic hook, which chains to it, so a panic restores the terminal and then prints
    // color-eyre's report.
    if let Err(e) = color_eyre::install() {
        eprintln!("Error: {e}");
        return ExitCode::FAILURE;
    }
    // A broken config stops here, with the terminal untouched and nothing logged: running on the defaults would
    // overwrite the user's file on the next save.
    let (paths, config) = match ConfigPaths::discover().and_then(|paths| Config::load(&paths).map(|c| (paths, c))) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Held until the end of this function, after the terminal is restored, so the last lines reach the file.
    let logging = logging::init(LoggingOptions::from(&config.core));
    tracing::debug!("Starting LazyGithub {}", env!("CARGO_PKG_VERSION"));
    if let Some(pwd) = &original_pwd {
        tracing::debug!("Working directory: {}", pwd.display());
    }

    // Read once, before the terminal is set up, like Textual and Rich do.
    let color_mode = ColorMode::from_env();
    tracing::debug!("Color mode: {color_mode:?}");
    let options = AppOptions::from_env();
    tracing::debug!("Options: {options:?}");

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(color_eyre::Report::from)
        .and_then(|runtime| {
            let gh = GhCli::new(GhCliOptions::from(&config));
            let git = GitCli::new(original_pwd);
            let theme = Theme::from_config(&config.appearance.theme, color_mode);
            let app = App::new(config, theme, options, SystemClock);
            runtime.block_on(lazy_github_tui::tui::run(app, gh, git, logging.command_log, paths))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(report) => {
            // One line with the cause chain, in both places. color-eyre's full report adds ANSI colors and a
            // backtrace that help neither the user nor the log; panics are logged in full by the panic hook.
            tracing::error!("LazyGithub stopped: {report:#}");
            eprintln!("Error: {report:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, original_pwd};
    use clap::{CommandFactory, Parser, error::ErrorKind};
    use std::io;
    use std::path::PathBuf;

    // Shaped like `std::env::current_dir`, which is what `main` passes.
    #[allow(clippy::unnecessary_wraps)]
    fn cwd() -> io::Result<PathBuf> {
        Ok(PathBuf::from("cwd"))
    }

    fn no_cwd() -> io::Result<PathBuf> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    #[test]
    fn original_pwd_prefers_the_variable() {
        assert_eq!(
            original_pwd(Some("launched".into()), cwd),
            Some(PathBuf::from("launched"))
        );
        assert_eq!(
            original_pwd(Some("launched".into()), no_cwd),
            Some(PathBuf::from("launched"))
        );
    }

    #[test]
    fn an_empty_original_pwd_falls_back_to_the_cwd() {
        assert_eq!(original_pwd(Some("".into()), cwd), Some(PathBuf::from("cwd")));
        assert_eq!(original_pwd(Some("".into()), no_cwd), None);
    }

    #[test]
    fn an_unset_original_pwd_falls_back_to_the_cwd() {
        assert_eq!(original_pwd(None, cwd), Some(PathBuf::from("cwd")));
        assert_eq!(original_pwd(None, no_cwd), None);
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_flag_displays_version() {
        let err = Cli::try_parse_from(["lazy-github", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn run_is_the_default() {
        assert!(Cli::try_parse_from(["lazy-github"]).unwrap().command.is_none());
        assert!(matches!(
            Cli::try_parse_from(["lazy-github", "run"]).unwrap().command,
            Some(Command::Run)
        ));
    }
}
