//! What happened, in app terms. `App::update` turns each one into state changes and effects.

use lazy_github_core::logging::LogLine;
use lazy_github_core::models::Repository;

use crate::effect::{GitResponse, GithubResponse, RequestId, TimerId};
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
    /// Ring the terminal bell (Textual's `App.bell`): a click on a disabled footer item.
    Bell,
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
    /// Refresh the unread notification count now: the poll's timer, and in M4 the command palette, a Settings save
    /// and closing the Notifications modal. Skipped while a refresh runs; blanks the count when notifications are off.
    RefreshNotifications,
    /// Make this repo the current one: the status box, the sections and the details follow. Every source of a
    /// selection dispatches it (the repos table, the current-directory default, and later lookups and links), and
    /// selecting the current repo again reloads everything. Boxed: a `Repository` is big.
    SelectRepo(Box<Repository>),

    // Screens.
    /// The authentication screen's own actions.
    Auth(AuthAction),
    /// The main screen's own actions.
    Main(MainAction),

    // Plumbing: results of effects and other runtime inputs, routed to their owner.
    /// A GitHub request finished.
    Github(RequestId, GithubResponse),
    /// A git request finished.
    Git(RequestId, GitResponse),
    /// A config save finished.
    ConfigSaved(Result<(), String>),
    /// A timer ticked.
    Timer(TimerId),
    /// A command-log line arrived.
    CommandLog(LogLine),

    /// An input a component took and already applied, such as a table key moving the cursor: nothing else happens,
    /// and nothing is logged. Returning it stops the key at that link of the chain.
    Consumed,

    /// A binding whose feature lands in a later milestone: logged at debug, otherwise a no-op.
    Unimplemented(BindingId),
}
