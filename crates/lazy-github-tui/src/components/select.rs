//! Textual's `Select`: a one-line box in a `tall` border showing the chosen option or a prompt, which opens a list of
//! the options (its `SelectOverlay`, an `OptionList`) right below it to pick from.
//!
//! Like a switch, a select doesn't know whether its box has focus: the dialog's [`crate::modal::FocusRing`] does.
//! While the list is open it has the focus instead, and the dialog gives it the keys: [`Select::bindings`] says which
//! link the dialog puts in its chain, and [`Select::consumes_key`] takes the printable keys, which search the list.
//!
//! The dialog draws the box with its other fields and the open list last, over everything else in the dialog and past
//! its frame if need be, as Textual's `overlay: screen` does.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::text_input::{TallBorder, printable};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{AnsiColor, Paint, Theme};

/// A closed select's binding: `enter`, `down`, `space` or `up` opens the list (hidden).
pub const SELECT_BINDINGS: &[BindingId] = &[BindingId::TextualSelectShowOverlay];

/// The open list's bindings: `OptionList.BINDINGS`, then `SelectOverlay`'s `escape` (shown, "Dismiss menu").
pub const SELECT_OVERLAY_BINDINGS: &[BindingId] = &[
    BindingId::TextualOptionListCursorDown,
    BindingId::TextualOptionListLast,
    BindingId::TextualOptionListSelect,
    BindingId::TextualOptionListFirst,
    BindingId::TextualOptionListPageDown,
    BindingId::TextualOptionListPageUp,
    BindingId::TextualOptionListCursorUp,
    BindingId::TextualSelectDismiss,
];

/// Rows the box takes: its text row between the top and bottom border.
pub const HEIGHT: u16 = 3;

/// The box's `padding: 0 2`.
const PADDING: u16 = 2;

/// The list's `max-height: 12`, border included.
const MAX_OVERLAY_HEIGHT: u16 = 12;

/// How long type-to-search keeps the typed text after the last key.
const SEARCH_RESET: Duration = Duration::from_millis(700);

/// The prompt Textual's `Select` shows without one of its own.
pub const DEFAULT_PROMPT: &str = "Select";

/// Rich's `dim`, as Textual draws it: the text color at this alpha over its background.
const DIM_ALPHA: f64 = 0.66;

/// The box's `▼` / `▲` and the label's color without a value: `$foreground 50%`.
const MUTED_ALPHA: f64 = 0.5;

/// What a key, a binding or a click did to a select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum SelectOutcome {
    /// The list opened: it has the focus now.
    Opened,
    /// An option other than the current one was picked (the blank included): the list closed and the box has the
    /// focus. The owner reads [`Select::value`].
    Changed,
    /// The list closed without a change (`escape`, or the current option picked): the box has the focus.
    Closed,
    /// The list's highlight or search moved, or nothing visible happened, but the key was the select's.
    Consumed,
    /// Not the select's.
    Ignored,
}

/// The open list's state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Overlay {
    /// The highlighted row: 0 is the blank, `i + 1` is option `i`.
    highlight: usize,
    /// The first row in view.
    scroll: usize,
    /// Type-to-search's text so far.
    query: String,
    /// When the last key reached the list, for resetting the query.
    last_key: Option<Instant>,
}

