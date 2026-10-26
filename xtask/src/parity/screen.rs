//! The `.screen` format for recorded Python references (#150), and the `fake-gh.log` normalization stored next to it.
//!
//! A `.screen` file is a [`Grid`] written one line per screen row, so reference changes read as text diffs. It is
//! lossless: [`parse`] of [`render`]'s output is the same `Grid`.
//!
//! ```text
//! lazy-github-screen 1
//! size 8x2
//! cursor 1 4 hidden
//! --- text
//! ┌ hi ──┐
//! a漢b
//! --- styles
//! aaaaaaaa
//! --- legend
//! . bg=#121212
//! a fg=#4ebf71 bg=#121212 bold
//! --- cells
//! 1 ["a","漢","","b"," "," "," "," "]
//! ```
//!
//! - The header gives the format version, the size and the 0-based cursor position.
//! - `--- text` is exactly `rows` lines, each row's [`Line::text`] (the `<capture>.txt` row). Because the parser
//!   knows the count from `size`, a row that happens to read `--- styles` is still text.
//! - `--- styles` holds one symbol per column. `.` is the most common style (ties go to the first seen in row-major
//!   order) and the others get [`SYMBOLS`] in order of first appearance. Trailing `.` is trimmed and trailing empty
//!   rows are dropped, so a style row may be shorter than `cols`, or missing, but never longer.
//! - `--- legend` has one `<symbol> <words>` line per symbol: `fg=<color>`, `bg=<color>`, `bold`, `italic`,
//!   `underline`, `inverse`, in that order, or `default` for none. Colors are as in `grid.json` (`#rrggbb` or
//!   `idx:<n>`); a terminal-default color is left out.
//! - `--- cells` is present only when some row's text can't be split back into one `char` per cell (a wide
//!   character, its `""` continuation, or a multi-codepoint cell). Each line is `<row> <JSON cells array>`, with
//!   0-based rows in ascending order.

use std::fmt::Display;

use super::grid::{Cursor, GRID_VERSION, Grid, Line, Run, Style};
use super::{HarnessError, Result};

/// The `.screen` format version, written in the header and the only one [`parse`] accepts.
pub const SCREEN_VERSION: u32 = 1;

/// The symbol of the most common style on a screen.
pub const DEFAULT_SYMBOL: char = '.';

/// The symbols for the other styles, assigned in this order. Never `-` (section headers start with it) or a space.
pub const SYMBOLS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!#$%&*+/:;<=>?@^_~";

const MAGIC: &str = "lazy-github-screen";
const TEXT: &str = "--- text";
const STYLES: &str = "--- styles";
const LEGEND: &str = "--- legend";
const CELLS: &str = "--- cells";

