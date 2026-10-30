//! A hunk pane: Python's `UnifiedDiffPane`, a box holding one hunk's lines with a gutter, a current line and the
//! `Line: n/m` row under them.
//!
//! The code itself, syntax colored, comes from [`code_spans`], once per hunk, the first time the pane is laid out;
//! the pane only adds the gutter, the line backgrounds and the cut at its width.

use lazy_github_core::diff::{ChangedFile, Hunk, LineKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::buffer_line;
use super::highlight::code_spans;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::display;
use crate::theme::{Accent, Paint, Rgb, RichAnsi, Theme};

/// The most rows the pane takes, its border included (`max-height: 25`).
const MAX_HEIGHT: usize = 25;
/// Rows that aren't lines: the two border rows and the `Line: n/m` row.
const CHROME_ROWS: usize = 3;
/// The gutter before the code: `► ` or two spaces, the number in 4 cells, then ` │ `.
const GUTTER_WIDTH: usize = 9;
/// How close the current line gets to the pane's top or bottom before the lines scroll.
const SCROLL_MARGIN: usize = 3;
/// The vertical scrollbar's width, inside the border.
const VERTICAL_BAR: usize = scrollbar::VERTICAL_WIDTH as usize;

/// One hunk's pane: its code, the current line and how far its lines are scrolled.
#[derive(Debug, Clone)]
pub struct HunkPane {
    /// The hunk's code, colored the first time the pane is laid out: highlighting can be slow, and most hunks are
    /// never opened.
    code: Option<Vec<Line<'static>>>,
    /// How many lines the hunk has.
    len: usize,
    /// The current line, an index into the hunk's lines.
    cursor: usize,
    /// The first line shown.
    scroll: usize,
    /// How many lines the last layout showed at once, which the cursor keeps its margin in.
    viewport: usize,
}

/// A pane laid out: its rows, the top margin to the bottom one.
#[derive(Debug, Clone, Default)]
pub struct PaneLayout {
    /// The rows, each as wide as the pane.
    pub lines: Vec<Line<'static>>,
    /// How many of the hunk's lines show at once.
    pub viewport: usize,
}

impl HunkPane {
    /// The pane of `hunk`, its first line current.
    pub fn new(hunk: &Hunk) -> Self {
        let len = hunk.lines.len();
        Self {
            code: None,
            len,
            cursor: 0,
            scroll: 0,
            viewport: len.min(MAX_HEIGHT - CHROME_ROWS),
        }
    }

    /// The current line, from 0.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The first line shown, from 0.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// `diff.cursor_down`: the next line, stopping at the last. Says whether it moved.
    pub fn cursor_down(&mut self) -> bool {
        if self.cursor + 1 >= self.len {
            return false;
        }
        self.cursor += 1;
        self.follow_cursor();
        true
    }

    /// `diff.cursor_up`: the previous line, stopping at the first. Says whether it moved.
    pub fn cursor_up(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.follow_cursor();
        true
    }

    /// Python's `_scroll_to_current_if_needed`: scrolls only when the current line comes within three rows of the
    /// top or the bottom, so it stays three rows in.
    fn follow_cursor(&mut self) {
        let viewport = self.viewport.max(1);
        if self.cursor < self.scroll + SCROLL_MARGIN {
            self.scroll = self.cursor.saturating_sub(SCROLL_MARGIN);
        } else if self.cursor + SCROLL_MARGIN > self.scroll + viewport {
            self.scroll = self.cursor + SCROLL_MARGIN - viewport;
        }
        self.scroll = self.scroll.min(self.len.saturating_sub(viewport));
    }

    /// Remembers how many lines the last layout showed, which the next cursor move scrolls by.
    pub fn set_viewport(&mut self, viewport: usize) {
        self.viewport = viewport;
    }

    /// Lays out the pane of `file`'s `hunk`, the one it was made for, `width` cells wide over `under`, the
    /// background of the section holding it: a blank row, the box, a blank row (`SplitDiffHunk`'s `margin: 1 0`).
    /// Colors the hunk's code the first time.
    pub fn layout(
        &mut self,
        file: &ChangedFile,
        hunk: &Hunk,
        width: u16,
        theme: &Theme,
        focused: bool,
        under: Paint,
    ) -> PaneLayout {
        if self.code.is_none() {
            self.code = Some(code_spans(file, hunk));
        }
        let code = self.code.as_deref().unwrap_or_default();
        let mut out = PaneLayout::default();
        out.lines.push(Line::default());
        let width = usize::from(width);
        if width < 2 {
            out.lines.push(Line::default());
            return out;
        }
        let styles = Styles::new(theme, focused, under);
        let inner = width - 2;
        let len = self.len;
        let widest = code.iter().map(Line::width).max().unwrap_or(0) + GUTTER_WIDTH;

        // Each scrollbar narrows or shortens the room for the other, so settle them together.
        let (mut horizontal, mut vertical) = (false, false);
        for _ in 0..2 {
            let text_width = inner.saturating_sub(if vertical { VERTICAL_BAR } else { 0 });
            horizontal = widest > text_width;
            vertical = len > max_lines(horizontal);
        }
        let viewport = len.min(max_lines(horizontal));
        let text_width = inner.saturating_sub(if vertical { VERTICAL_BAR } else { 0 });
        let scroll = self.scroll.min(len - viewport);
        out.viewport = viewport;

        let border = |left: &str, fill: &str, right: &str| {
            Line::from(Span::styled(
                format!("{left}{}{right}", fill.repeat(inner)),
                styles.border,
            ))
        };
        let boxed = |mut spans: Vec<Span<'static>>| {
            spans.insert(0, Span::styled("│", styles.border));
            spans.push(Span::styled("│", styles.border));
            Line::from(spans)
        };

        out.lines.push(border("┌", "─", "┐"));
        let bar = vertical.then(|| {
            let state = ScrollbarState {
                content: len,
                window: viewport,
                position: scroll,
            };
            bar_rows(Axis::Vertical, VERTICAL_BAR, viewport, state, styles.scrollbar)
        });
        for (row, index) in (scroll..scroll + viewport).enumerate() {
            let mut spans = self.row(hunk, code, index, text_width, &styles, theme);
            if let Some(bar) = &bar {
                spans.extend(bar[row].spans.iter().cloned());
            }
            out.lines.push(boxed(spans));
        }
        if horizontal {
            let state = ScrollbarState {
                content: widest,
                window: text_width,
                position: 0,
            };
            let mut spans = bar_rows(Axis::Horizontal, text_width, 1, state, styles.scrollbar)
                .swap_remove(0)
                .spans;
            if vertical {
                spans.push(Span::styled(" ".repeat(VERTICAL_BAR), styles.corner));
            }
            out.lines.push(boxed(spans));
        }
        // `Line: n/m`, right-aligned with one cell of padding each side.
        let indicator = format!("Line: {}/{} ", self.cursor + 1, len);
        let pad = inner.saturating_sub(indicator.len());
        let row = vec![Span::styled(
            format!("{}{indicator}", " ".repeat(pad)),
            styles.indicator,
        )];
        out.lines.push(boxed(cut(row, inner, styles.indicator)));
        out.lines.push(border("└", "─", "┘"));
        out.lines.push(Line::default());
        out
    }

    /// Line `index` of the hunk: the gutter, then its code on its background, cut and filled to `width`.
    fn row(
        &self,
        hunk: &Hunk,
        code: &[Line<'static>],
        index: usize,
        width: usize,
        styles: &Styles,
        theme: &Theme,
    ) -> Vec<Span<'static>> {
        let line = hunk.lines.get(index);
        let current = index == self.cursor;
        let marker = if current { "► " } else { "  " };
        // Py bug B22: a removed line shows its old-file number, every other line its new-file number.
        let number = line
            .and_then(|line| match line.kind {
                LineKind::Removed => line.old,
                LineKind::Context | LineKind::Added | LineKind::NoNewline => line.new,
            })
            .map_or_else(|| " ".repeat(4), |n| format!("{n:>4}"));
        let kind = line.map_or(LineKind::Context, |line| line.kind);
        let code = code.get(index);
        let background = Styles::background(kind, current, theme);
        // A context line keeps the highlighter's background (monokai's), as Python's do.
        let code_style = styles.code(
            background.or_else(|| code.and_then(|code| code.style.bg).map(|bg| reduce(bg, theme))),
            theme,
        );
        let mut spans = vec![Span::styled(format!("{marker}{number} │ "), styles.gutter)];
        if let Some(code) = code {
            spans.extend(code.spans.iter().map(|span| {
                let mut style = code_style.patch(span.style);
                style.fg = style.fg.map(|fg| reduce(fg, theme));
                // The line's own background wins over any the highlighter gives, as Python's `stylize` does.
                style.bg = background.or_else(|| style.bg.map(|bg| reduce(bg, theme)));
                Span::styled(span.content.clone(), style)
            }));
        }
        cut(spans, width, code_style)
    }
}

