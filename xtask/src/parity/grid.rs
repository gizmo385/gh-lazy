//! Screen dumps: the `<capture>.grid.json` format and its plain-text form, `<capture>.txt`.
//!
//! The structural check (#93) and the HTML report (#94) read these files, so the JSON shape is a contract. It is
//! documented in `parity/README.md`.

use serde::{Deserialize, Serialize};

/// The only grid format version written and read.
pub const GRID_VERSION: u32 = 1;

/// A full screen: every cell's text plus run-length styles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid {
    pub version: u32,
    pub cols: u16,
    pub rows: u16,
    pub cursor: Cursor,
    /// One entry per screen row, top to bottom.
    pub lines: Vec<Line>,
}

/// Where the cursor is, 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// One screen row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// One entry per column. A blank cell is `" "`; the continuation cell after a wide character is `""`.
    pub cells: Vec<String>,
    /// Consecutive columns with identical style, left to right, covering the whole row.
    pub runs: Vec<Run>,
}

/// A run of columns that share one style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub col: u16,
    pub len: u16,
    #[serde(flatten)]
    pub style: Style,
}

/// A cell's style. Colors are `"#rrggbb"` for truecolor, `"idx:<n>"` for an indexed color and `None` (JSON
/// `null`) for the terminal default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the attributes are independent flags in the JSON format"
)]
pub struct Style {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Style {
    fn of(cell: &vt100::Cell) -> Self {
        Self {
            fg: color(cell.fgcolor()),
            bg: color(cell.bgcolor()),
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }
}

fn color(c: vt100::Color) -> Option<String> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(n) => Some(format!("idx:{n}")),
        vt100::Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
    }
}

impl Grid {
    /// Dumps the visible screen.
    pub fn from_screen(screen: &vt100::Screen) -> Self {
        let (rows, cols) = screen.size();
        let (rows, cols) = (rows.get(), cols.get());
        let (cursor_row, cursor_col) = screen.cursor_position();
        let lines = (0..rows)
            .map(|row| {
                let mut cells = Vec::with_capacity(usize::from(cols));
                let mut runs: Vec<Run> = Vec::new();
                for col in 0..cols {
                    let (text, style) = match screen.cell(row, col) {
                        Some(cell) if cell.is_wide_continuation() => (String::new(), Style::of(cell)),
                        Some(cell) if cell.has_contents() => (cell.contents().to_owned(), Style::of(cell)),
                        Some(cell) => (" ".to_owned(), Style::of(cell)),
                        None => (" ".to_owned(), Style::default()),
                    };
                    cells.push(text);
                    match runs.last_mut() {
                        Some(run) if run.style == style => run.len += 1,
                        _ => runs.push(Run { col, len: 1, style }),
                    }
                }
                Line { cells, runs }
            })
            .collect();
        Self {
            version: GRID_VERSION,
            cols,
            rows,
            cursor: Cursor {
                row: cursor_row,
                col: cursor_col,
                visible: !screen.hide_cursor(),
            },
            lines,
        }
    }

    /// Each row's text: continuation cells skipped, trailing spaces trimmed.
    pub fn text_rows(&self) -> Vec<String> {
        self.lines.iter().map(Line::text).collect()
    }

    /// The `<capture>.txt` form: one line per screen row, each ending in `\n`.
    pub fn to_text(&self) -> String {
        self.lines.iter().fold(String::new(), |mut out, line| {
            out.push_str(&line.text());
            out.push('\n');
            out
        })
    }

    pub fn to_json(&self) -> String {
        // A `Grid` holds only strings, integers and booleans, so serializing it can't fail.
        serde_json::to_string(self).expect("grid serializes")
    }
}

impl Line {
    /// The row's text, with continuation cells skipped and trailing spaces trimmed.
    pub fn text(&self) -> String {
        let text: String = self.cells.concat();
        text.trim_end_matches(' ').to_owned()
    }
}

/// The characters on screen, rows joined by `\n`: what `wait.text`, `wait.regex` and `wait.stable_ms` look at.
/// Cheaper than a full [`Grid`] because it skips styles.
pub fn screen_text(screen: &vt100::Screen) -> String {
    let (_, cols) = screen.size();
    screen
        .rows(0, cols.get())
        .map(|row| row.trim_end_matches(' ').to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;

    fn parser(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut p = vt100::Parser::new(NonZeroU16::new(rows).unwrap(), NonZeroU16::new(cols).unwrap(), 0);
        p.process(bytes);
        p
    }

    #[test]
    fn dumps_cells_runs_and_cursor() {
        let p = parser(
            2,
            6,
            b"\x1b[1;38;2;78;191;113mab\x1b[0m c\r\n\x1b[31;44mx\x1b[0m\x1b[?25l",
        );
        let grid = Grid::from_screen(p.screen());
        assert_eq!((grid.cols, grid.rows), (6, 2));
        assert_eq!(
            grid.cursor,
            Cursor {
                row: 1,
                col: 1,
                visible: false
            }
        );
        assert_eq!(grid.lines[0].cells, ["a", "b", " ", "c", " ", " "]);
        let bold_green = Style {
            fg: Some("#4ebf71".into()),
            bold: true,
            ..Style::default()
        };
        assert_eq!(
            grid.lines[0].runs,
            [
                Run {
                    col: 0,
                    len: 2,
                    style: bold_green
                },
                Run {
                    col: 2,
                    len: 4,
                    style: Style::default()
                }
            ]
        );
        let red_on_blue = Style {
            fg: Some("idx:1".into()),
            bg: Some("idx:4".into()),
            ..Style::default()
        };
        assert_eq!(
            grid.lines[1].runs[0],
            Run {
                col: 0,
                len: 1,
                style: red_on_blue
            }
        );
        assert_eq!(grid.to_text(), "ab c\nx\n");
    }

    #[test]
    fn wide_characters_leave_an_empty_continuation_cell() {
        let p = parser(1, 5, "a漢b".as_bytes());
        let grid = Grid::from_screen(p.screen());
        assert_eq!(grid.lines[0].cells, ["a", "漢", "", "b", " "]);
        assert_eq!(grid.lines[0].text(), "a漢b");
        assert_eq!(screen_text(p.screen()), "a漢b");
    }

    #[test]
    fn json_round_trips_with_flattened_styles() {
        let p = parser(1, 2, b"\x1b[7mz");
        let grid = Grid::from_screen(p.screen());
        let json = grid.to_json();
        assert!(json.starts_with(r#"{"version":1,"cols":2,"rows":1,"cursor":{"row":0,"col":1,"visible":true},"#));
        assert!(json.contains(
            r#"{"col":0,"len":1,"fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":true}"#
        ));
        assert_eq!(serde_json::from_str::<Grid>(&json).unwrap(), grid);
    }
}
