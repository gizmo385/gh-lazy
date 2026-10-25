//! The "Introduction" text, laid out the way Textual's `Markdown` shows Python's `LAZY_GITHUB_INFO`, in a vertically
//! scrolling view. The M2 markdown renderer may replace the internals; the auth screen (#106) embeds it too.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::keymap::BindingId;
use crate::theme::{Theme, ThemeColor};

/// Textual's `Markdown`: `padding: 0 2`.
const PADDING: u16 = 2;
/// Textual's vertical scrollbar width.
const SCROLLBAR_WIDTH: u16 = 2;

/// The bindings the view answers while it has focus, in Textual's `ScrollableContainer` order. #75 puts them in the
/// binding chain.
pub const INFO_BINDINGS: &[BindingId] = &[
    BindingId::TextualScrollUp,
    BindingId::TextualScrollDown,
    BindingId::TextualScrollHome,
    BindingId::TextualScrollEnd,
    BindingId::TextualScrollPageUp,
    BindingId::TextualScrollPageDown,
];

/// One Markdown block of the intro.
#[derive(Debug, Clone, Copy)]
enum Block {
    H1,
    H2(&'static str),
    Paragraph(&'static str),
    Strong(&'static str),
}

impl Block {
    /// Textual's margins (top, bottom): headers `2 0 1 0`, paragraphs `0 0 1 0`. Adjacent margins collapse.
    fn margins(self) -> (usize, usize) {
        match self {
            Block::H1 | Block::H2(_) => (2, 1),
            Block::Paragraph(_) | Block::Strong(_) => (0, 1),
        }
    }
}

const TITLE: &str = concat!("LazyGithub: A Terminal UI For GitHub (", env!("CARGO_PKG_VERSION"), ")");

/// Python's `LAZY_GITHUB_INFO`, with soft line breaks joined.
const INTRO: &[Block] = &[
    Block::H1,
    Block::Paragraph(
        "LazyGithub is a terminal UI for interacting with GitHub. You can get started by selecting one of the \
         repositories listed in the table on the left. Once you have selected a repository, any issues, pull \
         requests, and action workflows will be retrieved and displayed below it.",
    ),
    Block::Strong("Issues, pull requests, and action lists can also be interacted with!"),
    Block::H2("Pull Requests"),
    Block::Paragraph(
        "You can interact with a PR by viewing its details, viewing the PR diff, and participating in any active \
         conversations happening on the PR. You can also create a new PR for the currently selected repo.",
    ),
    Block::H2("Issues"),
    Block::Paragraph(
        "You can interact with an issue by viewing its details, partipcating in any activate conversations \
         happening on the issue, and by editing the issue itself. This means that you can close an issue from \
         within LazyGithub.",
    ),
    Block::H2("Actions"),
    Block::Paragraph(
        "Github action and workflow support is pretty limited. It currently only supports listing the workflows \
         and the most recent runs across all actions on the currently selected repo, as well as kicking off a \
         workflow that supports manual triggers.",
    ),
];

/// A scroll command, from the `textual.scroll.*` bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl Scroll {
    /// The command a binding stands for.
    pub fn from_binding(id: BindingId) -> Option<Scroll> {
        Some(match id {
            BindingId::TextualScrollUp => Scroll::Up,
            BindingId::TextualScrollDown => Scroll::Down,
            BindingId::TextualScrollHome => Scroll::Home,
            BindingId::TextualScrollEnd => Scroll::End,
            BindingId::TextualScrollPageUp => Scroll::PageUp,
            BindingId::TextualScrollPageDown => Scroll::PageDown,
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
        }
        .min(max);
    }

    /// Draws the view, with a scrollbar in the last two columns when the text overflows.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let window = usize::from(area.height);
        let mut lines = intro_lines(area.width, theme);
        let overflows = lines.len() > window;
        let text_area = if overflows {
            lines = intro_lines(area.width.saturating_sub(SCROLLBAR_WIDTH), theme);
            Rect {
                width: area.width.saturating_sub(SCROLLBAR_WIDTH),
                ..area
            }
        } else {
            area
        };
        self.last = (lines.len(), window);
        self.offset = self.offset.min(lines.len().saturating_sub(window));

        let visible: Vec<Line<'static>> = lines.into_iter().skip(self.offset).take(window).collect();
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
}

/// The intro as lines `width` cells wide, Markdown padding included.
fn intro_lines(width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let palette = theme.palette;
    let under = theme.background();
    let fg = |c: ThemeColor| Style::new().fg(theme.color(c, under));
    let text_width = usize::from(width.saturating_sub(2 * PADDING)).max(1);
    let indent = " ".repeat(usize::from(PADDING));

    let mut lines = Vec::new();
    let mut pending_margin = 0;
    for block in INTRO {
        let (top, bottom) = block.margins();
        lines.extend(std::iter::repeat_n(Line::default(), pending_margin.max(top)));
        pending_margin = bottom;
        let (text, style) = match *block {
            Block::H1 => {
                let style = palette.markdown_h1_text_style.apply(fg(palette.markdown_h1_color));
                // `content-align: center`: the whole row is the header, padding cells included.
                for row in wrap(TITLE, text_width) {
                    let left = (text_width.saturating_sub(row.len())) / 2;
                    let right = text_width.saturating_sub(left + row.len());
                    let cells = format!("{}{row}{}", " ".repeat(left), " ".repeat(right));
                    lines.push(Line::from(vec![Span::raw(indent.clone()), Span::styled(cells, style)]));
                }
                continue;
            }
            Block::H2(text) => (
                text,
                palette.markdown_h2_text_style.apply(fg(palette.markdown_h2_color)),
            ),
            Block::Paragraph(text) => (text, fg(palette.foreground)),
            Block::Strong(text) => (text, fg(palette.foreground).add_modifier(Modifier::BOLD)),
        };
        for row in wrap(text, text_width) {
            lines.push(Line::from(vec![Span::raw(indent.clone()), Span::styled(row, style)]));
        }
    }
    lines.extend(std::iter::repeat_n(Line::default(), pending_margin));
    lines
}

/// Greedy word wrap, as Rich does it; a word longer than `width` is split.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split_whitespace() {
        let mut word = word;
        loop {
            let needed = if row.is_empty() {
                word.len()
            } else {
                row.len() + 1 + word.len()
            };
            if needed <= width {
                if !row.is_empty() {
                    row.push(' ');
                }
                row.push_str(word);
                break;
            }
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                continue;
            }
            // The text is ASCII, so byte offsets are char boundaries.
            let (head, tail) = word.split_at(width);
            rows.push(head.to_owned());
            word = tail;
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
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
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 2), (5, 40)] {
            draw(&mut InfoView::default(), w, h);
        }
    }
}