/// A highlighter's exact color reduced to what the terminal shows, as the theme's own colors are.
fn reduce(color: Color, theme: &Theme) -> Color {
    match color {
        Color::Rgb(r, g, b) => theme.paint(Rgb::new(r, g, b).into()),
        other => other,
    }
}

/// How many lines fit at once: the pane's height, less its border, the `Line: n/m` row and the horizontal scrollbar.
fn max_lines(horizontal: bool) -> usize {
    MAX_HEIGHT - CHROME_ROWS - usize::from(horizontal)
}

/// `spans` cut to `width` cells, then filled out to it in `fill`.
fn cut(spans: Vec<Span<'static>>, width: usize, fill: Style) -> Vec<Span<'static>> {
    let (mut kept, left) = display::cut_spans(spans, width);
    if left > 0 {
        kept.push(Span::styled(" ".repeat(left), fill));
    }
    kept
}

/// A scrollbar `length` cells along `axis` and `rows` tall, as one line per row.
fn bar_rows(
    axis: Axis,
    length: usize,
    rows: usize,
    state: ScrollbarState,
    colors: ScrollbarColors,
) -> Vec<Line<'static>> {
    let (width, height) = match axis {
        Axis::Vertical => (length, rows),
        Axis::Horizontal => (length, 1),
    };
    // Pane sizes are small screen counts.
    let area = Rect::new(
        0,
        0,
        u16::try_from(width).unwrap_or(u16::MAX),
        u16::try_from(height).unwrap_or(u16::MAX),
    );
    let mut buf = Buffer::empty(area);
    scrollbar::render(&mut buf, area, axis, state, colors);
    (0..area.height).map(|y| buffer_line(&buf, y)).collect()
}

