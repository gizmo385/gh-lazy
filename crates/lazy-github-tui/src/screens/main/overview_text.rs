//! The layout the Overview tabs share (Repo, pull request and issue): header lines at the left edge, a rule, and a
//! markdown body below, laid out as one [`Rendered`] that scrolls as one.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::component::Cx;
use crate::components::markdown::{self, Markdown, MarkdownSource, Rendered, TextRun};
use crate::effect::{Effect, RequestId};
use crate::theme::{Accent, Theme};

/// An overview's markdown body: parsed, or being parsed off the frame. Nothing shows below the rule until it's
/// ready, as Python's `Markdown` is empty until its first blocks mount.
#[derive(Debug)]
pub(super) enum OverviewMarkdown {
    Ready(Markdown),
    Parsing(RequestId),
}

impl OverviewMarkdown {
    /// Parses `source` inline at or under [`markdown::OFF_FRAME_BYTES`], else asks the runtime to.
    pub(super) fn new(source: MarkdownSource, cx: &mut Cx<'_>) -> Self {
        if source.is_large() {
            OverviewMarkdown::Parsing(cx.parse_markdown(source))
        } else {
            OverviewMarkdown::Ready(source.parse())
        }
    }

    /// The document, once it's parsed.
    pub(super) fn ready(&self) -> Option<&Markdown> {
        match self {
            OverviewMarkdown::Ready(markdown) => Some(markdown),
            OverviewMarkdown::Parsing(_) => None,
        }
    }

    /// The parse still running, if any.
    pub(super) fn pending(&self) -> Option<RequestId> {
        match self {
            OverviewMarkdown::Ready(_) => None,
            OverviewMarkdown::Parsing(id) => Some(*id),
        }
    }

    /// Cancels the parse if it's still running: the view is going away.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending() {
            cx.effect(Effect::Cancel(id));
        }
    }

    /// Takes the parsed document if it's this one's, and hands it back otherwise.
    pub(super) fn on_parsed(&mut self, id: RequestId, markdown: Box<Markdown>) -> Option<Box<Markdown>> {
        if self.pending() == Some(id) {
            *self = OverviewMarkdown::Ready(*markdown);
            None
        } else {
            Some(markdown)
        }
    }
}

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

#[cfg(test)]
mod tests {
    use lazy_github_core::config::Config;

    use super::*;
    use crate::test_util::TestApp;

    fn source(bytes: usize) -> MarkdownSource {
        MarkdownSource {
            text: "x".repeat(bytes),
            references_of: Some("parity-bot/issue-fixture".into()),
        }
    }

    fn parses(app: &TestApp) -> Vec<&Effect> {
        app.effects()
            .iter()
            .filter(|effect| matches!(effect, Effect::ParseMarkdown { .. }))
            .collect()
    }

    #[test]
    fn a_body_of_64_kib_parses_inline() {
        let mut app = TestApp::new(Config::default());
        let markdown = app.with_cx(|cx| OverviewMarkdown::new(source(markdown::OFF_FRAME_BYTES), cx));
        assert_eq!(markdown.ready(), Some(&source(markdown::OFF_FRAME_BYTES).parse()));
        assert_eq!(markdown.pending(), None);
        assert!(parses(&app).is_empty());
    }

    #[test]
    fn a_larger_body_parses_off_the_frame_and_takes_only_its_own_result() {
        let mut app = TestApp::new(Config::default());
        let large = source(markdown::OFF_FRAME_BYTES + 1);
        let mut markdown = app.with_cx(|cx| OverviewMarkdown::new(large.clone(), cx));
        let Some(id) = markdown.pending() else {
            panic!("not parsing: {markdown:?}");
        };
        assert_eq!(markdown.ready(), None);
        assert_eq!(parses(&app), [&Effect::ParseMarkdown { id, source: large }]);

        let other = RequestId::new(u64::MAX);
        let parsed = Box::new(Markdown::parse("parsed"));
        let handed_back = markdown.on_parsed(other, parsed.clone());
        assert_eq!(handed_back, Some(parsed.clone()));
        assert_eq!(markdown.pending(), Some(id));

        assert_eq!(markdown.on_parsed(id, parsed.clone()), None);
        assert_eq!(markdown.ready(), Some(&*parsed));
        // Once it's ready, the same id is someone else's.
        assert_eq!(markdown.on_parsed(id, parsed.clone()), Some(parsed));
    }

    #[test]
    fn closing_cancels_only_a_running_parse() {
        let mut app = TestApp::new(Config::default());
        let large = app.with_cx(|cx| OverviewMarkdown::new(source(markdown::OFF_FRAME_BYTES + 1), cx));
        let small = app.with_cx(|cx| OverviewMarkdown::new(source(1), cx));
        let before = app.effects().len();
        app.with_cx(|cx| {
            small.close(cx);
            large.close(cx);
        });
        let id = large.pending().unwrap();
        assert_eq!(app.effects()[before..], [Effect::Cancel(id)]);
    }
}
