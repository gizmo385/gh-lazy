//! fake-gh: a stand-in for the `gh` CLI that answers from fixture files, so the Python and Rust apps see the same
//! data with no network. The format and behavior are documented in `parity/README.md`.
//!
//! The library is shared with `xtask` (#68), which writes fixtures in the format defined in [`fixture`].

pub mod app;
pub mod argv;
pub mod fixture;
mod install;
pub mod lint;
pub mod log;
pub mod output;
pub mod store;

pub use install::install;

#[cfg(test)]
mod test_support;