/// A select over options of type `T`, with Textual's blank first option (`allow_blank=True`).
#[derive(Debug, Clone)]
pub struct Select<T> {
    prompt: Cow<'static, str>,
    options: Vec<(Cow<'static, str>, T)>,
    /// The chosen option's index, `None` for the blank.
    value: Option<usize>,
    overlay: Option<Overlay>,
    /// Where the open list was drawn last, for clicks and paging. Empty while closed.
    overlay_area: Rect,
}

impl<T: PartialEq> Select<T> {
    /// A closed select with these options, showing `prompt` when nothing is chosen, and `value` chosen if it's one of
    /// them.
    pub fn new(
        prompt: impl Into<Cow<'static, str>>,
        options: impl IntoIterator<Item = (impl Into<Cow<'static, str>>, T)>,
        value: Option<&T>,
    ) -> Self {
        let mut select = Self {
            prompt: prompt.into(),
            options: Vec::new(),
            value: None,
            overlay: None,
            overlay_area: Rect::default(),
        };
        select.set_options(options);
        select.set_value(value);
        select
    }

    /// The chosen option's value, `None` for the blank.
    pub fn value(&self) -> Option<&T> {
        self.value.map(|index| &self.options[index].1)
    }

    /// The chosen option's index, `None` for the blank.
    pub fn selected_index(&self) -> Option<usize> {
        self.value
    }

    /// Chooses the option whose value is `value`; `None`, or a value no option has, chooses the blank.
    pub fn set_value(&mut self, value: Option<&T>) {
        self.value = value.and_then(|value| self.options.iter().position(|(_, v)| v == value));
    }

    /// Replaces the options, keeping the chosen value if an option still has it, and closes the list.
    pub fn set_options(&mut self, options: impl IntoIterator<Item = (impl Into<Cow<'static, str>>, T)>) {
        let options: Vec<(Cow<'static, str>, T)> = options.into_iter().map(|(label, v)| (label.into(), v)).collect();
        let chosen = self.value.map(|index| &self.options[index].1);
        let value = chosen.and_then(|chosen| options.iter().position(|(_, v)| v == chosen));
        self.options = options;
        self.value = value;
        self.close();
    }

    /// The options' labels, in order.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.options.iter().map(|(label, _)| label.as_ref())
    }
}

impl<T> Select<T> {
    /// Whether the list is open.
    pub fn is_open(&self) -> bool {
        self.overlay.is_some()
    }

    /// The highlighted row while open: 0 is the blank, `i + 1` is option `i`.
    pub fn highlighted(&self) -> Option<usize> {
        self.overlay.as_ref().map(|overlay| overlay.highlight)
    }

    /// The link the dialog puts in its chain for the select: the box's while closed, the list's while open.
    pub fn bindings(&self) -> &'static [BindingId] {
        if self.is_open() {
            SELECT_OVERLAY_BINDINGS
        } else {
            SELECT_BINDINGS
        }
    }

    /// Rows of the list: the blank and the options.
    fn rows(&self) -> usize {
        self.options.len() + 1
    }

    /// Opens the list with the chosen option highlighted, or the blank without one (`_watch_expanded`, then
    /// `action_show_overlay`'s first row). Already open, nothing changes.
    pub fn open(&mut self) -> SelectOutcome {
        if self.overlay.is_none() {
            let highlight = self.value.map_or(0, |index| index + 1);
            self.overlay = Some(Overlay {
                highlight,
                scroll: 0,
                query: String::new(),
                last_key: None,
            });
            self.scroll_to_highlight();
        }
        SelectOutcome::Opened
    }

    /// Closes the list without a change: `escape`, or the list losing focus (`tab`, a click elsewhere).
    pub fn close(&mut self) {
        self.overlay = None;
        self.overlay_area = Rect::default();
    }

    /// A click on the box: opens or closes the list (`SelectCurrent.Toggle`).
    pub fn toggle(&mut self) -> SelectOutcome {
        if self.is_open() {
            self.close();
            SelectOutcome::Closed
        } else {
            self.open()
        }
    }

    /// Whether the list takes `key` before any binding: while open, the printable keys, which search it
    /// (`SelectOverlay.check_consume_key`), and the list's own keys.
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.is_open() && (printable(key).is_some() || keymap.match_in(SELECT_OVERLAY_BINDINGS, key).is_some())
    }

    /// A key at `now`: while open, a printable key searches the list and the list's keys move or pick; while closed,
    /// the box's keys open it.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, now: Instant) -> SelectOutcome {
        if let Some(overlay) = &mut self.overlay {
            // Every key restarts the search's reset timer, as `SelectOverlay._on_key` does.
            let expired = overlay
                .last_key
                .is_some_and(|last| now.saturating_duration_since(last) > SEARCH_RESET);
            if expired {
                overlay.query.clear();
            }
            overlay.last_key = Some(now);
            if let Some(c) = printable(key) {
                overlay.query.push(c);
                self.search();
                return SelectOutcome::Consumed;
            }
        }
        match keymap.match_in(self.bindings(), key) {
            Some(id) => self.on_binding(id),
            None => SelectOutcome::Ignored,
        }
    }

