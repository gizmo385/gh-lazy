//! Development tasks, run with `cargo xtask <task>`. Tasks (`parity`, `parity-matrix`, `record-fixtures`,
//! `release-plan`) are added by the issues that need them.

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
    /// Check the parity matrix (docs/rust-rewrite/parity/) and the scenarios and golden manifests that name its rows,
    /// and print its rows per area and status, then per area and coverage.
    ParityMatrix(ParityMatrixArgs),
    /// Write the parity site's landing page (run results, matrix status and coverage) from a parity report and the
    /// matrix. The Pages workflow publishes it with the report under report/.
    ParitySite(xtask::site::SiteArgs),
    /// Run the large-data timing tests (#325) in a release build, one process per case, and print each case's slowest
    /// frame and peak RSS. Not part of the required CI job.
    Perf(xtask::perf::PerfArgs),
    /// Write the generated `large-account` parity fixture set (1,000 repos over 20 pages) under parity/fixtures/.
    LargeAccountFixtures,
    /// Decide what a run of the release workflow releases (tag, version, prerelease, crates) and print it as
    /// `key=value` lines for `$GITHUB_OUTPUT`. Exits non-zero, with the reason, when the run must not release.
    ReleasePlan(xtask::release::ReleasePlanArgs),
}

#[derive(Debug, clap::Args)]
struct ParityMatrixArgs {
    /// Print the tables as Markdown.
    #[arg(long, conflicts_with = "uncovered")]
    markdown: bool,
    /// Print the rows no scenario or golden names, except intentional diffs, as `<id>  <status>` lines in area order.
    #[arg(long)]
    uncovered: bool,
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
        Some(Task::ParityMatrix(args)) => {
            xtask::matrix::run(&xtask::parity::cli::repo_root(), args.markdown, args.uncovered)
        }
        Some(Task::ParitySite(args)) => xtask::site::run(&xtask::parity::cli::repo_root(), &args),
        Some(Task::Perf(args)) => xtask::perf::run(&xtask::parity::cli::repo_root(), &args),
        Some(Task::LargeAccountFixtures) => xtask::large_account::run(&xtask::parity::cli::repo_root()),
        Some(Task::ReleasePlan(args)) => xtask::release::run(&xtask::parity::cli::repo_root(), &args),
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
