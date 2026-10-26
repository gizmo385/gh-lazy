//! The "Introduction" text: Python's `LAZY_GITHUB_INFO` through the markdown renderer, in a vertically scrolling
//! view. The auth screen (#106) embeds it too.

use std::sync::LazyLock;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

// The intro's wrapping tests predate the renderer, which now owns `wrap`.
#[cfg(test)]
use super::markdown::wrap;
use super::markdown::{Markdown, Rendered};
use crate::keymap::BindingId;
use crate::theme::Theme;

/// Textual's vertical scrollbar width.
const SCROLLBAR_WIDTH: u16 = 2;

/// The bindings the view answers while it has focus, in Textual's `ScrollableContainer` order. #75 puts them in the
/// binding chain. The sideways ones do nothing, since the intro wraps, but Python's help panel lists them.
pub const INFO_BINDINGS: &[BindingId] = &[
    BindingId::TextualScrollUp,
    BindingId::TextualScrollDown,
    BindingId::TextualScrollLeft,
    BindingId::TextualScrollRight,
    BindingId::TextualScrollHome,
    BindingId::TextualScrollEnd,
    BindingId::TextualScrollPageUp,
    BindingId::TextualScrollPageDown,
    BindingId::TextualScrollPageLeft,
    BindingId::TextualScrollPageRight,
];

/// The intro's title, with this crate's version where Python has its own.
macro_rules! title {
    () => {
        concat!("LazyGithub: A Terminal UI For GitHub (", env!("CARGO_PKG_VERSION"), ")")
    };
}

#[cfg(test)]
const TITLE: &str = title!();

/// Python's `LAZY_GITHUB_INFO`, line for line.
const LAZY_GITHUB_INFO: &str = concat!(
    "# ",
    title!(),
    "\n",
    "\n",
    "LazyGithub is a terminal UI for interacting with GitHub. You can get started by selecting one of the \
     repositories listed\n",
    "in the table on the left. Once you have selected a repository, any issues, pull requests, and action \
     workflows will be\n",
    "retrieved and displayed below it.\n",
    "\n",
    "\n",
    "**Issues, pull requests, and action lists can also be interacted with!**\n",
    "\n",
    "## Pull Requests\n",
    "\n",
    "You can interact with a PR by viewing its details, viewing the PR diff, and participating in any active \
     conversations\n",
    "happening on the PR. You can also create a new PR for the currently selected repo.\n",
    "\n",
    "## Issues\n",
    "\n",
    "You can interact with an issue by viewing its details, partipcating in any activate conversations \
     happening on the\n",
    "issue, and by editing the issue itself. This means that you can close an issue from within LazyGithub.\n",
    "\n",
    "## Actions\n",
    "\n",
    "Github action and workflow support is pretty limited. It currently only supports listing the workflows and \
     the most\n",
    "recent runs across all actions on the currently selected repo, as well as kicking off a workflow that \
     supports manual\n",
    "triggers.",
);

/// The intro, parsed once.
static INTRO: LazyLock<Markdown> = LazyLock::new(|| Markdown::parse(LAZY_GITHUB_INFO));

/// A scroll command, from the `textual.scroll.*` bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    PageLeft,
    PageRight,
}

impl Scroll {
    /// The command a binding stands for.
    pub fn from_binding(id: BindingId) -> Option<Scroll> {
        Some(match id {
            BindingId::TextualScrollUp => Scroll::Up,
            BindingId::TextualScrollDown => Scroll::Down,
            BindingId::TextualScrollLeft => Scroll::Left,
            BindingId::TextualScrollRight => Scroll::Right,
            BindingId::TextualScrollHome => Scroll::Home,
            BindingId::TextualScrollEnd => Scroll::End,
            BindingId::TextualScrollPageUp => Scroll::PageUp,
            BindingId::TextualScrollPageDown => Scroll::PageDown,
            BindingId::TextualScrollPageLeft => Scroll::PageLeft,
            BindingId::TextualScrollPageRight => Scroll::PageRight,
            _ => return None,
        })
    }
}

/// The intro view and its scroll position.
#[derive(Debug, Default)]
pub struct InfoView {
    offset: usize,
    /// Content and window heights at the last draw, which scrolling is clamped to.
    last: (usize, usize),
    /// The intro's latest layouts by width and theme: a draw that overflows lays it out at two widths.
    layouts: Vec<(u16, Theme, Rendered)>,
}

impl InfoView {
    /// The first visible line.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Scrolls as of the last draw's size.
    pub fn scroll(&mut self, scroll: Scroll) {
        let (content, window) = self.last;
        let max = content.saturating_sub(window);
        self.offset = match scroll {
            Scroll::Up => self.offset.saturating_sub(1),
            Scroll::Down => self.offset + 1,
            Scroll::Home => 0,
            Scroll::End => max,
            Scroll::PageUp => self.offset.saturating_sub(window),
            Scroll::PageDown => self.offset + window,
            // The intro wraps, so there's nothing to scroll sideways.
            Scroll::Left | Scroll::Right | Scroll::PageLeft | Scroll::PageRight => self.offset,
        }
        .min(max);
    }

