//! A tab strip drawn like Textual's `Tabs`: a label row, and under it an underline bar with the active tab's segment
//! highlighted. Focused, the active label takes the block cursor and the bar brightens. M2 adds switching tabs.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::theme::Theme;

/// Rows the strip takes.
pub const TAB_STRIP_HEIGHT: u16 = 2;

/// Tab names and which one is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabStrip {
    names: Vec<String>,
    active: usize,
}

impl TabStrip {
    /// A strip of `names` with the first active.
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            names: names.into_iter().map(Into::into).collect(),
            active: 0,
        }
    }

    /// The tab names, in order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The active tab's index.
    pub fn active(&self) -> usize {
        self.active
    }

    /// Whether there are no tabs, so the strip can't take focus.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Draws the label row and the bar into the first two rows of `area`, in the focused style when `focused`.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        let under = theme.background();
        let palette = theme.palette;
        // Textual's `Tab`: `padding: 0 1`, `$foreground 50%`, and `$foreground` when active. In a focused `Tabs`,
        // the active tab is `$block-cursor-foreground` on `$block-cursor-background` with its text style, padding
        // included.
        let (active_label, active_padding) = if focused {
            let style = palette.block_cursor_text_style.apply(theme.fg_on(
                palette.block_cursor_foreground,
                palette.block_cursor_background,
                under,
            ));
            let padding = Style::new().bg(theme.color(palette.block_cursor_background, under));
            (style, padding)
        } else {
            (Style::new().fg(theme.color(palette.foreground, under)), Style::new())
        };
        let label = Style::new().fg(theme.color(palette.foreground.multiply_alpha(0.5), under));
        let mut spans = Vec::new();
        let mut highlight = (0, 0);
        let mut x = 0;
        for (i, name) in self.names.iter().enumerate() {
            let width = u16::try_from(Span::raw(name.as_str()).width()).unwrap_or(u16::MAX);
            let (style, padding) = if i == self.active {
                highlight = (x + 1, x + 1 + width);
                (active_label, active_padding)
            } else {
                (label, Style::new())
            };
            spans.extend([
                Span::styled(" ", padding),
                Span::styled(name.as_str(), style),
                Span::styled(" ", padding),
            ]);
            x = x.saturating_add(width + 2);
        }
        let labels = Rect {
            height: area.height.min(1),
            ..area
        };
        frame.render_widget(Paragraph::new(Line::from(spans)), labels);

        if area.height >= TAB_STRIP_HEIGHT {
            // Textual's `Underline`: `$block-cursor-background` on a `$foreground 10%` bar, `30%` when focused.
            let bar_alpha = if focused { 0.3 } else { 0.1 };
            let bar = UnderlineBar {
                highlight,
                highlight_style: Style::new().fg(theme.color(palette.block_cursor_background, under)),
                style: Style::new().fg(theme.color(palette.foreground.multiply_alpha(bar_alpha), under)),
            };
            frame.render_widget(
                bar,
                Rect {
                    y: area.y + 1,
                    height: 1,
                    ..area
                },
            );
        }
    }
}

/// Textual's `Bar` renderable for whole-cell ranges: `━` across the width, with `╸` before and `╺` after the
/// highlighted cells.
struct UnderlineBar {
    /// Highlighted columns, start inclusive and end exclusive, relative to the bar.
    highlight: (u16, u16),
    highlight_style: Style,
    style: Style,
}

impl Widget for UnderlineBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        let (start, end) = (self.highlight.0, self.highlight.1.min(area.width));
        for dx in 0..area.width {
            let (symbol, style) = if (start..end).contains(&dx) {
                ("━", self.highlight_style)
            } else if start > 0 && dx == start - 1 {
                ("╸", self.style)
            } else if dx == end && start < end {
                ("╺", self.style)
            } else {
                ("━", self.style)
            };
            buf[(area.x + dx, area.y)].set_symbol(symbol).set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::test_util::buffer_to_string;

    fn draw(strip: &TabStrip, width: u16) -> Buffer {
        draw_focused(strip, width, false)
    }

    fn draw_focused(strip: &TabStrip, width: u16, focused: bool) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, 2)).unwrap();
        terminal
            .draw(|frame| strip.render(frame, frame.area(), &Theme::default(), focused))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn focused_matches_textual() {
        // Python's focused Details tabs at 160x48: the active tab in the block cursor, the bar at `$foreground 30%`.
        let buffer = draw_focused(&TabStrip::new(["Introduction"]), 20, true);
        assert_eq!(buffer_to_string(&buffer), " Introduction\n╸━━━━━━━━━━━━╺━━━━━━\n");
        let cursor = ratatui::style::Color::Rgb(0x01, 0x78, 0xd4);
        let text = &buffer[(1, 0)];
        assert_eq!(
            (text.fg, text.bg, text.modifier),
            (
                ratatui::style::Color::Rgb(0xdd, 0xed, 0xf9),
                cursor,
                ratatui::style::Modifier::BOLD
            )
        );
        assert_eq!([buffer[(0, 0)].bg, buffer[(13, 0)].bg], [cursor, cursor]);
        assert_eq!(buffer[(14, 0)].bg, ratatui::style::Color::Reset);
        let bar = ratatui::style::Color::Rgb(0x4f, 0x4f, 0x4f);
        assert_eq!(
            [buffer[(0, 1)].fg, buffer[(1, 1)].fg, buffer[(13, 1)].fg],
            [bar, cursor, bar]
        );
    }

    #[test]
    fn one_tab_matches_textual() {
        let buffer = draw(&TabStrip::new(["Introduction"]), 20);
        assert_eq!(buffer_to_string(&buffer), " Introduction\n╸━━━━━━━━━━━━╺━━━━━━\n");
        let fg = |x: u16, y: u16| buffer[(x, y)].fg;
        let (active, muted) = (
            ratatui::style::Color::Rgb(0x01, 0x78, 0xd4),
            ratatui::style::Color::Rgb(0x26, 0x26, 0x26),
        );
        assert_eq!(fg(1, 0), ratatui::style::Color::Rgb(0xe0, 0xe0, 0xe0));
        assert_eq!(
            [fg(0, 1), fg(1, 1), fg(12, 1), fg(13, 1), fg(19, 1)],
            [muted, active, active, muted, muted]
        );
    }

    #[test]
    fn later_tabs_are_muted_and_the_bar_follows_the_active_one() {
        let mut strip = TabStrip::new(["A", "Bee"]);
        strip.active = 1;
        let buffer = draw(&strip, 12);
        assert_eq!(buffer_to_string(&buffer), " A  Bee\n━━━╸━━━╺━━━━\n");
        assert_eq!(buffer[(1, 0)].fg, ratatui::style::Color::Rgb(0x79, 0x79, 0x79));
        assert_eq!(buffer[(4, 1)].fg, ratatui::style::Color::Rgb(0x01, 0x78, 0xd4));
    }

    #[test]
    fn a_bar_narrower_than_the_labels_does_not_overflow() {
        let buffer = draw(&TabStrip::new(["Introduction"]), 5);
        assert_eq!(buffer_to_string(&buffer), " Intr\n╸━━━━\n");
    }
}
