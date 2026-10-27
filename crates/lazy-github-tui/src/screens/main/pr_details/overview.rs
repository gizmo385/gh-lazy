//! A pull request's Overview tab: the state, title and author, the change summary and the dates at the left edge,
//! then a rule and the description, scrolling as one. Python's `PrOverviewTabPane.compose`.

use std::sync::Arc;

use lazy_github_core::models::FullPullRequest;
use lazy_github_core::{references, time};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::theme::{Accent, Theme};

use super::super::overview_text::{HeaderStyles, accent, append, push_label, push_rule};

/// The Overview of one pull request, and its scroll position.
#[derive(Debug)]
pub(in crate::screens::main) struct PrOverview {
    scroll: ScrollView,
    body: Body,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    pr: Arc<FullPullRequest>,
    /// The description with its references expanded, parsed once. `None` when there's none.
    description: Option<Markdown>,
    layouts: LayoutCache,
}

impl PrOverview {
    /// The Overview of `pr`, scrolled to the top.
    pub(super) fn new(pr: Arc<FullPullRequest>) -> Self {
        let issue = &pr.pr.issue;
        let description = issue
            .body
            .as_deref()
            .filter(|body| !body.is_empty())
            .map(|body| Markdown::parse(&references::expand(body, &issue.repo.full_name)));
        Self {
            scroll: ScrollView::default(),
            body: Body {
                pr,
                description,
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
            pr,
            description,
            layouts,
        } = self;
        layouts.get(width, theme, || overview(pr, description.as_ref(), width, theme))
    }
}

/// The whole tab laid out `width` cells wide.
fn overview(pr: &FullPullRequest, description: Option<&Markdown>, width: u16, theme: &Theme) -> Rendered {
    let mut out = Rendered::default();
    if width == 0 {
        return out;
    }
    let HeaderStyles { text, link } = HeaderStyles::new(theme);
    let issue = &pr.pr.issue;

    let state = if pr.merged_at.is_some() {
        TextRun::new("Merged", accent(theme, Accent::Orchid))
    } else if issue.closed_at.is_some() {
        TextRun::new("closed", accent(theme, Accent::Red))
    } else {
        TextRun::new("Open", accent(theme, Accent::GreenYellow))
    };
    // The title as typed: Python passes it as a markup variable, so brackets in it are text.
    let title = [
        state,
        TextRun::new(" ", text),
        TextRun::new(issue.title.as_str(), text.add_modifier(Modifier::BOLD)),
        TextRun::new(" ", text),
        TextRun::link(format!("(#{})", issue.number), link, issue.html_url.as_str()),
        TextRun::new(" by ", text),
        TextRun::link(issue.user.login.as_str(), link, issue.user.html_url.as_str()),
    ];
    push_label(&mut out, &title, width);

    let bold = text.add_modifier(Modifier::BOLD);
    let separator = || TextRun::new(" • ", text);
    let summary = [
        TextRun::new(plural(pr.commits, "commit", "commits"), text),
        separator(),
        TextRun::new(plural(pr.changed_files, "file changed", "files changed"), text),
        separator(),
        TextRun::new(format!("+{}", pr.additions), accent(theme, Accent::GreenYellow)),
        separator(),
        TextRun::new(format!("-{}", pr.deletions), accent(theme, Accent::Red)),
        separator(),
        TextRun::new(format!("{}:{}", pr.head.user.login, pr.head.name), bold),
        TextRun::new(" → ", text),
        TextRun::new(format!("{}:{}", pr.base.user.login, pr.base.name), bold),
    ];
    push_label(&mut out, &summary, width);

    // Python's label starts with a newline, which draws as a blank row.
    let merged = pr
        .merged_at
        .map(|at| format!(" • Merged on {}", time::c_format(at)))
        .unwrap_or_default();
    let dates = format!("\nOpened on {}{merged}", time::c_format(issue.created_at));
    push_label(&mut out, &[TextRun::new(dates, text)], width);

    // The reactions, status checks and reviews sections go here (#232, #234).

    push_rule(&mut out, width, theme);
    if let Some(description) = description {
        append(&mut out, description.layout(width, theme));
    }
    out
}

/// `1 commit`, `3 commits`.
fn plural(count: u64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::main::pr_details::tests::fixture_pr;

    fn text(rendered: &Rendered) -> Vec<String> {
        rendered
            .lines
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect()
    }

    fn lines(pr: &FullPullRequest, width: u16) -> Vec<String> {
        let tab = PrOverview::new(Arc::new(pr.clone()));
        text(&overview(pr, tab.body.description.as_ref(), width, &Theme::default()))
    }

    #[test]
    fn an_open_pull_request() {
        let pr = fixture_pr(5);
        let theme = Theme::default();
        let tab = PrOverview::new(Arc::new(pr.clone()));
        let rendered = overview(&pr, tab.body.description.as_ref(), 94, &theme);
        assert_eq!(
            text(&rendered)[..8],
            [
                "Open Add a cursor to the viewer (#5) by parity-friend",
                "3 commits • 5 files changed • +12 • -6 • parity-friend:cursor → parity-bot:main",
                "",
                "Opened on Sun Feb  1 08:30:00 2026",
                "",
                &"─".repeat(94),
                "",
                "  Adds a cursor to the viewer, as discussed in #3.",
            ]
        );
        let header = &rendered.lines[0].spans;
        assert_eq!(header[0].style.fg, Some(theme.accent(Accent::GreenYellow)));
        assert!(header[2].style.add_modifier.contains(Modifier::BOLD));
        assert!(!header[4].style.add_modifier.contains(Modifier::BOLD));
        let summary = &rendered.lines[1].spans;
        let plus = summary.iter().find(|span| span.content == "+12").unwrap();
        let minus = summary.iter().find(|span| span.content == "-6").unwrap();
        assert_eq!(plus.style.fg, Some(theme.accent(Accent::GreenYellow)));
        assert_eq!(minus.style.fg, Some(theme.accent(Accent::Red)));
        let head = summary
            .iter()
            .find(|span| span.content == "parity-friend:cursor")
            .unwrap();
        assert!(head.style.add_modifier.contains(Modifier::BOLD));

        let links: Vec<(usize, &str)> = rendered.links.iter().map(|link| (link.row, &*link.target)).collect();
        assert_eq!(
            links[..2],
            [
                (0, "https://github.com/parity-bot/pr-details/pull/5"),
                (0, "https://github.com/parity-friend")
            ]
        );
        // The description's references are links.
        assert!(
            links.contains(&(7, "https://github.com/parity-bot/pr-details/issues/3")),
            "{links:?}"
        );
        assert!(links.contains(&(12, "https://github.com/parity-bot")), "{links:?}");
    }

    #[test]
    fn closed_and_merged_states() {
        let theme = Theme::default();
        let closed = fixture_pr(4);
        let rendered = overview(&closed, None, 94, &theme);
        assert_eq!(text(&rendered)[0], "closed Drop the legacy renderer (#4) by parity-bot");
        assert_eq!(rendered.lines[0].spans[0].style.fg, Some(theme.accent(Accent::Red)));
        assert_eq!(
            lines(&closed, 94)[..8],
            [
                "closed Drop the legacy renderer (#4) by parity-bot",
                "1 commit • 1 file changed • +0 • -12 • parity-bot:drop-legacy → parity-bot:main",
                "",
                "Opened on Tue Jan 20 09:00:00 2026",
                "",
                &"─".repeat(94),
                "",
                "  Superseded by the new renderer.",
            ]
        );

        let merged = fixture_pr(3);
        let rendered = overview(&merged, None, 94, &theme);
        assert_eq!(rendered.lines[0].spans[0].style.fg, Some(theme.accent(Accent::Orchid)));
        // A merged pull request is closed too: Merged wins.
        assert!(merged.pr.issue.closed_at.is_some());
        assert_eq!(
            lines(&merged, 94),
            [
                "Merged Release 0.2.0 (#3) by parity-bot",
                "2 commits • 2 files changed • +3 • -2 • parity-bot:release-0.2.0 → parity-bot:main",
                "",
                "Opened on Sat Jan 10 14:00:00 2026 • Merged on Mon Jan 12 16:20:00 2026",
                "",
                &"─".repeat(94),
                "",
            ]
        );
    }

    #[test]
    fn a_title_with_brackets_is_literal() {
        let mut pr = fixture_pr(5);
        pr.pr.issue.title = "[docs] Fix [b]bold[/b] and [/]".into();
        assert_eq!(
            lines(&pr, 94)[0],
            "Open [docs] Fix [b]bold[/b] and [/] (#5) by parity-friend"
        );
    }

    #[test]
    fn an_empty_description_shows_nothing_below_the_rule() {
        let mut pr = fixture_pr(5);
        pr.pr.issue.body = Some(String::new());
        assert_eq!(lines(&pr, 40).last().unwrap(), "");
        assert_eq!(lines(&pr, 40).len(), 9);
    }

    #[test]
    fn the_header_wraps_at_the_width() {
        let pr = fixture_pr(5);
        assert_eq!(
            lines(&pr, 40)[..5],
            [
                "Open Add a cursor to the viewer (#5) by",
                "parity-friend",
                "3 commits • 5 files changed • +12 • -6 •",
                "parity-friend:cursor → parity-bot:main",
                "",
            ]
        );
        assert!(overview(&pr, None, 0, &Theme::default()).lines.is_empty());
    }

    #[test]
    fn plurals() {
        assert_eq!(plural(1, "commit", "commits"), "1 commit");
        assert_eq!(plural(0, "commit", "commits"), "0 commits");
        assert_eq!(plural(2, "file changed", "files changed"), "2 files changed");
    }
}
