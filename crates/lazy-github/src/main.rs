use clap::Parser;
use lazy_github_core::config::{Config, ConfigPaths};
use lazy_github_core::logging::{self, LoggingHandle, LoggingOptions};

/// A terminal UI for GitHub.
#[derive(Debug, Parser)]
#[command(name = "lazy-github", version, about)]
struct Cli {}

fn main() {
    let logging = start_logging();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // `exit` skips destructors, so flush the log first.
            drop(logging);
            err.exit();
        }
    };
    // Only `--version` and `--help` exist so far; the TUI and subcommands arrive in M1.
    let _ = cli;
}

/// Loads the config for its log settings and starts logging.
///
/// Reporting a broken config to the user belongs to app startup, which doesn't exist yet. Until then the defaults
/// are used and the error is logged, so nothing reaches the terminal.
fn start_logging() -> Option<LoggingHandle> {
    let paths = ConfigPaths::discover().ok()?;
    let (config, error) = match Config::load(&paths) {
        Ok(config) => (config, None),
        Err(e) => (Config::default_in(&paths), Some(e)),
    };
    let handle = logging::init(LoggingOptions::from(&config.core));
    if let Some(error) = error {
        tracing::warn!(
            error = &error as &(dyn std::error::Error + 'static),
            "Could not load the config; using the defaults"
        );
    }
    tracing::debug!("Starting LazyGithub {}", env!("CARGO_PKG_VERSION"));
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::Cli;
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
}
