//! Textual's `Collapsible`: a section with a title that opens and closes it. The PR Overview's reactions, checks and
//! reviews use it, and the comment reactions, the Conversation's reviews and the diff viewer's files after them.
//!
//! Laid out, a section is a row of `▔` in the screen's background color (its top border), the title row (`▶ title`
//! closed, `▼ title` open, in one cell of padding each side and one cell in from the edge), the contents when open
//! (after a blank row, indented 4 cells), and a blank row, all on `$surface`. The section doesn't own focus: the view
//! that holds it keeps the title (and whatever is inside) in its inner focus ring, and says which has focus when it
//! lays the section out.

use std::ops::Range;
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::time::Instant;

use super::list_view::{append, pad_line};
use super::loading;
use super::markdown::{Rendered, TextRun};
use crate::theme::{Paint, Theme};

/// The symbol before a closed section's title.
const CLOSED: &str = "▶";
/// The symbol before an open section's title.
const OPEN: &str = "▼";
/// The section's own padding on the left (`padding-left: 1`).
const PADDING_LEFT: u16 = 1;
/// How far the contents sit in from the section's edge: its padding and the contents' (`padding: 1 0 0 3`).
pub const CONTENT_INDENT: u16 = 4;

/// Whether a section is open, and since when its loading overlay has shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Collapsible {
    open: bool,
    loading: bool,
    /// When the loading overlay was first drawn, which sets the animation's phase.
    loading_since: Option<Instant>,
}

/// Where focus is, as far as one section's look goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SectionFocus {
    /// The title has focus: it's drawn in the block cursor style.
    pub title: bool,
    /// Focus is on the title or inside the contents (Textual's `:focus-within`): the section is tinted.
    pub within: bool,
}

/// What the contents get to lay themselves out with.
#[derive(Debug, Clone, Copy)]
pub struct ContentCx {
    /// The width they have: the section's, less the indent.
    pub width: u16,
    /// The section's background, which they're drawn over.
    pub under: Paint,
}

/// Where a section landed in the [`Rendered`] it was added to, by row index into its lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// Every row of the section, the top border to the last blank row.
    pub rows: Range<usize>,
    /// The title row.
    pub title_row: usize,
    /// The columns the title takes (its symbol and padding included), from the section's left edge: what a click
    /// toggles.
    pub title_columns: Range<u16>,
    /// The contents' rows: empty while closed.
    pub content: Range<usize>,
}

impl Collapsible {
    /// A section, open or not, that isn't loading.
    pub fn new(open: bool) -> Self {
        Self {
            open,
            ..Self::default()
        }
    }