/// Writes `grid` as a `.screen` file. Fails if the grid is inconsistent (rows, cells or runs that don't match its
/// size), has more distinct styles than there are symbols, or for any other reason wouldn't parse back to `grid`.
pub fn render(grid: &Grid) -> Result<String> {
    let styles = cell_styles(grid)?;

    // Distinct styles in order of first appearance, with how many cells use each. A screen has a few dozen at most,
    // so a linear search is cheaper than hashing.
    let mut seen: Vec<(&Style, usize)> = Vec::new();
    for style in styles.iter().flatten() {
        match seen.iter_mut().find(|(s, _)| *s == *style) {
            Some((_, n)) => *n += 1,
            None => seen.push((style, 1)),
        }
    }
    let max = seen.iter().map(|&(_, n)| n).max().unwrap_or(0);
    let common = seen.iter().position(|&(_, n)| n == max);
    let others = seen.len().saturating_sub(1);
    if others > SYMBOLS.len() {
        return Err(HarnessError(format!(
            "the screen has {} distinct styles; a .screen file holds at most {}",
            seen.len(),
            SYMBOLS.len() + 1
        )));
    }
    let mut extra = SYMBOLS.chars();
    let legend: Vec<(char, &Style)> = seen
        .iter()
        .enumerate()
        .map(|(i, &(style, _))| {
            // `others <= SYMBOLS.len()` was checked above, so `extra` can't run out.
            let symbol = if Some(i) == common {
                DEFAULT_SYMBOL
            } else {
                extra.next().unwrap_or('?')
            };
            (symbol, style)
        })
        .collect();
    let symbol_of = |style: &Style| {
        legend
            .iter()
            .find(|(_, s)| *s == style)
            .map_or(DEFAULT_SYMBOL, |&(c, _)| c)
    };

    let mut out = format!(
        "{MAGIC} {SCREEN_VERSION}\nsize {}x{}\ncursor {} {} {}\n{TEXT}\n",
        grid.cols,
        grid.rows,
        grid.cursor.row,
        grid.cursor.col,
        if grid.cursor.visible { "visible" } else { "hidden" },
    );
    for line in &grid.lines {
        out.push_str(&line.text());
        out.push('\n');
    }

    out.push_str(STYLES);
    out.push('\n');
    let mut style_rows: Vec<String> = styles
        .iter()
        .map(|row| {
            let row: String = row.iter().map(|s| symbol_of(s)).collect();
            row.trim_end_matches(DEFAULT_SYMBOL).to_owned()
        })
        .collect();
    while style_rows.last().is_some_and(String::is_empty) {
        style_rows.pop();
    }
    for row in &style_rows {
        out.push_str(row);
        out.push('\n');
    }

    out.push_str(LEGEND);
    out.push('\n');
    let mut legend = legend;
    // `.` first, then the others in symbol order, which is also their order of first appearance.
    legend.sort_by_key(|&(c, _)| (c != DEFAULT_SYMBOL, SYMBOLS.find(c)));
    for (symbol, style) in legend {
        out.push(symbol);
        out.push(' ');
        out.push_str(&style_words(style));
        out.push('\n');
    }

    let mut wrote_cells = false;
    for (r, line) in grid.lines.iter().enumerate() {
        if split_text(&line.text(), line.cells.len()) != line.cells {
            if !wrote_cells {
                out.push_str(CELLS);
                out.push('\n');
                wrote_cells = true;
            }
            // A `Vec<String>` always serializes.
            let json = serde_json::to_string(&line.cells).unwrap_or_default();
            out.push_str(&r.to_string());
            out.push(' ');
            out.push_str(&json);
            out.push('\n');
        }
    }

    match parse(&out) {
        Ok(back) if back == *grid => Ok(out),
        Ok(back) => Err(HarnessError(format!(
            "the screen doesn't round-trip through the .screen format: {}",
            first_difference(grid, &back)
        ))),
        Err(e) => Err(HarnessError(format!(
            "the screen doesn't round-trip through the .screen format: {e}"
        ))),
    }
}

/// Reads a `.screen` file. Errors name the 1-based line they're about.
pub fn parse(text: &str) -> Result<Grid> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut reader = Reader {
        lines: body.split('\n').collect(),
        next: 0,
    };
    let (cols, rows, cursor) = parse_header(&mut reader)?;

    reader.header(TEXT)?;
    let mut texts: Vec<(usize, &str)> = Vec::with_capacity(usize::from(rows));
    for i in 0..rows {
        let Some(line) = reader.take() else {
            return Err(at(
                reader.line_number(),
                format!("expected {rows} text rows, found {i}"),
            ));
        };
        texts.push(line);
    }

    reader.header(STYLES)?;
    let mut style_rows: Vec<(usize, Vec<char>)> = Vec::new();
    while let Some((n, line)) = reader.take_until(LEGEND)? {
        if style_rows.len() == usize::from(rows) {
            return Err(at(n, format!("more style rows than the screen's {rows} rows")));
        }
        let symbols: Vec<char> = line.chars().collect();
        if symbols.len() > usize::from(cols) {
            return Err(at(
                n,
                format!(
                    "style row has {} symbols, more than the screen's {cols} columns",
                    symbols.len()
                ),
            ));
        }
        style_rows.push((n, symbols));
    }

    let legend = parse_legend(&mut reader)?;
    let mut cells_rows = parse_cells(&mut reader, cols, rows)?;

    let lines = texts
        .iter()
        .enumerate()
        .map(|(r, &text)| {
            let cells = row_cells(r, text, cells_rows[r].take(), cols)?;
            let runs = row_runs(style_rows.get(r), &legend, cols)?;
            Ok(Line { cells, runs })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Grid {
        version: GRID_VERSION,
        cols,
        rows,
        cursor,
        lines,
    })
}

