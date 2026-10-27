//! An issue's Overview tab: the state, title, number and author at the left edge, then a rule and the body, scrolling
//! as one. Python's `IssueOverviewTabPane.compose`.

use std::sync::Arc;

use lazy_github_core::models::{Issue, IssueState};
use lazy_github_core::references;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::theme::{Accent, Theme};

use super::super::overview_text::{HeaderStyles, accent, append, push_label, push_rule};

/// The Overview of one issue, and its scroll position.
#[derive(Debug)]
pub(in crate::screens::main) struct IssueOverview {
    scroll: ScrollView,
    body: Body,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    issue: Arc<Issue>,
    /// The body with its references expanded, parsed once. `None` when there's none.
    markdown: Option<Markdown>,
    layouts: LayoutCache,
}

impl IssueOverview {
    /// The Overview of `issue`, scrolled to the top.
    pub(super) fn new(issue: Arc<Issue>) -> Self {
        let markdown = issue
            .body
            .as_deref()
            .filter(|body| !body.is_empty())
            .map(|body| Markdown::parse(&references::expand(body, &issue.repo.full_name)));
        Self {
            scroll: ScrollView::default(),
            body: Body {
                issue,
                markdown,
                layouts: LayoutCache::default(),
            },
        }
    }

    /// The first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.scroll.render(frame, area, theme, &mut self.body);
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let Body {
            issue,
            markdown,
            layouts,
        } = self;
        layouts.get(width, theme, || overview(issue, markdown.as_ref(), width, theme))
    }
}

/// The whole tab laid out `width` cells wide.
fn overview(issue: &Issue, markdown: Option<&Markdown>, width: u16, theme: &Theme) -> Rendered {
    let mut out = Rendered::default();
    if width == 0 {
        return out;
    }
    let HeaderStyles { text, link } = HeaderStyles::new(theme);
    let state = match issue.state {
        IssueState::Open => TextRun::new("Open", accent(theme, Accent::GreenYellow)),
        IssueState::Closed => TextRun::new("Closed", accent(theme, Accent::Orchid)),
    };
    // Py bug B11: Python reads the title as markup and leaves its `[b]` open. The title is drawn as typed, in bold,
    // and only the title.
    let header = [
        state,
        TextRun::new(" ", text),
        TextRun::new(issue.title.as_str(), text.add_modifier(Modifier::BOLD)),
        TextRun::new(" ", text),
        TextRun::link(format!("(#{})", issue.number), link, issue.html_url.as_str()),
        TextRun::new(" by ", text),
        TextRun::link(issue.user.login.as_str(), link, issue.user.html_url.as_str()),
    ];
    push_label(&mut out, &header, width);
    push_rule(&mut out, width, theme);
    if let Some(markdown) = markdown {
        append(&mut out, markdown.layout(width, theme));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::main::issue_details::tests::fixture_issue;

    fn text(rendered: &Rendered) -> Vec<String> {
        rendered
            .lines
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect()
    }

    fn render(issue: Issue, width: u16) -> Rendered {
        let tab = IssueOverview::new(Arc::new(issue));
        overview(&tab.body.issue, tab.body.markdown.as_ref(), width, &Theme::default())
    }

    #[test]
    fn an_open_issue() {
        let theme = Theme::default();
        let rendered = render(fixture_issue(12), 94);
        assert_eq!(
            text(&rendered)[..8],
            [
                "Open Crash when the config file is empty (#12) by parity-friend",
                "",
                &"─".repeat(94),
                "",
                "",
                "",
                "  Steps to reproduce",
                "",
            ]
        );
        let header = &rendered.lines[0].spans;
        assert_eq!(header[0].content, "Open");
        assert_eq!(header[0].style.fg, Some(theme.accent(Accent::GreenYellow)));
        assert!(header[2].style.add_modifier.contains(Modifier::BOLD));
        for span in &header[3..] {
            assert!(!span.style.add_modifier.contains(Modifier::BOLD), "{span:?}");
        }
        assert!(header[4].style.add_modifier.contains(Modifier::UNDERLINED));
        assert!(header[6].style.add_modifier.contains(Modifier::UNDERLINED));

        let links: Vec<(usize, &str)> = rendered.links.iter().map(|link| (link.row, &*link.target)).collect();
        assert_eq!(
            links[..2],
            [
                (0, "https://github.com/parity-bot/issue-fixture/issues/12"),
                (0, "https://github.com/parity-friend")
            ]
        );
        // The body's references are expanded into links.
        assert!(
            links
                .iter()
                .any(|&(_, target)| target == "https://github.com/parity-bot/issue-fixture/issues/7"),
            "{links:?}"
        );
        assert!(
            links
                .iter()
                .any(|&(_, target)| target == "https://github.com/parity-bot/dotfiles/issues/3"),
            "{links:?}"
        );
    }

    #[test]
    fn a_closed_issue_without_a_body() {
        let theme = Theme::default();
        let issue = fixture_issue(11);
        assert_eq!(issue.body, None);
        let rendered = render(issue, 94);
        assert_eq!(
            text(&rendered),
            [
                "Closed Closed without a body (#11) by parity-bot",
                "",
                &"─".repeat(94),
                "",
            ]
        );
        assert_eq!(rendered.lines[0].spans[0].style.fg, Some(theme.accent(Accent::Orchid)));
    }

    #[test]
    fn an_empty_body_shows_nothing_below_the_rule() {
        let mut issue = fixture_issue(12);
        issue.body = Some(String::new());
        assert_eq!(text(&render(issue, 94)).len(), 4);
    }

    #[test]
    fn a_title_with_markup_is_literal_and_only_it_is_bold() {
        let mut issue = fixture_issue(12);
        issue.title = "[b]Bold[/] and [link=x]a link[/link]".into();
        let rendered = render(issue, 94);
        assert_eq!(
            text(&rendered)[0],
            "Open [b]Bold[/] and [link=x]a link[/link] (#12) by parity-friend"
        );
        let header = &rendered.lines[0].spans;
        let title = header
            .iter()
            .find(|span| span.content == "[b]Bold[/] and [link=x]a link[/link]")
            .unwrap();
        assert!(title.style.add_modifier.contains(Modifier::BOLD));
        for span in header.iter().filter(|span| span.content != title.content) {
            assert!(!span.style.add_modifier.contains(Modifier::BOLD), "{span:?}");
        }
        // The title's `[link=x]` isn't a link.
        assert_eq!(rendered.links.iter().filter(|link| link.row == 0).count(), 2);
    }

    #[test]
    fn the_header_wraps_at_the_width() {
        assert_eq!(
            text(&render(fixture_issue(12), 40))[..3],
            ["Open Crash when the config file is empty", "(#12) by parity-friend", ""]
        );
        assert!(render(fixture_issue(12), 0).lines.is_empty());
    }
}