    /// Whether it's open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes it: `enter` on its title, or a click on it.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Opens or closes it.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Shows or hides the loading overlay, which [`Self::render_loading`] draws over the section while its data
    /// loads.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if !loading {
            self.loading_since = None;
        }
    }

    /// Whether the loading overlay shows.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// When the overlay's next animation frame is due, while it's shown and animates.
    pub fn next_deadline(&self, now: Instant, animations: bool) -> Option<Instant> {
        (self.loading && animations).then_some(now + loading::FRAME_INTERVAL)
    }

    /// Adds the section to `out`, `width` cells wide, titled `title`, and returns where it landed. `content` lays out
    /// the contents when the section is open; their lines are filled out to their width with the section's
    /// background, and their links move with the indent.
    pub fn layout(
        &self,
        out: &mut Rendered,
        title: &[TextRun],
        focus: SectionFocus,
        width: u16,
        theme: &Theme,
        content: impl FnOnce(ContentCx) -> Rendered,
    ) -> Placement {
        let start = out.lines.len();
        let palette = theme.palette;
        let under = theme.background();
        let mut background = palette.surface.over(under);
        if focus.within {
            background = theme.tint(background, palette.foreground, 5.0);
        }
        let section = Style::new().bg(theme.paint(background));
        let row = |spans: Vec<Span<'static>>| {
            let mut line = Line::from(spans);
            pad_line(&mut line, width, section);
            line
        };

        // `border-top: hkey $background`.
        let border = Style::new().fg(theme.paint(under)).bg(theme.paint(background));
        out.lines
            .push(Line::from(Span::styled("▔".repeat(usize::from(width)), border)));

        // `CollapsibleTitle`: blurred, the block cursor's blurred text colors on the section; focused, the block
        // cursor, padding included. Markup colors in the title (a check state) keep theirs.
        let (text, padding) = if focus.title {
            let text = palette.block_cursor_text_style.apply(theme.fg_on(
                palette.block_cursor_foreground,
                palette.block_cursor_background,
                background,
            ));
            (
                text,
                Style::new().bg(theme.color(palette.block_cursor_background, background)),
            )
        } else {
            let text = palette
                .block_cursor_blurred_text_style
                .apply(Style::new().fg(theme.color(palette.block_cursor_blurred_foreground, background)))
                .bg(theme.paint(background));
            (text, section)
        };
        let symbol = if self.open { OPEN } else { CLOSED };
        let mut spans = vec![
            Span::styled(" ".repeat(usize::from(PADDING_LEFT)), section),
            Span::styled(" ", padding),
            Span::styled(format!("{symbol} "), text),
        ];
        spans.extend(
            title
                .iter()
                .map(|run| Span::styled(crate::display::sanitize(&run.text).into_owned(), text.patch(run.style))),
        );
        spans.push(Span::styled(" ", padding));
        let mut title_line = Line::from(spans);
        // The title is cut at the section's width, never wrapped.
        truncate(&mut title_line, width);
        let title_end = u16::try_from(title_line.width()).unwrap_or(u16::MAX);
        pad_line(&mut title_line, width, section);
        let title_row = out.lines.len();
        out.lines.push(title_line);

        let mut content_rows = out.lines.len()..out.lines.len();
        if self.open {
            out.lines.push(row(Vec::new()));
            let inner = width.saturating_sub(CONTENT_INDENT);
            let mut inside = content(ContentCx {
                width: inner,
                under: background,
            });
            let indent = " ".repeat(usize::from(CONTENT_INDENT.min(width)));
            for line in &mut inside.lines {
                pad_line(line, inner, section);
                line.spans.insert(0, Span::styled(indent.clone(), section));
            }
            let first = out.lines.len();
            append(out, inside, CONTENT_INDENT);
            content_rows = first..out.lines.len();
        }
        // `padding-bottom: 1`.
        out.lines.push(row(Vec::new()));

        Placement {
            rows: start..out.lines.len(),
            title_row,
            title_columns: PADDING_LEFT.min(title_end)..title_end,
            content: content_rows,
        }
    }

    /// Draws the loading overlay over `area`, the section's rows on screen, while it's loading. `now` is the frame's
    /// time, which sets the animation's phase.
    pub fn render_loading(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now: Instant, animations: bool) {
        if !self.loading {
            return;
        }
        let since = *self.loading_since.get_or_insert(now);
        let elapsed = now.checked_duration_since(since).unwrap_or(Duration::ZERO);
        loading::render(buf, area, theme, animations, elapsed);
    }
}

