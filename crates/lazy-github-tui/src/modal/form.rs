//! The scrolling form that New issue, Edit issue and Edit PR share: bold labels, fields and rules stacked in a
//! scroll area that takes the top 80% of the dialog, and a flush Save / Cancel row centered in the rest. Python's
//! `ScrollableContainer { height: 80% }` and `Horizontal#button_holder { align: center middle }`.
//!
//! The scroll area is focusable, as Textual's `ScrollableContainer` is, so a dialog keeps a field for it first in its
//! [`super::FocusRing`]. Its fields are drawn whole into an offscreen buffer as tall as the form, and the rows in view
//! are copied out ([`ScrollView::render_buffer`]); mouse events are moved into that buffer's coordinates with
//! [`FormScroll::to_content`].

use std::ops::Range;

use crossterm::event::MouseEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::scroll_view::ScrollView;
use crate::theme::{Paint, Theme};

/// The scroll area's share of the interior's height: `height: 80%`, rounded down as Textual does.
const SCROLL_PERCENT: u16 = 80;

/// A `Rule`: its line with `margin: 1 0`.
const RULE_ROWS: u16 = 3;

/// One block of a form, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block<F> {
    /// A bold label: `Label("[bold]Title[/bold]")`.
    Label(&'static str),
    /// Field `F`, this many rows tall.
    Field(F, u16),
    /// A horizontal `Rule` in `$secondary`, with a blank row above and below.
    Rule,
}

/// Where each block of a form goes, in rows from the form's top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormLayout<F> {
    blocks: Vec<(Block<F>, u16)>,
    height: u16,
}

impl<F: Copy + Eq> FormLayout<F> {
    /// `blocks` stacked from the top, with no space between them beyond the rules' margins.
    pub fn new(blocks: impl IntoIterator<Item = Block<F>>) -> Self {
        let mut top = 0u16;
        let blocks = blocks
            .into_iter()
            .map(|block| {
                let placed = (block, top);
                top = top.saturating_add(match block {
                    Block::Label(_) => 1,
                    Block::Field(_, height) => height,
                    Block::Rule => RULE_ROWS,
                });
                placed
            })
            .collect();
        Self { blocks, height: top }
    }

    /// The form's height: every block's rows.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The rows `field` takes, from the form's top.
    pub fn rows(&self, field: F) -> Option<Range<u16>> {
        self.blocks.iter().find_map(|&(block, top)| match block {
            Block::Field(f, height) if f == field => Some(top..top + height),
            _ => None,
        })
    }

    /// The field on form row `row`, if a field is there.
    pub fn field_at(&self, row: u16) -> Option<F> {
        self.blocks.iter().find_map(|&(block, top)| match block {
            Block::Field(field, height) if (top..top + height).contains(&row) => Some(field),
            _ => None,
        })
    }

    /// Where `field` goes in `content`, the form's offscreen area: its rows, full width.
    pub fn area(&self, field: F, content: Rect) -> Rect {
        self.rows(field).map_or_else(Rect::default, |rows| Rect {
            y: content.y + rows.start,
            height: rows.end - rows.start,
            ..content
        })
    }

    /// Draws the labels and the rules into `content`; the dialog draws the fields.
    pub fn render_static(&self, buf: &mut Buffer, content: Rect, theme: &Theme, under: Paint) {
        let p = theme.palette;
        let label = Style::new()
            .fg(theme.color(p.foreground, under))
            .add_modifier(Modifier::BOLD);
        // `Rule { color: $secondary }`.
        let rule = Style::new().fg(theme.color(p.secondary, under));
        let width = usize::from(content.width);
        for &(block, top) in &self.blocks {
            let y = content.y + top;
            match block {
                Block::Label(text) => {
                    buf.set_stringn(content.x, y, text, width, label);
                }
                Block::Rule => {
                    buf.set_stringn(content.x, y + 1, "─".repeat(width), width, rule);
                }
                Block::Field(..) => {}
            }
        }
    }
}

/// The scroll area and the button row's area of a form dialog's interior: the top 80% and the rest, with a button
/// row `button_height` tall centered vertically in the rest.
pub fn split(interior: Rect, button_height: u16) -> (Rect, Rect) {
    let scroll_height = interior.height * SCROLL_PERCENT / 100;
    let scroll = Rect {
        height: scroll_height,
        ..interior
    };
    let rest = interior.height - scroll_height;
    let buttons = Rect {
        y: interior.y + scroll_height + rest.saturating_sub(button_height) / 2,
        height: button_height.min(rest),
        ..interior
    };
    (scroll, buttons)
}

/// The scroll area's position and where it was drawn last.
#[derive(Debug)]
pub struct FormScroll {
    view: ScrollView,
    /// Whether an overflowing form shows a scrollbar, narrowing it by the bar's two columns.
    scrollbar: bool,
    /// The scroll area on screen at the last draw. Empty until then.
    area: Rect,
    /// The offscreen area the form was drawn into at the last draw.
    content: Rect,
}

