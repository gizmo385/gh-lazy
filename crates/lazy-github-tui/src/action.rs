//! What happened, in app terms. `App::update` turns each one into state changes and effects.

use lazy_github_core::logging::LogLine;

use crate::effect::{GithubResponse, RequestId, TimerId};
use crate::keymap::BindingId;
use crate::toast::Toast;

/// The closed set of things `App::update` handles. It grows one variant per feature: screens and components add one
/// variant each, holding that component's own action enum.
#[derive(Debug)]
pub enum Action {
    // App-level actions.
    /// Stop the app (`app.quit`, `ctrl+q`, `SIGTERM`, `SIGHUP`).
    Quit,
    /// `ctrl+c`: explain how to quit, as Textual's `help_quit` does.
    QuitHint,
    /// Show a toast.
    Notify(Toast),

    // Plumbing: results of effects and other runtime inputs, routed to their owner.
    /// A GitHub request finished.
    Github(RequestId, GithubResponse),
    /// A config save finished.
    ConfigSaved(Result<(), String>),
    /// A timer ticked.
    Timer(TimerId),
    /// A command-log line arrived.
    CommandLog(LogLine),

    /// A binding whose feature lands in a later milestone: logged at debug, otherwise a no-op.
    Unimplemented(BindingId),
}