/// The `lazy-github-screen`, `size` and `cursor` lines: `(cols, rows, cursor)`.
fn parse_header(reader: &mut Reader<'_>) -> Result<(u16, u16, Cursor)> {
    let (n, header) = reader.expect("the header")?;
    let version = header
        .strip_prefix(MAGIC)
        .and_then(|v| v.strip_prefix(' '))
        .ok_or_else(|| at(n, format!("expected `{MAGIC} {SCREEN_VERSION}`, found {header:?}")))?;
    if version != SCREEN_VERSION.to_string() {
        return Err(at(
            n,
            format!("unsupported .screen version {version:?}; this build reads version {SCREEN_VERSION}"),
        ));
    }

    let (n, size) = reader.expect("`size`")?;
    let (cols, rows) = size
        .strip_prefix("size ")
        .and_then(|s| s.split_once('x'))
        .and_then(|(c, r)| Some((number(c)?, number(r)?)))
        .ok_or_else(|| at(n, format!("expected `size <cols>x<rows>`, found {size:?}")))?;

    let (n, cursor) = reader.expect("`cursor`")?;
    let cursor = parse_cursor(cursor).ok_or_else(|| {
        at(
            n,
            format!("expected `cursor <row> <col> visible|hidden`, found {cursor:?}"),
        )
    })?;
    Ok((cols, rows, cursor))
}

/// The legend section, header included.
struct Legend {
    /// The `--- legend` line, which an implied `.` missing from the legend is reported against.
    line: usize,
    entries: Vec<(char, Style)>,
}

fn parse_legend(reader: &mut Reader<'_>) -> Result<Legend> {
    let line = reader.header(LEGEND)?;
    let mut entries: Vec<(char, Style)> = Vec::new();
    while let Some((n, text)) = reader.take_until(CELLS)? {
        let (symbol, words) = text
            .split_once(' ')
            .ok_or_else(|| at(n, format!("expected `<symbol> <style>`, found {text:?}")))?;
        let mut chars = symbol.chars();
        let symbol = match (chars.next(), chars.next()) {
            (Some(c), None) if c == DEFAULT_SYMBOL || SYMBOLS.contains(c) => c,
            _ => return Err(at(n, format!("{symbol:?} is not a style symbol"))),
        };
        if entries.iter().any(|&(c, _)| c == symbol) {
            return Err(at(n, format!("symbol `{symbol}` is defined twice")));
        }
        let style = parse_style(words).map_err(|e| at(n, e))?;
        entries.push((symbol, style));
    }
    Ok(Legend { line, entries })
}

/// The optional `--- cells` section: each listed row's cells.
fn parse_cells(reader: &mut Reader<'_>, cols: u16, rows: u16) -> Result<Vec<Option<Vec<String>>>> {
    let mut cells_rows = vec![None; usize::from(rows)];
    if reader.peek().is_none() {
        return Ok(cells_rows);
    }
    reader.header(CELLS)?;
    let mut last: Option<usize> = None;
    while let Some((n, line)) = reader.take() {
        let (row, json) = line
            .split_once(' ')
            .ok_or_else(|| at(n, format!("expected `<row> <JSON cells array>`, found {line:?}")))?;
        let row = number(row)
            .map(usize::from)
            .filter(|&r| r < usize::from(rows))
            .ok_or_else(|| at(n, format!("{row:?} is not a row number below {rows}")))?;
        if last.is_some_and(|l| row <= l) {
            return Err(at(n, format!("row {row} is out of order or repeated")));
        }
        last = Some(row);
        let cells: Vec<String> = serde_json::from_str(json)
            .map_err(|e| at(n, format!("row {row}'s cells aren't a JSON string array: {e}")))?;
        if cells.len() != usize::from(cols) {
            return Err(at(
                n,
                format!("row {row} has {} cells; the screen has {cols} columns", cells.len()),
            ));
        }
        cells_rows[row] = Some(cells);
    }
    Ok(cells_rows)
}

/// Row `r`'s cells: its `--- cells` entry if it has one (which must agree with the text), otherwise one per `char`.
fn row_cells(r: usize, (n, text): (usize, &str), entry: Option<Vec<String>>, cols: u16) -> Result<Vec<String>> {
    if let Some(cells) = entry {
        if cells.concat().trim_end_matches(' ') != text {
            return Err(at(n, format!("text row {r} doesn't match its `--- cells` entry")));
        }
        return Ok(cells);
    }
    let cells = split_text(text, usize::from(cols));
    if cells.len() > usize::from(cols) {
        return Err(at(
            n,
            format!(
                "text row {r} has {} characters, more than the screen's {cols} columns",
                cells.len()
            ),
        ));
    }
    Ok(cells)
}

