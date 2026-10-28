//! Textual's `SelectionList`: a list of options in a `tall` border, each with a check box (`▐X▌`), where `space` or
//! `enter` toggles the highlighted one. Add reaction's list and the PR reviewers list use it.
//!
//! Like a switch, the list doesn't know whether it has focus: the dialog's [`crate::modal::FocusRing`] does, and the
//! dialog puts [`SELECTION_LIST_BINDINGS`] in its chain while it has ([`crate::modal::FieldKind::SelectionList`]).
//! The list takes no printable keys: type-to-search belongs to the `Select` overlay alone.

use std::borrow::Cow;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::option_nav;
use super::text_input::TallBorder;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

/// A focused list's bindings, all hidden: `OptionList.BINDINGS`, then `SelectionList`'s `space`, in Textual's merged
/// order.
pub const SELECTION_LIST_BINDINGS: &[BindingId] = &[
    BindingId::TextualOptionListCursorDown,
    BindingId::TextualOptionListLast,
    BindingId::TextualOptionListSelect,
    BindingId::TextualOptionListFirst,
    BindingId::TextualOptionListPageDown,
    BindingId::TextualOptionListPageUp,
    BindingId::TextualOptionListCursorUp,
    BindingId::TextualSelectionListSelect,
];

/// The check box in front of each option, and the space after it: `ToggleButton.BUTTON_LEFT`, `BUTTON_INNER` and
/// `BUTTON_RIGHT` (`SelectionList._get_left_gutter_width`).
const BUTTON_LEFT: &str = "▐";
const BUTTON_INNER: &str = "X";
const BUTTON_RIGHT: &str = "▌";
const GUTTER: u16 = 4;

/// `OptionList`'s `padding: 0 1`, inside its border.
const PADDING: u16 = 1;

/// Textual's `text-overflow: ellipsis`.
const ELLIPSIS: &str = "…";

/// What a key, a binding or a click did to the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ListOutcome {
    /// Option `index` was toggled and is now `selected`: Textual's `SelectionToggled`. The owner reads the value
    /// with [`SelectionList::value`].
    Toggled {
        /// The option's index.
        index: usize,
        /// Whether it's selected now.
        selected: bool,
    },
    /// The key was the list's: the highlight may have moved, or nothing changed (a toggle key with nothing
    /// highlighted).
    Moved,
    /// Not the list's.
    Ignored,
}

/// One option: its label, its value and when it was selected.
#[derive(Debug, Clone)]
struct Item<T> {
    label: Cow<'static, str>,
    value: T,
    /// The order it was selected in, while it's selected: Textual keeps the selected values in a dict, so
    /// `selected` lists them in the order they were selected.
    selected: Option<u64>,
}

/// A list of options of type `T`, any of which can be selected.
#[derive(Debug, Clone)]
pub struct SelectionList<T> {
    options: Vec<Item<T>>,
    highlighted: Option<usize>,
    /// The first option in view.
    scroll: usize,
    /// The options in view at the last draw, which a page key moves by. Every option until the first draw.
    window: Option<usize>,
    /// The next selection's order stamp.
    next_stamp: u64,
    /// Where the options were drawn last (inside the border and the padding), for clicks.
    rows_area: Rect,
}

impl<T> Default for SelectionList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SelectionList<T> {
    /// An empty list with nothing highlighted. Options added later aren't highlighted either: Textual highlights
    /// only a list built with options, so the first `down` highlights the first option (the PR reviewers list).
    pub fn new() -> Self {
        Self {
            options: Vec::new(),
            highlighted: None,
            scroll: 0,
            window: None,
            next_stamp: 0,
            rows_area: Rect::default(),
        }
    }

