//! Textual's scrollbars: `ScrollBarRender.render_bar`, vertical and horizontal, with the same float arithmetic so the
//! thumb lands on the same cells.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// Textual's vertical scrollbar width (`scrollbar-size-vertical: 2`).
pub const VERTICAL_WIDTH: u16 = 2;
/// Textual's horizontal scrollbar height (`scrollbar-size-horizontal: 1`).
pub const HORIZONTAL_HEIGHT: u16 = 1;

/// Which way a scrollbar runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Top to bottom, at a widget's right edge.
    Vertical,
    /// Left to right, along a widget's bottom edge.
    Horizontal,
}

impl Axis {
    /// The eighth-block glyphs for the thumb's ends, indexed as Textual indexes them.
    fn bars(self) -> [&'static str; 8] {
        match self {
            Axis::Vertical => ["▁", "▂", "▃", "▄", "▅", "▆", "▇", " "],
            Axis::Horizontal => ["▉", "▊", "▋", "▌", "▍", "▎", "▏", " "],
        }
    }
}

/// The terminal colors a scrollbar is drawn with, already composited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarColors {
    /// The thumb: `$scrollbar` over the track.
    pub bar: Color,
    /// The track: `$scrollbar-background`.
    pub back: Color,
    /// The widget's text color, which the track's blank cells inherit.
    pub fg: Color,
    /// The widget's background, which the thumb's reversed blank cells inherit.
    pub bg: Color,
}

/// What a scrollbar shows: the content's size, the window onto it, and where the window is, all in cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarState {
    /// The content's length along the axis.
    pub content: usize,
    /// The visible length.
    pub window: usize,
    /// The scroll offset.
    pub position: usize,
}

/// The thumb's extent in eighths of a cell along a bar `size` cells long, or `None` when there's no thumb.
fn thumb(size: usize, state: ScrollbarState) -> Option<(usize, usize)> {
    let ScrollbarState {
        content,
        window,
        position,
    } = state;
    if window == 0 || size == 0 || content == 0 || size == content {
        return None;
    }
    // Screen sizes are small counts, so the float conversions are exact, and the results are non-negative.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    {
        let (size, content, window, position) = (size as f64, content as f64, window as f64, position as f64);
        let thumb_size = (window / (content / size)).max(1.0);
        let scrollable = content - window;
        let ratio = if scrollable > 0.0 { position / scrollable } else { 0.0 };
        let position = (size - thumb_size) * ratio;
        let start = (position * 8.0).max(0.0) as usize;
        Some((start, start + (thumb_size * 8.0).ceil() as usize))
    }
}

