//! The `gh api` runner: typed requests in, parsed responses or a [`GhError`] out.
//!
//! All GitHub access goes through the `gh` CLI. Single requests run `gh api -i` so the status and headers can be
//! parsed; [`GhCli::paginate`] runs `gh api --paginate` for endpoints whose every page is wanted at once.

mod cli;
mod error;
mod request;
mod response;

pub use cli::{GhCli, GhTimeouts};
pub use error::GhError;
pub use request::{Accept, GhRequest, Method};
pub use response::{GhResponse, interpret};

#[cfg(test)]
mod fixture_tests;
