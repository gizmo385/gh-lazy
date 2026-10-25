//! The status box across the top of the main screen: the current repo on the left, the unread notification count on
//! the right. Python's `LazyGithubStatusSummary`.

use lazy_github_core::github::NOTIFICATIONS_PER_PAGE;
use lazy_github_core::models::Repository;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::theme::{Accent, Theme};

/// From this count on, the badge reads `N+`.
///
/// Py bug B6 (pending Chris): Python shows `N+` from 30 on, though it counts a first page of 50, so 37 unread shows
/// `37+`. Rust follows the recommendation and shows `50+` only when the page is full. Set this to 30 to match Python.
pub const UNREAD_PLUS_FROM: usize = NOTIFICATIONS_PER_PAGE;

/// What the right half shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnreadState {
    /// Blank: notifications are disabled, or the first count hasn't arrived.
    #[default]
    Hidden,
    /// The unread count from the last refresh, 0 to [`NOTIFICATIONS_PER_PAGE`].
    Count(usize),
}

/// The status box, drawn from the app's state.
pub struct StatusBar<'a> {
    /// The current repo, if one is selected.
    pub repo: Option<&'a Repository>,
    /// The unread count.
    pub unread: UnreadState,
    /// The theme.
    pub theme: &'a Theme,
}

/// The left half's text.
pub fn repo_text(repo: Option<&Repository>, theme: &Theme) -> Line<'static> {
    match repo {
        Some(repo) => Line::from(vec![
            Span::raw("Current repo: "),
            Span::styled(
                repo.full_name.clone(),
                Style::new().fg(theme.accent(Accent::GreenYellow)),
            ),
        ]),
        None => Line::raw("No repository selected"),
    }
}

/// The right half's text, or `None` for blank.
pub fn unread_text(unread: UnreadState, theme: &Theme) -> Option<Line<'static>> {
    let UnreadState::Count(count) = unread else {
        return None;
    };
    let (text, accent) = match count {
        0 => ("No unread notifications".to_owned(), Accent::GreenYellow),
        n if n >= UNREAD_PLUS_FROM => (format!("• Unread Notifications: {n}+"), Accent::Red),
        n => (format!("• Unread Notifications: {n}"), Accent::Red),
    };
    Some(Line::styled(text, Style::new().fg(theme.accent(accent))))
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let under = theme.background();
        let block = Block::bordered().border_style(Style::new().fg(theme.color(theme.palette.secondary, under)));
        let inner = block.inner(area);
        block.render(area, buf);

        let [left, right] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(inner);
        let text_style = Style::new().fg(theme.color(theme.palette.foreground, under));
        // No `wrap`: a long repo name is cut at its half rather than wrapped onto rows the box doesn't have.
        Paragraph::new(repo_text(self.repo, theme))
            .style(text_style)
            .alignment(Alignment::Left)
            .render(left, buf);
        if let Some(unread) = unread_text(self.unread, theme) {
            Paragraph::new(unread)
                .style(text_style)
                .alignment(Alignment::Right)
                .render(right, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use lazy_github_core::models::User;
    use ratatui::style::Color;

    use super::*;
    use crate::test_util::buffer_to_string;

    fn repo(full_name: &str) -> Repository {
        let (owner, name) = full_name.split_once('/').unwrap();
        Repository {
            name: name.into(),
            full_name: full_name.into(),
            default_branch: Some("main".into()),
            private: false,
            archived: Some(false),
            owner: User {
                login: owner.into(),
                id: 1,
                name: None,
                html_url: format!("https://github.com/{owner}"),
            },
            description: None,
            permissions: None,
        }
    }

    fn text(unread: UnreadState) -> Option<String> {
        unread_text(unread, &Theme::default()).map(|line| line.to_string())
    }

    #[test]
    fn unread_texts() {
        assert_eq!(text(UnreadState::Hidden), None);
        assert_eq!(text(UnreadState::Count(0)).as_deref(), Some("No unread notifications"));
        for (count, expected) in [
            (1, "• Unread Notifications: 1"),
            (29, "• Unread Notifications: 29"),
            // Python shows `30+` and `49+` here (Py bug B6).
            (30, "• Unread Notifications: 30"),
            (49, "• Unread Notifications: 49"),
            (50, "• Unread Notifications: 50+"),
        ] {
            assert_eq!(text(UnreadState::Count(count)).as_deref(), Some(expected), "{count}");
        }
    }

    #[test]
    fn unread_colors() {
        let theme = Theme::default();
        let fg = |count| unread_text(UnreadState::Count(count), &theme).unwrap().style.fg;
        assert_eq!(fg(0), Some(Color::Rgb(0xAD, 0xFF, 0x2F)));
        assert_eq!(fg(3), Some(Color::Rgb(0xFF, 0x00, 0x00)));
        assert_eq!(fg(50), Some(Color::Rgb(0xFF, 0x00, 0x00)));
    }

    #[test]
    fn the_repo_name_is_greenyellow() {
        let theme = Theme::default();
        let line = repo_text(Some(&repo("parity-bot/lazy-fixture")), &theme);
        assert_eq!(line.to_string(), "Current repo: parity-bot/lazy-fixture");
        assert_eq!(line.spans[0].style.fg, None);
        assert_eq!(line.spans[1].style.fg, Some(Color::Rgb(0xAD, 0xFF, 0x2F)));
    }

    fn draw(repo: Option<&Repository>, unread: UnreadState) -> Buffer {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 3));
        StatusBar {
            repo,
            unread,
            theme: &theme,
        }
        .render(buf.area, &mut buf);
        buf
    }

    fn screen(repo: Option<&Repository>, unread: UnreadState) -> String {
        buffer_to_string(&draw(repo, unread))
    }

    #[test]
    fn status_no_repo_hidden() {
        insta::assert_snapshot!(screen(None, UnreadState::Hidden));
    }

    #[test]
    fn status_repo_zero() {
        insta::assert_snapshot!(screen(Some(&repo("parity-bot/lazy-fixture")), UnreadState::Count(0)));
    }

    #[test]
    fn status_repo_three() {
        insta::assert_snapshot!(screen(Some(&repo("parity-bot/lazy-fixture")), UnreadState::Count(3)));
    }

    #[test]
    fn status_repo_fifty() {
        insta::assert_snapshot!(screen(Some(&repo("parity-bot/lazy-fixture")), UnreadState::Count(50)));
    }

    #[test]
    fn status_long_repo_clipped() {
        let long = repo("parity-org/quebec-service-with-a-deliberately-long-repository-name-16");
        insta::assert_snapshot!(screen(Some(&long), UnreadState::Count(50)));
    }

    #[test]
    fn styles() {
        let theme = Theme::default();
        let buf = draw(Some(&repo("o/r")), UnreadState::Count(3));
        let under = theme.background();
        let secondary = theme.color(theme.palette.secondary, under);
        let foreground = theme.color(theme.palette.foreground, under);
        assert_eq!(buf[(0, 0)].symbol(), "┌");
        assert_eq!(buf[(0, 0)].fg, secondary);
        assert_eq!(buf[(99, 2)].symbol(), "┘");
        // "Current repo: " in the foreground, then the name.
        assert_eq!(buf[(1, 1)].fg, foreground);
        assert_eq!(buf[(15, 1)].symbol(), "o");
        assert_eq!(buf[(15, 1)].fg, Color::Rgb(0xAD, 0xFF, 0x2F));
        // Right-aligned against the right border.
        assert_eq!(buf[(98, 1)].symbol(), "3");
        assert_eq!(buf[(98, 1)].fg, Color::Rgb(0xFF, 0x00, 0x00));
    }
}
