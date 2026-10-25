//! The status box across the top. Python's `LazyGithubStatusSummary`; #81 adds the repo and notifications.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::theme::Theme;

/// The status box. #81 replaces the text with the current repo and adds the notification count.
#[derive(Debug)]
pub(super) struct StatusBar {
    repo: &'static str,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            repo: "No repository selected",
        }
    }
}

impl StatusBar {
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let under = theme.background();
        let block = Block::bordered().border_style(Style::new().fg(theme.color(theme.palette.secondary, under)));
        let text = Paragraph::new(self.repo)
            .style(Style::new().fg(theme.color(theme.palette.foreground, under)))
            .block(block);
        frame.render_widget(text, area);
    }
}