impl Default for FormScroll {
    fn default() -> Self {
        Self {
            view: ScrollView::default(),
            scrollbar: true,
            area: Rect::default(),
            content: Rect::default(),
        }
    }
}

impl FormScroll {
    /// A scroll area that shows no scrollbar and keeps its whole width: for a dialog whose fields fill it, which
    /// scrolls only to reveal a field below the screen's edge (Textual scrolls the modal screen itself).
    pub fn without_scrollbar() -> Self {
        Self {
            scrollbar: false,
            ..Self::default()
        }
    }

    /// The scroll area on screen at the last draw.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The offscreen area the form was drawn into at the last draw: from the scroll area's left edge and row 0, as
    /// wide as the area less any scrollbar, as tall as the form. Fields' areas are in its coordinates.
    pub fn content(&self) -> Rect {
        self.content
    }

    /// The first form row in view.
    pub fn offset(&self) -> usize {
        self.view.offset()
    }

    /// Draws a form `height` rows tall into `area`: `draw` paints it into an offscreen buffer (the `Rect` it gets,
    /// which starts at `area`'s left edge and at row 0, and is narrower by the scrollbar when the form overflows),
    /// whose rows in view are then copied into `buf` beside the scrollbar. The buffer starts out in `fill`, the
    /// dialog's background.
    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        theme: &Theme,
        fill: Style,
        height: u16,
        draw: impl FnOnce(&mut Buffer, Rect),
    ) {
        self.area = area;
        let content = Rect {
            x: area.x,
            y: 0,
            width: if self.scrollbar {
                ScrollView::content_width(area, usize::from(height))
            } else {
                area.width
            },
            height,
        };
        self.content = content;
        let mut scratch = Buffer::empty(content);
        scratch.set_style(content, fill);
        draw(&mut scratch, content);
        self.view.render_buffer(buf, area, theme, &scratch, self.scrollbar);
    }

    /// The form row on screen row `row`, if it's in the scroll area.
    pub fn content_row(&self, row: u16) -> Option<u16> {
        (self.area.y..self.area.bottom())
            .contains(&row)
            .then(|| self.form_row(row))
    }

    /// `event` moved into the offscreen buffer's coordinates, where the fields were drawn: rows are offset by the
    /// scroll position, columns stay. A row above the area clamps to the form's top.
    pub fn to_content(&self, event: MouseEvent) -> MouseEvent {
        MouseEvent {
            row: self.form_row(event.row),
            ..event
        }
    }

    fn form_row(&self, row: u16) -> u16 {
        let offset = i64::try_from(self.view.offset()).unwrap_or(i64::MAX);
        let form_row = i64::from(row) - i64::from(self.area.y) + offset;
        u16::try_from(form_row.max(0)).unwrap_or(u16::MAX)
    }

    /// Where form rows `rows` are on screen as of the last draw, at the content's width, whether or not they're in
    /// view: for drawing an overlay below a field. `None` when they'd start above the screen's top.
    pub fn screen_rows(&self, rows: Range<u16>) -> Option<Rect> {
        let offset = i64::try_from(self.view.offset()).ok()?;
        let y = i64::from(self.area.y) + i64::from(rows.start) - offset;
        Some(Rect {
            x: self.content.x,
            y: u16::try_from(y).ok()?,
            width: self.content.width,
            height: rows.end.saturating_sub(rows.start),
        })
    }

    /// Whether form rows `rows` are at least partly in view on screen cell `(column, row)`.
    pub fn shows_at(&self, rows: Range<u16>, column: u16, row: u16) -> bool {
        self.area.contains((column, row).into()) && rows.contains(&self.form_row(row))
    }

    /// Scrolls as little as possible to show form rows `rows`: Textual's `scroll_to_widget` when focus moves to a
    /// field inside the area.
    pub fn reveal(&mut self, rows: Range<u16>) {
        self.view.reveal(usize::from(rows.start)..usize::from(rows.end));
    }

    /// Textual's scroll when focus moves to a field (`Screen.set_focus`): nothing while the field's `rows` are all in
    /// view; otherwise its rows with their margins, `with_margin`, are centered in the area (`scroll_to_center`), as
    /// far as the content allows, keeping their first row in view (`origin_visible`).
    pub fn center(&mut self, rows: Range<u16>, with_margin: Range<u16>) {
        let offset = self.view.offset();
        let (_, window) = self.view.last_size();
        let in_view = offset..offset + window;
        if in_view.contains(&usize::from(rows.start)) && usize::from(rows.end) <= in_view.end {
            return;
        }
        // `Region.center` and the `int(... + 0.5)` of `scroll_to_region`, which truncates toward zero.
        let middle = f64::from(with_margin.start) + f64::from(with_margin.end - with_margin.start) / 2.0;
        #[allow(clippy::cast_precision_loss)] // offsets and windows are screen-sized
        let window_middle = offset as f64 + (window / 2) as f64;
        #[allow(clippy::cast_possible_truncation)] // a difference of screen-sized rows
        let delta = (middle - window_middle + 0.5).trunc() as i64;
        let target = i64::try_from(offset).unwrap_or(i64::MAX).saturating_add(delta).max(0);
        self.view.scroll_to(usize::try_from(target).unwrap_or(0));
        let start = usize::from(with_margin.start);
        let offset = self.view.offset();
        if !(offset..offset + window).contains(&start) {
            self.view.scroll_to(start);
        }
    }

    /// A scroll key while the area has focus.
    pub fn scroll(&mut self, scroll: Scroll) {
        self.view.scroll(scroll);
    }

    /// A wheel notch over the area.
    pub fn wheel(&mut self, wheel: Wheel) {
        self.view.wheel(wheel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Field {
        Title,
        Body,
    }

    fn layout(editor: u16) -> FormLayout<Field> {
        FormLayout::new([
            Block::Label("Title"),
            Block::Field(Field::Title, 3),
            Block::Rule,
            Block::Label("Description"),
            Block::Field(Field::Body, editor),
        ])
    }

    #[test]
    fn blocks_stack_from_the_top() {
        let layout = layout(21);
        assert_eq!(layout.height(), 29);
        assert_eq!(layout.rows(Field::Title), Some(1..4));
        assert_eq!(layout.rows(Field::Body), Some(8..29));
        assert_eq!(layout.field_at(0), None);
        assert_eq!(layout.field_at(3), Some(Field::Title));
        assert_eq!(layout.field_at(5), None);
        assert_eq!(layout.field_at(8), Some(Field::Body));
        assert_eq!(
            layout.area(Field::Body, Rect::new(29, 0, 96, 29)),
            Rect::new(29, 8, 96, 21)
        );
    }

    #[test]
    fn the_scroll_area_takes_80_percent_and_the_buttons_center_in_the_rest() {
        // New issue at 160x48: a 35-row interior from row 6.
        let (scroll, buttons) = split(Rect::new(29, 6, 98, 35), 3);
        assert_eq!(scroll, Rect::new(29, 6, 98, 28));
        assert_eq!(buttons, Rect::new(29, 36, 98, 3));
        // At 100x30: 21 rows from row 4.
        let (scroll, buttons) = split(Rect::new(1, 4, 98, 21), 3);
        assert_eq!(scroll.height, 16);
        assert_eq!(buttons, Rect::new(1, 21, 98, 3));
    }

    #[test]
    fn focus_centers_a_field_out_of_view_and_keeps_its_top_in_view() {
        let theme = Theme::default();
        let mut form = FormScroll::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 48));
        // Create PR at 160x48: 59 rows of content in a 32-row area.
        form.render(&mut buf, Rect::new(31, 8, 96, 32), &theme, Style::new(), 59, |_, _| {});
        // In view already: no scroll.
        form.center(12..15, 12..16);
        assert_eq!(form.offset(), 0);
        // The buttons' block, centered, clamped to the end.
        form.center(55..58, 54..59);
        assert_eq!(form.offset(), 27);
        // The title, centered, clamped to the top (Textual's `scroll_to_center`, not a minimal scroll).
        form.center(12..15, 12..16);
        assert_eq!(form.offset(), 0);
        // The editor from the end: its middle (32.5) to the window's (offset + 16), `int(x + 0.5)`.
        form.center(55..58, 54..59);
        form.center(17..47, 17..48);
        assert_eq!(form.offset(), 17);
        // A field taller than the area keeps its top row in view.
        form.center(0..59, 0..59);
        assert_eq!(form.offset(), 0);
    }

    #[test]
    fn mouse_rows_move_by_the_scroll_offset() {
        let theme = Theme::default();
        let mut form = FormScroll::default();
        let area = Rect::new(29, 6, 98, 28);
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 48));
        form.render(&mut buf, area, &theme, Style::new(), 29, |_, content| {
            assert_eq!(content, Rect::new(29, 0, 96, 29));
        });
        assert_eq!(form.content(), Rect::new(29, 0, 96, 29));
        assert_eq!(form.content_row(6), Some(0));
        assert_eq!(form.content_row(34), None);
        form.reveal(8..29);
        assert_eq!(form.offset(), 1);
        assert_eq!(form.content_row(6), Some(1));
        let event = |row| MouseEvent {
            kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            column: 40,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert_eq!(form.to_content(event(10)).row, 5);
        assert_eq!(form.to_content(event(2)).row, 0);
        assert!(form.shows_at(8..29, 40, 13));
        assert_eq!(form.screen_rows(8..29), Some(Rect::new(29, 13, 96, 21)));
        assert_eq!(form.screen_rows(0..1), Some(Rect::new(29, 5, 96, 1)));
        assert!(!form.shows_at(8..29, 40, 12));
    }
}
