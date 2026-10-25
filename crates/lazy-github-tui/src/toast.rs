//! Toast notifications, as data. [`Toasts`] is the stack `App` owns, which expires and draws them.

use std::time::Duration;

use ratatui::text::Line;

pub use crate::components::toasts::Toasts;

/// How long a toast stays up unless it says otherwise: Textual's `NOTIFICATION_TIMEOUT`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// How a toast is styled, as Textual's `notify(severity=...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    /// `information`, the default.
    #[default]
    Information,
    /// `warning`.
    Warning,
    /// `error`.
    Error,
}

/// One toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    /// The styling.
    pub severity: Severity,
    /// The bold first line, if any.
    pub title: Option<String>,
    /// The body, word-wrapped to the toast. Spans may be bold (or carry other modifiers) and may set an exact RGB
    /// foreground from an [`Accent`](crate::theme::Accent), which is how Python's markup renders; the rest is colored
    /// from the theme. A `\n` starts a new line.
    pub message: Line<'static>,
    /// How long after it's raised the toast disappears by itself.
    pub timeout: Duration,
}

impl Toast {
    /// A toast with no title and the default timeout.
    pub fn new(severity: Severity, message: impl Into<Line<'static>>) -> Self {
        Self {
            severity,
            title: None,
            message: message.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// An `information` toast.
    pub fn info(message: impl Into<Line<'static>>) -> Self {
        Self::new(Severity::Information, message)
    }

    /// A `warning` toast.
    pub fn warning(message: impl Into<Line<'static>>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// An `error` toast.
    pub fn error(message: impl Into<Line<'static>>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Sets the title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets how long the toast stays up, as `notify(timeout=...)`.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}