    /// A list of `options` (label, value, selected), the first one highlighted when there are any, as Textual's
    /// `OptionList.__init__` does (Add reaction's list).
    pub fn with_options(options: impl IntoIterator<Item = (impl Into<Cow<'static, str>>, T, bool)>) -> Self {
        let mut list = Self::new();
        for (label, value, selected) in options {
            list.add(label, value, selected);
        }
        if !list.options.is_empty() {
            list.highlighted = Some(0);
        }
        list
    }

    /// Adds an option at the end, selected or not. The highlight stays where it is.
    pub fn add(&mut self, label: impl Into<Cow<'static, str>>, value: T, selected: bool) {
        let stamp = selected.then(|| self.stamp());
        self.options.push(Item {
            label: label.into(),
            value,
            selected: stamp,
        });
    }

    fn stamp(&mut self) -> u64 {
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        stamp
    }

    /// How many options there are.
    pub fn len(&self) -> usize {
        self.options.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }

    /// Option `index`'s value.
    pub fn value(&self, index: usize) -> Option<&T> {
        self.options.get(index).map(|item| &item.value)
    }

    /// The options' labels, in order.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.options.iter().map(|item| item.label.as_ref())
    }

    /// Whether option `index` is selected.
    pub fn is_selected(&self, index: usize) -> bool {
        self.options.get(index).is_some_and(|item| item.selected.is_some())
    }

    /// The selected values, in the order they were selected (Textual's `SelectionList.selected`).
    pub fn selected(&self) -> Vec<&T> {
        let mut selected: Vec<(u64, &T)> = self
            .options
            .iter()
            .filter_map(|item| item.selected.map(|stamp| (stamp, &item.value)))
            .collect();
        selected.sort_unstable_by_key(|&(stamp, _)| stamp);
        selected.into_iter().map(|(_, value)| value).collect()
    }

    /// The highlighted option's index.
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// Removes option `index`. The highlight keeps its index, or moves to the new last option, and goes away with
    /// the last option (`OptionList._remove_option`'s `self.highlighted = self.highlighted`).
    pub fn remove_at(&mut self, index: usize) -> Option<T> {
        if index >= self.options.len() {
            return None;
        }
        let item = self.options.remove(index);
        self.highlighted = self
            .highlighted
            .and_then(|highlighted| self.options.len().checked_sub(1).map(|last| highlighted.min(last)));
        self.scroll = self.scroll.min(self.options.len().saturating_sub(1));
        Some(item.value)
    }

    /// Toggles option `index`.
    fn toggle(&mut self, index: usize) -> ListOutcome {
        let stamp = self.stamp();
        let Some(item) = self.options.get_mut(index) else {
            return ListOutcome::Moved;
        };
        item.selected = match item.selected {
            Some(_) => None,
            None => Some(stamp),
        };
        ListOutcome::Toggled {
            index,
            selected: item.selected.is_some(),
        }
    }

