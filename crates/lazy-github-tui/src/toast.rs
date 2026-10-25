//! Toast notifications, as data. #79 renders them and expires them; until then `App` only records them.

use ratatui::text::Line;

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
    /// The body. Spans carry modifiers such as bold, never colors; #79 colors it from the theme.
    pub message: Line<'static>,
}

impl Toast {
    /// A toast with no title.
    pub fn new(severity: Severity, message: impl Into<Line<'static>>) -> Self {
        Self {
            severity,
            title: None,
            message: message.into(),
        }
    }

    /// Sets the title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}
