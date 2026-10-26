//! The "GitHub Authentication" screen, shown while `gh` can't be used: how to log in, the intro text, and a footer
//! with only `q Quit`. It re-checks `gh auth status` every five seconds and moves on to the main screen once that
//! succeeds. Python's `AuthenticationModal` with `GithubCliAuthInstructions`.

use std::time::Duration;

use lazy_github_core::gh::GhError;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::action::Action;
use crate::component::{Component, Cx, RenderCx};
use crate::components::info::InfoView;
use crate::effect::{Effect, GithubRequest, RequestId, TimerId};
use crate::keymap::BindingId;
use crate::theme::{Accent, Theme};

/// How often `gh auth status` is re-run, as Python's `set_interval(5, …)`.
pub const AUTH_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// The screen's only binding. The screen is modal, so the app's other bindings don't reach past it.
const AUTH_BINDINGS: &[BindingId] = &[BindingId::AppQuit];

/// Textual's `border: ascii`.
const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// Why `gh` can't be used, which picks the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthReason {
    /// `gh` isn't installed or isn't on `PATH`.
    NotInstalled,
    /// Any other failure: not logged in, or `gh auth status` failed for another reason.
    NotLoggedIn,
}

impl AuthReason {
    /// The reason a failed auth check stands for.
    pub fn from_error(error: &GhError) -> Self {
        match error {
            GhError::NotInstalled => AuthReason::NotInstalled,
            _ => AuthReason::NotLoggedIn,
        }
    }
}

/// The authentication screen.
#[derive(Debug)]
pub struct AuthScreen {
    reason: AuthReason,
    /// The poll's `CheckAuth`, while it's in flight.
    pending: Option<RequestId>,
    info: InfoView,
}

/// The authentication screen's actions.
#[derive(Debug)]
pub enum AuthAction {
    /// The poll timer ticked: check again, unless a check is already running.
    Poll,
    /// A `CheckAuth` answered.
    Checked(RequestId, Result<(), GhError>),
}

impl AuthScreen {
    /// The screen for `reason`, with its poll timer started.
    pub fn open(reason: AuthReason, cx: &mut Cx<'_>) -> Self {
        cx.effect(Effect::StartInterval {
            id: TimerId::AuthPoll,
            every: AUTH_POLL_INTERVAL,
        });
        Self {
            reason,
            pending: None,
            info: InfoView::default(),
        }
    }

    /// Why the screen is up.
    pub fn reason(&self) -> AuthReason {
        self.reason
    }
}

impl Component for AuthScreen {
    type Action = AuthAction;

    fn bindings(&self) -> &'static [BindingId] {
        AUTH_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        (id == BindingId::AppQuit).then_some(Action::Quit)
    }

    fn update(&mut self, action: AuthAction, cx: &mut Cx<'_>) {
        match action {
            AuthAction::Poll => {
                if self.pending.is_none() {
                    self.pending = Some(cx.request(GithubRequest::CheckAuth));
                }
            }
            AuthAction::Checked(id, result) => {
                if self.pending != Some(id) {
                    tracing::debug!("Dropping the stale response to request {id}");
                    return;
                }
                self.pending = None;
                match result {
                    Ok(()) => {
                        tracing::debug!("Github CLI access confirmed");
                        // Python's poll doesn't write `auth_last_checked`, so neither does this.
                        cx.dispatch(Action::EnterMain);
                    }
                    Err(error) => {
                        tracing::debug!("Continuing to wait for Github CLI access...");
                        // Installing gh while the screen is up moves it on to the login message.
                        self.reason = AuthReason::from_error(&error);
                    }
                }
            }
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        // Python's literal CSS `red`, not a theme slot (#77 may change this).
        let border_style = Style::new().fg(theme.accent(Accent::Red));
        let block = Self::block(border_style).title(Line::from(vec![
            Span::styled("-", border_style),
            Span::styled(" GitHub Authentication ", border_style),
        ]));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        // The content stops above the row `App` draws the footer on.
        let Some(footer) = Self::footer_area(area) else {
            return;
        };

        let message = self.message_lines(usize::from(inner.width), theme);
        let footer_y = footer.y;
        let row = |y: u16, height: u16| Rect {
            y,
            height: height.min(footer_y.saturating_sub(y)),
            ..inner
        };

        // The message, then the `Rule` with its one-row margins, then the intro; the footer is docked on the last row.
        let message_height = u16::try_from(message.len()).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(message), row(inner.y, message_height));
        let rule_y = inner.y.saturating_add(message_height).saturating_add(1);
        let rule_style = Style::new().fg(theme.color(theme.palette.secondary, theme.background()));
        frame.render_widget(
            Paragraph::new(Line::styled("─".repeat(usize::from(inner.width)), rule_style)),
            row(rule_y, 1),
        );
        let info_y = rule_y.saturating_add(2);
        self.info
            .render_clipped(frame, row(info_y, footer_y.saturating_sub(info_y)), theme);
        // `App` draws the shared footer on the last row, `Self::footer_area`.
    }
}