/// Cuts `line` to `width` cells, dropping whole spans past it and shortening the one that crosses it.
fn truncate(line: &mut Line<'static>, width: u16) {
    let spans = std::mem::take(&mut line.spans);
    line.spans = crate::display::cut_spans(spans, usize::from(width)).0;
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::test_util::buffer_to_string;

    fn lines(rendered: &Rendered) -> Vec<String> {
        rendered.lines.iter().map(ToString::to_string).collect()
    }

    fn title(text: &str) -> Vec<TextRun> {
        vec![TextRun::new(text, Style::new())]
    }

    fn contents(cx: ContentCx) -> Rendered {
        let mut out = Rendered::default();
        out.lines.push(Line::from("one"));
        out.lines.push(Line::from("two"));
        assert_eq!(cx.width, 16);
        out
    }

    #[test]
    fn closed_a_rule_the_title_and_a_blank_row() {
        let mut out = Rendered::default();
        out.lines.push(Line::from("above"));
        let placement = Collapsible::new(false).layout(
            &mut out,
            &title("Checks"),
            SectionFocus::default(),
            20,
            &Theme::default(),
            |_| panic!("closed sections lay out no contents"),
        );
        assert_eq!(
            lines(&out),
            [
                "above",
                "▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔",
                "  ▶ Checks          ",
                "                    ",
            ]
        );
        assert_eq!(
            placement,
            Placement {
                rows: 1..4,
                title_row: 2,
                title_columns: 1..11,
                content: 3..3,
            }
        );
    }

    #[test]
    fn open_the_contents_indented_between_blank_rows() {
        let mut out = Rendered::default();
        let placement = Collapsible::new(true).layout(
            &mut out,
            &title("Checks"),
            SectionFocus::default(),
            20,
            &Theme::default(),
            contents,
        );
        assert_eq!(
            lines(&out),
            [
                "▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔",
                "  ▼ Checks          ",
                "                    ",
                "    one             ",
                "    two             ",
                "                    ",
            ]
        );
        assert_eq!(placement.content, 3..5);
        assert_eq!(placement.rows, 0..6);
    }

    #[test]
    fn toggling_flips_the_symbol() {
        let mut section = Collapsible::default();
        assert!(!section.is_open());
        section.toggle();
        assert!(section.is_open());
        section.set_open(false);
        assert!(!section.is_open());
    }

    #[test]
    fn a_long_title_is_cut_at_the_width() {
        let mut out = Rendered::default();
        Collapsible::new(false).layout(
            &mut out,
            &title("Status checks: ✓ Passed"),
            SectionFocus::default(),
            12,
            &Theme::default(),
            |_| Rendered::default(),
        );
        assert_eq!(lines(&out)[1], "  ▶ Status c");
    }

    #[test]
    fn colors_follow_textual() {
        let theme = Theme::default();
        let mut out = Rendered::default();
        let red = Style::new().fg(Color::Red);
        let runs = [TextRun::new("Checks: ", Style::new()), TextRun::new("✘ Failed", red)];
        Collapsible::new(false).layout(&mut out, &runs, SectionFocus::default(), 30, &theme, |_| {
            Rendered::default()
        });
        let surface = Color::Rgb(0x1e, 0x1e, 0x1e);
        let rule = &out.lines[0].spans[0].style;
        assert_eq!((rule.fg, rule.bg), (Some(Color::Rgb(0x12, 0x12, 0x12)), Some(surface)));
        let spans = &out.lines[1].spans;
        assert_eq!(spans[2].style.fg, Some(Color::Rgb(0xe0, 0xe0, 0xe0)));
        assert_eq!(spans[2].style.bg, Some(surface));
        // Markup colors keep theirs.
        assert_eq!(spans[4].style.fg, Some(Color::Red));
        assert_eq!(out.lines[2].spans[0].style.bg, Some(surface));

        // Focused, the title takes the block cursor and the section the focus tint.
        let mut out = Rendered::default();
        let focus = SectionFocus {
            title: true,
            within: true,
        };
        Collapsible::new(false).layout(&mut out, &title("Reactions"), focus, 30, &theme, |_| {
            Rendered::default()
        });
        let tinted = Color::Rgb(0x27, 0x27, 0x27);
        assert_eq!(out.lines[0].spans[0].style.bg, Some(tinted));
        let spans = &out.lines[1].spans;
        assert_eq!(spans[0].style.bg, Some(tinted));
        assert_eq!(spans[1].style.bg, Some(Color::Rgb(0x01, 0x78, 0xd4)));
        assert_eq!(spans[2].style.fg, Some(Color::Rgb(0xdd, 0xed, 0xf9)));
        assert!(spans[2].style.add_modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn collapsible_focused_title() {
        use ratatui::widgets::{Paragraph, Widget};

        use crate::components::list_view::ListView;
        use crate::test_util::buffer_to_styled_string;

        let theme = Theme::default();
        let mut out = Rendered::default();
        let focus = SectionFocus {
            title: true,
            within: true,
        };
        let runs = [
            TextRun::new("Status checks: ", Style::new()),
            TextRun::new("✘ Failed", Style::new().fg(theme.accent(crate::theme::Accent::Red))),
        ];
        Collapsible::new(true).layout(&mut out, &runs, focus, 32, &theme, |cx| {
            let items = [
                vec![TextRun::new("✘ Failed test", Style::new())],
                vec![TextRun::new("✓ Passed build", Style::new())],
            ];
            ListView::default()
                .layout(&items, cx.width, &theme, false, cx.under)
                .rendered
        });
        let area = Rect::new(0, 0, 32, u16::try_from(out.lines.len()).unwrap());
        let mut buf = Buffer::empty(area);
        Paragraph::new(out.lines).render(area, &mut buf);
        insta::assert_snapshot!(buffer_to_styled_string(&buf));
    }

    #[test]
    fn the_loading_overlay_covers_the_area_while_loading() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 20, 3);
        let mut section = Collapsible::default();
        let mut buf = Buffer::empty(area);
        section.render_loading(&mut buf, area, &theme, Instant::now(), false);
        assert_eq!(buffer_to_string(&buf), "\n\n\n");
        section.set_loading(true);
        assert!(section.is_loading());
        section.render_loading(&mut buf, area, &theme, Instant::now(), false);
        assert_eq!(buffer_to_string(&buf).lines().nth(1), Some("     Loading..."));
        let now = Instant::now();
        assert_eq!(section.next_deadline(now, true), Some(now + loading::FRAME_INTERVAL));
        assert_eq!(section.next_deadline(now, false), None);
        section.set_loading(false);
        assert_eq!(section.next_deadline(now, true), None);
    }
}