    /// A binding of [`Self::bindings`]: the box's opens the list; the list's move the highlight (wrapping at the ends,
    /// as `OptionList` does), pick, or dismiss it.
    pub fn on_binding(&mut self, id: BindingId) -> SelectOutcome {
        let rows = self.rows();
        // `OptionList._move_page` moves by the rows in view.
        let page = usize::from(Self::overlay_height(rows).saturating_sub(2)).max(1);
        let Some(overlay) = &mut self.overlay else {
            return if id == BindingId::TextualSelectShowOverlay {
                self.open()
            } else {
                SelectOutcome::Ignored
            };
        };
        let last = rows - 1;
        overlay.highlight = match id {
            BindingId::TextualOptionListCursorDown => (overlay.highlight + 1) % rows,
            BindingId::TextualOptionListCursorUp => overlay.highlight.checked_sub(1).unwrap_or(last),
            BindingId::TextualOptionListFirst => 0,
            BindingId::TextualOptionListLast => last,
            BindingId::TextualOptionListPageDown => (overlay.highlight + page).min(last),
            BindingId::TextualOptionListPageUp => overlay.highlight.saturating_sub(page),
            BindingId::TextualOptionListSelect => {
                let row = overlay.highlight;
                return self.pick(row);
            }
            BindingId::TextualSelectDismiss => {
                self.close();
                return SelectOutcome::Closed;
            }
            _ => return SelectOutcome::Ignored,
        };
        self.scroll_to_highlight();
        SelectOutcome::Consumed
    }

    /// Picks list row `row`: the value changes (the blank clears it) and the list closes.
    fn pick(&mut self, row: usize) -> SelectOutcome {
        let value = row.checked_sub(1).filter(|&index| index < self.options.len());
        self.close();
        if value == self.value {
            SelectOutcome::Closed
        } else {
            self.value = value;
            SelectOutcome::Changed
        }
    }

    /// Jumps to the row whose label contains the query earliest, case-insensitively (`_find_search_match`); the blank
    /// is searched by the prompt. No match leaves the highlight.
    fn search(&mut self) {
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        let query = overlay.query.to_lowercase();
        let labels = std::iter::once(self.prompt.as_ref()).chain(self.options.iter().map(|(label, _)| label.as_ref()));
        let best = labels
            .enumerate()
            .filter_map(|(row, label)| label.to_lowercase().find(&query).map(|at| (at, row)))
            .min();
        if let Some((_, row)) = best {
            overlay.highlight = row;
            self.scroll_to_highlight();
        }
    }

    /// Scrolls the list so the highlighted row is in view.
    fn scroll_to_highlight(&mut self) {
        let window = usize::from(Self::overlay_height(self.rows()).saturating_sub(2));
        if let Some(overlay) = &mut self.overlay {
            if overlay.highlight < overlay.scroll {
                overlay.scroll = overlay.highlight;
            } else if overlay.highlight >= overlay.scroll + window {
                overlay.scroll = overlay.highlight + 1 - window;
            }
        }
    }

    /// The list's height, border included, for this many rows.
    fn overlay_height(rows: usize) -> u16 {
        u16::try_from(rows)
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(MAX_OVERLAY_HEIGHT)
    }

    /// Whether screen cell `(column, row)` is on the open list.
    pub fn overlay_contains(&self, column: u16, row: u16) -> bool {
        self.is_open() && self.overlay_area.contains(Position::new(column, row))
    }

