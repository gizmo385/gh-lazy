use clap::Parser;

/// A terminal UI for GitHub.
#[derive(Debug, Parser)]
#[command(name = "lazy-github", version, about)]
struct Cli {}

fn main() {
    // Only `--version` and `--help` exist so far; the TUI and subcommands arrive in M1.
    let _cli = Cli::parse();
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
