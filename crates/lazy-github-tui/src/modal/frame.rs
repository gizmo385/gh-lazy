//! What the stack draws around a dialog: the dimmed screen below, and the centered frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::theme::{Palette, Theme, ThemeColor};

/// How far each cell below a modal moves toward `$background`: Textual's `ModalScreen` has
/// `background: $background 60%`.
pub const DIM_FACTOR: f64 = 0.6;

/// The frame's border color, as a dialog's `border: thick $background 80%` writes it.
const BORDER_ALPHA: f64 = 0.8;

/// The border most dialogs have: `thick $background 80%`, for [`crate::modal::Dialog::border`].
pub fn default_border(palette: &Palette) -> ThemeColor {
    palette.background.multiply_alpha(BORDER_ALPHA)
}

/// Dims every cell of `area` in `buf`, foreground and background, as a modal screen's translucent background does to
/// the screens under it. A no-op on the ANSI themes ([`Theme::dim`]).
pub fn dim(buf: &mut Buffer, area: Rect, theme: &Theme) {
    if theme.ansi {
        return;
    }
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.fg = theme.dim(cell.fg, DIM_FACTOR);
            cell.bg = theme.dim(cell.bg, DIM_FACTOR);
        }
    }
}

/// Where a dialog of outer size `(width, height)` goes on a screen of `area`: centered in the area above the
/// bottom row (the footer's), and clipped to that area when it's larger.
pub fn frame_rect(area: Rect, (width, height): (u16, u16)) -> Rect {
    let available = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let width = width.min(available.width);
    let height = height.min(available.height);
    Rect {
        x: available.x + (available.width - width) / 2,
        y: available.y + (available.height - height) / 2,
        width,
        height,
    }
}

/// Where a dialog of outer size `(width, height)` with `margin` cells around it goes on a screen of `area`, as
/// Textual's `ModalScreen { align: center middle }` places it: centered with its margins in the area above the bottom
/// row. When it's taller than that area, the modal screen scrolls, and its vertical scrollbar takes the two rightmost
/// columns, which the centering leaves out; the frame then starts `margin` rows down and runs past the bottom, and
/// isn't clipped (drawing cuts it at the screen's edge).
pub fn frame_rect_with_margin(area: Rect, (width, height): (u16, u16), margin: u16) -> Rect {
    let available = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let outer = (width + 2 * margin, height + 2 * margin);
    let scrollbar = if outer.1 > available.height { 2 } else { 0 };
    let across = available.width.saturating_sub(scrollbar);
    Rect {
        x: available.x + across.saturating_sub(outer.0) / 2 + margin,
        y: available.y + available.height.saturating_sub(outer.1) / 2 + margin,
        width,
        height,
    }
}

/// The part of `rect`, inside a frame that may run past the screen's edge, that's on `screen`. A rect wholly off
/// the screen comes back with no width either, so a row drawn by its width draws nothing.
pub(crate) fn on_screen(rect: Rect, screen: Rect) -> Rect {
    let rect = rect.intersection(screen);
    if rect.is_empty() {
        Rect {
            width: 0,
            height: 0,
            ..rect
        }
    } else {
        rect
    }
}

/// Textual's `thick` border in `border` (usually `$background 80%`), around an interior painted `background`.
pub struct DialogFrame<'a> {
    /// The theme.
    pub theme: &'a Theme,
    /// The dialog's [`crate::modal::Dialog::background`].
    pub background: ThemeColor,
    /// The dialog's [`crate::modal::Dialog::border`].
    pub border: ThemeColor,
}

impl DialogFrame<'_> {
    /// The area inside the border.
    pub fn interior(area: Rect) -> Rect {
        area.inner(ratatui::layout::Margin::new(1, 1))
    }
}

impl Widget for DialogFrame<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let theme = self.theme;
        let palette = theme.palette;
        // A modal screen's own background is `$background` at 60% over the app's `$background`, which is
        // `$background` again: that's what the dialog's translucent border is drawn on.
        let screen = theme.background();
        let background = self.background.over(screen);
        let border = self.border.over(screen);
        let border = Style::new().fg(theme.paint(border)).bg(theme.paint(background));
        let interior = Style::new()
            .fg(theme.color(palette.foreground, background))
            .bg(theme.paint(background));
        let (left, right, top, bottom) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
        for y in top..=bottom {
            for x in left..=right {
                let edge = x == left || x == right;
                let (symbol, style) = match (y == top, y == bottom) {
                    _ if edge => ("█", border),
                    (true, _) => ("▀", border),
                    (_, true) => ("▄", border),
                    _ => (" ", interior),
                };
                buf[(x, y)].reset();
                buf[(x, y)].set_symbol(symbol).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_centered_above_the_footer_row() {
        // Lookup's 60 x 25, captured from Python.
        assert_eq!(
            frame_rect(Rect::new(0, 0, 160, 48), (60, 25)),
            Rect::new(50, 11, 60, 25)
        );
        assert_eq!(frame_rect(Rect::new(0, 0, 100, 30), (60, 25)), Rect::new(20, 2, 60, 25));
    }

    #[test]
    fn a_frame_with_a_margin_taller_than_the_screen_starts_below_it() {
        // New comment's 100 x 50 with `margin: 1`, captured from Python.
        assert_eq!(
            frame_rect_with_margin(Rect::new(0, 0, 160, 48), (100, 50), 1),
            Rect::new(29, 1, 100, 50)
        );
        assert_eq!(
            frame_rect_with_margin(Rect::new(0, 0, 100, 30), (100, 50), 1),
            Rect::new(1, 1, 100, 50)
        );
        // One that fits is centered as without a margin.
        assert_eq!(
            frame_rect_with_margin(Rect::new(0, 0, 160, 80), (100, 50), 1),
            Rect::new(30, 14, 100, 50)
        );
    }

    #[test]
    fn a_frame_larger_than_the_screen_is_clipped() {
        assert_eq!(frame_rect(Rect::new(0, 0, 50, 20), (60, 25)), Rect::new(0, 0, 50, 19));
        assert_eq!(frame_rect(Rect::new(0, 0, 0, 0), (60, 25)), Rect::new(0, 0, 0, 0));
    }

    #[test]
    fn dimming_blends_toward_the_background() {
        use ratatui::style::Color;

        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)]
            .set_fg(Color::Rgb(0xe0, 0xe0, 0xe0))
            .set_bg(Color::Rgb(0x12, 0x12, 0x12));
        let area = buf.area;
        dim(&mut buf, area, &theme);
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(0x64, 0x64, 0x64));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(0x12, 0x12, 0x12));

        let ansi = Theme::by_name("ansi-dark").unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)].set_fg(Color::Rgb(0xe0, 0xe0, 0xe0));
        let area = buf.area;
        dim(&mut buf, area, &ansi);
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(0xe0, 0xe0, 0xe0));
    }

    #[test]
    fn dimming_a_reduced_color_reduces_the_blend() {
        use ratatui::style::Color;

        use crate::theme::{ColorDepth, ColorMode};

        let theme = Theme::default().with_color_mode(ColorMode {
            depth: ColorDepth::EightBit,
            no_color: false,
        });
        // 254 is the gray ramp's #e4e4e4; blended 60% toward #121212 it's #666666, index 241.
        assert_eq!(theme.dim(Color::Indexed(254), DIM_FACTOR), Color::Indexed(241));
        assert_eq!(theme.dim(Color::Reset, DIM_FACTOR), Color::Reset);
    }
}
