//! The "Terminal too small" screen, drawn instead of everything else while the terminal is below [`MIN_SIZE`]
//! (`decisions/terminal-too-small.md`). Python has no minimum: Textual clips its layout at any size.

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Widget};

use crate::theme::Theme;

/// The smallest terminal the app lays itself out in (`decisions/terminal-too-small.md`). Under 15 rows, the main
/// screen with the default sections and a repo selected no longer draws the Pull Requests section at all, so `2`
/// focuses something that isn't there. The width is the proposed 40: narrower, nothing vanishes, but the section
/// titles and tables shrink to a few letters.
pub const MIN_SIZE: Size = Size::new(40, 15);

const LABEL: &str = "Terminal too small";

/// Whether a terminal of `size` is below [`MIN_SIZE`] in either direction.
pub fn is_too_small(size: Size) -> bool {
    size.width < MIN_SIZE.width || size.height < MIN_SIZE.height
}

/// The whole screen: the theme's background and `Terminal too small (<w>×<h>)` in the middle, the size on a line of
/// its own when the terminal is too narrow for both.
pub struct TooSmall<'a> {
    /// The theme, for the screen's colors.
    pub theme: &'a Theme,
}

impl Widget for TooSmall<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let screen = theme.background();
        let style = Style::new()
            .fg(theme.color(theme.palette.foreground, screen))
            .bg(theme.paint(screen));
        Block::new().style(style).render(area, buf);
        if area.is_empty() {
            return;
        }
        let size = format!("({}×{})", area.width, area.height);
        let one_line = format!("{LABEL} {size}");
        // Too narrow for one line, it breaks before the size rather than losing it off the edge.
        let lines = if one_line.chars().count() <= usize::from(area.width) {
            vec![one_line]
        } else {
            vec![LABEL.to_owned(), size]
        };
        let count = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let top = area.y + area.height.saturating_sub(count) / 2;
        for (row, text) in (top..area.bottom()).zip(lines) {
            Line::from(text).centered().render(
                Rect {
                    y: row,
                    height: 1,
                    ..area
                },
                buf,
            );
        }
    }
}
