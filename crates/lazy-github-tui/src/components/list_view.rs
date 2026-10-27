//! Textual's `ListView` of one-label items, as the PR Overview's sections hold them: each item wraps at the list's
//! width on `$surface`, and `up` / `down` move a highlighted item. The list doesn't own focus or its items: the view
//! that holds it passes them in, and keeps it in its inner focus ring.

use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::markdown::{self, Rendered, TextRun};
use crate::theme::{Paint, Theme};

/// The highlighted item of a list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListView {
    /// Textual's `index`: none until a key or a click picks one, as items added after the list mounts leave it unset.
    highlight: Option<usize>,
}

/// A list laid out: its lines, and the rows each item took.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListLayout {
    /// The lines, each as wide as the list.
    pub rendered: Rendered,
    /// The rows of each item, in order, into [`Self::rendered`].
    pub items: Vec<Range<usize>>,
}

impl ListView {
    /// The highlighted item, if any.
    pub fn highlight(&self) -> Option<usize> {
        self.highlight
    }

    /// Highlights `index`, or nothing.
    pub fn set_highlight(&mut self, index: Option<usize>) {
        self.highlight = index;
    }

    /// `down` over `len` items: the first when none is highlighted, then the next, stopping at the last.
    pub fn cursor_down(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.highlight = Some(self.highlight.map_or(0, |i| (i + 1).min(len - 1)));
    }

    /// `up` over `len` items: the last when none is highlighted, then the previous, stopping at the first.
    pub fn cursor_up(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.highlight = Some(self.highlight.map_or(len - 1, |i| i.saturating_sub(1).min(len - 1)));
    }

    /// Lays out `items` `width` cells wide over `under`, the list `focused` or not. Each item fills its rows with its
    /// background: `$surface`, tinted `$foreground 5%` while the list has focus, and the block cursor's colors on the
    /// highlighted item.
    pub fn layout(&self, items: &[Vec<TextRun>], width: u16, theme: &Theme, focused: bool, under: Paint) -> ListLayout {
        let mut out = ListLayout::default();
        if width == 0 {
            return out;
        }
        let palette = theme.palette;
        let mut background = palette.surface.over(under);
        if focused {
            background = theme.tint(background, palette.foreground, 5.0);
        }
        let plain = Style::new()
            .fg(theme.color(palette.foreground, background))
            .bg(theme.paint(background));
        let highlighted = if focused {
            palette.block_cursor_text_style.apply(theme.fg_on(
                palette.block_cursor_foreground,
                palette.block_cursor_background,
                background,
            ))
        } else {
            palette.block_cursor_blurred_text_style.apply(theme.fg_on(
                palette.block_cursor_blurred_foreground,
                palette.block_cursor_blurred_background,
                background,
            ))
        };
        for (index, runs) in items.iter().enumerate() {
            let base = if self.highlight == Some(index) {
                highlighted
            } else {
                plain
            };
            let runs: Vec<TextRun> = runs
                .iter()
                .map(|run| TextRun {
                    style: base.patch(run.style),
                    ..run.clone()
                })
                .collect();
            let mut item = markdown::wrap_text(&runs, width);
            if item.lines.is_empty() {
                item.lines.push(Line::default());
            }
            for line in &mut item.lines {
                pad_line(line, width, base);
            }
            let start = out.rendered.lines.len();
            append(&mut out.rendered, item, 0);
            out.items.push(start..out.rendered.lines.len());
        }
        out
    }
}

/// Fills `line` out to `width` cells with spaces in `style`.
pub(crate) fn pad_line(line: &mut Line<'static>, width: u16, style: Style) {
    let used = line.width();
    let width = usize::from(width);
    if used < width {
        line.spans.push(Span::styled(" ".repeat(width - used), style));
    }
}

/// Adds `more` below what's in `out`, `indent` cells right, moving its links with it.
pub(crate) fn append(out: &mut Rendered, more: Rendered, indent: u16) {
    let shift = out.lines.len();
    out.links.extend(more.links.into_iter().map(|mut span| {
        span.row += shift;
        span.columns = span.columns.start + indent..span.columns.end + indent;
        span
    }));
    out.lines.extend(more.lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(texts: &[&str]) -> Vec<Vec<TextRun>> {
        texts
            .iter()
            .map(|text| vec![TextRun::new(*text, Style::new())])
            .collect()
    }

    fn text(layout: &ListLayout) -> Vec<String> {
        layout.rendered.lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn up_and_down_start_at_either_end_and_stop_there() {
        let mut list = ListView::default();
        list.cursor_down(3);
        assert_eq!(list.highlight(), Some(0));
        list.cursor_down(3);
        list.cursor_down(3);
        list.cursor_down(3);
        assert_eq!(list.highlight(), Some(2));
        list.cursor_up(3);
        assert_eq!(list.highlight(), Some(1));

        let mut list = ListView::default();
        list.cursor_up(3);
        assert_eq!(list.highlight(), Some(2));
        list.cursor_up(3);
        list.cursor_up(3);
        list.cursor_up(3);
        assert_eq!(list.highlight(), Some(0));

        let mut empty = ListView::default();
        empty.cursor_down(0);
        empty.cursor_up(0);
        assert_eq!(empty.highlight(), None);
    }

    #[test]
    fn items_wrap_and_fill_the_width() {
        let theme = Theme::default();
        let layout = ListView::default().layout(&items(&["one two three", "x"]), 8, &theme, false, theme.background());
        assert_eq!(text(&layout), ["one two ", "three   ", "x       "]);
        assert_eq!(layout.items, [0..2, 2..3]);
    }

    #[test]
    fn the_highlight_takes_the_block_cursor_when_focused() {
        let theme = Theme::default();
        let palette = theme.palette;
        let mut list = ListView::default();
        list.set_highlight(Some(1));
        let under = theme.background();
        let surface = palette.surface.over(under);

        let blurred = list.layout(&items(&["a", "b"]), 4, &theme, false, under);
        assert_eq!(blurred.rendered.lines[0].spans[0].style.bg, Some(theme.paint(surface)));
        let cursor = palette.block_cursor_blurred_background.over(surface);
        assert_eq!(blurred.rendered.lines[1].spans[0].style.bg, Some(theme.paint(cursor)));

        let focused = list.layout(&items(&["a", "b"]), 4, &theme, true, under);
        let tinted = theme.tint(surface, palette.foreground, 5.0);
        assert_eq!(focused.rendered.lines[0].spans[1].style.bg, Some(theme.paint(tinted)));
        let cursor = palette.block_cursor_background.over(tinted);
        assert_eq!(focused.rendered.lines[1].spans[0].style.bg, Some(theme.paint(cursor)));
    }

    #[test]
    fn links_move_with_the_indent() {
        let mut out = Rendered::default();
        out.lines.push(Line::default());
        let runs = [TextRun::link("go", Style::new(), "https://example.com")];
        append(&mut out, markdown::wrap_text(&runs, 10), 4);
        assert_eq!(out.links[0].row, 1);
        assert_eq!(out.links[0].columns, 4..6);
    }
}
