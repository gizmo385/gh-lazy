//! A tab strip drawn like Textual's `Tabs`: a label row, and under it an underline bar with the active tab's segment
//! highlighted. M2 adds switching tabs and the focused style.

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

    /// The active tab's index.
    pub fn active(&self) -> usize {
        self.active
    }

    /// Draws the label row and the bar into the first two rows of `area`.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let under = theme.background();
        let palette = theme.palette;
        // Textual's `Tab`: `padding: 0 1`, `$foreground 50%`, and `$foreground` when active.
        let active_label = Style::new().fg(theme.color(palette.foreground, under));
        let label = Style::new().fg(theme.color(palette.foreground.multiply_alpha(0.5), under));
        let mut spans = Vec::new();
        let mut highlight = (0, 0);
        let mut x = 0;
        for (i, name) in self.names.iter().enumerate() {
            let width = u16::try_from(Span::raw(name.as_str()).width()).unwrap_or(u16::MAX);
            let style = if i == self.active { active_label } else { label };
            if i == self.active {
                highlight = (x + 1, x + 1 + width);
            }
            spans.extend([Span::raw(" "), Span::styled(name.as_str(), style), Span::raw(" ")]);
            x = x.saturating_add(width + 2);
        }
        let labels = Rect {
            height: area.height.min(1),
            ..area
        };
        frame.render_widget(Paragraph::new(Line::from(spans)), labels);

        if area.height >= TAB_STRIP_HEIGHT {
            let bar = UnderlineBar {
                highlight,
                // Textual's `Underline`: `$block-cursor-background` on a `$foreground 10%` bar.
                highlight_style: Style::new().fg(theme.color(palette.block_cursor_background, under)),
                style: Style::new().fg(theme.color(palette.foreground.multiply_alpha(0.1), under)),
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
        let mut terminal = Terminal::new(TestBackend::new(width, 2)).unwrap();
        terminal
            .draw(|frame| strip.render(frame, frame.area(), &Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
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
