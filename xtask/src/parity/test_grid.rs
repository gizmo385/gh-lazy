//! Builds [`Grid`]s from the compact text fixtures in `xtask/tests/grids/*.txt`, so synthetic structure tests stay
//! readable.
//!
//! A fixture is the screen's characters, one line per row, optionally followed by a style layer and its legend:
//!
//! ```text
//! ┌─ Title ─┐
//! │ hi      │
//! └─────────┘
//! --- styles
//! ggggggggggg
//! g.......g
//! --- legend
//! g fg=#4ebf71
//! h fg=#e0e0e0 bg=#2d3740 bold
//! ```
//!
//! Rows are padded with spaces to the widest one. Every character is one cell, so fixtures can't hold wide
//! characters. In the style layer, each letter names a legend entry for the cell below it; `.`, a space and cells past
//! the end of a style row get the default style (`bg=#121212`, like a Textual screen). A legend entry lists `fg=`,
//! `bg=`, `bold` and `inverse`; an entry without `bg=` gets the default background.

use std::collections::BTreeMap;

use super::grid::{Cursor, GRID_VERSION, Grid, Line, Run, Style};

pub const SCREEN_BG: &str = "#121212";

fn default_style() -> Style {
    Style {
        bg: Some(SCREEN_BG.into()),
        ..Style::default()
    }
}

fn parse_style(spec: &str) -> Style {
    let mut style = default_style();
    for word in spec.split_whitespace() {
        match word.split_once('=') {
            Some(("fg", v)) => style.fg = Some(v.into()),
            Some(("bg", v)) => style.bg = Some(v.into()),
            None if word == "bold" => style.bold = true,
            None if word == "inverse" => style.inverse = true,
            _ => panic!("bad legend word {word:?}"),
        }
    }
    style
}

/// Parses a fixture. Panics on a malformed one: fixtures are test inputs.
pub fn parse(source: &str) -> Grid {
    let mut sections = source.split("\n--- ");
    let chars: Vec<Vec<char>> = sections
        .next()
        .unwrap_or_default()
        .lines()
        .map(|l| l.chars().collect())
        .collect();
    let mut style_rows: Vec<Vec<char>> = Vec::new();
    let mut legend: BTreeMap<char, Style> = BTreeMap::new();
    for section in sections {
        let (name, body) = section.split_once('\n').unwrap_or((section, ""));
        match name.trim() {
            "styles" => style_rows = body.lines().map(|l| l.chars().collect()).collect(),
            "legend" => {
                for line in body.lines().filter(|l| !l.trim().is_empty()) {
                    let mut it = line.chars();
                    let letter = it.next().unwrap();
                    legend.insert(letter, parse_style(it.as_str()));
                }
            }
            other => panic!("unknown fixture section {other:?}"),
        }
    }
    let cols = chars.iter().map(Vec::len).max().unwrap_or(0);
    let lines = chars
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let mut cells = Vec::with_capacity(cols);
            let mut runs: Vec<Run> = Vec::new();
            for c in 0..cols {
                cells.push(row.get(c).copied().unwrap_or(' ').to_string());
                let letter = style_rows.get(r).and_then(|s| s.get(c)).copied().unwrap_or('.');
                let style = match letter {
                    '.' | ' ' => default_style(),
                    l => legend
                        .get(&l)
                        .cloned()
                        .unwrap_or_else(|| panic!("no legend entry {l:?}")),
                };
                let col = u16::try_from(c).unwrap();
                match runs.last_mut() {
                    Some(run) if run.style == style => run.len += 1,
                    _ => runs.push(Run { col, len: 1, style }),
                }
            }
            Line { cells, runs }
        })
        .collect();
    Grid {
        version: GRID_VERSION,
        cols: u16::try_from(cols).unwrap(),
        rows: u16::try_from(chars.len()).unwrap(),
        cursor: Cursor {
            row: 0,
            col: 0,
            visible: false,
        },
        lines,
    }
}

/// Reads `xtask/tests/grids/<name>`.
pub fn load(name: &str) -> Grid {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/grids")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if path.extension().is_some_and(|e| e == "json") {
        serde_json::from_str(&text).unwrap()
    } else {
        parse(&text)
    }
}
