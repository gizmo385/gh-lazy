//! `cargo xtask parity`: runs scenarios against the Python and Rust apps in pseudo-terminals and dumps their screens.
//! The CLI, environment, scenario format and report layout are documented in `parity/README.md`.

pub mod check;
pub mod cli;
pub mod diff;
pub mod env;
pub mod grid;
pub mod keys;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod structure;
pub mod summary;
#[cfg(test)]
mod test_grid;

use std::fmt::Display;

/// A failure of the harness itself (exit code 2), as opposed to an app failing a scenario.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct HarnessError(pub String);

pub type Result<T> = std::result::Result<T, HarnessError>;

/// Adds what the harness was doing to an error.
pub trait Context<T> {
    fn context(self, doing: &str) -> Result<T>;
    fn with_context(self, doing: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: Display> Context<T> for std::result::Result<T, E> {
    fn context(self, doing: &str) -> Result<T> {
        self.map_err(|e| HarnessError(format!("{doing}: {e}")))
    }

    fn with_context(self, doing: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| HarnessError(format!("{}: {e}", doing())))
    }
}
