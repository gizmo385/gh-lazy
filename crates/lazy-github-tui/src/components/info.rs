//! The "Introduction" text: Python's `LAZY_GITHUB_INFO` through the markdown renderer, in a vertically scrolling
//! view. The auth screen (#106) embeds it too.

use std::sync::LazyLock;

use ratatui::Frame;
use ratatui::layout::Rect;
#[cfg(test)]
use ratatui::text::Line;

// The intro's wrapping tests predate the renderer, which now owns `wrap`.
use super::command_log::Wheel;
#[cfg(test)]
use super::markdown::wrap;
use super::markdown::{Markdown, Rendered};
use super::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::keymap::BindingId;
use crate::theme::Theme;

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
    scroll: ScrollView,
    intro: Intro,
}

/// The intro's layouts.
#[derive(Debug, Default)]
struct Intro(LayoutCache);

impl Layout for Intro {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        self.0.get(width, theme, || INTRO.layout(width, theme))
    }
}

impl InfoView {
    /// The first visible line.
    pub fn offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Scrolls as of the last draw's size.
    pub fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    /// A wheel notch over the view.
    pub fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    /// Draws the view, with a scrollbar in the last two columns when the text overflows.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.scroll.render(frame, area, theme, &mut self.intro);
    }

    /// Draws the view without a scrollbar, cutting off what doesn't fit: Textual's `Markdown` outside a scrolling
    /// container, as on the authentication screen.
    pub fn render_clipped(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.scroll.render_clipped(frame, area, theme, &mut self.intro);
    }
}

/// The intro as lines `width` cells wide, Markdown padding included.
#[cfg(test)]
fn intro_lines(width: u16, theme: &Theme) -> Vec<Line<'static>> {
    INTRO.layout(width, theme).lines
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

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
        assert_eq!(view.scroll.last_size(), (41, 22));
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