/// The pane's styles for one layout, from `UnifiedDiffPane.DEFAULT_CSS` and `_render_lines`.
struct Styles {
    /// The border: `$primary-lighten-1`, or `$accent` while the pane has focus, on the section's background.
    border: Style,
    /// The gutter: the text color on the `RichLog`'s `$surface`.
    gutter: Style,
    /// The surface the lines are drawn on.
    surface: Paint,
    /// `Line: n/m`: `$text` on `$accent`.
    indicator: Style,
    scrollbar: ScrollbarColors,
    /// Where the two scrollbars meet.
    corner: Style,
}

impl Styles {
    fn new(theme: &Theme, focused: bool, under: Paint) -> Self {
        let palette = theme.palette;
        let border = if focused {
            palette.accent
        } else {
            palette.primary_lighten_1
        };
        let surface = palette.surface.over(under);
        let back = palette.scrollbar_background.over(surface);
        Self {
            border: Style::new().fg(theme.color(border, under)).bg(theme.paint(under)),
            gutter: Style::new()
                .fg(theme.color(palette.foreground, surface))
                .bg(theme.paint(surface)),
            surface,
            indicator: theme.fg_on(palette.text, palette.accent, under),
            scrollbar: ScrollbarColors {
                bar: theme.color(palette.scrollbar, back),
                back: theme.paint(back),
                fg: theme.color(palette.foreground, surface),
                bg: theme.paint(surface),
            },
            corner: Style::new()
                .fg(Color::Reset)
                .bg(theme.color(palette.scrollbar_corner_color, surface)),
        }
    }

