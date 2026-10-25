//! The toast stack: Textual's `ToastRack`, drawn over everything in the bottom-right corner.
//!
//! `App` owns one [`Toasts`], outside the screens, so a toast outlives the screen or modal that raised it. The rack
//! keeps its (hidden) scrollbar's two columns on the right, so each toast is `min(60, (W - 2) / 2)` columns wide and
//! ends two columns short of the right edge: a severity bar (`▌`), then a `$panel-lighten-1` box with one cell of
//! padding all round holding the bold title and the word-wrapped message. The newest toast sits at the bottom, its
//! last row just above the footer, and each toast has one transparent row above it. When the stack is taller than
//! the screen, the oldest toasts are clipped off the top.

use std::collections::VecDeque;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use tokio::time::Instant;

use crate::theme::{Paint, Rgb, Theme};
use crate::toast::{Severity, Toast};

/// Textual's `Toast { width: 60 }`.
const MAX_WIDTH: u16 = 60;
/// Columns that aren't content: the bar and one cell of padding each side.
const CHROME_WIDTH: u16 = 3;
/// Rows that aren't content: one row of padding above and below.
const CHROME_HEIGHT: u16 = 2;
/// The rack's `overflow-y: scroll` gutter: Textual's default `scrollbar-size-vertical`, never drawn.
const GUTTER: u16 = 2;
/// Textual's `outer` left border edge.
const BAR: &str = "▌";

/// The live toasts, oldest first.
#[derive(Debug, Default)]
pub struct Toasts {
    live: VecDeque<Live>,
    /// The area of the last draw, which clicks are hit-tested against.
    last_area: Option<Rect>,
}

#[derive(Debug)]
struct Live {
    toast: Toast,
    /// `None` until [`Toasts::stamp`] starts its clock.
    expires: Option<Instant>,
}

/// Where one toast goes. `top` may be above the screen, for a toast clipped by the ones below it.
#[derive(Debug, PartialEq, Eq)]
struct Placement {
    /// Index into `live`.
    index: usize,
    x: u16,
    top: i32,
    width: u16,
    /// Title and message rows, styled.
    lines: Vec<Line<'static>>,
}

impl Placement {
    fn height(&self) -> i32 {
        // A toast has at most a few dozen lines.
        i32::try_from(self.lines.len()).unwrap_or(i32::MAX) + i32::from(CHROME_HEIGHT)
    }

    /// The rows of `area` this toast covers.
    fn visible(&self, area: Rect) -> Rect {
        let top = self.top.max(i32::from(area.top()));
        let bottom = (self.top + self.height()).min(i32::from(area.bottom()));
        // Both are clamped into `area`'s rows, so they fit in a u16.
        let y = u16::try_from(top).unwrap_or(area.top());
        let height = u16::try_from(bottom - top).unwrap_or(0);
        Rect::new(self.x, y, self.width, height)
    }
}

impl Toasts {
    /// Raises `toast`. Its timeout starts at the next [`Self::stamp`], which `App` calls at the end of the update that
    /// raised it, so the clock reads the same as it would have here. Crate-private, since a toast that is never
    /// stamped never expires; [`Self::push_at`] is the public way in.
    pub(crate) fn push(&mut self, toast: Toast) {
        self.live.push_back(Live { toast, expires: None });
    }

    /// Raises `toast` at `now`.
    pub fn push_at(&mut self, toast: Toast, now: Instant) {
        self.push(toast);
        self.stamp(now);
    }

    /// Starts the timeout of every toast raised since the last call, at `now`.
    pub fn stamp(&mut self, now: Instant) {
        for live in self.live.iter_mut().filter(|live| live.expires.is_none()) {
            live.expires = Some(now + live.toast.timeout);
        }
    }

