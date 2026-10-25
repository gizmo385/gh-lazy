//! What happened, in app terms. `App::update` turns each one into state changes and effects.

use lazy_github_core::logging::LogLine;

use crate::effect::{GithubResponse, RequestId, TimerId};
use crate::keymap::BindingId;
use crate::screens::auth::AuthAction;
use crate::screens::main::{FocusTarget, MainAction};
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
    /// Move focus to a target on the main screen, from a focus key or a click. The screen applies Python's rules:
    /// a section hidden by a runtime toggle or a hidden command log doesn't take focus.
    Focus(FocusTarget),
    /// Tab (`textual.screen.focus_next`): the next target in the focus ring.
    FocusNext,
    /// Shift+Tab (`textual.screen.focus_previous`): the previous target in the focus ring.
    FocusPrevious,
    /// `app.maximize_widget`: show only the focused widget. A no-op while something is maximized.
    Maximize,
    /// Escape while maximized: back to the normal layout, focus unchanged.
    Minimize,
    /// `app.open_help`: open the keybindings panel, or close it if it's open.
    ToggleHelp,
    /// Replace the startup or authentication screen with the main screen.
    EnterMain,

    // Screens.
    /// The authentication screen's own actions.
    Auth(AuthAction),
    /// The main screen's own actions.
    Main(MainAction),

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
