//! Models, configuration, the `gh` client and git helpers for LazyGithub.
//!
//! This crate has no UI dependencies, so all GitHub and config logic is testable without a terminal.

mod atomic_write;
pub mod browser;
pub mod cache;
pub mod config;
pub mod debug;
pub mod diff;
pub mod drafts;
pub mod gh;
pub mod git;
pub mod github;
pub mod links;
pub mod logging;
pub mod models;
pub mod references;
#[cfg(test)]
mod test_fs;
#[cfg(any(test, feature = "test-util"))]
pub mod test_log;
pub mod time;
