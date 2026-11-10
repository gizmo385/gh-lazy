//! A compose sheet's one-row fields and its heading row: the labels in a column on the left, each field's input to
//! their right (decision compose-dialogs, D4 a).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::theme::{Paint, Theme};

/// The blank columns between the labels' column and the fields.
const LABEL_GAP: u16 = 2;

/// How a field's label is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelState {
    /// In the sheet's muted text color.
    Plain,
    /// The field has focus: in the accent color.
    Focused,
    /// The field's last check failed (a missing title): in the error color.
    Invalid,
}

/// Where the fields start, in columns from the left of the parts: the widest of `labels`, then a gap.
pub fn label_column<'a>(labels: impl IntoIterator<Item = &'a str>) -> u16 {
    let widest = labels
        .into_iter()
        .map(|label| Span::raw(label).width())
        .max()
        .unwrap_or(0);
    u16::try_from(widest).unwrap_or(u16::MAX).saturating_add(LABEL_GAP)
}

/// Draws `label` at the start of `row` over `under` (the sheet's background), cut at `column`, where the field
/// starts.
pub fn render_label(
    buf: &mut Buffer,
    row: Rect,
    column: u16,
    label: &str,
    state: LabelState,
    theme: &Theme,
    under: Paint,
) {
    let roles = theme.roles();
    let color = match state {
        LabelState::Plain => roles.text.muted,
        LabelState::Focused => roles.text.accent,
        LabelState::Invalid => roles.text.error,
    };
    let width = column.min(row.width);
    buf.set_stringn(
        row.x,
        row.y,
        label,
        usize::from(width),
        Style::new().fg(theme.color(color, under)),
    );
}

/// The cells right of the labels' column in `row`, where its field goes.
pub fn field_area(row: Rect, column: u16) -> Rect {
    let column = column.min(row.width);
    Rect {
        x: row.x + column,
        width: row.width - column,
        ..row
    }
}

/// The heading row: `heading` in `text`, and while a request is in flight `tail` (`Saving…`) dim at the row's end.
pub fn render_heading(buf: &mut Buffer, row: Rect, heading: &str, tail: Option<&str>, text: Style, dim: Style) {
    let tail_width = tail.map_or(0, |tail| u16::try_from(Span::raw(tail).width()).unwrap_or(u16::MAX));
    let room = match tail {
        Some(_) => row.width.saturating_sub(tail_width + 1),
        None => row.width,
    };
    buf.set_span(row.x, row.y, &Span::styled(heading, text), room);
    if let Some(tail) = tail {
        buf.set_span(
            row.right().saturating_sub(tail_width),
            row.y,
            &Span::styled(tail, dim),
            tail_width,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buf: &Buffer) -> String {
        (0..buf.area.width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    #[test]
    fn the_labels_column_fits_the_widest_label_and_a_gap() {
        assert_eq!(label_column(["Title", "Status"]), 8);
        assert_eq!(label_column(["Title"]), 7);
        let row = Rect::new(2, 5, 40, 1);
        assert_eq!(field_area(row, 8), Rect::new(10, 5, 32, 1));
    }

    #[test]
    fn the_heading_keeps_its_tail_at_the_row_s_end() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        render_heading(
            &mut buf,
            Rect::new(0, 0, 30, 1),
            "Edit issue #12",
            Some("Saving…"),
            Style::new(),
            Style::new(),
        );
        assert_eq!(row(&buf), "Edit issue #12         Saving…");
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        render_heading(
            &mut buf,
            Rect::new(0, 0, 30, 1),
            "New issue",
            None,
            Style::new(),
            Style::new(),
        );
        assert_eq!(row(&buf).trim_end(), "New issue");
    }
}
