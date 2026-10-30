//! What happened, in app terms. `App::update` turns each one into state changes and effects.

use lazy_github_core::logging::LogLine;
use lazy_github_core::models::Repository;

use crate::effect::{GitResponse, GithubResponse, RequestId, TimerId};
use crate::keymap::BindingId;
use crate::modal::{DialogAction, ModalEntry, SettingsOutcome};
use crate::screens::auth::AuthAction;
use crate::screens::main::{FocusTarget, MainAction, Toggle};
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
    /// Put this text on the in-app clipboard and offer it to the terminal's (Textual's `App.copy_to_clipboard`): a
    /// text input's copy or cut.
    Copy(String),
    /// Move focus to a target on the main screen, from a focus key or a click. The screen applies Python's rules:
    /// a section hidden by a runtime toggle or a hidden command log doesn't take focus.
    Focus(FocusTarget),
    /// Tab (`textual.screen.focus_next`): the next target in the focus ring.
    FocusNext,
    /// Shift+Tab (`textual.screen.focus_previous`): the previous target in the focus ring.
    FocusPrevious,
    /// `app.maximize_widget`: show only the focused widget. A no-op while something is maximized. `toast` raises the
    /// one-time "View Maximized" toast (LazyGithub's `action_maximize`); the palette's "Maximize" (Textual's
    /// `Screen.action_maximize`) doesn't, and leaves it for the next `M`.
    Maximize {
        /// Whether to raise the one-time toast, if it hasn't been shown.
        toast: bool,
    },
    /// Escape while maximized: back to the normal layout, focus unchanged.
    Minimize,
    /// `app.open_help`: open the keybindings panel, or close it if it's open.
    ToggleHelp,
    /// `app.command_palette`: open the command palette over the main screen, with the commands that apply now.
    OpenCommandPalette,
    /// The palette's "Theme" command, once the palette has closed: open the theme palette over the main screen.
    OpenThemePalette,
    /// A theme palette hit: switch the whole app to the built-in theme of this name for the session, keeping the
    /// terminal's color mode. `appearance.theme` isn't written (Py bug B27, answered: keep parity).
    SetTheme(&'static str),
    /// The palette's "Change Settings" command, once the palette has closed: open the Settings modal over the main
    /// screen, reading the config afresh.
    OpenSettings,
    /// The Settings modal closed: saved with the edited config, or cancelled.
    SettingsClosed(SettingsOutcome),
    /// A palette "Toggle …" command: show or hide a section or the command log on the main screen. `App` applies it,
    /// since hiding the maximized section also restores the layout (Py bug B88's answer).
    Toggle(Toggle),
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

    // Dialogs.
    /// Push a dialog onto the modal stack, from [`Cx::open_dialog`](crate::component::Cx::open_dialog).
    OpenDialog(ModalEntry),
    /// A dialog's own action, for the topmost open dialog of its kind.
    Dialog(DialogAction),

    // Plumbing: results of effects and other runtime inputs, routed to their owner.
    /// A GitHub request finished.
    Github(RequestId, GithubResponse),
    /// A git request finished.
    Git(RequestId, GitResponse),
    /// A cache file read finished: its text, `None` when there's no file, or the error as text.
    CacheRead(RequestId, Result<Option<String>, String>),
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