/// A row's runs from its style row (missing when it was trimmed), padded with `.`.
fn row_runs(style_row: Option<&(usize, Vec<char>)>, legend: &Legend, cols: u16) -> Result<Vec<Run>> {
    let symbols = style_row.map(|(_, s)| s.as_slice()).unwrap_or_default();
    let mut runs: Vec<Run> = Vec::new();
    for col in 0..cols {
        let explicit = symbols.get(usize::from(col)).copied();
        let symbol = explicit.unwrap_or(DEFAULT_SYMBOL);
        let Some((_, style)) = legend.entries.iter().find(|&&(c, _)| c == symbol) else {
            // An explicit symbol is the style row's fault; an implied `.` is the legend's.
            let n = match (style_row, explicit) {
                (Some(&(n, _)), Some(_)) => n,
                _ => legend.line,
            };
            return Err(at(n, format!("symbol `{symbol}` is not in the legend")));
        };
        match runs.last_mut() {
            Some(run) if run.style == *style => run.len += 1,
            _ => runs.push(Run {
                col,
                len: 1,
                style: style.clone(),
            }),
        }
    }
    Ok(runs)
}

/// Turns a `fake-gh.log` into `requests.jsonl`: every `seq` set to 0, exact duplicate lines removed and the rest
/// sorted by their text, so two runs whose concurrent requests were logged in a different order compare equal.
/// Blank lines are dropped; lines that aren't JSON objects are kept as they are (`RequestLog::parse` skips them).
pub fn normalize_log(text: &str) -> String {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::Object(mut entry)) => {
                if let Some(seq) = entry.get_mut("seq") {
                    *seq = 0.into();
                }
                // A `Value` always serializes.
                serde_json::to_string(&entry).unwrap_or_else(|_| line.to_owned())
            }
            _ => line.to_owned(),
        })
        .collect();
    lines.sort();
    lines.dedup();
    lines.into_iter().fold(String::new(), |mut out, line| {
        out.push_str(&line);
        out.push('\n');
        out
    })
}

/// Each cell's style, row by row. Fails if the grid's rows, cells or runs don't match its size.
fn cell_styles(grid: &Grid) -> Result<Vec<Vec<&Style>>> {
    let cols = usize::from(grid.cols);
    if grid.lines.len() != usize::from(grid.rows) {
        return Err(HarnessError(format!(
            "the grid has {} lines but {} rows",
            grid.lines.len(),
            grid.rows
        )));
    }
    grid.lines
        .iter()
        .enumerate()
        .map(|(r, line)| {
            if line.cells.len() != cols {
                return Err(HarnessError(format!(
                    "row {r} has {} cells but the grid has {cols} columns",
                    line.cells.len()
                )));
            }
            let mut styles = Vec::with_capacity(cols);
            for run in &line.runs {
                if usize::from(run.col) != styles.len() {
                    return Err(HarnessError(format!(
                        "row {r}: a run starts at column {} instead of {}",
                        run.col,
                        styles.len()
                    )));
                }
                styles.extend(std::iter::repeat_n(&run.style, usize::from(run.len)));
            }
            if styles.len() != cols {
                return Err(HarnessError(format!(
                    "row {r}: runs cover {} columns, not {cols}",
                    styles.len()
                )));
            }
            Ok(styles)
        })
        .collect()
}

/// A row's cells as its text implies: one per `char`, padded with blanks to `cols`.
fn split_text(text: &str, cols: usize) -> Vec<String> {
    let mut cells: Vec<String> = text.chars().map(String::from).collect();
    if cells.len() < cols {
        cells.resize(cols, " ".to_owned());
    }
    cells
}

fn style_words(style: &Style) -> String {
    let mut words: Vec<String> = Vec::new();
    if let Some(fg) = &style.fg {
        words.push(format!("fg={fg}"));
    }
    if let Some(bg) = &style.bg {
        words.push(format!("bg={bg}"));
    }
    for (on, word) in [
        (style.bold, "bold"),
        (style.italic, "italic"),
        (style.underline, "underline"),
        (style.inverse, "inverse"),
    ] {
        if on {
            words.push(word.to_owned());
        }
    }
    if words.is_empty() {
        "default".to_owned()
    } else {
        words.join(" ")
    }
}