impl AuthScreen {
    fn block(border_style: Style) -> Block<'static> {
        Block::bordered().border_set(ASCII_BORDER).border_style(border_style)
    }

    /// The footer row for the screen drawn in `area`: the last row inside the border, where Textual docks the
    /// screen's `Footer`. `None` when the border leaves no room.
    pub fn footer_area(area: Rect) -> Option<Rect> {
        let inner = Self::block(Style::new()).inner(area);
        (!inner.is_empty()).then(|| Rect {
            y: inner.bottom() - 1,
            height: 1,
            ..inner
        })
    }

    /// The message, word-wrapped to `width` as Textual's `Static` does.
    fn message_lines(&self, width: usize, theme: &Theme) -> Vec<Line<'static>> {
        let plain = Style::new().fg(theme.color(theme.palette.foreground, theme.background()));
        let words: Vec<(String, Style)> = match self.reason {
            AuthReason::NotLoggedIn => {
                let green = plain.fg(theme.accent(Accent::Green));
                let lead = "To proceed, please log into the Github CLI via".split(' ');
                lead.map(|w| (w.to_owned(), plain))
                    .chain(["gh", "auth", "login"].map(|w| (w.to_owned(), green)))
                    .collect()
            }
            AuthReason::NotInstalled => GhError::NotInstalled
                .to_string()
                .split(' ')
                .map(|w| (w.to_owned(), plain))
                .collect(),
        };
        wrap_words(&words, width.max(1))
    }
}