/// Draws a scrollbar along `axis` filling `area`.
pub fn render(buf: &mut Buffer, area: Rect, axis: Axis, state: ScrollbarState, colors: ScrollbarColors) {
    let area = area.intersection(buf.area);
    let size = usize::from(match axis {
        Axis::Vertical => area.height,
        Axis::Horizontal => area.width,
    });
    let track = Style::new().fg(colors.fg).bg(colors.back);
    let thumb_style = Style::new()
        .fg(colors.bar)
        .bg(colors.bg)
        .add_modifier(Modifier::REVERSED);
    let plain = Style::new().fg(colors.bar).bg(colors.back);
    let reversed = plain.add_modifier(Modifier::REVERSED);
    // Textual reverses the head glyph of a horizontal bar and the tail glyph of a vertical one.
    let (head, tail) = match axis {
        Axis::Vertical => (plain, reversed),
        Axis::Horizontal => (reversed, plain),
    };
    let bars = axis.bars();
    let cells = thumb(size, state).map(|(start, end)| (start / 8, start % 8, end / 8, end % 8));

    for i in 0..size {
        let (symbol, style) = match cells {
            // Textual draws the tail glyph last, so it wins when both ends share a cell.
            Some((_, _, end, end_bar)) if i == end && end_bar != 0 => (bars[7 - end_bar], tail),
            Some((start, start_bar, ..)) if i == start && start_bar != 0 => (bars[7 - start_bar], head),
            Some((start, _, end, _)) if (start..end).contains(&i) => (" ", thumb_style),
            _ => (" ", track),
        };
        // `i` is below the area's width or height, a u16.
        let i = u16::try_from(i).unwrap_or(u16::MAX);
        match axis {
            Axis::Vertical => {
                for x in area.left()..area.right() {
                    buf[(x, area.y + i)].set_symbol(symbol).set_style(style);
                }
            }
            Axis::Horizontal => {
                for y in area.top()..area.bottom() {
                    buf[(area.x + i, y)].set_symbol(symbol).set_style(style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(content: usize, window: usize, position: usize) -> ScrollbarState {
        ScrollbarState {
            content,
            window,
            position,
        }
    }

    const COLORS: ScrollbarColors = ScrollbarColors {
        bar: Color::Rgb(0, 0x30, 0x54),
        back: Color::Rgb(0, 0, 0),
        fg: Color::Rgb(0xe0, 0xe0, 0xe0),
        bg: Color::Rgb(0x27, 0x27, 0x27),
    };

    fn draw(axis: Axis, size: u16, state: ScrollbarState) -> Buffer {
        let area = match axis {
            Axis::Vertical => Rect::new(0, 0, VERTICAL_WIDTH, size),
            Axis::Horizontal => Rect::new(0, 0, size, HORIZONTAL_HEIGHT),
        };
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, axis, state, COLORS);
        buf
    }

    fn symbols(buf: &Buffer, axis: Axis) -> String {
        let area = buf.area;
        let n = match axis {
            Axis::Vertical => area.height,
            Axis::Horizontal => area.width,
        };
        (0..n)
            .map(|i| match axis {
                Axis::Vertical => buf[(0, i)].symbol().to_owned(),
                Axis::Horizontal => buf[(i, 0)].symbol().to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_python_repos_table_at_the_bottom() {
        // Python's 160x48 repos table scrolled to the end: 51 lines of content, 14 visible, a 14-row bar. The
        // thumb's head `▇` is on row 10 and the thumb fills the rest.
        let buf = draw(Axis::Vertical, 14, state(51, 14, 37));
        assert_eq!(symbols(&buf, Axis::Vertical), "          ▇   ");
        let head = &buf[(1, 10)];
        assert_eq!(
            (head.fg, head.bg, head.modifier),
            (COLORS.bar, COLORS.back, Modifier::empty())
        );
        let thumb = &buf[(0, 13)];
        assert_eq!(
            (thumb.fg, thumb.bg, thumb.modifier),
            (COLORS.bar, COLORS.bg, Modifier::REVERSED)
        );
        let track = &buf[(0, 0)];
        assert_eq!((track.fg, track.bg), (COLORS.fg, COLORS.back));
    }

    #[test]
    fn the_python_repos_table_horizontal_bar() {
        // 86 columns of content in a 60-column viewport at offset 0: the thumb covers 0-40 and ends in `▉`.
        let buf = draw(Axis::Horizontal, 60, state(86, 60, 0));
        let expected = format!("{}▉{}", " ".repeat(41), " ".repeat(18));
        assert_eq!(symbols(&buf, Axis::Horizontal), expected);
        let tail = &buf[(41, 0)];
        assert_eq!(
            (tail.fg, tail.bg, tail.modifier),
            (COLORS.bar, COLORS.back, Modifier::empty())
        );
        assert_eq!(buf[(40, 0)].modifier, Modifier::REVERSED);
        assert_eq!(buf[(42, 0)].bg, COLORS.back);
    }

    #[test]
    fn content_that_fits_draws_only_the_track() {
        assert_eq!(thumb(10, state(10, 10, 0)), None);
        assert_eq!(thumb(0, state(10, 5, 0)), None);
        let buf = draw(Axis::Vertical, 4, state(4, 4, 0));
        assert_eq!(symbols(&buf, Axis::Vertical), "    ");
    }

    #[test]
    fn a_thumb_never_overflows_the_bar() {
        for position in 0..=40 {
            let buf = draw(Axis::Vertical, 3, state(50, 10, position));
            assert_eq!(buf.area.height, 3);
        }
    }
}
