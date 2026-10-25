//! The `╱` hatch Textual fills a screen with around a maximized widget that doesn't fill it.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::theme::Theme;

/// Textual's `Screen.-maximized` hatch: `hatch: right $panel` on `$background`.
#[derive(Debug, Clone, Copy)]
pub struct Hatch {
    style: Style,
}

impl Hatch {
    /// The hatch in `theme`'s colors.
    pub fn new(theme: &Theme) -> Self {
        let under = theme.background();
        Self {
            style: Style::new()
                .fg(theme.color(theme.palette.panel, under))
                .bg(theme.paint(under)),
        }
    }
}

impl Widget for Hatch {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].reset();
                buf[(x, y)].set_symbol("╱").set_style(self.style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn fills_with_panel_slashes_on_the_background() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        Hatch::new(&Theme::default()).render(buf.area, &mut buf);
        for cell in &buf.content {
            assert_eq!(
                (cell.symbol(), cell.fg, cell.bg),
                ("╱", Color::Rgb(0x24, 0x2f, 0x38), Color::Rgb(0x12, 0x12, 0x12))
            );
        }
    }
}