    /// Removes the toasts whose timeout has passed. True if any was removed, so the screen changed.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.live.len();
        self.live.retain(|live| live.expires.is_none_or(|at| at > now));
        self.live.len() != before
    }

    /// When the next toast expires.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.live.iter().filter_map(|live| live.expires).min()
    }

    /// The live toasts, oldest first.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Toast> {
        self.live.iter().map(|live| &live.toast)
    }

    /// How many toasts are up.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Whether no toast is up.
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// A left button press on a toast, as of the last draw, dismisses it and is consumed: true. Anything else, or a
    /// press anywhere else, is left for the app: false.
    pub fn handle_mouse(&mut self, mouse: &MouseEvent) -> bool {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return false;
        }
        match self.last_area {
            Some(area) => self.dismiss_at(mouse.column, mouse.row, area),
            None => false,
        }
    }

    /// Dismisses the toast drawn at `(col, row)` when the stack fills `area`. The transparent row above each toast,
    /// and the clipped-off part of one, aren't part of it. True if a toast was dismissed.
    pub fn dismiss_at(&mut self, col: u16, row: u16, area: Rect) -> bool {
        // The placements don't depend on the theme, only on the text.
        let hit = self.layout(area, &Theme::default()).into_iter().find(|placement| {
            let visible = placement.visible(area);
            visible.contains(ratatui::layout::Position::new(col, row))
        });
        match hit {
            Some(placement) => {
                self.live.remove(placement.index);
                true
            }
            None => false,
        }
    }

    /// Draws the stack over whatever is in `area`, which is the whole screen. Call it last.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.last_area = Some(area);
        let screen = theme.background();
        let palette = theme.palette;
        let panel = palette.panel_lighten_1.over(screen);
        let box_style = Style::new()
            .fg(theme.color(palette.foreground, panel))
            .bg(theme.paint(panel));
        for placement in self.layout(area, theme) {
            let visible = placement.visible(area);
            let bar = match self.live[placement.index].toast.severity {
                Severity::Information => palette.success,
                Severity::Warning => palette.warning,
                Severity::Error => palette.error,
            };
            // The half block's other half is the toast's own background, as in Python's captures.
            let bar_style = Style::new().fg(theme.color(bar, screen)).bg(theme.paint(panel));
            frame.render_widget(Clear, visible);
            let buf = frame.buffer_mut();
            buf.set_style(visible, box_style);
            let content_width = placement.width.saturating_sub(CHROME_WIDTH);
            for y in visible.top()..visible.bottom() {
                buf.set_string(visible.x, y, BAR, bar_style);
                // Row 0 is padding; rows 1..=lines are content.
                let row = i32::from(y) - placement.top;
                if let Some(line) = usize::try_from(row - 1).ok().and_then(|i| placement.lines.get(i)) {
                    buf.set_line(visible.x + 2, y, line, content_width);
                }
            }
        }
    }

    /// Where each toast goes, newest (bottom) first, leaving out the ones entirely above `area`.
    fn layout(&self, area: Rect, theme: &Theme) -> Vec<Placement> {
        let width = toast_width(area.width);
        if width <= CHROME_WIDTH || area.height < 2 {
            return Vec::new();
        }
        let x = area.right() - GUTTER - width;
        // The bottom row (the footer's) stays clear.
        let mut bottom = i32::from(area.bottom()) - 1;
        let mut placements = Vec::new();
        for (index, live) in self.live.iter().enumerate().rev() {
            if bottom <= i32::from(area.top()) {
                break;
            }
            let lines = toast_lines(&live.toast, width - CHROME_WIDTH, theme);
            let mut placement = Placement {
                index,
                x,
                top: 0,
                width,
                lines,
            };
            placement.top = bottom - placement.height();
            // The transparent margin row above this toast.
            bottom = placement.top - 1;
            placements.push(placement);
        }
        placements
    }
}

/// A toast's width on a screen `screen_width` columns wide: Textual's `width: 60; max-width: 50%`, where 50% is of
/// the rack inside its scrollbar gutter.
pub fn toast_width(screen_width: u16) -> u16 {
    MAX_WIDTH.min(screen_width.saturating_sub(GUTTER) / 2)
}

/// A toast's height when it's `width` columns wide: the padding rows, the title's lines and the message's lines.
pub fn toast_height(toast: &Toast, width: u16) -> u16 {
    let lines = toast_lines(toast, width.saturating_sub(CHROME_WIDTH), &Theme::default()).len();
    CHROME_HEIGHT.saturating_add(u16::try_from(lines).unwrap_or(u16::MAX))
}

