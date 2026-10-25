//! Development tasks, run with `cargo xtask <task>`. Tasks (`parity`, `record-fixtures`, release helpers) are added
//! by the issues that need them.

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about = "Development tasks for LazyGithub")]
struct Cli {
    #[command(subcommand)]
    task: Option<Task>,
}

#[derive(Debug, Subcommand)]
enum Task {}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    match cli.task {
        // With no task given, show what's available and exit successfully.
        None => Cli::command().print_help(),
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