    /// A line's own background. Removed and added lines are tinted red and green, brighter on the current line; the
    /// current context line is yellow, and other context lines have none.
    fn background(kind: LineKind, current: bool, theme: &Theme) -> Option<Color> {
        match (kind, current) {
            (LineKind::Removed, false) => Some(theme.accent(Accent::DiffRemoved)),
            (LineKind::Added, false) => Some(theme.accent(Accent::DiffAdded)),
            (LineKind::Removed, true) => Some(theme.accent(Accent::DiffRemovedCurrent)),
            (LineKind::Added, true) => Some(theme.accent(Accent::DiffAddedCurrent)),
            (LineKind::Context | LineKind::NoNewline, true) => Some(theme.rich_ansi(RichAnsi::Yellow)),
            (LineKind::Context | LineKind::NoNewline, false) => None,
        }
    }

    /// A line's code: the text color on `background`, or on the surface without one.
    fn code(&self, background: Option<Color>, theme: &Theme) -> Style {
        let palette = theme.palette;
        Style::new()
            .fg(theme.color(palette.foreground, self.surface))
            .bg(background.unwrap_or_else(|| theme.paint(self.surface)))
    }
}

#[cfg(test)]
mod tests {
    use lazy_github_core::diff;

    use super::*;

    fn hunk(text: &str) -> (ChangedFile, Hunk) {
        let diff = diff::parse(text).unwrap();
        let file = diff.files.into_iter().next().unwrap();
        let hunk = file.hunks[0].clone();
        (file, hunk)
    }

    fn text(layout: &PaneLayout) -> Vec<String> {
        layout.lines.iter().map(ToString::to_string).collect()
    }

    const README: &str = "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -3,3 +3,5 @@ A tiny pager.\n ## Usage\n \n-Run `viewer FILE`.\n+Run `viewer FILE`, then press `j` and `k` to move.\n+\n+The current line is marked with `>`.\n";

    #[test]
    fn the_gutter_numbers_removed_lines_from_the_old_file_and_the_rest_from_the_new() {
        let (file, hunk) = hunk(README);
        let mut pane = HunkPane::new(&hunk);
        let layout = pane.layout(
            &file,
            &hunk,
            70,
            &Theme::default(),
            false,
            Theme::default().background(),
        );
        assert_eq!(
            text(&layout),
            [
                "",
                "┌────────────────────────────────────────────────────────────────────┐",
                "│►    3 │ ## Usage                                                   │",
                "│     4 │                                                            │",
                "│     5 │ Run `viewer FILE`.                                         │",
                "│     5 │ Run `viewer FILE`, then press `j` and `k` to move.         │",
                "│     6 │                                                            │",
                "│     7 │ The current line is marked with `>`.                       │",
                "│                                                          Line: 1/6 │",
                "└────────────────────────────────────────────────────────────────────┘",
                "",
            ]
        );
        assert_eq!(layout.viewport, 6);
    }

    #[test]
    fn a_line_wider_than_the_pane_is_cut_over_a_horizontal_scrollbar() {
        let (file, hunk) = hunk(README);
        let mut pane = HunkPane::new(&hunk);
        let layout = pane.layout(
            &file,
            &hunk,
            40,
            &Theme::default(),
            false,
            Theme::default().background(),
        );
        let text = text(&layout);
        assert_eq!(text[5], "│     5 │ Run `viewer FILE`, then press│");
        assert_eq!(text.len(), 12, "{text:#?}");
        assert_eq!(text[8].chars().count(), 40, "{text:#?}");
        assert_eq!(text[9], "│                            Line: 1/6 │");
    }

    /// The backgrounds of row `row`'s code, after the gutter and up to the border, fill included.
    fn code_backgrounds(layout: &PaneLayout, row: usize) -> Vec<Option<Color>> {
        let spans = &layout.lines[row].spans;
        spans[2..spans.len() - 1].iter().map(|span| span.style.bg).collect()
    }

