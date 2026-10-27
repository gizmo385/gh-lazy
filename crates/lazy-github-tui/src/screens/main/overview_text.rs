//! The layout the Overview tabs share (Repo, pull request and issue): header lines at the left edge, a rule, and a
//! markdown body below, laid out as one [`Rendered`] that scrolls as one.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::components::markdown::{self, Rendered, TextRun};
use crate::theme::{Accent, Theme};

/// The styles header lines are drawn in.
#[derive(Debug, Clone, Copy)]
pub(super) struct HeaderStyles {
    /// Plain text: Textual's `Label` in `$foreground`.
    pub(super) text: Style,
    /// A link (`@click` markup): the text underlined.
    pub(super) link: Style,
}

impl HeaderStyles {
    pub(super) fn new(theme: &Theme) -> Self {
        let text = Style::new().fg(theme.color(theme.palette.foreground, theme.background()));
        Self {
            text,
            link: text.add_modifier(Modifier::UNDERLINED),
        }
    }
}

/// Text in one of the fixed accent colors, as Python's `[greenyellow]` markup.
pub(super) fn accent(theme: &Theme, accent: Accent) -> Style {
    Style::new().fg(theme.accent(accent))
}

/// Adds `more` below what's in `out`, moving its links down with it.
pub(super) fn append(out: &mut Rendered, more: Rendered) {
    let shift = out.lines.len();
    out.links.extend(more.links.into_iter().map(|mut span| {
        span.row += shift;
        span
    }));
    out.lines.extend(more.lines);
}

/// Adds `runs` as a `Label` wraps them, `width` cells wide.
pub(super) fn push_label(out: &mut Rendered, runs: &[TextRun], width: u16) {
    append(out, markdown::wrap_text(runs, width));
}

/// Adds Textual's `Rule` as the overviews style it (`color: $secondary; margin: 1 0`): a blank row, a line of `─`
/// across `width`, and a blank row.
pub(super) fn push_rule(out: &mut Rendered, width: u16, theme: &Theme) {
    let rule = Style::new().fg(theme.color(theme.palette.secondary, theme.background()));
    out.lines.push(Line::default());
    out.lines
        .push(Line::from(Span::styled("─".repeat(usize::from(width)), rule)));
    out.lines.push(Line::default());
}
