//! The text a reply answers, quoted at the top of a compose sheet behind a dim bar, cut short with a `⋯ N more lines`
//! row once it passes its cap.

use std::cell::RefCell;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::components::markdown::{LinkSpan, Markdown, Rendered};
use crate::display;
use crate::theme::{Paint, Theme};

/// What stands before each quoted row.
const BAR: &str = "▎ ";
/// [`BAR`]'s width in cells.
const BAR_WIDTH: u16 = 2;

/// A quoted markdown body. Its layout is kept for the width, theme and background it was last laid out at, since the
/// sheet measures it each frame.
#[derive(Debug, Default)]
pub struct Quote {
    markdown: Markdown,
    layout: RefCell<Option<(u16, Theme, Paint, Rendered)>>,
}

impl Quote {
    /// The quote of `source`, a markdown body.
    pub fn new(source: &str) -> Self {
        Self {
            markdown: Markdown::parse(source),
            layout: RefCell::new(None),
        }
    }

    /// The rows the quote takes `width` cells wide over `under`, at most `cap`. An empty body takes none.
    pub fn rows(&self, width: u16, cap: u16, theme: &Theme, under: Paint) -> u16 {
        let lines = self.with_layout(width, theme, under, |rendered| rendered.lines.len());
        u16::try_from(lines).unwrap_or(u16::MAX).min(cap)
    }

    /// Draws the quote into `area` (as tall as [`Self::rows`] said), each row behind a dim bar, the last row a dim
    /// `⋯ N more lines` when the rest doesn't fit. Returns the links drawn, in rows and columns from `area`'s top left.
    pub fn render(&self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint) -> Vec<LinkSpan> {
        let dim = Style::new().fg(theme.color(theme.roles().text.muted, under));
        let rows = usize::from(area.height);
        self.with_layout(area.width, theme, under, |rendered| {
            let total = rendered.lines.len();
            // Cut short, the last row says how many weren't shown, itself included.
            let shown = if total > rows { rows.saturating_sub(1) } else { total };
            for (y, line) in (area.top()..area.bottom()).zip(rendered.lines.iter().take(shown)) {
                buf.set_span(area.x, y, &Span::styled(BAR, dim), BAR_WIDTH);
                buf.set_line(area.x + BAR_WIDTH, y, line, area.width.saturating_sub(BAR_WIDTH));
            }
            if shown < total
                && let Some(y) = (area.top()..area.bottom()).nth(shown)
            {
                let more = Line::from(vec![
                    Span::styled(BAR, dim),
                    Span::styled(format!("⋯ {} more lines", total - shown), dim),
                ]);
                buf.set_line(area.x, y, &more, area.width);
            }
            rendered
                .links
                .iter()
                .filter(|span| span.row < shown)
                .map(|span| LinkSpan {
                    row: span.row,
                    columns: span.columns.start + BAR_WIDTH..span.columns.end + BAR_WIDTH,
                    target: span.target.clone(),
                })
                .collect()
        })
    }

    /// Runs `f` over the layout for a quote `width` wide (bar included), laying it out first if that isn't the one
    /// kept. Blank rows at the end (a paragraph's margin) are dropped.
    fn with_layout<T>(&self, width: u16, theme: &Theme, under: Paint, f: impl FnOnce(&Rendered) -> T) -> T {
        let mut layout = self.layout.borrow_mut();
        let stale = !matches!(&*layout, Some((w, t, u, _)) if *w == width && t == theme && *u == under);
        if stale {
            let mut rendered = self
                .markdown
                .layout_unpadded_over(width.saturating_sub(BAR_WIDTH), theme, under);
            trim_blank_tail(&mut rendered);
            *layout = Some((width, *theme, under, rendered));
        }
        match &*layout {
            Some((.., rendered)) => f(rendered),
            // Set just above when it wasn't already.
            None => f(&Rendered::default()),
        }
    }
}

