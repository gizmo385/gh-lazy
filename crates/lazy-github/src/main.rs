use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lazy_github_core::config::{Config, ConfigPaths};
use lazy_github_core::github::{GhCli, GhCliOptions};
use lazy_github_core::logging::{self, LoggingOptions};
use lazy_github_tui::app::App;
use lazy_github_tui::clock::SystemClock;
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
    let original_pwd = original_pwd();
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(original_pwd.as_deref()),
    }
}

/// The user's working directory: `LAZY_GITHUB_ORIGINAL_PWD` if set, otherwise the current directory.
///
/// Python exports it into its own environment for the git helpers. Setting an environment variable is `unsafe` in
/// Rust 2024 and the workspace forbids `unsafe`, so the value is resolved once here and passed down instead.
fn original_pwd() -> Option<PathBuf> {
    std::env::var_os(ORIGINAL_PWD_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
}

/// Loads the config, starts logging and runs the TUI.
fn run(original_pwd: Option<&Path>) -> ExitCode {
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
    if let Some(pwd) = original_pwd {
        tracing::debug!("Working directory: {}", pwd.display());
    }

    // Read once, before the terminal is set up, like Textual and Rich do.
    let color_mode = ColorMode::from_env();
    tracing::debug!("Color mode: {color_mode:?}");

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(color_eyre::Report::from)
        .and_then(|runtime| {
            let gh = GhCli::new(GhCliOptions::from(&config));
            let theme = Theme::from_config(&config.appearance.theme, color_mode);
            let app = App::new(config, theme, SystemClock);
            runtime.block_on(lazy_github_tui::tui::run(app, gh, logging.command_log, paths))
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
    use super::{Cli, Command};
    use clap::{CommandFactory, Parser, error::ErrorKind};

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