    /// A mouse event on the open list, in screen cells: a left press on a row picks it, anywhere else on the list
    /// does nothing. Off the list it's [`SelectOutcome::Ignored`]; the dialog decides what that click means.
    pub fn handle_overlay_mouse(&mut self, event: &MouseEvent) -> SelectOutcome {
        if !self.overlay_contains(event.column, event.row) {
            return SelectOutcome::Ignored;
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return SelectOutcome::Consumed;
        }
        let area = self.overlay_area;
        let inside = event.row > area.y && event.row + 1 < area.bottom();
        let scroll = self.overlay.as_ref().map_or(0, |overlay| overlay.scroll);
        let row = scroll + usize::from(event.row.saturating_sub(area.y + 1));
        if inside && row < self.rows() {
            self.pick(row)
        } else {
            SelectOutcome::Consumed
        }
    }

    /// Draws the box into the top [`HEIGHT`] rows of `area`: the chosen option's label, or the prompt muted, then the
    /// arrow, `▲` while open. `under` is the parent's background, which shows in the half-cell `▊`/`▎` edges.
    /// `focused` is the box's own focus, which it doesn't have while the list is open.
    pub fn render_box(&self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let area = Rect {
            height: area.height.min(HEIGHT),
            ..area
        };
        if area.is_empty() {
            return;
        }
        let styles = BoxStyles::new(theme, under, focused);
        TallBorder {
            left: styles.left,
            inner: styles.inner,
            right: styles.right,
            fill: styles.text,
        }
        .draw(buf, area);
        if area.height < 2 || area.width <= 2 * (PADDING + 1) {
            return;
        }
        let y = area.y + 1;
        let left = area.x + 1 + PADDING;
        let width = area.width - 2 * (PADDING + 1);
        let arrow_x = left + width - 1;
        let (label, style) = match self.value {
            Some(index) => (self.options[index].0.as_ref(), styles.text),
            None => (self.prompt.as_ref(), styles.muted),
        };
        // The label stops a cell short of the arrow (`.arrow { padding: 0 0 0 1 }`).
        buf.set_stringn(left, y, label, usize::from(width.saturating_sub(2)), style);
        let arrow = if self.is_open() { "▲" } else { "▼" };
        buf.set_string(arrow_x, y, arrow, styles.muted);
    }

    /// Draws the open list right below `below` (the box's cells on screen), as wide as it, moved up as far as needed
    /// to stay inside `clip` (the screen), and clipped to it. `under` is what's under the list's edges. Nothing while
    /// closed.
    pub fn render_overlay(&mut self, buf: &mut Buffer, below: Rect, clip: Rect, theme: &Theme, under: Paint) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let height = Self::overlay_height(self.rows());
        // `constrain: none inside`: moved up to fit the screen, never down.
        let y = below.bottom().min(clip.bottom().saturating_sub(height)).max(clip.y);
        let area = Rect {
            x: below.x,
            y,
            width: below.width,
            height,
        }
        .intersection(clip)
        .intersection(buf.area);
        self.overlay_area = area;
        if area.is_empty() {
            return;
        }
        let styles = OverlayStyles::new(theme, under);
        TallBorder {
            left: styles.left,
            inner: styles.inner,
            right: styles.right,
            fill: styles.option,
        }
        .draw(buf, area);
        // Inside the border, the list's `padding: 0 1` and each option's.
        if area.width <= 4 {
            return;
        }
        let row_x = area.x + 2;
        let row_width = area.width - 4;
        let window = usize::from(area.height.saturating_sub(2));
        let labels = std::iter::once((self.prompt.as_ref(), true))
            .chain(self.options.iter().map(|(label, _)| (label.as_ref(), false)));
        for (dy, (row, (label, blank))) in labels.enumerate().skip(overlay.scroll).take(window).enumerate() {
            // Bounded by the list's height.
            let y = area.y + 1 + u16::try_from(dy).unwrap_or(u16::MAX);
            let highlighted = row == overlay.highlight;
            let base = if highlighted { styles.highlight } else { styles.option };
            let style = if blank { styles.dimmed(base, theme) } else { base };
            for x in row_x..row_x + row_width {
                buf[(x, y)].set_symbol(" ").set_style(base);
            }
            let text = Span::styled(label, style);
            buf.set_span(row_x + 1, y, &text, row_width.saturating_sub(2));
        }
    }
}

