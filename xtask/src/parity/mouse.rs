//! Mouse input for the `click`, `wheel` and `move` steps: SGR 1006 reports and text targets.
//!
//! Both apps turn on any-motion reporting with SGR encoding (Textual writes `?1000h ?1003h ?1015h ?1006h`, crossterm
//! `?1000h ?1002h ?1003h ?1015h ?1006h`), so SGR is the only encoding the harness writes. Cells are the grid's 0-based
//! `[col, row]`; the reports are 1-based.

use std::num::NonZeroU16;

use super::scenario::WheelDirection;

/// One SGR mouse report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseReport {
    /// Motion with no button held, as a terminal sends it in any-motion mode.
    Motion,
    /// The left button going down.
    Press,
    /// The left button coming up.
    Release,
    /// One wheel notch. Terminals send no release for it.
    Wheel(WheelDirection),
}

/// The SGR 1006 bytes for `report` at the 0-based cell (`col`, `row`).
pub fn sgr(report: MouseReport, col: u16, row: u16) -> Vec<u8> {
    let (button, last) = match report {
        // 32 (motion) + 3 (no button).
        MouseReport::Motion => (35, 'M'),
        MouseReport::Press => (0, 'M'),
        MouseReport::Release => (0, 'm'),
        MouseReport::Wheel(WheelDirection::Up) => (64, 'M'),
        MouseReport::Wheel(WheelDirection::Down) => (65, 'M'),
        MouseReport::Wheel(WheelDirection::Left) => (66, 'M'),
        MouseReport::Wheel(WheelDirection::Right) => (67, 'M'),
    };
    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    format!("\x1b[<{button};{x};{y}{last}").into_bytes()
}

/// The first cell of the `nth` match of `text` on `screen`: rows top to bottom, matches left to right and
/// non-overlapping within a row. Rows are read as `<capture>.txt` has them, with wide-character continuation cells
/// skipped, and the match's character index is mapped back to its cell. Returns `(col, row)`.
pub fn locate(screen: &vt100::Screen, text: &str, nth: NonZeroU16) -> Option<(u16, u16)> {
    if text.is_empty() {
        return None;
    }
    let (rows, cols) = screen.size();
    let mut left = nth.get();
    for row in 0..rows.get() {
        let (line, starts) = row_text(screen, row, cols.get());
        for (byte, _) in line.match_indices(text) {
            left -= 1;
            if left == 0 {
                // A match starts on a char boundary, which `row_text` recorded.
                let col = starts.iter().find(|&&(b, _)| b == byte).map(|&(_, c)| c)?;
                return Some((col, row));
            }
        }
    }
    None
}

/// `row`'s text and, for each char in it, its byte offset and the cell it came from.
fn row_text(screen: &vt100::Screen, row: u16, cols: u16) -> (String, Vec<(usize, u16)>) {
    let mut line = String::new();
    let mut starts = Vec::new();
    for col in 0..cols {
        let contents = match screen.cell(row, col) {
            Some(cell) if cell.is_wide_continuation() => continue,
            Some(cell) if cell.has_contents() => cell.contents(),
            _ => " ",
        };
        for (i, _) in contents.char_indices() {
            starts.push((line.len() + i, col));
        }
        line.push_str(contents);
    }
    (line, starts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut p = vt100::Parser::new(NonZeroU16::new(rows).unwrap(), NonZeroU16::new(cols).unwrap(), 0);
        p.process(bytes);
        p
    }

    fn nth(n: u16) -> NonZeroU16 {
        NonZeroU16::new(n).unwrap()
    }

    #[test]
    fn encodes_each_report_one_based() {
        assert_eq!(sgr(MouseReport::Motion, 0, 0), b"\x1b[<35;1;1M");
        assert_eq!(sgr(MouseReport::Press, 9, 4), b"\x1b[<0;10;5M");
        assert_eq!(sgr(MouseReport::Release, 9, 4), b"\x1b[<0;10;5m");
        assert_eq!(sgr(MouseReport::Wheel(WheelDirection::Up), 20, 9), b"\x1b[<64;21;10M");
        assert_eq!(sgr(MouseReport::Wheel(WheelDirection::Down), 20, 9), b"\x1b[<65;21;10M");
        assert_eq!(sgr(MouseReport::Wheel(WheelDirection::Left), 20, 9), b"\x1b[<66;21;10M");
        assert_eq!(
            sgr(MouseReport::Wheel(WheelDirection::Right), 20, 9),
            b"\x1b[<67;21;10M"
        );
        // No overflow at the edge of the u16 range.
        assert_eq!(sgr(MouseReport::Press, u16::MAX, 0), b"\x1b[<0;65536;1M");
    }

    #[test]
    fn locates_the_first_match_in_reading_order() {
        let p = screen(3, 20, b"  ab\r\nxx ab ab\r\nab");
        assert_eq!(locate(p.screen(), "ab", nth(1)), Some((2, 0)));
        assert_eq!(locate(p.screen(), "x ab", nth(1)), Some((1, 1)));
    }

    #[test]
    fn locates_the_nth_match() {
        let p = screen(3, 20, b"  ab\r\nxx ab ab\r\nab");
        assert_eq!(locate(p.screen(), "ab", nth(2)), Some((3, 1)));
        assert_eq!(locate(p.screen(), "ab", nth(3)), Some((6, 1)));
        assert_eq!(locate(p.screen(), "ab", nth(4)), Some((0, 2)));
        assert_eq!(locate(p.screen(), "ab", nth(5)), None);
        // Matches don't overlap.
        let p = screen(1, 10, b"aaaa");
        assert_eq!(locate(p.screen(), "aa", nth(2)), Some((2, 0)));
        assert_eq!(locate(p.screen(), "aa", nth(3)), None);
    }

    #[test]
    fn no_match_is_none() {
        let p = screen(2, 10, b"hello");
        assert_eq!(locate(p.screen(), "world", nth(1)), None);
        assert_eq!(locate(p.screen(), "", nth(1)), None);
        // A match never spans two rows.
        let p = screen(2, 5, b"hello\r\nworld");
        assert_eq!(locate(p.screen(), "oworld", nth(1)), None);
    }

    #[test]
    fn a_match_after_a_wide_character_lands_on_its_cell() {
        // 漢 takes columns 1 and 2, so `b` is in column 3 though it's the third char.
        let p = screen(1, 10, "a漢b ★ c".as_bytes());
        assert_eq!(locate(p.screen(), "b", nth(1)), Some((3, 0)));
        assert_eq!(locate(p.screen(), "漢", nth(1)), Some((1, 0)));
        assert_eq!(locate(p.screen(), "c", nth(1)), Some((7, 0)));
    }
}