/// Greedy word wrap over styled words. A space between two words of one style takes that style, so a green
/// `gh auth login` is green throughout. Words longer than `width` are split; the messages are ASCII, so bytes are
/// columns.
fn wrap_words(words: &[(String, Style)], width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    let mut last_style = None;
    for (word, style) in words {
        let mut word = word.as_str();
        loop {
            let gap = usize::from(used > 0);
            if used + gap + word.len() <= width {
                if gap == 1 {
                    let space_style = if last_style == Some(*style) {
                        *style
                    } else {
                        Style::new()
                    };
                    spans.push(Span::styled(" ", space_style));
                }
                spans.push(Span::styled(word.to_owned(), *style));
                used += gap + word.len();
                last_style = Some(*style);
                break;
            }
            if used > 0 {
                lines.push(Line::from(std::mem::take(&mut spans)));
                used = 0;
                continue;
            }
            let (head, tail) = word.split_at(width);
            lines.push(Line::from(Span::styled(head.to_owned(), *style)));
            word = tail;
        }
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_reasons() {
        assert_eq!(AuthReason::from_error(&GhError::NotInstalled), AuthReason::NotInstalled);
        let not_logged_in = GhError::NotAuthenticated {
            hostname: None,
            detail: String::new(),
        };
        assert_eq!(AuthReason::from_error(&not_logged_in), AuthReason::NotLoggedIn);
        let command = GhError::Command {
            code: Some(1),
            stderr: String::new(),
        };
        assert_eq!(AuthReason::from_error(&command), AuthReason::NotLoggedIn);
        let timeout = GhError::Timeout {
            after: Duration::from_secs(20),
        };
        assert_eq!(AuthReason::from_error(&timeout), AuthReason::NotLoggedIn);
    }

    /// A `gh auth status` that ran past its limit, at startup and then on the poll: the screen stays up with the login
    /// message, and every tick checks again until one succeeds.
    #[test]
    fn a_timed_out_check_keeps_the_screen_and_the_next_tick_rechecks() {
        use futures_util::FutureExt as _;
        use lazy_github_core::config::Config;
        use lazy_github_core::github::{FakeGithub, GithubCall};

        use crate::component::ScreenKind;
        use crate::test_util::TestApp;

        let timed_out = || GhError::Timeout {
            after: Duration::from_secs(20),
        };
        let settle = |app: &mut TestApp| app.settle().now_or_never().expect("the fake answers at once");
        let check_auths = |app: &TestApp| {
            app.effects()
                .iter()
                .filter(|effect| {
                    matches!(
                        effect,
                        Effect::Github {
                            request: GithubRequest::CheckAuth,
                            ..
                        }
                    )
                })
                .count()
        };

        let mut app = TestApp::on_startup(Config::default(), FakeGithub::new().with_auth_error(timed_out()));
        settle(&mut app);
        assert_eq!(app.app().screen(), Some(ScreenKind::Auth));
        let reason = |app: &TestApp| app.app().auth_screen().map(AuthScreen::reason);
        assert_eq!(reason(&app), Some(AuthReason::NotLoggedIn));
        assert!(
            app.screen(100, 30)
                .contains("To proceed, please log into the Github CLI via gh auth login")
        );

        // The poll's check times out too: the screen stays, and the next tick isn't skipped as still in flight.
        app.advance(AUTH_POLL_INTERVAL);
        assert_eq!(check_auths(&app), 2);
        settle(&mut app);
        assert_eq!(app.app().screen(), Some(ScreenKind::Auth));
        assert_eq!(reason(&app), Some(AuthReason::NotLoggedIn));
        app.advance(AUTH_POLL_INTERVAL);
        assert_eq!(check_auths(&app), 3);

        app.gh().set_auth_error(None);
        settle(&mut app);
        assert_eq!(app.app().screen(), Some(ScreenKind::Main));
        assert_eq!(
            app.gh().calls(),
            [
                GithubCall::CheckAuth,
                GithubCall::CheckAuth,
                GithubCall::CheckAuth,
                GithubCall::ListRepositories(lazy_github_core::github::RepoListQuery::all())
            ]
        );
    }

    #[test]
    fn words_wrap_greedily_and_long_words_split() {
        let words =
            |text: &str| -> Vec<(String, Style)> { text.split(' ').map(|w| (w.to_owned(), Style::new())).collect() };
        let text = |lines: Vec<Line<'_>>| lines.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(text(wrap_words(&words("aa bb cc"), 5)), ["aa bb", "cc"]);
        assert_eq!(text(wrap_words(&words("abcdefg h"), 3)), ["abc", "def", "g h"]);
    }

    #[test]
    fn the_space_inside_a_styled_run_takes_its_style() {
        let green = Style::new().fg(ratatui::style::Color::Green);
        let words = [
            ("via".to_owned(), Style::new()),
            ("gh".to_owned(), green),
            ("auth".to_owned(), green),
        ];
        let lines = wrap_words(&words, 80);
        let styles: Vec<Style> = lines[0].spans.iter().map(|s| s.style).collect();
        assert_eq!(styles, [Style::new(), Style::new(), green, green, green]);
    }
}
