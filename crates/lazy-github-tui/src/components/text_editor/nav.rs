//! Where each movement key takes the cursor: Textual's `DocumentNavigator`, and the word rules of `TextArea`'s
//! `get_cursor_word_*_location` and `action_delete_word_right`. Every function is pure: it reads the lines and the wrap
//! layout and returns a location; the editor moves the cursor or deletes up to it.

use super::Location;
use super::wrap::{Section, WrapLayout, cell_len, is_space};
use crate::components::text_input::is_word;

/// The text and its wrapped rows, read-only.
#[derive(Clone, Copy)]
pub(super) struct Nav<'a> {
    pub(super) lines: &'a [String],
    pub(super) wrap: &'a WrapLayout,
}

impl Nav<'_> {
    fn len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |line| line.chars().count())
    }

    fn last_row(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    /// The location moved into the text: the row clamped to the lines, the column to its row (`clamp_visitable`).
    pub(super) fn clamp(&self, (row, column): Location) -> Location {
        let row = row.min(self.last_row());
        (row, column.min(self.len(row)))
    }

    /// The end of the text.
    pub(super) fn end_of_text(&self) -> Location {
        (self.last_row(), self.len(self.last_row()))
    }

    pub(super) fn is_end(&self, (row, column): Location) -> bool {
        row == self.last_row() && column == self.len(row)
    }

    /// `get_location_left`: one char back, or the end of the line above.
    pub(super) fn left(&self, (row, column): Location) -> Location {
        match (row, column) {
            (0, 0) => (0, 0),
            (row, 0) => (row - 1, self.len(row - 1)),
            (row, column) => (row, column - 1),
        }
    }

    /// `get_location_right`: one char on, or the start of the line below.
    pub(super) fn right(&self, location: Location) -> Location {
        let (row, column) = location;
        if self.is_end(location) {
            location
        } else if column == self.len(row) {
            (row + 1, 0)
        } else {
            (row, column + 1)
        }
    }

    /// Which section of its line `column` is in, and how many cells into it (tabs count nothing, as in Textual's
    /// `cell_len`). `above` uses `bisect_right` for the section; `below` uses `bisect`, which is the same.
    fn visual_offset(&self, (row, column): Location) -> (usize, usize) {
        let offsets = self.wrap.offsets(row);
        let section = offsets.partition_point(|&offset| offset <= column);
        let start = if section == 0 { 0 } else { offsets[section - 1] };
        let text = self.wrap.sections(self.lines, row)[section];
        let before: String = text.chars().take(column.saturating_sub(start)).collect();
        (section, cell_len(&before))
    }

    /// `get_location_above`: the wrapped row above, aiming for `last_x_offset` or the cursor's own offset, whichever
    /// is further right. From the first row, the start of the text.
    pub(super) fn above(&self, location: Location, last_x_offset: usize) -> Location {
        let (row, _) = location;
        let (section, offset) = self.visual_offset(location);
        let target = offset.max(last_x_offset);
        if section > 0 {
            return (
                row,
                self.wrap
                    .target_column(self.lines, row, target, Section::At(section - 1)),
            );
        }
        if row == 0 {
            return (0, 0);
        }
        (
            row - 1,
            self.wrap.target_column(self.lines, row - 1, target, Section::Last),
        )
    }

    /// `get_location_below`: the wrapped row below, as [`Self::above`]. From the last row, the end of the text.
    pub(super) fn below(&self, location: Location, last_x_offset: usize) -> Location {
        let (row, _) = location;
        let (section, offset) = self.visual_offset(location);
        let target = offset.max(last_x_offset);
        if section < self.wrap.offsets(row).len() {
            return (
                row,
                self.wrap
                    .target_column(self.lines, row, target, Section::At(section + 1)),
            );
        }
        if row == self.last_row() {
            return (row, self.len(row));
        }
        (
            row + 1,
            self.wrap.target_column(self.lines, row + 1, target, Section::At(0)),
        )
    }

    /// `get_location_end`: the char before the next wrap point, or the end of the line.
    pub(super) fn line_end(&self, (row, column): Location) -> Location {
        let offsets = self.wrap.offsets(row);
        let next = offsets.partition_point(|&offset| offset <= column);
        match offsets.get(next) {
            Some(&offset) => (row, offset - 1),
            None => (row, self.len(row)),
        }
    }

    /// `get_location_home`: the start of the wrapped row. On a line that doesn't wrap, with `smart`, the first
    /// non-space char, unless the cursor is already there or between it and the line's start, which goes to 0.
    pub(super) fn line_start(&self, (row, column): Location, smart: bool) -> Location {
        let offsets = self.wrap.offsets(row);
        if !offsets.is_empty() {
            let previous = offsets.partition_point(|&offset| offset <= column);
            return (row, if previous == 0 { 0 } else { offsets[previous - 1] });
        }
        if smart {
            let line = self.lines.get(row).map_or("", String::as_str);
            let first = line.chars().position(|c| !is_space(c)).unwrap_or(0);
            if column == 0 || column > first {
                return (row, first);
            }
        }
        (row, 0)
    }

    /// `get_location_at_y_offset`: the location drawn `rows` wrapped rows above (negative) or below the cursor.
    pub(super) fn at_row_offset(&self, location: Location, rows: isize) -> Location {
        let (x, y) = self.wrap.location_to_offset(self.lines, location);
        let x = isize::try_from(x).unwrap_or(isize::MAX);
        let y = isize::try_from(y).unwrap_or(isize::MAX);
        self.wrap.offset_to_location(self.lines, x, y.saturating_add(rows))
    }

    /// `get_cursor_word_left_location`: the end of the line above from a line's start; otherwise the last word
    /// boundary before the cursor, ignoring the whitespace right before it, or the line's start.
    pub(super) fn word_left(&self, (row, column): Location) -> Location {
        if row > 0 && column == 0 {
            return (row - 1, self.len(row - 1));
        }
        let chars: Vec<char> = self.chars(row).take(column).collect();
        let end = chars.iter().rposition(|&c| !is_space(c)).map_or(0, |i| i + 1);
        (row, boundaries(&chars[..end]).last().unwrap_or(0))
    }

    /// `get_cursor_word_right_location`: the start of the line below from a line's end (but the last); otherwise the
    /// first word boundary after the whitespace at the cursor, or the line's end.
    pub(super) fn word_right(&self, (row, column): Location) -> Location {
        let len = self.len(row);
        if row < self.last_row() && column == len {
            return (row + 1, 0);
        }
        let chars: Vec<char> = self.chars(row).skip(column).collect();
        let stripped = chars.iter().position(|&c| !is_space(c)).unwrap_or(chars.len());
        match boundaries(&chars[stripped..]).next() {
            Some(boundary) => (row, column + stripped + boundary),
            None => (row, len),
        }
    }

    /// Where `ctrl+f` deletes to (`action_delete_word_right`): the first word boundary after the cursor, without
    /// skipping whitespace; at a line's end the start of the next line; otherwise the line's end.
    pub(super) fn delete_word_right_target(&self, (row, column): Location) -> Location {
        let len = self.len(row);
        let chars: Vec<char> = self.chars(row).skip(column).collect();
        match boundaries(&chars).next() {
            Some(boundary) => (row, column + boundary),
            None if row < self.last_row() && column == len => (row + 1, 0),
            None => (row, len),
        }
    }

    fn chars(&self, row: usize) -> std::str::Chars<'_> {
        self.lines.get(row).map_or("", String::as_str).chars()
    }
}

/// The positions Textual's `_word_pattern` (`(?<=\W)(?=\w)|(?<=\w)(?=\W)`) matches in `chars`: every index between
/// a word char and a non-word char, in order. Never 0 or the end, which have nothing on one side.
fn boundaries(chars: &[char]) -> impl DoubleEndedIterator<Item = usize> + '_ {
    (1..chars.len()).filter(|&i| is_word(chars[i - 1]) != is_word(chars[i]))
}