/// Quotes `line`, one line of plain text (the diff line Add comment answers), on `area`'s first row behind the dim bar,
/// cut with `…` at the row's edge. Tabs are drawn as four spaces.
pub fn quote_line(buf: &mut Buffer, area: Rect, line: &str, theme: &Theme, under: Paint) {
    if area.height == 0 {
        return;
    }
    let dim = Style::new().fg(theme.color(theme.roles().text.muted, under));
    let text = Style::new().fg(theme.color(theme.roles().text.plain, under));
    let room = area.width.saturating_sub(BAR_WIDTH);
    let expanded = line.replace('\t', "    ");
    let sanitized = display::sanitize(&expanded);
    let shown = display::truncate(&sanitized, usize::from(room));
    buf.set_span(area.x, area.y, &Span::styled(BAR, dim), BAR_WIDTH.min(area.width));
    buf.set_span(area.x + BAR_WIDTH, area.y, &Span::styled(shown, text), room);
}

/// Drops `rendered`'s blank rows at the end, which a block's bottom margin leaves.
pub(super) fn trim_blank_tail(rendered: &mut Rendered) {
    while rendered
        .lines
        .last()
        .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
    {
        rendered.lines.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    fn draw(quote: &Quote, width: u16, cap: u16) -> (Vec<String>, Vec<LinkSpan>) {
        let theme = Theme::default();
        let under = theme.background();
        let rows = quote.rows(width, cap, &theme, under);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, rows));
        let area = buf.area;
        let links = quote.render(&mut buf, area, &theme, under);
        ((0..rows).map(|y| text(&buf, y)).collect(), links)
    }

    #[test]
    fn a_short_quote_shows_whole_behind_the_bar() {
        let (rows, _) = draw(&Quote::new("No, it stops at the last line."), 60, 7);
        assert_eq!(rows, ["▎ No, it stops at the last line."]);
    }

    #[test]
    fn a_long_quote_is_capped_with_a_more_row() {
        let source = (1..=9).map(|n| format!("Line {n}")).collect::<Vec<_>>().join("\n\n");
        // Nine paragraphs, a blank row between each: 17 rows.
        let (rows, _) = draw(&Quote::new(&source), 60, 7);
        assert_eq!(
            rows,
            ["▎ Line 1", "▎", "▎ Line 2", "▎", "▎ Line 3", "▎", "▎ ⋯ 11 more lines"]
        );
        // As many rows as the cap, nothing is cut.
        let source = (1..=4).map(|n| format!("Line {n}")).collect::<Vec<_>>().join("\n\n");
        let (rows, _) = draw(&Quote::new(&source), 60, 7);
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[6], "▎ Line 4");
    }

    #[test]
    fn an_empty_quote_takes_no_rows() {
        let theme = Theme::default();
        assert_eq!(Quote::new("").rows(60, 7, &theme, theme.background()), 0);
    }

    #[test]
    fn links_move_past_the_bar_and_cut_ones_go() {
        let source = "Read [the guide](https://example.com/guide)\n\nmid\n\nend [gone](https://example.com/gone)";
        let (rows, links) = draw(&Quote::new(source), 60, 3);
        assert_eq!(rows, ["▎ Read the guide", "▎", "▎ ⋯ 3 more lines"]);
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].row, links[0].columns.clone()), (0, 7..16));
        assert_eq!(&*links[0].target, "https://example.com/guide");
    }

    #[test]
    fn a_quoted_line_is_cut_with_an_ellipsis_at_the_edge() {
        let theme = Theme::default();
        let under = theme.background();
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        let area = buf.area;
        quote_line(&mut buf, area, "+let cursor = 0;", &theme, under);
        assert_eq!(text(&buf, 0), "▎ +let curs…");
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        let area = buf.area;
        quote_line(&mut buf, area, "-\tgone", &theme, under);
        assert_eq!(text(&buf, 0), "▎ -    gone");
    }
}
