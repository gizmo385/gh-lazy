//! A vertically scrolling view over laid-out lines: Textual's `VerticalScroll` around wrapping text. The text is laid
//! out at the view's width; when it's taller than the view, it's laid out again two columns narrower, beside a
//! scrollbar. The intro and the Repo Overview body both draw through it.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::command_log::{WHEEL_ROWS, Wheel};
use super::info::Scroll;
use super::markdown::Rendered;
use crate::theme::Theme;

/// Textual's vertical scrollbar width.
const SCROLLBAR_WIDTH: u16 = 2;

/// Text that lays itself out at a width, in a theme's colors. Implementations cache: a draw that overflows asks for
/// two widths.
pub trait Layout {
    /// The text laid out `width` cells wide.
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered;
}

/// The latest layouts of some text by width and theme: the two a draw beside a scrollbar needs.
#[derive(Debug, Default)]
pub struct LayoutCache {
    layouts: Vec<(u16, Theme, Rendered)>,
}

impl LayoutCache {
    /// The layout for `width` and `theme`, from the cache or else from `layout`.
    pub fn get(&mut self, width: u16, theme: &Theme, layout: impl FnOnce() -> Rendered) -> &Rendered {
        let index = if let Some(index) = self.layouts.iter().position(|(w, t, _)| *w == width && t == theme) {
            index
        } else {
            if self.layouts.len() == 2 {
                self.layouts.remove(0);
            }
            self.layouts.push((width, *theme, layout()));
            self.layouts.len() - 1
        };
        &self.layouts[index].2
    }

    /// Forgets every layout, for when the text changes.
    pub fn clear(&mut self) {
        self.layouts.clear();
    }
}

/// A scroll position, clamped to the content and window sizes of the last draw.
#[derive(Debug, Default)]
pub struct ScrollView {
    offset: usize,
    /// Content and window heights at the last draw, which scrolling is clamped to.
    last: (usize, usize),
}

impl ScrollView {
    /// The first visible line.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The content and window heights at the last draw.
    pub fn last_size(&self) -> (usize, usize) {
        self.last
    }

    fn max_offset(&self) -> usize {
        let (content, window) = self.last;
        content.saturating_sub(window)
    }

    /// Scrolls as of the last draw's size.
    pub fn scroll(&mut self, scroll: Scroll) {
        let window = self.last.1;
        self.offset = match scroll {
            Scroll::Up => self.offset.saturating_sub(1),
            Scroll::Down => self.offset + 1,
            Scroll::Home => 0,
            Scroll::End => self.max_offset(),
            Scroll::PageUp => self.offset.saturating_sub(window),
            Scroll::PageDown => self.offset + window,
            // The text wraps, so there's nothing to scroll sideways.
            Scroll::Left | Scroll::Right | Scroll::PageLeft | Scroll::PageRight => self.offset,
        }
        .min(self.max_offset());
    }

    /// Scrolls as little as possible to show the content lines `rows`, as of the last draw's size: Textual's
    /// `scroll_to_widget` when focus moves to a widget inside the view. When they don't all fit, the first ones show.
    pub fn reveal(&mut self, rows: std::ops::Range<usize>) {
        let window = self.last.1;
        let mut offset = self.offset;
        if rows.end > offset + window {
            offset = rows.end.saturating_sub(window);
        }
        if rows.start < offset {
            offset = rows.start;
        }
        self.offset = offset.min(self.max_offset());
    }

    /// A wheel notch, as many rows as the command log's.
    pub fn wheel(&mut self, wheel: Wheel) {
        self.offset = match wheel {
            Wheel::Up => self.offset.saturating_sub(WHEEL_ROWS),
            Wheel::Down => self.offset + WHEEL_ROWS,
            Wheel::Left | Wheel::Right => self.offset,
        }
        .min(self.max_offset());
    }

    /// Lays `text` out as [`Self::render`] would in `area` and takes its size, without drawing, so a view can
    /// [`Self::reveal`] what changed since the last draw before the next one. Returns the width the text gets: the
    /// area's, less the scrollbar when it overflows.
    pub fn measure(&mut self, area: Rect, theme: &Theme, text: &mut impl Layout) -> u16 {
        let window = usize::from(area.height);
        let width = if text.layout(area.width, theme).lines.len() > window {
            area.width.saturating_sub(SCROLLBAR_WIDTH)
        } else {
            area.width
        };
        self.last = (text.layout(width, theme).lines.len(), window);
        width
    }

    /// Draws `text`, with a scrollbar in the last two columns when it overflows.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, text: &mut impl Layout) {
        self.draw(frame, area, theme, text, true);
    }

    /// Draws `text` without a scrollbar, cutting off what doesn't fit: Textual's `Markdown` outside a scrolling
    /// container, as on the authentication screen.
    pub fn render_clipped(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, text: &mut impl Layout) {
        self.draw(frame, area, theme, text, false);
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, text: &mut impl Layout, scrollbar: bool) {
        let window = usize::from(area.height);
        let overflows = scrollbar && text.layout(area.width, theme).lines.len() > window;
        let text_area = if overflows {
            Rect {
                width: area.width.saturating_sub(SCROLLBAR_WIDTH),
                ..area
            }
        } else {
            area
        };
        let lines = &text.layout(text_area.width, theme).lines;
        let content = lines.len();
        let offset = self.offset.min(content.saturating_sub(window));
        let visible: Vec<Line<'static>> = lines.iter().skip(offset).take(window).cloned().collect();
        self.last = (content, window);
        self.offset = offset;

        frame.render_widget(Paragraph::new(visible), text_area);
        if overflows {
            let bar = Rect {
                x: text_area.right(),
                width: area.width - text_area.width,
                ..area
            };
            render_scrollbar(frame.buffer_mut(), bar, content, offset, theme);
        }
    }
}