fn parse_style(words: &str) -> std::result::Result<Style, String> {
    let mut style = Style::default();
    if words == "default" {
        return Ok(style);
    }
    // Each word's position in the canonical order; they must strictly increase.
    let mut last = 0;
    for word in words.split(' ') {
        let rank = match word.split_once('=') {
            Some(("fg", c)) => {
                style.fg = Some(parse_color(c)?);
                1
            }
            Some(("bg", c)) => {
                style.bg = Some(parse_color(c)?);
                2
            }
            None if word == "bold" => {
                style.bold = true;
                3
            }
            None if word == "italic" => {
                style.italic = true;
                4
            }
            None if word == "underline" => {
                style.underline = true;
                5
            }
            None if word == "inverse" => {
                style.inverse = true;
                6
            }
            _ => return Err(format!("unknown legend word {word:?}")),
        };
        if rank <= last {
            return Err(format!(
                "legend word {word:?} is repeated or out of order (fg, bg, bold, italic, underline, inverse)"
            ));
        }
        last = rank;
    }
    Ok(style)
}

/// Accepts colors exactly as `grid.json` writes them.
fn parse_color(c: &str) -> std::result::Result<String, String> {
    let hex = c
        .strip_prefix('#')
        .is_some_and(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    let idx = c
        .strip_prefix("idx:")
        .and_then(|n| n.parse::<u8>().ok().filter(|v| v.to_string() == n))
        .is_some();
    if hex || idx {
        Ok(c.to_owned())
    } else {
        Err(format!("{c:?} is not a color (`#rrggbb` or `idx:<n>`)"))
    }
}

fn parse_cursor(line: &str) -> Option<Cursor> {
    let mut words = line.strip_prefix("cursor ")?.split(' ');
    let row = number(words.next()?)?;
    let col = number(words.next()?)?;
    let visible = match words.next()? {
        "visible" => true,
        "hidden" => false,
        _ => return None,
    };
    words.next().is_none().then_some(Cursor { row, col, visible })
}

/// A plain decimal `u16`: no sign, no leading zeros.
fn number(s: &str) -> Option<u16> {
    s.parse().ok().filter(|n: &u16| n.to_string() == s)
}

fn at(line: usize, message: impl Display) -> HarnessError {
    HarnessError(format!("line {line}: {message}"))
}

fn first_difference(want: &Grid, got: &Grid) -> String {
    if (want.version, want.cols, want.rows, want.cursor) != (got.version, got.cols, got.rows, got.cursor) {
        return "the header differs".to_owned();
    }
    want.lines
        .iter()
        .zip(&got.lines)
        .position(|(a, b)| a != b)
        .map_or_else(|| "the rows differ".to_owned(), |r| format!("row {r} differs"))
}

/// The file's lines with 1-based numbers.
struct Reader<'a> {
    lines: Vec<&'a str>,
    next: usize,
}