/// The box's styles for one frame, from `SelectCurrent`'s and `Select:focus > SelectCurrent`'s CSS.
struct BoxStyles {
    /// The padding and the chosen label: `$foreground` on `$surface`, tinted `$foreground 5%` while focused.
    text: Style,
    /// The prompt without a value, and the arrow: `$foreground 50%`.
    muted: Style,
    left: Style,
    inner: Style,
    right: Style,
}

impl BoxStyles {
    fn new(theme: &Theme, under: Paint, focused: bool) -> Self {
        let p = theme.palette;
        let border_color = if focused { p.border } else { p.border_blurred };
        if theme.ansi {
            // `&:ansi`: the terminal's own colors.
            let default = theme.paint(Paint::Ansi(AnsiColor::Default));
            let text = Style::new().fg(default).bg(default);
            let border = theme.color(border_color, under);
            return Self {
                text,
                muted: text.add_modifier(Modifier::DIM),
                left: Style::new().fg(border).bg(default).add_modifier(Modifier::REVERSED),
                inner: Style::new().fg(border).bg(default),
                right: Style::new().fg(border).bg(default),
            };
        }
        let surface = p.surface.over(under);
        let bg = if focused {
            theme.tint(surface, p.foreground, 5.0)
        } else {
            surface
        };
        let border = theme.color(border_color, bg);
        let text = Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg));
        Self {
            text,
            muted: text.fg(theme.color(p.foreground.multiply_alpha(MUTED_ALPHA), bg)),
            left: Style::new()
                .fg(border)
                .bg(theme.paint(under))
                .add_modifier(Modifier::REVERSED),
            inner: Style::new().fg(border).bg(theme.paint(bg)),
            right: Style::new().fg(border).bg(theme.paint(under)),
        }
    }
}

/// The open list's styles for one frame, from `Select > SelectOverlay`'s CSS while it has focus (it always does
/// while open) and `OptionList`'s highlight.
struct OverlayStyles {
    /// An option: `$foreground` on `$surface` tinted `$foreground 5%`.
    option: Style,
    /// The highlighted option: `$block-cursor-foreground` on `$block-cursor-background`, in
    /// `$block-cursor-text-style`, across the row.
    highlight: Style,
    left: Style,
    inner: Style,
    right: Style,
    /// What the dim blank row is drawn over in [`Self::option`] and [`Self::highlight`].
    option_bg: Paint,
    highlight_bg: Paint,
}

impl OverlayStyles {
    fn new(theme: &Theme, under: Paint) -> Self {
        let p = theme.palette;
        let bg = theme.tint(p.surface.over(under), p.foreground, 5.0);
        let border = theme.color(p.border, bg);
        let highlight_bg = p.block_cursor_background.over(bg);
        Self {
            option: Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg)),
            highlight: p.block_cursor_text_style.apply(
                Style::new()
                    .fg(theme.color(p.block_cursor_foreground, highlight_bg))
                    .bg(theme.paint(highlight_bg)),
            ),
            left: Style::new()
                .fg(border)
                .bg(theme.paint(under))
                .add_modifier(Modifier::REVERSED),
            inner: Style::new().fg(border).bg(theme.paint(bg)),
            right: Style::new().fg(border).bg(theme.paint(under)),
            option_bg: bg,
            highlight_bg,
        }
    }

    /// `base` with Rich's `dim` text: the blank row's prompt (`Text(prompt, style="dim")`).
    fn dimmed(&self, base: Style, theme: &Theme) -> Style {
        if theme.ansi {
            return base.add_modifier(Modifier::DIM);
        }
        let p = theme.palette;
        let (fg, bg) = if base == self.highlight {
            (p.block_cursor_foreground, self.highlight_bg)
        } else {
            (p.foreground, self.option_bg)
        };
        base.fg(theme.color(fg.multiply_alpha(DIM_ALPHA), bg))
    }
}

#[cfg(test)]
mod tests;