/// The title's and the message's lines, wrapped to `content_width` and styled as Textual's `Toast` CSS.
fn toast_lines(toast: &Toast, content_width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let palette = theme.palette;
    let panel = palette.panel_lighten_1.over(theme.background());
    let base = Style::new()
        .fg(theme.color(palette.foreground, panel))
        .bg(theme.paint(panel));
    let width = usize::from(content_width.max(1));
    let mut lines = Vec::new();
    if let Some(title) = &toast.title {
        // `.toast--title { text-style: bold; color: $text-success | $text-warning | $text-error }`.
        let color = match toast.severity {
            Severity::Information => palette.text_success,
            Severity::Warning => palette.text_warning,
            Severity::Error => palette.text_error,
        };
        let style = base.fg(theme.color(color, panel)).bold();
        let cells: Vec<(char, Style)> = title.chars().map(|c| (c, style)).collect();
        lines.extend(wrap(&cells, width));
    }
    let cells: Vec<(char, Style)> = toast
        .message
        .spans
        .iter()
        .flat_map(|span| {
            let style = span_style(base, toast.message.style.patch(span.style), theme);
            span.content.chars().map(move |c| (c, style))
        })
        .collect();
    lines.extend(wrap(&cells, width));
    lines
}

/// A message span's style on the toast: `$foreground` on the box, with the span's modifiers, and its accent color
/// reduced to the terminal's color mode.
fn span_style(base: Style, span: Style, theme: &Theme) -> Style {
    let mut style = base.add_modifier(span.add_modifier).remove_modifier(span.sub_modifier);
    match span.fg {
        Some(Color::Rgb(r, g, b)) => style = style.fg(theme.paint(Paint::from(Rgb::new(r, g, b)))),
        Some(other) => style = style.fg(other),
        None => {}
    }
    style
}

/// The width of `c` in cells.
fn char_width(c: char) -> usize {
    let mut utf8 = [0; 4];
    Span::raw(&*c.encode_utf8(&mut utf8)).width()
}

/// Wraps styled characters to `width` cells as Textual's `Content.wrap` (and Rich's `Text.wrap`) does: each `\n` starts a
/// line, lines break before the word that doesn't fit, a word wider than the line is chopped, and trailing spaces
/// are dropped.
fn wrap(cells: &[(char, Style)], width: usize) -> Vec<Line<'static>> {
    cells
        .split(|&(c, _)| c == '\n')
        .flat_map(|paragraph| {
            let breaks = break_positions(paragraph, width);
            let starts = std::iter::once(0).chain(breaks.iter().copied());
            let ends = breaks.iter().copied().chain(std::iter::once(paragraph.len()));
            starts
                .zip(ends)
                .map(|(start, end)| to_line(&paragraph[start..end]))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Rich's `divide_line`: the indices where new lines start.
fn break_positions(text: &[(char, Style)], width: usize) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut offset = 0;
    for (start, end) in words(text) {
        let word = &text[start..end];
        let word_width: usize = trim_end(word).iter().map(|&(c, _)| char_width(c)).sum();
        let full_width: usize = word.iter().map(|&(c, _)| char_width(c)).sum();
        if width.saturating_sub(offset) >= word_width {
            offset += full_width;
        } else if word_width > width {
            // Chop the word into lines of `width` cells; the last piece starts the next line's offset.
            let mut piece_start = start;
            let mut piece_width = 0;
            for (i, &(c, _)) in word.iter().enumerate() {
                let w = char_width(c);
                if piece_width + w > width && piece_width > 0 {
                    if piece_start > 0 {
                        breaks.push(piece_start);
                    }
                    piece_start = start + i;
                    piece_width = 0;
                }
                piece_width += w;
            }
            if piece_start > 0 {
                breaks.push(piece_start);
            }
            offset = piece_width;
        } else if offset > 0 && start > 0 {
            breaks.push(start);
            offset = full_width;
        }
    }
    breaks.dedup();
    breaks
}

/// Rich's `words`: the spans of `\s*\S+\s*`.
fn words(text: &[(char, Style)]) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let start = i;
        while i < text.len() && text[i].0.is_whitespace() {
            i += 1;
        }
        if i == text.len() {
            break;
        }
        while i < text.len() && !text[i].0.is_whitespace() {
            i += 1;
        }
        while i < text.len() && text[i].0.is_whitespace() {
            i += 1;
        }
        words.push((start, i));
    }
    words
}

