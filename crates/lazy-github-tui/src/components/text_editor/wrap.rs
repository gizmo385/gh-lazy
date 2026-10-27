//! Soft wrap as Textual does it: `textual/_wrap.py` `compute_wrap_offsets` breaks a line into rows, and the parts of
//! `WrappedDocument` the navigator and the drawing use map between locations in the text and cells on screen.
//!
//! Columns are char indices (Python's code point indices) and widths are terminal cells. Tabs expand to the next
//! multiple of [`TAB_WIDTH`], counted within the string being measured, as Textual's `get_tab_widths` counts them.

use ratatui::text::Span;

use super::Location;

/// Textual's `TextArea.indent_width`, which its tab expansion uses.
pub(super) const TAB_WIDTH: usize = 4;

/// rich's `get_character_cell_size`: C0 and C1 control characters (tabs among them) take no cells.
pub(super) fn char_cells(c: char) -> usize {
    let code = u32::from(c);
    if (code != 0 && code < 32) || (0x7f..0xa0).contains(&code) {
        return 0;
    }
    Span::raw(c.encode_utf8(&mut [0; 4]) as &str).width()
}

/// rich's `cell_len`, which counts tabs as nothing.
pub(super) fn cell_len(s: &str) -> usize {
    s.chars().map(char_cells).sum()
}

/// Python's `str.isspace`, as the `\s` of Textual's patterns matches it.
pub(super) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Textual's `get_tab_widths`: the line split after each tab into the text before the tab (without it) and the width
/// the tab expands to. Text after the last tab is a part of its own with width 0; an empty line has no parts.
fn tab_sections(line: &str) -> Vec<(&str, usize)> {
    let mut parts = Vec::new();
    let mut cells = 0;
    let mut rest = line;
    while let Some(tab) = rest.find('\t') {
        let part = &rest[..tab];
        cells += cell_len(part);
        let expansion = TAB_WIDTH - cells % TAB_WIDTH;
        cells += expansion;
        parts.push((part, expansion));
        rest = &rest[tab + 1..];
    }
    if !rest.is_empty() {
        parts.push((rest, 0));
    }
    parts
}

/// The cells `s` takes once its tabs are expanded (`cell_len(expand_tabs_inline(s))`).
pub(super) fn expanded_width(s: &str) -> usize {
    tab_sections(s).iter().map(|&(part, tab)| cell_len(part) + tab).sum()
}

/// Textual's `cell_width_to_column_index`: the column whose cell holds offset `cells`, or the line's length past it.
fn cell_width_to_column_index(line: &str, cells: usize) -> usize {
    let mut column = 0;
    let mut total = 0;
    for (part, tab) in tab_sections(line) {
        for c in part.chars() {
            total += char_cells(c);
            if total > cells {
                return column;
            }
            column += 1;
        }
        total += tab;
        if total > cells {
            return column;
        }
        column += 1;
    }
    line.chars().count()
}

/// The chunks Textual wraps by (`\S+\s*|\s+`): a word with the whitespace after it, or a run of whitespace. Each is
/// `(start, end)` in chars.
fn chunks(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = start;
        while end < chars.len() && !is_space(chars[end]) {
            end += 1;
        }
        while end < chars.len() && is_space(chars[end]) {
            end += 1;
        }
        out.push((start, end));
        start = end;
    }
    out
}