/// Textual's `ScrollBarRender.render_bar` for a vertical bar: a thumb in reverse `$scrollbar` over the track's
/// `$scrollbar-background`, with eighth-block glyphs at its ends.
fn render_scrollbar(buf: &mut Buffer, area: Rect, content: usize, offset: usize, theme: &Theme) {
    const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", " "];
    let area = area.intersection(buf.area);
    let size = usize::from(area.height);
    if size == 0 || content <= size {
        return;
    }
    let palette = theme.palette;
    let under = theme.background();
    let bar = theme.color(palette.scrollbar, under);
    let back = theme.color(palette.scrollbar_background, under);
    let track = Style::new().fg(theme.color(palette.foreground, under)).bg(back);
    let thumb = Style::new().fg(bar).add_modifier(Modifier::REVERSED);
    let head = Style::new().fg(bar).bg(back);
    let tail = head.add_modifier(Modifier::REVERSED);

    // The same float arithmetic as Textual, so the thumb lands on the same cells. Sizes are small screen counts.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let (start, end) = {
        let (size_f, content_f) = (size as f64, content as f64);
        let thumb_size = (size_f / (content_f / size_f)).max(1.0);
        let position = (size_f - thumb_size) * (offset as f64 / (content_f - size_f));
        let start = (position * 8.0) as usize;
        (start, start + (thumb_size * 8.0).ceil() as usize)
    };
    let (start_index, start_bar) = (start / 8, start % 8);
    let (end_index, end_bar) = (end / 8, end % 8);

    for dy in 0..size {
        // Textual draws the tail glyph last, so it wins when both ends share a cell.
        let (symbol, style) = if dy == end_index && end_bar != 0 {
            (BARS[7 - end_bar], tail)
        } else if dy == start_index && start_bar != 0 {
            (BARS[7 - start_bar], head)
        } else if (start_index..end_index).contains(&dy) {
            (" ", thumb)
        } else {
            (" ", track)
        };
        // `dy` is below the area's height, a u16.
        let y = area.y + u16::try_from(dy).unwrap_or(u16::MAX);
        for x in area.left()..area.right() {
            buf[(x, y)].set_symbol(symbol).set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    /// `n` numbered lines, whatever the width.
    struct Numbered(Rendered);

    impl Numbered {
        fn new(n: usize) -> Self {
            Numbered(Rendered {
                lines: (0..n).map(|i| Line::from(i.to_string())).collect(),
                links: Vec::new(),
            })
        }
    }

    impl Layout for Numbered {
        fn layout(&mut self, _width: u16, _theme: &Theme) -> &Rendered {
            &self.0
        }
    }

    fn draw(view: &mut ScrollView, text: &mut Numbered, height: u16) {
        let mut terminal = Terminal::new(TestBackend::new(10, height)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area(), &Theme::default(), text))
            .unwrap();
    }

    #[test]
    fn the_wheel_scrolls_two_rows_and_clamps() {
        let mut view = ScrollView::default();
        let mut text = Numbered::new(10);
        draw(&mut view, &mut text, 5);
        view.wheel(Wheel::Down);
        assert_eq!(view.offset(), 2);
        view.wheel(Wheel::Right);
        assert_eq!(view.offset(), 2);
        for _ in 0..5 {
            view.wheel(Wheel::Down);
        }
        assert_eq!(view.offset(), 5);
        view.wheel(Wheel::Up);
        assert_eq!(view.offset(), 3);
    }

    #[test]
    fn a_resize_keeps_the_offset_clamped() {
        let mut view = ScrollView::default();
        let mut text = Numbered::new(10);
        draw(&mut view, &mut text, 5);
        view.scroll(Scroll::End);
        assert_eq!(view.offset(), 5);
        draw(&mut view, &mut text, 8);
        assert_eq!(view.offset(), 2);
        draw(&mut view, &mut text, 20);
        assert_eq!(view.offset(), 0);
    }

    #[test]
    fn measure_takes_the_size_without_drawing() {
        let mut view = ScrollView::default();
        let theme = Theme::default();
        assert_eq!(view.measure(Rect::new(0, 0, 10, 5), &theme, &mut Numbered::new(10)), 8);
        assert_eq!(view.last_size(), (10, 5));
        view.reveal(7..8);
        assert_eq!(view.offset(), 3);
        assert_eq!(
            view.measure(Rect::new(0, 0, 10, 20), &theme, &mut Numbered::new(10)),
            10
        );
    }

    #[test]
    fn the_cache_keeps_two_layouts() {
        let mut cache = LayoutCache::default();
        let theme = Theme::default();
        let mut built = 0;
        for width in [10, 8, 10, 8, 6, 10] {
            cache.get(width, &theme, || {
                built += 1;
                Rendered::default()
            });
        }
        // 10 and 8 once each, then 6 evicts 10, which is built again.
        assert_eq!(built, 4);
        cache.clear();
        cache.get(8, &theme, || {
            built += 1;
            Rendered::default()
        });
        assert_eq!(built, 5);
    }
}
