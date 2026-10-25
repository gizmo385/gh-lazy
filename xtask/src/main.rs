//! Development tasks, run with `cargo xtask <task>`. Tasks (`parity`, `parity-matrix`, `record-fixtures`, release
//! helpers) are added by the issues that need them.

mod fixtures;

use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about = "Development tasks for LazyGithub")]
struct Cli {
    #[command(subcommand)]
    task: Option<Task>,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Record a parity fixture set from the real gh and scrub it. Run by hand, never in CI.
    RecordFixtures(fixtures::RecordArgs),
    /// Check every committed parity fixture against the scrub rules, without network.
    CheckFixtures(fixtures::CheckArgs),
    /// Run parity scenarios against the Python and Rust apps and capture their screens.
    Parity(xtask::parity::cli::ParityArgs),
    /// Check the parity matrix (docs/rust-rewrite/parity/) and print its rows per area and status.
    ParityMatrix(ParityMatrixArgs),
}

#[derive(Debug, clap::Args)]
struct ParityMatrixArgs {
    /// Print the summary as a Markdown table.
    #[arg(long)]
    markdown: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.task {
        // With no task given, show what's available and exit successfully.
        None => match Cli::command().print_help() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Some(Task::RecordFixtures(args)) => fixtures::record_fixtures(&args),
        Some(Task::CheckFixtures(args)) => fixtures::check_fixtures(&args),
        Some(Task::Parity(args)) => xtask::parity::cli::run(&args),
        Some(Task::ParityMatrix(args)) => xtask::matrix::run(&xtask::parity::cli::repo_root(), args.markdown),
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