/// `compute_wrap_offsets` with folding on: the char indices to break `text` at so each row fits `width` cells.
/// Words break at whitespace, and a word wider than a row is folded across rows. `width` must not be 0.
///
/// A faithful port, quirks included: a folded tab takes the width of the next tab the fold loop hasn't looked at yet
/// (only folds advance that count), and a character wider than the row leaves an empty row before it.
pub(super) fn compute_wrap_offsets(text: &str, width: usize) -> Vec<usize> {
    let sections = tab_sections(text);
    let chars: Vec<char> = text.chars().collect();
    // The width of the tabs before each char, from Textual's prefix sum (`cumulative_widths`).
    let mut tabs_before = Vec::with_capacity(chars.len() + 1);
    let mut cumulative = 0;
    for (part, tab) in &sections {
        let count = part.chars().count() + usize::from(*tab > 0);
        tabs_before.extend(std::iter::repeat_n(cumulative, count));
        cumulative += tab;
    }
    tabs_before.push(cumulative);

    let mut breaks = Vec::new();
    let mut cell_offset = 0;
    let mut tab_index = 0;
    for (start, end) in chunks(&chars) {
        let chunk: String = chars[start..end].iter().collect();
        let chunk_width = cell_len(&chunk) + tabs_before[end] - tabs_before[start];
        if cell_offset + chunk_width <= width {
            cell_offset += chunk_width;
        } else if chunk_width > width {
            // The word fits no row, so it's folded.
            let mut rows: Vec<Vec<char>> = vec![Vec::new()];
            let mut total = 0;
            for &c in &chars[start..end] {
                let cells = if c == '\t' {
                    let cells = sections.get(tab_index).map_or(0, |&(_, tab)| tab);
                    tab_index += 1;
                    cells
                } else {
                    char_cells(c)
                };
                if total + cells > width {
                    rows.push(vec![c]);
                    total = cells;
                } else if let Some(row) = rows.last_mut() {
                    row.push(c);
                    total += cells;
                }
            }
            let mut start = start;
            let last = rows.len() - 1;
            for (i, row) in rows.iter().enumerate() {
                if start != 0 {
                    breaks.push(start);
                }
                if i == last {
                    let row_text: String = row.iter().collect();
                    cell_offset = cell_len(&row_text) + tabs_before[start + row.len()] - tabs_before[start];
                } else {
                    start += row.len();
                }
            }
        } else if cell_offset != 0 && start != 0 {
            // It fits on the next row.
            breaks.push(start);
            cell_offset = chunk_width;
        }
    }
    breaks
}

/// Which wrapped row of a line: Textual indexes sections from the front, or with `-1` for the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    /// The section at this index.
    At(usize),
    /// The line's last section.
    Last,
}

/// Every line's wrap offsets at one width, and the wrapped rows they make: Textual's `WrappedDocument`.
#[derive(Debug, Clone, Default)]
pub(super) struct WrapLayout {
    /// Each line's break offsets, in chars.
    offsets: Vec<Vec<usize>>,
    /// The wrapped row each line starts on.
    first_row: Vec<usize>,
    /// Each wrapped row's line and section.
    rows: Vec<(usize, usize)>,
}

impl WrapLayout {
    /// The layout of `lines` wrapped at `width` cells (0 for no wrapping).
    pub(super) fn new(lines: &[String], width: usize) -> Self {
        let mut layout = Self::default();
        for (index, line) in lines.iter().enumerate() {
            let offsets = if width == 0 {
                Vec::new()
            } else {
                compute_wrap_offsets(line, width)
            };
            layout.first_row.push(layout.rows.len());
            layout.rows.extend((0..=offsets.len()).map(|section| (index, section)));
            layout.offsets.push(offsets);
        }
        layout
    }

    /// The number of wrapped rows.
    pub(super) fn height(&self) -> usize {
        self.rows.len()
    }

    /// The line and section of wrapped row `y`, when there is one.
    pub(super) fn row(&self, y: usize) -> Option<(usize, usize)> {
        self.rows.get(y).copied()
    }

    /// Line `row`'s break offsets.
    pub(super) fn offsets(&self, row: usize) -> &[usize] {
        self.offsets.get(row).map_or(&[], Vec::as_slice)
    }

    /// Line `row` split at its offsets (`get_sections`). Always at least one section, maybe empty.
    pub(super) fn sections<'a>(&self, lines: &'a [String], row: usize) -> Vec<&'a str> {
        let line = lines.get(row).map_or("", String::as_str);
        let mut sections = Vec::new();
        let mut from = 0;
        for &offset in self.offsets(row) {
            let to = byte_at(line, offset);
            sections.push(&line[from..to.max(from)]);
            from = to.max(from);
        }
        sections.push(&line[from..]);
        sections
    }

    /// Where `location` is drawn, as (cells from the text's left edge, wrapped row): `location_to_offset`.
    pub(super) fn location_to_offset(&self, lines: &[String], (row, column): Location) -> (usize, usize) {
        let row = row.min(lines.len().saturating_sub(1));
        let offsets = self.offsets(row);
        let section = offsets.partition_point(|&offset| offset <= column);
        let section_start = if section == 0 { 0 } else { offsets[section - 1] };
        let text = self.sections(lines, row)[section];
        let before: String = text.chars().take(column.saturating_sub(section_start)).collect();
        (expanded_width(&before), self.first_row[row] + section)
    }

    /// The location drawn at (`x` cells, wrapped row `y`), clamped to the text: `offset_to_location`.
    pub(super) fn offset_to_location(&self, lines: &[String], x: isize, y: isize) -> Location {
        let x = usize::try_from(x).unwrap_or(0);
        let y = usize::try_from(y).unwrap_or(0);
        // A layout always has a row: an empty text is one empty line.
        let (row, section) = self.rows.get(y).or(self.rows.last()).copied().unwrap_or_default();
        (row, self.target_column(lines, row, x, Section::At(section)))
    }

    /// The column of line `row` drawn `x` cells into `section` (`get_target_document_column`). Except on the last
    /// section, a column past the section's end stays on its last char, so it doesn't land on the next row.
    pub(super) fn target_column(&self, lines: &[String], row: usize, x: usize, section: Section) -> usize {
        let sections = self.sections(lines, row);
        let last = sections.len() - 1;
        let index = match section {
            Section::At(index) => index.min(last),
            Section::Last => last,
        };
        let start: usize = sections[..index].iter().map(|s| s.chars().count()).sum();
        let target = start + cell_width_to_column_index(sections[index], x);
        if index == last {
            return target;
        }
        // An empty section that isn't the last (a fold quirk) puts it one before its start, as Python's arithmetic
        // does.
        let section_end = (start + sections[index].chars().count()).checked_sub(1);
        section_end.map_or(target.min(start).saturating_sub(1), |end| target.min(end))
    }
}

