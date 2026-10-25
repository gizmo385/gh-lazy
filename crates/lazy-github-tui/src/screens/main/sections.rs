//! The bordered boxes of the main screen. Python's `LazyGithubContainer`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Block;

use crate::theme::Theme;

/// A box titled `┌─ {title} ─…┐`, its border and title in `$success` when focus is within it and
/// `$primary-lighten-3` otherwise. Section bodies arrive with #78 and M2.
pub(super) fn titled_box<'a>(title: &str, focused: bool, theme: &Theme) -> Block<'a> {
    let slot = if focused {
        theme.palette.success
    } else {
        theme.palette.primary_lighten_3
    };
    let border = Style::new().fg(theme.color(slot, theme.background()));
    Block::bordered().border_style(border).title(Line::from(vec![
        Span::styled("─", border),
        Span::styled(format!(" {title} "), border),
    ]))
}