    /// A key: one of [`SELECTION_LIST_BINDINGS`] moves the highlight or toggles, anything else is
    /// [`ListOutcome::Ignored`].
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap) -> ListOutcome {
        match keymap.match_in(SELECTION_LIST_BINDINGS, key) {
            Some(id) => self.on_binding(id),
            None => ListOutcome::Ignored,
        }
    }

    /// A binding of [`SELECTION_LIST_BINDINGS`]: `up` and `down` move the highlight, wrapping at the ends, `home`
    /// and `end` jump to them, the page keys move by the options in view and stop at the ends, and `space` and
    /// `enter` toggle the highlighted option. With nothing highlighted, `down`, `home` and `pageup` go to the first
    /// option and `up`, `end` and `pagedown` to the last (Textual's `_widget_navigation`).
    pub fn on_binding(&mut self, id: BindingId) -> ListOutcome {
        use BindingId as B;
        if matches!(id, B::TextualOptionListSelect | B::TextualSelectionListSelect) {
            return self.highlighted.map_or(ListOutcome::Moved, |index| self.toggle(index));
        }
        let Some(last) = self.options.len().checked_sub(1) else {
            return if SELECTION_LIST_BINDINGS.contains(&id) {
                ListOutcome::Moved
            } else {
                ListOutcome::Ignored
            };
        };
        let page = self.window.unwrap_or(self.options.len()).max(1);
        let highlighted = self.highlighted;
        self.highlighted = Some(match (id, highlighted) {
            (B::TextualOptionListCursorDown, Some(h)) => option_nav::next(h, self.options.len()),
            (B::TextualOptionListCursorUp, Some(h)) => option_nav::previous(h, self.options.len()),
            (B::TextualOptionListPageDown, Some(h)) => option_nav::page_down(h, self.options.len(), 1, page),
            (B::TextualOptionListPageUp, Some(h)) => option_nav::page_up(h, 1, page),
            (B::TextualOptionListCursorDown | B::TextualOptionListFirst | B::TextualOptionListPageUp, _) => 0,
            (B::TextualOptionListCursorUp | B::TextualOptionListLast | B::TextualOptionListPageDown, _) => last,
            _ => return ListOutcome::Ignored,
        });
        self.scroll_to_highlight();
        ListOutcome::Moved
    }

    /// Scrolls so the highlighted option is in the window of the last draw.
    fn scroll_to_highlight(&mut self) {
        let (Some(highlighted), Some(window)) = (self.highlighted, self.window) else {
            return;
        };
        self.scroll = option_nav::scroll_into_view(self.scroll, highlighted, 1, window.max(1));
    }

    /// The rows the list takes (`height: auto`): its options and the border.
    pub fn height(&self) -> u16 {
        u16::try_from(self.options.len()).unwrap_or(u16::MAX).saturating_add(2)
    }

    /// A mouse event in screen cells, against where the options were drawn last: a left press on an option
    /// highlights and toggles it (Textual's `_on_click`; the dialog focuses the list). Elsewhere it's
    /// [`ListOutcome::Ignored`].
    pub fn handle_mouse(&mut self, event: &MouseEvent) -> ListOutcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            || !self.rows_area.contains(Position::new(event.column, event.row))
        {
            return ListOutcome::Ignored;
        }
        let index = self.scroll + usize::from(event.row - self.rows_area.y);
        if index >= self.options.len() {
            return ListOutcome::Ignored;
        }
        self.highlighted = Some(index);
        self.toggle(index)
    }

    /// Draws the list into the top [`Self::height`] rows of `area`, clipped to it, scrolled to keep the highlight in
    /// view. `under` is the parent's background, which shows in the half-cell `▊`/`▎` edges. `focused` draws the
    /// focused border, tint and highlight.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let area = Rect {
            height: area.height.min(self.height()),
            ..area
        }
        .intersection(buf.area);
        self.rows_area = Rect::default();
        if area.is_empty() {
            return;
        }
        let styles = Styles::new(theme, under, focused);
        TallBorder {
            left: styles.left,
            inner: styles.inner,
            right: styles.right,
            fill: styles.option,
        }
        .draw(buf, area);
        if area.height == 2 && self.options.is_empty() {
            // An empty list's bottom edge, which `TallBorder` leaves off a box this short.
            for x in area.x + 1..area.right().saturating_sub(1) {
                buf[(x, area.y + 1)].set_symbol("▁").set_style(styles.inner);
            }
        }
        let window = usize::from(area.height.saturating_sub(2));
        self.window = Some(window);
        self.scroll_to_highlight();
        if area.width <= 2 * (1 + PADDING) || window == 0 {
            return;
        }
        let rows = Rect {
            x: area.x + 1 + PADDING,
            y: area.y + 1,
            width: area.width - 2 * (1 + PADDING),
            // At most the area's height, which is a u16.
            height: u16::try_from(window).unwrap_or(u16::MAX),
        };
        self.rows_area = rows;
        for (dy, (index, item)) in self
            .options
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(window)
            .enumerate()
        {
            // Bounded by the window, which fits in a u16.
            let y = rows.y + u16::try_from(dy).unwrap_or(u16::MAX);
            let row = if self.highlighted == Some(index) {
                styles.highlight
            } else {
                styles.option
            };
            styles.draw_row(buf, rows.x, y, rows.width, row, item);
        }
    }
}

impl<T: PartialEq> SelectionList<T> {
    /// Removes the option whose value is `value`, if there is one. See [`Self::remove_at`].
    pub fn remove(&mut self, value: &T) -> Option<T> {
        let index = self.options.iter().position(|item| item.value == *value)?;
        self.remove_at(index)
    }

    /// Whether an option has `value`.
    pub fn contains(&self, value: &T) -> bool {
        self.options.iter().any(|item| item.value == *value)
    }
}