/// The byte index of char `index` in `s`, or its length past the end.
pub(super) fn byte_at(s: &str, index: usize) -> usize {
    s.char_indices().nth(index).map_or(s.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    // From Textual 8.2.5's `compute_wrap_offsets(text, width, tab_size=4)`.
    #[rstest]
    #[case::fits("hello world", 20, &[])]
    #[case::breaks_at_a_space("hello world", 8, &[6])]
    #[case::exact_fit("hello world", 11, &[])]
    #[case::trailing_spaces_stay("hello     world", 8, &[8])]
    #[case::spaces_wider_than_the_row("a          b", 5, &[5, 10])]
    #[case::folded_word("abcdefghijklmnop", 5, &[5, 10, 15])]
    #[case::folded_after_a_word("ab abcdefghij", 5, &[3, 8])]
    #[case::several_rows("The quick brown fox jumps over the lazy dog", 10, &[10, 20, 26, 35])]
    #[case::wide_chars("中文字符测试中文", 5, &[2, 4, 6])]
    #[case::wide_char_odd_width("中文字符", 3, &[1, 2, 3])]
    #[case::wide_with_spaces("ab 中文字 cd", 6, &[3, 6])]
    #[case::tabs("a\tb\tc d\te", 6, &[2, 6])]
    #[case::leading_tab("\tabc def", 7, &[1])]
    #[case::empty("", 5, &[])]
    #[case::only_spaces("      ", 3, &[3])]
    #[case::wide_chars_in_one_cell("中文字符", 1, &[1, 2, 3])]
    fn wrap_offsets_match_textual(#[case] text: &str, #[case] width: usize, #[case] offsets: &[usize]) {
        assert_eq!(compute_wrap_offsets(text, width), offsets);
    }

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_owned).collect()
    }

    #[test]
    fn locations_and_offsets_round_trip_over_wrapped_rows() {
        let lines = lines("hello world foo\nab");
        let layout = WrapLayout::new(&lines, 8);
        assert_eq!(layout.offsets(0), [6, 12]);
        assert_eq!(layout.sections(&lines, 0), ["hello ", "world ", "foo"]);
        assert_eq!(layout.height(), 4);
        assert_eq!(layout.location_to_offset(&lines, (0, 7)), (1, 1));
        assert_eq!(layout.location_to_offset(&lines, (1, 2)), (2, 3));
        assert_eq!(layout.offset_to_location(&lines, 1, 1), (0, 7));
        // Past a middle row's end stays on its last char; past the last row's end goes to the line's end.
        assert_eq!(layout.offset_to_location(&lines, 30, 0), (0, 5));
        assert_eq!(layout.offset_to_location(&lines, 30, 2), (0, 15));
        // Below the text is its last row.
        assert_eq!(layout.offset_to_location(&lines, 1, 40), (1, 1));
    }

    #[test]
    fn tabs_expand_within_what_is_measured() {
        assert_eq!(expanded_width("a\tb"), 5);
        assert_eq!(expanded_width("\t"), 4);
        assert_eq!(expanded_width("中\t"), 4);
        assert_eq!(cell_width_to_column_index("a\tb", 2), 1);
        assert_eq!(cell_width_to_column_index("a\tb", 4), 2);
        assert_eq!(cell_width_to_column_index("a\tb", 9), 3);
    }
}