impl<'a> Reader<'a> {
    /// The 1-based number of the next line, or one past the end.
    fn line_number(&self) -> usize {
        self.next + 1
    }

    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.next).copied()
    }

    fn take(&mut self) -> Option<(usize, &'a str)> {
        let line = self.peek()?;
        self.next += 1;
        Some((self.next, line))
    }

    /// The next line of a section that ends at the header `stop` or the end of the file. Any other header is an
    /// error: no style or legend line starts with `-`.
    fn take_until(&mut self, stop: &str) -> Result<Option<(usize, &'a str)>> {
        match self.peek() {
            None => Ok(None),
            Some(l) if l == stop => Ok(None),
            Some(l) if l.starts_with("---") => Err(at(self.line_number(), format!("expected `{stop}`, found {l:?}"))),
            Some(_) => Ok(self.take()),
        }
    }

    fn expect(&mut self, what: &str) -> Result<(usize, &'a str)> {
        let n = self.line_number();
        self.take()
            .ok_or_else(|| at(n, format!("expected {what}, found the end of the file")))
    }

    /// Consumes the section header `name` and returns its line number.
    fn header(&mut self, name: &str) -> Result<usize> {
        let (n, line) = self.expect(&format!("`{name}`"))?;
        if line == name {
            Ok(n)
        } else {
            Err(at(n, format!("expected `{name}`, found {line:?}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::test_grid;
    use fake_gh::log::RequestLog;

    fn fg(color: &str) -> Style {
        Style {
            fg: Some(color.into()),
            ..Style::default()
        }
    }

    /// A row from its cells and one style per cell.
    fn line(cells: &[&str], styles: &[Style]) -> Line {
        assert_eq!(cells.len(), styles.len());
        let mut runs: Vec<Run> = Vec::new();
        for (col, style) in styles.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if run.style == *style => run.len += 1,
                _ => runs.push(Run {
                    col: u16::try_from(col).unwrap(),
                    len: 1,
                    style: style.clone(),
                }),
            }
        }
        Line {
            cells: cells.iter().map(|&c| c.to_owned()).collect(),
            runs,
        }
    }

    /// A row of one-`char` cells from `text`, padded to `cols`, all in `style`.
    fn plain(text: &str, cols: usize, style: &Style) -> Line {
        let cells = split_text(text, cols);
        let cells: Vec<&str> = cells.iter().map(String::as_str).collect();
        line(&cells, &vec![style.clone(); cols])
    }

    fn grid(lines: Vec<Line>, cursor: Cursor) -> Grid {
        Grid {
            version: GRID_VERSION,
            cols: u16::try_from(lines[0].cells.len()).unwrap(),
            rows: u16::try_from(lines.len()).unwrap(),
            cursor,
            lines,
        }
    }

    const HIDDEN: Cursor = Cursor {
        row: 0,
        col: 0,
        visible: false,
    };

    fn round_trip(grid: &Grid) -> String {
        let text = render(grid).unwrap();
        assert_eq!(parse(&text).unwrap(), *grid, "{text}");
        text
    }

    fn error(text: &str) -> String {
        parse(text).unwrap_err().to_string()
    }

    #[test]
    fn python_startup_grids_round_trip() {
        for name in ["python/startup-160x48.grid.json", "python/startup-100x30.grid.json"] {
            round_trip(&test_grid::load(name));
        }
    }

    #[test]
    fn startup_screen_is_under_thirty_percent_of_its_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/grids/python/startup-160x48.grid.json");
        let json = std::fs::read_to_string(path).unwrap();
        let screen = render(&serde_json::from_str(&json).unwrap()).unwrap();
        // 18.0 KiB against 64 KiB (28%): half of it is the text, whose box-drawing characters take 3 bytes each.
        // Across all 111 Python captures of a full run the ratio is 25.6%.
        assert!(
            screen.len() * 10 < json.len() * 3,
            "{} bytes against {}",
            screen.len(),
            json.len()
        );
    }

    #[test]
    fn wide_and_multi_codepoint_cells_go_in_the_cells_section() {
        let d = Style::default();
        let g = grid(
            vec![
                line(&["a", "漢", "", "b", " "], &vec![d.clone(); 5]),
                line(&["e\u{301}", "x", " ", " ", " "], &vec![d.clone(); 5]),
                plain("plain", 5, &d),
            ],
            HIDDEN,
        );
        let text = round_trip(&g);
        assert!(
            text.contains("\n--- cells\n0 [\"a\",\"漢\",\"\",\"b\",\" \"]\n1 [\"e\u{301}\",\"x\",\" \",\" \",\" \"]\n")
        );
        assert!(!text.contains("\n2 ["), "{text}");
    }

    #[test]
    fn a_text_row_reading_like_a_header_is_still_text() {
        let d = Style::default();
        let g = grid(
            vec![
                plain(STYLES, 12, &d),
                plain(LEGEND, 12, &fg("#ffffff")),
                plain(CELLS, 12, &d),
            ],
            HIDDEN,
        );
        round_trip(&g);
    }

    #[test]
    fn cursors_round_trip() {
        let d = Style::default();
        for cursor in [
            HIDDEN,
            Cursor {
                row: 1,
                col: 3,
                visible: true,
            },
        ] {
            let text = round_trip(&grid(vec![plain("ab", 4, &d), plain("cd", 4, &d)], cursor));
            let want = if cursor.visible {
                "cursor 1 3 visible"
            } else {
                "cursor 0 0 hidden"
            };
            assert_eq!(text.lines().nth(2), Some(want));
        }
    }

    #[test]
    fn every_attribute_and_color_kind_round_trips() {
        let all = Style {
            fg: Some("#4ebf71".into()),
            bg: Some("idx:4".into()),
            bold: true,
            italic: true,
            underline: true,
            inverse: true,
        };
        let styles = [
            Style::default(),
            all.clone(),
            fg("idx:255"),
            Style {
                bg: Some("#000000".into()),
                ..Style::default()
            },
            Style {
                italic: true,
                ..Style::default()
            },
            Style {
                underline: true,
                ..Style::default()
            },
        ];
        let g = grid(vec![line(&["a", "b", "c", "d", "e", "f"], &styles)], HIDDEN);
        let text = round_trip(&g);
        assert!(
            text.contains("\na fg=#4ebf71 bg=idx:4 bold italic underline inverse\n"),
            "{text}"
        );
    }

    #[test]
    fn a_default_styled_screen_has_no_style_rows() {
        let d = Style::default();
        let text = round_trip(&grid(vec![plain("hi", 3, &d), plain("", 3, &d)], HIDDEN));
        assert!(
            text.ends_with("--- text\nhi\n\n--- styles\n--- legend\n. default\n"),
            "{text}"
        );
    }

    #[test]
    fn many_styles_use_every_symbol_kind() {
        let styles: Vec<Style> = (0..40).map(|i| fg(&format!("idx:{i}"))).collect();
        let cells = vec!["x"; 40];
        round_trip(&grid(vec![line(&cells, &styles)], HIDDEN));

        let styles: Vec<Style> = (0..=SYMBOLS.len()).map(|i| fg(&format!("idx:{i}"))).collect();
        let cells = vec!["x"; styles.len()];
        round_trip(&grid(vec![line(&cells, &styles)], HIDDEN));

        let styles: Vec<Style> = (0..=SYMBOLS.len() + 1).map(|i| fg(&format!("idx:{i}"))).collect();
        let cells = vec!["x"; styles.len()];
        let err = render(&grid(vec![line(&cells, &styles)], HIDDEN)).unwrap_err();
        assert!(err.to_string().contains("82 distinct styles"), "{err}");
    }

    #[test]
    fn the_most_common_style_is_the_default_symbol_with_ties_to_the_first_seen() {
        let (r, g, b) = (fg("idx:1"), fg("idx:2"), fg("idx:3"));
        let text = round_trip(&grid(
            vec![
                line(&["a", "b", "c", "d"], &[r.clone(), g.clone(), g.clone(), r.clone()]),
                line(&["e", " ", " ", "f"], &[b.clone(), r, g, b]),
            ],
            HIDDEN,
        ));
        // idx:1 and idx:2 are both used three times; idx:1 is seen first.
        assert!(
            text.contains("--- styles\n.aa\nb.ab\n--- legend\n. fg=idx:1\na fg=idx:2\nb fg=idx:3\n"),
            "{text}"
        );
    }

    #[test]
    fn render_refuses_an_inconsistent_grid() {
        let d = Style::default();
        let mut g = grid(vec![plain("ab", 3, &d)], HIDDEN);
        g.lines[0].runs = vec![
            Run {
                col: 0,
                len: 1,
                style: d.clone(),
            },
            Run {
                col: 1,
                len: 2,
                style: d.clone(),
            },
        ];
        let err = render(&g).unwrap_err().to_string();
        assert!(err.contains("doesn't round-trip"), "{err}");

        g.lines[0].runs.pop();
        assert!(render(&g).unwrap_err().to_string().contains("runs cover 1 columns"));

        let mut g = grid(vec![plain("ab", 3, &d)], HIDDEN);
        g.lines[0].cells[0] = "\n".into();
        assert!(render(&g).is_err());
    }

    #[test]
    fn snapshot_of_a_small_styled_screen() {
        let bg = "#121212";
        let base = Style {
            bg: Some(bg.into()),
            ..Style::default()
        };
        let title = Style {
            fg: Some("#4ebf71".into()),
            bg: Some(bg.into()),
            bold: true,
            ..Style::default()
        };
        let selected = Style {
            fg: Some("#e0e0e0".into()),
            bg: Some("#242f38".into()),
            inverse: true,
            ..Style::default()
        };
        let b = base.clone();
        let t = title.clone();
        let s = selected.clone();
        let g = grid(
            vec![
                line(
                    &["┌", " ", "R", "e", "p", "o", "s", " ", "─", "┐"],
                    &[
                        b.clone(),
                        b.clone(),
                        t.clone(),
                        t.clone(),
                        t.clone(),
                        t.clone(),
                        t,
                        b.clone(),
                        b.clone(),
                        b.clone(),
                    ],
                ),
                line(
                    &["│", "a", "漢", "", "b", " ", " ", " ", " ", "│"],
                    &[
                        b.clone(),
                        s.clone(),
                        s.clone(),
                        s.clone(),
                        s,
                        b.clone(),
                        b.clone(),
                        b.clone(),
                        b.clone(),
                        b.clone(),
                    ],
                ),
                plain("└────────┘", 10, &base),
            ],
            Cursor {
                row: 1,
                col: 2,
                visible: true,
            },
        );
        insta::assert_snapshot!(round_trip(&g));
    }

    const GOOD: &str = "lazy-github-screen 1\nsize 4x2\ncursor 0 0 hidden\n--- text\nab\ncd\n--- styles\n.a\n--- legend\n. default\na bold\n";

    #[test]
    fn the_error_fixture_base_parses() {
        let g = parse(GOOD).unwrap();
        assert_eq!(render(&g).unwrap(), GOOD);
    }

    #[test]
    fn parse_errors_name_the_line() {
        let cases = [
            (
                GOOD.replace("lazy-github-screen 1", "lazy-github-grid 1"),
                "line 1: expected `lazy-github-screen 1`",
            ),
            (
                GOOD.replace("lazy-github-screen 1", "lazy-github-screen 2"),
                "line 1: unsupported .screen version \"2\"",
            ),
            (
                GOOD.replace("size 4x2", "size 4 by 2"),
                "line 2: expected `size <cols>x<rows>`",
            ),
            (
                GOOD.replace("cursor 0 0 hidden", "cursor 0 0 off"),
                "line 3: expected `cursor",
            ),
            // Text rows are counted from `size`, so section headers are read as text.
            (
                GOOD.replace("size 4x2", "size 4x5"),
                "line 10: expected `--- styles`, found \". default\"",
            ),
            (
                GOOD.replace(".a\n", ".a...\n"),
                "line 8: style row has 5 symbols, more than the screen's 4 columns",
            ),
            (GOOD.replace(".a\n", ".b\n"), "line 8: symbol `b` is not in the legend"),
            (
                GOOD.replace(". default\n", ""),
                "line 8: symbol `.` is not in the legend",
            ),
            (
                GOOD.replace(".a\n", "a\n").replace(". default\n", ""),
                "line 9: symbol `.` is not in the legend",
            ),
            (
                GOOD.replace("a bold", "a bold blink"),
                "line 11: unknown legend word \"blink\"",
            ),
            (GOOD.replace("a bold", "a fg=red"), "line 11: \"red\" is not a color"),
            (
                GOOD.replace("a bold", "a bold fg=#ffffff"),
                "line 11: legend word \"fg=#ffffff\" is repeated or out of order",
            ),
            (GOOD.replace("a bold", "- bold"), "line 11: \"-\" is not a style symbol"),
            (
                format!("{GOOD}--- cells\n0 [\"a\",\"b\"]\n"),
                "line 13: row 0 has 2 cells; the screen has 4 columns",
            ),
            (
                format!("{GOOD}--- cells\n0 [\"a\",\"b\",\" \",\n"),
                "line 13: row 0's cells aren't a JSON string array",
            ),
            (
                format!("{GOOD}--- cells\n2 [\"a\",\"b\",\" \",\" \"]\n"),
                "line 13: \"2\" is not a row number below 2",
            ),
            (
                format!("{GOOD}--- cells\n0 [\"x\",\"b\",\" \",\" \"]\n"),
                "line 5: text row 0 doesn't match its `--- cells` entry",
            ),
            (
                format!("{GOOD}--- nope\n"),
                "line 12: expected `--- cells`, found \"--- nope\"",
            ),
        ];
        for (text, want) in cases {
            let got = error(&text);
            assert!(got.starts_with(want), "want {want:?}\n got {got:?}\nfor\n{text}");
        }
    }

    #[test]
    fn too_few_text_rows() {
        let text = "lazy-github-screen 1\nsize 4x3\ncursor 0 0 hidden\n--- text\nab\n";
        assert_eq!(error(text), "line 6: expected 3 text rows, found 1");
    }

    fn entry(seq: u64, path: &str) -> String {
        format!(
            r#"{{"seq":{seq},"argv":["api",{path:?}],"command":"api","method":"GET","path":{path:?},"query":null,"accept":"json","cache":null,"cache_control":null,"body":null,"fixture":"base/x.json","status":200,"miss":false,"error":null}}"#
        )
    }

    #[test]
    fn normalized_logs_ignore_order_seq_and_duplicates() {
        let a = [entry(1, "/user"), entry(2, "/repos"), entry(3, "/user")].join("\n") + "\n";
        let b = [entry(1, "/repos"), entry(2, "/user")].join("\n") + "\n\n";
        let (na, nb) = (normalize_log(&a), normalize_log(&b));
        assert_eq!(na, nb);
        assert_eq!(na.lines().count(), 2);
        assert!(na.lines().all(|l| l.starts_with(r#"{"seq":0,"argv":"#)), "{na}");
        let log = RequestLog::parse(&na);
        assert_eq!(log.entries().len(), 2);
        assert!(log.entries().iter().all(|e| e.seq == 0 && e.is_served()));
        assert_eq!(normalize_log(""), "");
    }
}