/// `label` cut to `width` cells, ending in `…` when it doesn't fit (`text-wrap: nowrap; text-overflow: ellipsis`).
fn fit(label: &str, width: usize) -> Cow<'_, str> {
    if Span::raw(label).width() <= width {
        return Cow::Borrowed(label);
    }
    let Some(mut left) = width.checked_sub(1) else {
        return Cow::Borrowed("");
    };
    let mut cut = String::new();
    for grapheme in Span::raw(label).styled_graphemes(Style::new()) {
        let cells = Span::raw(grapheme.symbol).width();
        if cells > left {
            break;
        }
        left -= cells;
        cut.push_str(grapheme.symbol);
    }
    cut.push_str(ELLIPSIS);
    Cow::Owned(cut)
}

/// The list's styles for one frame, from `OptionList`'s and `SelectionList`'s CSS.
struct Styles {
    /// An option, and the padding: `$foreground` on `$surface`, tinted `$foreground 5%` while focused.
    option: Style,
    /// The highlighted option: `$block-cursor-*` while focused, else `$foreground` on
    /// `$block-cursor-blurred-background` in `$block-cursor-blurred-text-style`.
    highlight: Style,
    /// The check box's `X`: `$panel-darken-2` on `$panel`, `$text-success` once selected.
    button: Style,
    button_selected: Style,
    left: Style,
    inner: Style,
    right: Style,
}

impl Styles {
    fn new(theme: &Theme, under: Paint, focused: bool) -> Self {
        let p = theme.palette;
        let surface = p.surface.over(under);
        let bg = if focused {
            theme.tint(surface, p.foreground, 5.0)
        } else {
            surface
        };
        let border = theme.color(if focused { p.border } else { p.border_blurred }, bg);
        let highlight = if focused {
            let highlight_bg = p.block_cursor_background.over(bg);
            p.block_cursor_text_style.apply(
                Style::new()
                    .fg(theme.color(p.block_cursor_foreground, highlight_bg))
                    .bg(theme.paint(highlight_bg)),
            )
        } else {
            // The later `& > .option-list--option-highlighted` rule sets the color back to `$foreground`.
            let highlight_bg = p.block_cursor_blurred_background.over(bg);
            p.block_cursor_blurred_text_style.apply(
                Style::new()
                    .fg(theme.color(p.foreground, highlight_bg))
                    .bg(theme.paint(highlight_bg)),
            )
        };
        let panel = p.panel.over(bg);
        let button = Style::new().bg(theme.paint(panel)).remove_modifier(Modifier::all());
        Self {
            option: Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg)),
            highlight,
            button: button.fg(theme.color(p.panel_darken_2, panel)),
            button_selected: button.fg(theme.color(p.text_success, panel)),
            left: Style::new()
                .fg(border)
                .bg(theme.paint(under))
                .add_modifier(Modifier::REVERSED),
            inner: Style::new().fg(border).bg(theme.paint(bg)),
            right: Style::new().fg(border).bg(theme.paint(under)),
        }
    }

    /// Draws `item` on row `y` from `x`, `width` cells wide, in `row` (the option's or the highlight's style):
    /// `SelectionList.render_line`'s button, then the label in the rest, cut with `…`.
    fn draw_row<T>(&self, buf: &mut Buffer, x: u16, y: u16, width: u16, row: Style, item: &Item<T>) {
        for dx in 0..width {
            buf[(x + dx, y)].set_symbol(" ").set_style(row);
        }
        let button = if item.selected.is_some() {
            self.button_selected
        } else {
            self.button
        };
        // The sides are the button's background over the row's, without the row's text style.
        let side = Style::new()
            .fg(button.bg.unwrap_or_default())
            .bg(row.bg.unwrap_or_default())
            .remove_modifier(Modifier::all());
        let cells = [(BUTTON_LEFT, side), (BUTTON_INNER, button), (BUTTON_RIGHT, side)];
        for (dx, (symbol, style)) in (0..width).zip(cells) {
            buf[(x + dx, y)].set_symbol(symbol).set_style(style);
        }
        if width > GUTTER {
            let label_width = width - GUTTER;
            buf.set_stringn(
                x + GUTTER,
                y,
                fit(&item.label, usize::from(label_width)),
                usize::from(label_width),
                row,
            );
        }
    }
}

#[cfg(test)]
mod tests;
