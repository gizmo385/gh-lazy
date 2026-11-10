//! The pending comments the body shows under their lines (#391, decision `diff-inline-comments`): the comments added
//! with Add Comment and not sent yet. Each is always expanded: a `Pending comment` title, then its body as typed
//! (plain text, wrapped, not markdown), behind a bar in `$warning`. The rows depend on the width, so they're laid out
//! on demand and cached by width and theme, as an expanded thread's are.

use ratatui::text::{Line, Span};

use super::review::PendingComment;
use super::threads::{BAR, BAR_WIDTH, MIN_BODY_WIDTH, ThreadStyles};
use crate::components::markdown::{self, Rendered, TextRun};
use crate::theme::Theme;

/// A pending comment's first row, after the bar.
const TITLE: &str = "Pending comment";

/// One pending comment's rows, laid out at a width in a theme.
#[derive(Debug, Clone, Default)]
pub struct PendingLayout {
    cached: Option<(u16, Theme, Rendered)>,
}

impl PendingLayout {
    /// The rows `comment` takes laid out `width` cells wide (the code column's width): its title and its body.
    pub fn rows(&mut self, comment: &PendingComment, width: u16) -> usize {
        match &self.cached {
            Some((w, _, rendered)) if *w == width => rendered.lines.len(),
            // Wrapping doesn't depend on the colors: the default theme measures as well as the one drawn with.
            _ => self.lines(comment, width, &Theme::default()).lines.len(),
        }
    }

    /// `comment`'s rows laid out `width` cells wide in `theme`.
    pub fn lines(&mut self, comment: &PendingComment, width: u16, theme: &Theme) -> &Rendered {
        let fresh = matches!(&self.cached, Some((w, t, _)) if *w == width && t == theme);
        if !fresh {
            self.cached = Some((width, *theme, lay_out(comment, width, theme)));
        }
        match &self.cached {
            Some((_, _, rendered)) => rendered,
            // Set just above.
            None => unreachable!("the layout was just made"),
        }
    }
}

fn lay_out(comment: &PendingComment, width: u16, theme: &Theme) -> Rendered {
    let styles = ThreadStyles::new(theme);
    let room = width.saturating_sub(BAR_WIDTH).max(MIN_BODY_WIDTH);
    let body = markdown::wrap_text(&[TextRun::new(comment.body.as_str(), styles.fill)], room);
    let title = Line::from(vec![
        Span::styled(BAR, styles.pending_bar),
        Span::styled(TITLE, styles.pending_title),
    ]);
    let lines = std::iter::once(title)
        .chain(body.lines.into_iter().map(|line| {
            let mut spans = vec![Span::styled(BAR, styles.pending_bar)];
            spans.extend(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, styles.fill.patch(span.style))),
            );
            Line::from(spans)
        }))
        .collect();
    Rendered {
        lines,
        ..Rendered::default()
    }
}