    #[test]
    fn context_lines_take_monokai_s_background_and_the_others_keep_their_own() {
        let (file, hunk) = hunk(README);
        let theme = Theme::default();
        let monokai = Some(theme.paint(Rgb::new(0x27, 0x28, 0x22).into()));
        let mut pane = HunkPane::new(&hunk);
        pane.cursor_down();
        let layout = pane.layout(&file, &hunk, 70, &theme, false, theme.background());
        // Rows from 2: `## Usage` (context), the blank context line (current), the removed line, three added lines.
        let all = |row: usize, bg: Option<Color>| {
            let backgrounds = code_backgrounds(&layout, row);
            assert!(!backgrounds.is_empty());
            assert!(backgrounds.iter().all(|b| *b == bg), "row {row}: {backgrounds:?}");
        };
        all(2, monokai);
        all(3, Some(theme.rich_ansi(RichAnsi::Yellow)));
        all(4, Some(theme.accent(Accent::DiffRemoved)));
        all(5, Some(theme.accent(Accent::DiffAdded)));
        all(7, Some(theme.accent(Accent::DiffAdded)));
        // The token colors stay on the tinted lines: `viewer FILE` is a Markdown inline code span.
        let code = layout.lines[4]
            .spans
            .iter()
            .find(|span| span.content == "`viewer FILE`")
            .map(|span| span.style.fg);
        assert_eq!(code, Some(Some(theme.paint(Rgb::new(0xE6, 0xDB, 0x74).into()))));

        // The current line on an added line is the brighter green.
        pane.cursor_down();
        pane.cursor_down();
        let layout = pane.layout(&file, &hunk, 70, &theme, false, theme.background());
        let backgrounds = code_backgrounds(&layout, 5);
        assert!(
            backgrounds
                .iter()
                .all(|b| *b == Some(theme.accent(Accent::DiffAddedCurrent))),
            "{backgrounds:?}"
        );
        all(2, monokai);
    }

    #[test]
    fn a_file_with_no_grammar_keeps_the_surface_on_context_lines() {
        let text = "diff --git a/LICENSE b/LICENSE\n--- a/LICENSE\n+++ b/LICENSE\n@@ -1,2 +1,2 @@\n MIT\n-old\n+new\n";
        let (file, hunk) = hunk(text);
        let theme = Theme::default();
        let mut pane = HunkPane::new(&hunk);
        pane.cursor_down();
        let layout = pane.layout(&file, &hunk, 40, &theme, false, theme.background());
        let surface = theme.palette.surface.over(theme.background());
        let backgrounds = code_backgrounds(&layout, 2);
        assert!(
            backgrounds.iter().all(|b| *b == Some(theme.paint(surface))),
            "{backgrounds:?}"
        );
        let foreground = layout.lines[2].spans[2].style.fg;
        assert_eq!(foreground, Some(theme.color(theme.palette.foreground, surface)));
    }

    #[test]
    fn the_cursor_clamps_and_the_lines_scroll_to_keep_it_three_rows_in() {
        let lines: Vec<String> = (1..=40).map(|i| format!("+line {i}\n")).collect();
        let lines = lines.concat();
        let text = format!("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -0,0 +1,40 @@\n{lines}");
        let (file, hunk) = hunk(&text);
        let mut pane = HunkPane::new(&hunk);
        let theme = Theme::default();
        let layout = pane.layout(&file, &hunk, 40, &theme, true, theme.background());
        // At most 25 rows: the borders, 22 lines and `Line: n/m`.
        assert_eq!(layout.lines.len(), 25 + 2);
        assert_eq!(layout.viewport, 22);
        pane.set_viewport(layout.viewport);
        assert!(!pane.cursor_up());
        for _ in 0..18 {
            assert!(pane.cursor_down());
        }
        assert_eq!((pane.cursor(), pane.scroll()), (18, 0));
        assert!(pane.cursor_down());
        assert_eq!((pane.cursor(), pane.scroll()), (19, 0));
        assert!(pane.cursor_down());
        assert_eq!((pane.cursor(), pane.scroll()), (20, 1));
        for _ in 0..30 {
            pane.cursor_down();
        }
        assert_eq!((pane.cursor(), pane.scroll()), (39, 18));
        let layout = pane.layout(&file, &hunk, 40, &theme, true, theme.background());
        assert!(
            layout.lines[23].to_string().contains("40 │ line 40"),
            "{:#?}",
            layout.lines
        );
        assert!(layout.lines[24].to_string().ends_with("Line: 40/40 │"));
        for _ in 0..19 {
            pane.cursor_up();
        }
        assert_eq!((pane.cursor(), pane.scroll()), (20, 17));
    }
}
