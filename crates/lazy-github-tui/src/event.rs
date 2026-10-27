//! Everything the event loop can receive.

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::logging::LogLine;

use crate::effect::{GitResponse, GithubResponse, RequestId, TimerId};

/// An input to the event loop: from the terminal, the runtime, the logger, the clock or the OS.
#[derive(Debug)]
pub enum Event {
    /// A key press, repeat or release.
    Key(KeyEvent),
    /// A mouse event.
    Mouse(MouseEvent),
    /// Bracketed paste.
    Paste(String),
    /// The terminal was resized to `(columns, rows)`.
    Resize(u16, u16),
    /// The terminal gained focus.
    FocusGained,
    /// The terminal lost focus.
    FocusLost,
    /// A `GithubRequest` finished.
    Github {
        /// The id from the `Effect::Github` that started it.
        id: RequestId,
        /// Its result.
        response: GithubResponse,
    },
    /// A `GitRequest` finished.
    Git {
        /// The id from the `Effect::Git` that started it.
        id: RequestId,
        /// Its result.
        response: GitResponse,
    },
    /// An `Effect::ReadCache` finished: the file's text, `None` when there's no file, or the error as text.
    CacheRead {
        /// The id from the `Effect::ReadCache` that started it.
        id: RequestId,
        /// Its result.
        result: Result<Option<String>, String>,
    },
    /// An `Effect::SaveConfig` finished. The error is already rendered as text.
    ConfigSaved(Result<(), String>),
    /// A timer ticked.
    Timer(TimerId),
    /// A line for the command log, from the logging layer.
    CommandLog(LogLine),
    /// `App::next_deadline()` passed.
    Deadline,
    /// The OS asked the app to stop.
    Signal(Signal),
}

/// A termination signal. Only delivered on unix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// `SIGTERM`.
    Term,
    /// `SIGHUP`.
    Hup,
}

impl From<crossterm::event::Event> for Event {
    fn from(event: crossterm::event::Event) -> Self {
        use crossterm::event::Event as Term;
        match event {
            Term::Key(key) => Event::Key(key),
            Term::Mouse(mouse) => Event::Mouse(mouse),
            Term::Paste(text) => Event::Paste(text),
            Term::Resize(cols, rows) => Event::Resize(cols, rows),
            Term::FocusGained => Event::FocusGained,
            Term::FocusLost => Event::FocusLost,
        }
    }
}