    /// Draws the view, with a scrollbar in the last two columns when the text overflows.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.draw(frame, area, theme, true);
    }

    /// Draws the view without a scrollbar, cutting off what doesn't fit: Textual's `Markdown` outside a scrolling
    /// container, as on the authentication screen.
    pub fn render_clipped(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.draw(frame, area, theme, false);
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, scrollbar: bool) {
        let window = usize::from(area.height);
        let overflows = scrollbar && self.layout(area.width, theme).lines.len() > window;
        let text_area = if overflows {
            Rect {
                width: area.width.saturating_sub(SCROLLBAR_WIDTH),
                ..area
            }
        } else {
            area
        };
        let offset = self.offset;
        let lines = &self.layout(text_area.width, theme).lines;
        let content = lines.len();
        let offset = offset.min(content.saturating_sub(window));
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
            render_scrollbar(frame.buffer_mut(), bar, self.last.0, self.offset, theme);
        }
    }

    /// The intro laid out `width` cells wide, from the cache when it's there.
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let index = if let Some(index) = self.layouts.iter().position(|(w, t, _)| *w == width && t == theme) {
            index
        } else {
            if self.layouts.len() == 2 {
                self.layouts.remove(0);
            }
            self.layouts.push((width, *theme, INTRO.layout(width, theme)));
            self.layouts.len() - 1
        };
        &self.layouts[index].2
    }
}

/// The intro as lines `width` cells wide, Markdown padding included.
#[cfg(test)]
fn intro_lines(width: u16, theme: &Theme) -> Vec<Line<'static>> {
    INTRO.layout(width, theme).lines
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
    use ratatui::style::Color;

    use super::*;
    use crate::test_util::buffer_to_string;

    fn draw(view: &mut InfoView, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area(), &Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn wraps_greedily_and_splits_long_words() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("abcdefg h", 3), ["abc", "def", "g h"]);
        assert_eq!(wrap("", 3), Vec::<String>::new());
    }

    #[test]
    fn the_layout_matches_textual_at_90_columns() {
        // Python's 160x48 details pane: 94 columns inside the border, no scrollbar.
        let lines = intro_lines(94, &Theme::default());
        let text: Vec<String> = lines.iter().map(|l| l.to_string().trim_end().to_owned()).collect();
        assert_eq!(text.len(), 32);
        assert_eq!(text[..2], ["", ""]);
        assert_eq!(text[2].trim(), TITLE);
        assert_eq!(text[2].find('L'), Some(2 + 23));
        assert_eq!(
            text[4],
            "  LazyGithub is a terminal UI for interacting with GitHub. You can get started by selecting"
        );
        assert_eq!(
            text[9],
            "  Issues, pull requests, and action lists can also be interacted with!"
        );
        assert_eq!(text[10..13], ["", "", "  Pull Requests"]);
        assert_eq!(
            text[28..],
            [
                "  Github action and workflow support is pretty limited. It currently only supports listing",
                "  the workflows and the most recent runs across all actions on the currently selected repo,",
                "  as well as kicking off a workflow that supports manual triggers.",
                ""
            ]
        );
    }

    #[test]
    fn overflowing_text_rewraps_beside_a_scrollbar() {
        // Python's 100x30 details pane: 58 columns and 22 rows below the tabs.
        let mut view = InfoView::default();
        let buffer = draw(&mut view, 58, 22);
        assert_eq!(view.last, (41, 22));
        let screen = buffer_to_string(&buffer);
        let rows: Vec<&str> = screen.lines().collect();
        assert_eq!(rows[4], "  LazyGithub is a terminal UI for interacting with");
        // The thumb covers rows 0-10, keeping the background under it (none here), row 11 is its `▁` tail, and the
        // track follows.
        let bar = Color::Rgb(0x00, 0x30, 0x54);
        let back = Color::Rgb(0, 0, 0);
        for y in 0..11 {
            let cell = &buffer[(56, y)];
            assert_eq!(
                (cell.symbol(), cell.fg, cell.bg, cell.modifier),
                (" ", bar, Color::Reset, Modifier::REVERSED),
                "row {y}"
            );
        }
        let tail = &buffer[(57, 11)];
        assert_eq!(
            (tail.symbol(), tail.fg, tail.bg, tail.modifier),
            ("▁", bar, back, Modifier::REVERSED)
        );
        let track = &buffer[(56, 12)];
        assert_eq!(
            (track.symbol(), track.fg, track.bg),
            (" ", Color::Rgb(0xe0, 0xe0, 0xe0), back)
        );
    }

    #[test]
    fn scrolling_is_clamped_to_the_content() {
        let mut view = InfoView::default();
        draw(&mut view, 58, 22);
        view.scroll(Scroll::Up);
        assert_eq!(view.offset(), 0);
        view.scroll(Scroll::Down);
        assert_eq!(view.offset(), 1);
        view.scroll(Scroll::PageDown);
        assert_eq!(view.offset(), 19);
        view.scroll(Scroll::Home);
        view.scroll(Scroll::End);
        assert_eq!(view.offset(), 19);
        let buffer = draw(&mut view, 58, 22);
        // Scrolled to the end, the last visible row is the trailing margin.
        assert_eq!(
            buffer_to_string(&buffer).lines().nth(20).map(str::trim_end),
            Some("  triggers.")
        );
        view.scroll(Scroll::PageUp);
        assert_eq!(view.offset(), 0);
        assert_eq!(Scroll::from_binding(BindingId::TextualScrollEnd), Some(Scroll::End));
        assert_eq!(Scroll::from_binding(BindingId::AppQuit), None);
    }

    #[test]
    fn clipped_text_wraps_at_the_full_width() {
        let mut view = InfoView::default();
        let mut terminal = Terminal::new(TestBackend::new(98, 10)).unwrap();
        terminal
            .draw(|frame| view.render_clipped(frame, frame.area(), &Theme::default()))
            .unwrap();
        let screen = buffer_to_string(terminal.backend().buffer());
        let rows: Vec<&str> = screen.lines().collect();
        assert_eq!(
            rows[4],
            "  LazyGithub is a terminal UI for interacting with GitHub. You can get started by selecting one"
        );
        // Without a scrollbar the text wraps at the full 94 columns, so "one" stays on this row.
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 2), (5, 40)] {
            draw(&mut InfoView::default(), w, h);
        }
    }
}