fn trim_end(cells: &[(char, Style)]) -> &[(char, Style)] {
    let end = cells
        .iter()
        .rposition(|&(c, _)| !c.is_whitespace())
        .map_or(0, |i| i + 1);
    &cells[..end]
}

/// One wrapped line, with runs of equal style merged into spans.
fn to_line(cells: &[(char, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for &(c, style) in trim_end(cells) {
        match spans.last_mut() {
            Some(span) if span.style == style => span.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), style)),
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lazy_github_core::config::Config;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Modifier, Stylize as _};

    use super::*;
    use crate::clock::Clock as _;
    use crate::test_util::{TestApp, buffer_to_string};
    use crate::theme::Accent;
    use crate::toast::DEFAULT_TIMEOUT;

    const SECOND: Duration = Duration::from_secs(1);

    fn theme(name: &str) -> Theme {
        Theme::by_name(name).unwrap()
    }

    fn texts(toasts: &Toasts) -> Vec<String> {
        toasts.iter().map(|toast| toast.message.to_string()).collect()
    }

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    fn wrap_plain(text: &str, width: usize) -> Vec<String> {
        let cells: Vec<_> = text.chars().map(|c| (c, Style::new())).collect();
        plain(&wrap(&cells, width))
    }

    /// Python's maximize toast, with its `[b]escape[/b]`.
    fn maximize_toast() -> Toast {
        let message = Line::from(vec![
            Span::raw("Current view maximized. Press "),
            Span::raw("escape").bold(),
            Span::raw(" to return."),
        ]);
        Toast::info(message).with_title("View Maximized")
    }

    /// The toasts drawn at `cols` x `rows` over a screen of `╳`, so the transparent rows show.
    fn draw(toasts: &mut Toasts, theme: &Theme, cols: u16, rows: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let screen = theme.background();
                let style = Style::new()
                    .fg(theme.color(theme.palette.foreground, screen))
                    .bg(theme.paint(screen));
                for y in area.top()..area.bottom() {
                    frame
                        .buffer_mut()
                        .set_string(0, y, "╳".repeat(usize::from(cols)), style);
                }
                toasts.render(frame, area, theme);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn stack(toasts: impl IntoIterator<Item = Toast>) -> Toasts {
        let now = Instant::now();
        let mut stack = Toasts::default();
        for toast in toasts {
            stack.push_at(toast, now);
        }
        stack
    }

    #[test]
    fn the_default_timeout_is_five_seconds() {
        assert_eq!(Toast::info("x").timeout, Duration::from_secs(5));
        let now = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push_at(Toast::warning("x"), now);
        assert_eq!(toasts.next_deadline(), Some(now + DEFAULT_TIMEOUT));
        assert!(!toasts.expire(now + 5 * SECOND - Duration::from_millis(1)));
        assert_eq!(toasts.len(), 1);
        assert!(toasts.expire(now + 5 * SECOND));
        assert!(toasts.is_empty());
        assert_eq!(toasts.next_deadline(), None);
    }

    #[test]
    fn a_custom_timeout() {
        let now = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push_at(Toast::info("Updating issue #1...").with_timeout(SECOND), now);
        assert_eq!(toasts.next_deadline(), Some(now + SECOND));
        assert!(toasts.expire(now + SECOND));
        assert!(toasts.is_empty());
    }

    #[test]
    fn toasts_expire_independently() {
        let start = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push_at(Toast::info("first"), start);
        toasts.push_at(Toast::info("second"), start + 2 * SECOND);
        toasts.push_at(Toast::info("short").with_timeout(SECOND), start + 2 * SECOND);
        assert_eq!(toasts.next_deadline(), Some(start + 3 * SECOND));
        assert!(toasts.expire(start + 3 * SECOND));
        assert_eq!(texts(&toasts), ["first", "second"]);
        assert_eq!(toasts.next_deadline(), Some(start + 5 * SECOND));
        assert!(toasts.expire(start + 5 * SECOND));
        assert_eq!(texts(&toasts), ["second"]);
        assert!(!toasts.expire(start + 6 * SECOND));
        assert!(toasts.expire(start + 7 * SECOND));
        assert!(toasts.is_empty());
    }

    #[test]
    fn a_pushed_toast_starts_its_clock_when_stamped() {
        let start = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push(Toast::info("x"));
        assert_eq!(toasts.next_deadline(), None);
        assert!(!toasts.expire(start + 60 * SECOND));
        toasts.stamp(start + 60 * SECOND);
        assert_eq!(toasts.next_deadline(), Some(start + 65 * SECOND));
        // A later stamp doesn't restart it.
        toasts.stamp(start + 61 * SECOND);
        assert_eq!(toasts.next_deadline(), Some(start + 65 * SECOND));
    }

    #[test]
    fn identical_toasts_are_not_merged() {
        let toasts = stack([Toast::info("same"), Toast::info("same")]);
        assert_eq!(texts(&toasts), ["same", "same"]);
    }

    #[test]
    fn the_newest_is_at_the_bottom() {
        let toasts = stack([Toast::info("old"), Toast::info("mid"), Toast::info("new")]);
        let area = Rect::new(0, 0, 100, 30);
        let layout = toasts.layout(area, &Theme::default());
        // Newest first; one-line toasts are 3 rows with a blank row above.
        let tops: Vec<_> = layout.iter().map(|p| (p.index, p.top)).collect();
        assert_eq!(tops, [(2, 26), (1, 22), (0, 18)]);
        let screen = buffer_to_string(&draw(&mut stack(toasts.iter().cloned()), &Theme::default(), 100, 30));
        let rows: Vec<&str> = screen.lines().collect();
        // Columns 49..=97, then the two gutter columns.
        assert_eq!(
            rows[19].chars().take(54).collect::<String>(),
            format!("{}▌ old", "╳".repeat(49)),
            "{:?}",
            rows[19]
        );
        assert!(rows[19].ends_with(" ╳╳"));
        assert!(rows[23].contains("▌ mid"));
        assert!(rows[27].contains("▌ new"));
        assert!(rows[29].ends_with('╳'), "the footer row stays clear");
    }

    #[test]
    fn dismiss_at_hits_toasts_but_not_the_gaps() {
        let area = Rect::new(0, 0, 100, 30);
        // The bottom toast covers rows 26..=28 and columns 49..=97; the gap above it is row 25.
        let mut toasts = stack([Toast::info("old"), Toast::info("new")]);
        assert!(!toasts.dismiss_at(48, 27, area), "left of the toast");
        assert!(!toasts.dismiss_at(98, 27, area), "the gutter");
        assert!(!toasts.dismiss_at(70, 29, area), "the footer row");
        assert!(!toasts.dismiss_at(70, 25, area), "the transparent row");
        assert_eq!(toasts.len(), 2);
        assert!(toasts.dismiss_at(49, 26, area), "the bar, top row");
        assert_eq!(texts(&toasts), ["old"]);
        // `old` slid down into the gap.
        assert!(toasts.dismiss_at(97, 28, area));
        assert!(toasts.is_empty());
        assert!(!toasts.dismiss_at(97, 28, area));
    }

    #[test]
    fn only_a_left_press_after_a_draw_dismisses() {
        let mut toasts = stack([Toast::info("x")]);
        let press = |kind| MouseEvent {
            kind,
            column: 80,
            row: 27,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(
            !toasts.handle_mouse(&press(MouseEventKind::Down(MouseButton::Left))),
            "not drawn yet"
        );
        draw(&mut toasts, &Theme::default(), 100, 30);
        assert!(!toasts.handle_mouse(&press(MouseEventKind::Down(MouseButton::Right))));
        assert!(!toasts.handle_mouse(&press(MouseEventKind::Up(MouseButton::Left))));
        assert!(toasts.handle_mouse(&press(MouseEventKind::Down(MouseButton::Left))));
        assert!(toasts.is_empty());
    }

    #[test]
    fn width_is_sixty_or_half_the_screen() {
        // Half of the screen less the gutter, as in Python's captures: 49 at 100 columns.
        assert_eq!(toast_width(160), 60);
        assert_eq!(toast_width(122), 60);
        assert_eq!(toast_width(100), 49);
        assert_eq!(toast_width(80), 39);
        assert_eq!(toast_width(81), 39);
    }

    #[test]
    fn height_counts_padding_title_and_wrapped_message() {
        assert_eq!(toast_height(&Toast::info("short"), 50), 3);
        assert_eq!(toast_height(&Toast::info("short").with_title("Title"), 50), 4);
        // 47 content columns: the maximize message is 47 characters and fits exactly.
        assert_eq!(toast_height(&maximize_toast(), 50), 4);
        // 46 content columns (100 wide): it wraps before "return.".
        assert_eq!(toast_height(&maximize_toast(), 49), 5);
        let long = "Cannot load notifications - please login to the gh CLI: gh auth login";
        assert_eq!(toast_height(&Toast::error(long), 60), 4);
        assert_eq!(toast_height(&Toast::error(long), 40), 4);
        assert_eq!(toast_height(&Toast::error(long).with_title("A title"), 40), 5);
    }

    #[test]
    fn wrapping_matches_rich() {
        let text = "Current view maximized. Press escape to return.";
        assert_eq!(
            wrap_plain(text, 37),
            ["Current view maximized. Press escape", "to return."]
        );
        assert_eq!(wrap_plain(text, 47), [text]);
        // An overlong word starts its own line and is chopped, its trailing space included, as Textual's
        // `Content.wrap` does.
        assert_eq!(wrap_plain("ab cdefghij k", 4), ["ab", "cdef", "ghij", " k"]);
        assert_eq!(wrap_plain("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap_plain("abcdefgh x", 4), ["abcd", "efgh", " x"]);
        assert_eq!(wrap_plain("a  b", 1), ["a", "b"]);
        // A newline is a hard break; an empty message is one empty line.
        assert_eq!(wrap_plain("one\ntwo", 10), ["one", "two"]);
        assert_eq!(wrap_plain("", 10), [""]);
        // A word that fits without its trailing space stays on the line.
        assert_eq!(wrap_plain("abc def", 7), ["abc def"]);
        assert_eq!(wrap_plain("abc defg", 7), ["abc", "defg"]);
    }

    #[test]
    fn wrapping_keeps_span_styles() {
        let lines = toast_lines(&maximize_toast(), 37, &Theme::default());
        let bold: Vec<_> = lines
            .iter()
            .flat_map(|line| &line.spans)
            .filter(|span| span.style.add_modifier.contains(Modifier::BOLD))
            .map(|span| span.content.to_string())
            .collect();
        assert_eq!(bold, ["View Maximized", "escape"]);
    }

    #[test]
    fn accent_spans_keep_their_color() {
        let theme = Theme::default();
        let Rgb { r, g, b } = Accent::Yellow.rgb();
        let message = Line::from(vec![
            Span::raw("Removing "),
            Span::raw("someone").fg(Color::Rgb(r, g, b)),
        ]);
        let lines = toast_lines(&Toast::info(message), 47, &theme);
        assert_eq!(lines[0].spans[1].style.fg, Some(theme.accent(Accent::Yellow)));
    }

    #[test]
    fn toast_info_titled() {
        let mut toasts = stack([Toast::info("Settings updated").with_title("Saved")]);
        insta::assert_debug_snapshot!(draw(&mut toasts, &theme("textual-dark"), 100, 30));
    }

    #[test]
    fn toast_warning_untitled() {
        let mut toasts = stack([Toast::warning("This PR has already been merged!")]);
        insta::assert_debug_snapshot!(draw(&mut toasts, &theme("textual-dark"), 100, 30));
    }

    #[test]
    fn toast_error_long_wrapped() {
        let mut toasts = stack([Toast::error(
            "Cannot load notifications - please login to the gh CLI: gh auth login. This message is long enough to \
             wrap over several lines, with an overlong https://github.com/gizmo385/gh-lazy/issues/79/word.",
        )
        .with_title("Failed to Load Notifications")]);
        insta::assert_debug_snapshot!(draw(&mut toasts, &theme("textual-dark"), 100, 30));
    }

    #[test]
    fn toast_stack_three() {
        let mut toasts = stack([
            Toast::info("Pull request 12 merged. Note some cached information on the UI may be out of date.")
                .with_title("PR Merged"),
            Toast::warning("Comment cannot be empty!"),
            Toast::error("Pull request 12 could not be merged").with_title("Error Merging PR"),
        ]);
        insta::assert_debug_snapshot!(draw(&mut toasts, &theme("textual-dark"), 100, 30));
    }

    #[test]
    fn toast_stack_overflow() {
        // Six titled toasts, then three untitled: the third is cut through, its title above the screen.
        let mut toasts = stack((1..=9).map(|n| {
            let toast = Toast::info(format!("Toast number {n}"));
            if n <= 6 { toast.with_title("Stacked") } else { toast }
        }));
        let buffer = draw(&mut toasts, &theme("textual-dark"), 100, 30);
        let screen = buffer_to_string(&buffer);
        let rows: Vec<&str> = screen.lines().collect();
        assert!(rows[0].contains("▌ Toast number 3"), "{:?}", rows[0]);
        assert!(!screen.contains("Toast number 2"), "the oldest are clipped off the top");
        insta::assert_debug_snapshot!(buffer);
    }

    #[test]
    fn toast_bold_span() {
        let mut toasts = stack([maximize_toast()]);
        insta::assert_debug_snapshot!(draw(&mut toasts, &theme("textual-dark"), 100, 30));
    }

    #[test]
    fn toast_stack_ansi_dark() {
        let mut toasts = stack([
            Toast::info("Settings updated").with_title("Saved"),
            Toast::warning("Comment cannot be empty!"),
            Toast::error("Pull request 12 could not be merged").with_title("Error Merging PR"),
        ]);
        insta::assert_debug_snapshot!(draw(&mut toasts, &theme("ansi-dark"), 100, 30));
    }

    #[test]
    fn nothing_is_drawn_on_a_tiny_screen() {
        let mut toasts = stack([Toast::info("x")]);
        let screen = buffer_to_string(&draw(&mut toasts, &Theme::default(), 6, 3));
        assert_eq!(screen, "╳╳╳╳╳╳\n".repeat(3));
    }

    #[test]
    fn the_app_raises_draws_and_expires_toasts() {
        let mut app = TestApp::new(Config::default());
        app.press(&["ctrl+c"]);
        let start = app.clock().now();
        assert_eq!(app.app().next_deadline(), Some(start + DEFAULT_TIMEOUT));
        let screen = app.screen(100, 30);
        assert!(screen.contains("Do you want to quit?"), "{screen}");
        assert!(screen.contains("Press ctrl+q to quit the app"));
        app.advance(Duration::from_millis(4900));
        assert_eq!(app.app().toasts().len(), 1);
        app.press(&["ctrl+c"]);
        app.advance(Duration::from_millis(100));
        assert_eq!(app.app().toasts().len(), 1, "the first expired on its own");
        app.advance(Duration::from_millis(4900));
        assert!(app.app().toasts().is_empty());
        assert_eq!(app.app().next_deadline(), None);
        assert!(!app.screen(100, 30).contains("Do you want to quit?"));
    }

    #[test]
    fn a_click_on_a_toast_dismisses_it_and_goes_no_further() {
        let mut app = TestApp::new(Config::default());
        app.press(&["ctrl+c"]);
        app.render(100, 30);
        app.click(10, 27);
        assert_eq!(app.app().toasts().len(), 1, "a click elsewhere");
        app.click(80, 27);
        assert!(app.app().toasts().is_empty());
        assert_eq!(app.effects(), []);
    }
}
