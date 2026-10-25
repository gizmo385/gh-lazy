//! Structure extraction: turns a screen dump ([`Grid`]) into the panes, tabs, tables, footer and focus that the
//! structural check compares (`<capture>.structure.json`).
//!
//! [`extract`] is pure and built from small passes that run in order over the grid, each claiming the cells it
//! recognized in a `Mask` so later passes skip them: `find_boxes`, `assign_parents`, `find_footer`, then per box
//! `find_tabs`, `find_tables` and `collect_text`. The rules, their limits and the rendering contract they
//! impose on the Rust UI are documented in `parity/README.md` ("Structure extraction").

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::grid::{Grid, Style};
use super::{Context, HarnessError, Result};

/// The only structure format version written.
pub const STRUCTURE_VERSION: u32 = 1;

/// The theme the Python app uses when the config doesn't name one.
pub const DEFAULT_THEME: &str = "textual-dark";

/// What [`extract`] recognized on one screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Structure {
    pub version: u32,
    /// `[cols, rows]`.
    pub size: [u16; 2],
    /// Every box, in pre-order by (row, col).
    pub boxes: Vec<Pane>,
    /// A tab bar outside every box.
    pub tabs: Option<Tabs>,
    pub footer: Vec<Binding>,
    /// The footer reaches the screen's last column, so bindings past it may be cut off.
    pub footer_clipped: bool,
    /// The titles of the focused boxes, in pre-order.
    pub focused: Vec<String>,
    /// Text outside every box and the footer.
    pub loose_text: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The screen's plain text rows, for `contains`/`not_contains`. Not written: `<capture>.txt` already holds it.
    #[serde(skip)]
    pub screen: Vec<String>,
}

/// One bordered box (a pane, the status summary, a dialog).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    /// The box's index in [`Structure::boxes`].
    pub id: usize,
    /// The smallest box containing this one.
    pub parent: Option<usize>,
    pub title: String,
    pub subtitle: String,
    /// `[col, row, width, height]` of the outer border.
    pub rect: [u16; 4],
    pub border_fg: Option<String>,
    pub focused: bool,
    pub tabs: Option<Tabs>,
    pub tables: Vec<Table>,
    pub text: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tabs {
    pub names: Vec<String>,
    pub active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub cursor: Option<usize>,
}

/// One footer entry: the key as drawn (`^p`, `q`) and its description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub key: String,
    pub label: String,
}

impl Binding {
    /// The `"<key> <label>"` form `[expect] footer` uses.
    pub fn display(&self) -> String {
        if self.label.is_empty() {
            self.key.clone()
        } else {
            format!("{} {}", self.key, self.label)
        }
    }
}

/// The colors extraction needs from the app's theme.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeColors {
    pub name: String,
    /// The focused pane's border color; `None` when the theme isn't in `parity/themes.toml`.
    pub focus: Option<String>,
}

/// `parity/themes.toml`: per theme name, the colors measured from Python dumps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Themes(BTreeMap<String, ThemeEntry>);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeEntry {
    focus: String,
}

impl Themes {
    pub fn parse(source: &str) -> std::result::Result<Self, String> {
        let entries: BTreeMap<String, ThemeEntry> = toml::from_str(source).map_err(|e| e.to_string())?;
        for (name, entry) in &entries {
            if !is_hex_color(&entry.focus) {
                return Err(format!("[{name}] focus = {:?} isn't a #rrggbb color", entry.focus));
            }
        }
        Ok(Self(entries))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&source).map_err(|e| HarnessError(format!("{}: {e}", path.display())))
    }

    pub fn colors(&self, name: &str) -> ThemeColors {
        ThemeColors {
            name: name.to_owned(),
            focus: self.0.get(name).map(|e| e.focus.to_ascii_lowercase()),
        }
    }
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Extracts the structure of one screen. Pure: no I/O.
pub fn extract(grid: &Grid, theme: &ThemeColors) -> Structure {
    let view = View::new(grid);
    let mut mask = Mask::new(view.rows, view.cols);
    let found = assign_parents(&find_boxes(&view, &mut mask));
    let (footer, footer_clipped) = find_footer(&view, &mut mask, &found);

    let mut boxes = Vec::with_capacity(found.len());
    for (id, b) in found.iter().enumerate() {
        let holes = found.iter().filter(|c| c.parent == Some(id)).map(|c| c.rect).collect();
        let mut region = Region {
            top: b.rect.row + 1,
            bottom: b.rect.bottom(),
            left: b.rect.col + 1,
            right: b.rect.right(),
            holes,
        };
        region.right = strip_scrollbar(&view, &mask, &region);
        let tabs = find_tabs(&view, &mut mask, &region);
        let tables = find_tables(&view, &mut mask, &region, view.bg(b.rect.row, b.rect.col));
        let text = collect_text(&view, &mask, &region);
        let focused = theme.focus.is_some() && b.fg.as_deref() == theme.focus.as_deref();
        boxes.push(Pane {
            id,
            parent: b.parent,
            title: b.title.clone(),
            subtitle: b.subtitle.clone(),
            rect: b.rect.to_array(),
            border_fg: b.fg.clone(),
            focused,
            tabs,
            tables,
            text,
        });
    }

    let screen = Region {
        top: 0,
        bottom: view.rows,
        left: 0,
        right: view.cols,
        holes: found.iter().filter(|b| b.parent.is_none()).map(|b| b.rect).collect(),
    };
    let tabs = find_tabs(&view, &mut mask, &screen);
    let loose_text = collect_text(&view, &mask, &screen);

    let mut warnings = Vec::new();
    if theme.focus.is_none() {
        warnings.push(format!(
            "theme {:?} isn't in parity/themes.toml, so no box is detected as focused",
            theme.name
        ));
    }
    Structure {
        version: STRUCTURE_VERSION,
        size: [grid.cols, grid.rows],
        focused: boxes.iter().filter(|b| b.focused).map(|b| b.title.clone()).collect(),
        boxes,
        tabs,
        footer,
        footer_clipped,
        loose_text,
        warnings,
        screen: grid.text_rows(),
    }
}

// The grid, cell by cell.

static BLANK: Style = Style {
    fg: None,
    bg: None,
    bold: false,
    italic: false,
    underline: false,
    inverse: false,
};

/// Scrollbar glyphs: Textual draws thumbs with these eighth blocks.
const SCROLLBAR: &str = "▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐▔▕";
/// What a tab bar's underline is drawn with.
const TAB_BAR: &str = "━╸╺─▔▁";

struct View<'a> {
    rows: usize,
    cols: usize,
    cells: Vec<Vec<&'a str>>,
    styles: Vec<Vec<&'a Style>>,
}

impl<'a> View<'a> {
    fn new(grid: &'a Grid) -> Self {
        let cols = usize::from(grid.cols);
        let mut cells = Vec::with_capacity(grid.lines.len());
        let mut styles = Vec::with_capacity(grid.lines.len());
        for line in &grid.lines {
            cells.push(
                (0..cols)
                    .map(|c| line.cells.get(c).map_or(" ", String::as_str))
                    .collect(),
            );
            let mut row = vec![&BLANK; cols];
            for run in &line.runs {
                let start = usize::from(run.col);
                for slot in row.iter_mut().skip(start).take(usize::from(run.len)) {
                    *slot = &run.style;
                }
            }
            styles.push(row);
        }
        Self {
            rows: grid.lines.len(),
            cols,
            cells,
            styles,
        }
    }

    fn text(&self, r: usize, c: usize) -> &'a str {
        self.cells[r][c]
    }

    /// The cell's character, when it holds exactly one.
    fn ch(&self, r: usize, c: usize) -> Option<char> {
        let mut chars = self.cells[r][c].chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    }

    /// A blank cell. A wide character's continuation (`""`) isn't blank: it belongs to the text before it.
    fn is_space(&self, r: usize, c: usize) -> bool {
        self.cells[r][c] == " "
    }

    fn is_scrollbar(&self, r: usize, c: usize) -> bool {
        self.ch(r, c).is_some_and(|ch| SCROLLBAR.contains(ch))
    }

    fn style(&self, r: usize, c: usize) -> &'a Style {
        self.styles[r][c]
    }

    /// The color the cell's glyph is drawn in, with `inverse` applied.
    fn fg(&self, r: usize, c: usize) -> Option<&'a str> {
        let s = self.style(r, c);
        if s.inverse { s.bg.as_deref() } else { s.fg.as_deref() }
    }

    /// The color the cell's background is drawn in, with `inverse` applied.
    fn bg(&self, r: usize, c: usize) -> Option<&'a str> {
        let s = self.style(r, c);
        if s.inverse { s.fg.as_deref() } else { s.bg.as_deref() }
    }
}

/// Cells claimed by an earlier pass.
struct Mask {
    cols: usize,
    cells: Vec<bool>,
}

impl Mask {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            cols,
            cells: vec![false; rows * cols],
        }
    }

    fn get(&self, r: usize, c: usize) -> bool {
        self.cells[r * self.cols + c]
    }

    fn claim(&mut self, r: usize, c: usize) {
        self.cells[r * self.cols + c] = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    col: usize,
    row: usize,
    width: usize,
    height: usize,
}

impl Rect {
    /// The right border's column.
    fn right(self) -> usize {
        self.col + self.width - 1
    }

    /// The bottom border's row.
    fn bottom(self) -> usize {
        self.row + self.height - 1
    }

    fn contains(self, r: usize, c: usize) -> bool {
        (self.row..self.row + self.height).contains(&r) && (self.col..self.col + self.width).contains(&c)
    }

    /// `other` lies entirely inside this box's interior.
    fn encloses(self, other: Rect) -> bool {
        other.col > self.col
            && other.row > self.row
            && other.col + other.width < self.col + self.width
            && other.row + other.height < self.row + self.height
    }

    fn to_array(self) -> [u16; 4] {
        // Rects come from a grid whose size is a u16 pair.
        [self.col, self.row, self.width, self.height].map(|n| u16::try_from(n).unwrap_or(u16::MAX))
    }
}

/// An area the per-box passes look at: rows `top..bottom` and columns `left..right`, minus nested boxes (`holes`)
/// and claimed cells.
struct Region {
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
    holes: Vec<Rect>,
}

impl Region {
    fn free(&self, mask: &Mask, r: usize, c: usize) -> bool {
        !mask.get(r, c) && !self.holes.iter().any(|h| h.contains(r, c))
    }

    fn width(&self) -> usize {
        self.right.saturating_sub(self.left)
    }

    /// The end of the row's content: trailing blanks, claimed cells and scrollbar glyphs dropped.
    fn row_end(&self, view: &View<'_>, mask: &Mask, r: usize) -> usize {
        let mut end = self.right;
        while end > self.left {
            let c = end - 1;
            if !self.free(mask, r, c) || view.is_space(r, c) || view.is_scrollbar(r, c) {
                end -= 1;
            } else {
                break;
            }
        }
        end
    }

    /// The row's free, non-blank cells up to [`Region::row_end`].
    fn text_cells(&self, view: &View<'_>, mask: &Mask, r: usize) -> Vec<usize> {
        (self.left..self.row_end(view, mask, r))
            .filter(|&c| self.free(mask, r, c) && !view.is_space(r, c))
            .collect()
    }

    /// The text of columns `from..to` of a row, with claimed cells read as blanks.
    fn slice(&self, view: &View<'_>, mask: &Mask, r: usize, from: usize, to: usize) -> String {
        (from..to)
            .map(|c| if self.free(mask, r, c) { view.text(r, c) } else { " " })
            .collect()
    }

    /// The row's text segments separated by two or more blanks, as `(start col, end col, trimmed text)`.
    fn segments(&self, view: &View<'_>, mask: &Mask, r: usize) -> Vec<(usize, usize, String)> {
        let mut out: Vec<(usize, usize, String)> = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        for c in self.text_cells(view, mask, r) {
            current = match current {
                Some((start, last)) if c - last - 1 < 2 => Some((start, c)),
                Some((start, last)) => {
                    out.push((start, last + 1, self.slice(view, mask, r, start, last + 1)));
                    Some((c, c))
                }
                None => Some((c, c)),
            };
        }
        if let Some((start, last)) = current {
            out.push((start, last + 1, self.slice(view, mask, r, start, last + 1)));
        }
        out.into_iter().map(|(s, e, t)| (s, e, t.trim().to_owned())).collect()
    }

    /// The most common background over the given rows of the region.
    fn common_bg<'a>(&self, view: &View<'a>, mask: &Mask, rows: std::ops::Range<usize>) -> Option<&'a str> {
        most_common(rows.flat_map(|r| {
            (self.left..self.right)
                .filter(move |&c| self.free(mask, r, c))
                .map(move |c| view.bg(r, c))
        }))
        .flatten()
    }
}

/// The most frequent item; ties go to the one seen first.
fn most_common<T: PartialEq + Copy>(items: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(t, _)| *t == item) {
            Some((_, n)) => *n += 1,
            None => counts.push((item, 1)),
        }
    }
    let mut best: Option<(T, usize)> = None;
    for (item, n) in counts {
        if best.is_none_or(|(_, m)| n > m) {
            best = Some((item, n));
        }
    }
    best.map(|(t, _)| t)
}

// Boxes.

#[derive(Debug, Clone, Copy)]
struct Family {
    /// Top-left, top-right, bottom-left, bottom-right.
    corners: [char; 4],
    horizontal: char,
    vertical: char,
}

const TL: usize = 0;
const TR: usize = 1;
const BL: usize = 2;
const BR: usize = 3;

const FAMILIES: [Family; 5] = [
    Family {
        corners: ['┌', '┐', '└', '┘'],
        horizontal: '─',
        vertical: '│',
    },
    Family {
        corners: ['╭', '╮', '╰', '╯'],
        horizontal: '─',
        vertical: '│',
    },
    Family {
        corners: ['┏', '┓', '┗', '┛'],
        horizontal: '━',
        vertical: '┃',
    },
    Family {
        corners: ['╔', '╗', '╚', '╝'],
        horizontal: '═',
        vertical: '║',
    },
    Family {
        corners: ['+', '+', '+', '+'],
        horizontal: '-',
        vertical: '|',
    },
];

impl Family {
    fn is_ascii(self) -> bool {
        self.horizontal == '-'
    }

    /// Whether `ch` can close this family's box at `corner`. Box-drawing families accept each other's corners
    /// (mixed-family corners); ascii only accepts `+`.
    fn corner(self, corner: usize, ch: char) -> bool {
        if self.is_ascii() {
            ch == '+'
        } else {
            FAMILIES.iter().any(|f| !f.is_ascii() && f.corners[corner] == ch)
        }
    }

    /// A character that belongs to some border, so it can't be title text.
    fn is_border(self, ch: char) -> bool {
        if self.is_ascii() {
            matches!(ch, '+' | '-' | '|')
        } else {
            ('\u{2500}'..='\u{257f}').contains(&ch)
        }
    }

    fn min_size(self) -> usize {
        if self.is_ascii() { 3 } else { 2 }
    }
}

#[derive(Debug, Clone)]
struct Found {
    rect: Rect,
    fg: Option<String>,
    title: String,
    subtitle: String,
    parent: Option<usize>,
}

/// Traces every box from its top-left corner and claims its border cells.
fn find_boxes(view: &View<'_>, mask: &mut Mask) -> Vec<Found> {
    let mut found = Vec::new();
    for r in 0..view.rows {
        for c in 0..view.cols {
            let Some(ch) = view.ch(r, c) else { continue };
            if mask.get(r, c) {
                continue;
            }
            let Some(family) = FAMILIES.iter().copied().find(|f| f.corners[TL] == ch) else {
                continue;
            };
            if let Some(rect) = trace(view, family, r, c) {
                for y in rect.row..=rect.bottom() {
                    for x in rect.col..=rect.right() {
                        if y == rect.row || y == rect.bottom() || x == rect.col || x == rect.right() {
                            mask.claim(y, x);
                        }
                    }
                }
                found.push(Found {
                    rect,
                    fg: view.fg(r, c).map(str::to_owned),
                    title: edge_label(view, family, r, c + 1, rect.right()),
                    subtitle: edge_label(view, family, rect.bottom(), c + 1, rect.right()),
                    parent: None,
                });
            }
        }
    }
    found
}

/// Follows the border from the top-left corner at `(r, c)`; `None` when it isn't a closed box.
fn trace(view: &View<'_>, f: Family, r: usize, c: usize) -> Option<Rect> {
    let fg = view.fg(r, c);
    let same_fg = |y: usize, x: usize| view.fg(y, x) == fg;

    // Top edge: horizontals and title text up to the first top-right corner, which must match in color.
    let mut right = None;
    for x in c + 1..view.cols {
        match view.ch(r, x) {
            Some(ch) if f.corner(TR, ch) => {
                right = same_fg(r, x).then_some(x);
                break;
            }
            Some(ch) if f.is_border(ch) && ch != f.horizontal => return None,
            _ => {}
        }
    }
    let right = right?;

    // Left side down to a bottom-left corner.
    let mut bottom = None;
    for y in r + 1..view.rows {
        match view.ch(y, c) {
            Some(ch) if f.corner(BL, ch) && same_fg(y, c) => {
                bottom = Some(y);
                break;
            }
            Some(ch) if ch == f.vertical && same_fg(y, c) => {}
            _ => return None,
        }
    }
    let bottom = bottom?;

    // Right side and bottom-right corner.
    for y in r + 1..bottom {
        if view.ch(y, right) != Some(f.vertical) || !same_fg(y, right) {
            return None;
        }
    }
    if !view.ch(bottom, right).is_some_and(|ch| f.corner(BR, ch)) || !same_fg(bottom, right) {
        return None;
    }
    // Bottom edge: horizontals and subtitle text.
    for x in c + 1..right {
        if view
            .ch(bottom, x)
            .is_some_and(|ch| f.is_border(ch) && ch != f.horizontal)
        {
            return None;
        }
    }

    let rect = Rect {
        col: c,
        row: r,
        width: right - c + 1,
        height: bottom - r + 1,
    };
    (rect.width >= f.min_size() && rect.height >= f.min_size()).then_some(rect)
}

/// A title or subtitle: the edge's text between the corners, split into segments by runs of the horizontal
/// border char and joined with `" | "`. In an ascii box a single `-` inside text (`my-repo`) isn't a separator.
fn edge_label(view: &View<'_>, f: Family, row: usize, from: usize, to: usize) -> String {
    let cells: Vec<&str> = (from..to).map(|x| view.text(row, x)).collect();
    let is_horizontal = |s: &str| {
        if f.is_ascii() {
            s == "-"
        } else {
            matches!(s, "─" | "━" | "═")
        }
    };
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < cells.len() {
        if is_horizontal(cells[i]) {
            let start = i;
            while i < cells.len() && is_horizontal(cells[i]) {
                i += 1;
            }
            let separates = !f.is_ascii() || i - start >= 2 || start == 0 || i == cells.len();
            if separates {
                segments.push(std::mem::take(&mut current));
            } else {
                current.push_str(&cells[start..i].concat());
            }
        } else {
            current.push_str(cells[i]);
            i += 1;
        }
    }
    segments.push(current);
    segments
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Sets each box's parent to the smallest box enclosing it, and orders the boxes in pre-order by (row, col).
fn assign_parents(found: &[Found]) -> Vec<Found> {
    let area = |r: Rect| r.width * r.height;
    let parent_of: Vec<Option<usize>> = found
        .iter()
        .map(|b| {
            found
                .iter()
                .enumerate()
                .filter(|(_, p)| p.rect.encloses(b.rect))
                .min_by_key(|(_, p)| area(p.rect))
                .map(|(i, _)| i)
        })
        .collect();
    let key = |i: &usize| (found[*i].rect.row, found[*i].rect.col);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); found.len()];
    let mut roots = Vec::new();
    for (i, parent) in parent_of.iter().enumerate() {
        match parent {
            Some(p) => children[*p].push(i),
            None => roots.push(i),
        }
    }
    roots.sort_by_key(key);
    for list in &mut children {
        list.sort_by_key(key);
    }

    let mut order = Vec::with_capacity(found.len());
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        stack.extend(children[i].iter().rev());
    }
    let mut new_id = vec![0; found.len()];
    for (new, &old) in order.iter().enumerate() {
        new_id[old] = new;
    }
    order
        .iter()
        .map(|&old| Found {
            parent: parent_of[old].map(|p| new_id[p]),
            ..found[old].clone()
        })
        .collect()
}

// Inside a box.

/// Drops scrollbar columns from the region's right edge: a column whose background differs from its row's on at
/// least half of the rows, holding only blanks and scrollbar glyphs where it differs. Returns the new right edge.
fn strip_scrollbar(view: &View<'_>, mask: &Mask, region: &Region) -> usize {
    let rows: Vec<usize> = (region.top..region.bottom).collect();
    if rows.is_empty() {
        return region.right;
    }
    let row_bgs: Vec<Option<&str>> = rows.iter().map(|&r| region.common_bg(view, mask, r..r + 1)).collect();
    let mut right = region.right;
    while right > region.left {
        let c = right - 1;
        let differing: Vec<usize> = rows
            .iter()
            .zip(&row_bgs)
            .filter(|&(&r, bg)| region.free(mask, r, c) && view.bg(r, c) != *bg)
            .map(|(&r, _)| r)
            .collect();
        let glyphs_only = differing
            .iter()
            .all(|&r| view.is_space(r, c) || view.is_scrollbar(r, c));
        if differing.len() * 2 >= rows.len() && glyphs_only {
            right -= 1;
        } else {
            break;
        }
    }
    right
}

/// Finds a tab bar (an underline row of `━╸╺─▔▁` in two or more colors under a row of labels) and claims both rows.
fn find_tabs(view: &View<'_>, mask: &mut Mask, region: &Region) -> Option<Tabs> {
    let width = region.width();
    for r in region.top + 1..region.bottom {
        let cells: Vec<usize> = (region.left..region.right)
            .filter(|&c| region.free(mask, r, c) && !view.is_space(r, c))
            .collect();
        let (Some(&first), Some(&last)) = (cells.first(), cells.last()) else {
            continue;
        };
        if !cells
            .iter()
            .all(|&c| view.ch(r, c).is_some_and(|ch| TAB_BAR.contains(ch)))
        {
            continue;
        }
        if (last - first + 1) * 2 < width {
            continue;
        }
        let fgs: Vec<Option<&str>> = cells.iter().map(|&c| view.fg(r, c)).collect();
        let Some(common) = most_common(fgs.iter().copied()) else {
            continue;
        };
        if fgs.iter().all(|fg| *fg == common) {
            continue;
        }
        let labels = region.segments(view, mask, r - 1);
        if labels.is_empty() {
            continue;
        }
        let active_cols: Vec<usize> = cells
            .iter()
            .zip(&fgs)
            .filter(|(_, fg)| **fg != common)
            .map(|(&c, _)| c)
            .collect();
        let mut best: Option<(usize, &str)> = None;
        for (start, end, name) in &labels {
            let overlap = active_cols.iter().filter(|&&c| (*start..*end).contains(&c)).count();
            if overlap > 0 && best.is_none_or(|(n, _)| overlap > n) {
                best = Some((overlap, name));
            }
        }
        let tabs = Tabs {
            names: labels.iter().map(|(_, _, n)| n.clone()).collect(),
            active: best.map(|(_, n)| n.to_owned()),
        };
        for row in [r - 1, r] {
            for c in region.left..region.right {
                if region.free(mask, row, c) {
                    mask.claim(row, c);
                }
            }
        }
        return Some(tabs);
    }
    None
}

/// A row's style when all of its text cells share one background and boldness: `(bg, bold)`.
fn row_key<'a>(view: &View<'a>, region: &Region, mask: &Mask, r: usize) -> Option<(Option<&'a str>, bool)> {
    let cells = region.text_cells(view, mask, r);
    let (&first, rest) = cells.split_first()?;
    let key = (view.bg(r, first), view.style(r, first).bold);
    rest.iter()
        .all(|&c| (view.bg(r, c), view.style(r, c).bold) == key)
        .then_some(key)
}

/// Finds tables: a bold header row on a background other than the box's (`base_bg`, its border's), then data rows
/// split at the header's column starts.
fn find_tables(view: &View<'_>, mask: &mut Mask, region: &Region, base_bg: Option<&str>) -> Vec<Table> {
    let mut tables = Vec::new();
    let mut r = region.top;
    while r < region.bottom {
        let Some(key) = row_key(view, region, mask, r).filter(|&(bg, bold)| bold && bg != base_bg) else {
            r += 1;
            continue;
        };
        // A header's style is its own: a block of rows sharing it (a code block, say) isn't a table.
        let shares = |row: usize| row_key(view, region, mask, row) == Some(key);
        if (r + 1 < region.bottom && shares(r + 1)) || (r > region.top && shares(r - 1)) {
            r += 1;
            continue;
        }
        let headers = region.segments(view, mask, r);
        let starts: Vec<usize> = headers.iter().map(|(s, _, _)| *s).collect();

        let mut rows = Vec::new();
        let mut row_bgs = Vec::new();
        let mut row_bold = Vec::new();
        let mut end = r + 1;
        while end < region.bottom {
            let cells = region.text_cells(view, mask, end);
            if cells.is_empty() || row_key(view, region, mask, end) == Some(key) {
                break;
            }
            let row_end = region.row_end(view, mask, end);
            let row = (0..starts.len())
                .map(|i| {
                    let from = if i == 0 { region.left } else { starts[i] };
                    let to = starts.get(i + 1).copied().unwrap_or(row_end).min(row_end);
                    region.slice(view, mask, end, from, to.max(from)).trim().to_owned()
                })
                .collect();
            rows.push(row);
            row_bgs.push(region.common_bg(view, mask, end..end + 1));
            row_bold.push(cells.iter().all(|&c| view.style(end, c).bold));
            end += 1;
        }
        for row in r..end {
            for c in region.left..region.right {
                if region.free(mask, row, c) {
                    mask.claim(row, c);
                }
            }
        }
        tables.push(Table {
            headers: headers.into_iter().map(|(_, _, t)| t).collect(),
            cursor: find_cursor(&row_bgs, &row_bold),
            rows,
        });
        r = end;
    }
    tables
}

/// The cursor row: the single row whose background differs from the other rows' common one. With fewer than three
/// rows there's no majority, so the single bold row (Textual's cursor style) is the cursor.
fn find_cursor(bgs: &[Option<&str>], bold: &[bool]) -> Option<usize> {
    if bgs.len() >= 3 {
        let common = most_common(bgs.iter().copied())?;
        let odd: Vec<usize> = (0..bgs.len()).filter(|&i| bgs[i] != common).collect();
        if bgs.iter().filter(|bg| **bg == common).count() >= 2 && odd.len() == 1 {
            return Some(odd[0]);
        }
        return None;
    }
    let bold_rows: Vec<usize> = (0..bold.len()).filter(|&i| bold[i]).collect();
    match bold_rows.as_slice() {
        [only] if bgs.len() == 1 || bgs[0] != bgs[1] => Some(*only),
        _ => None,
    }
}

/// The region's remaining text: each row's unclaimed runs of cells, trimmed, with scrollbar glyphs dropped and
/// empty parts skipped.
fn collect_text(view: &View<'_>, mask: &Mask, region: &Region) -> Vec<String> {
    let mut out = Vec::new();
    for r in region.top..region.bottom {
        let mut c = region.left;
        while c < region.right {
            if !region.free(mask, r, c) {
                c += 1;
                continue;
            }
            let start = c;
            while c < region.right && region.free(mask, r, c) {
                c += 1;
            }
            let part: String = (start..c).map(|x| view.text(r, x)).collect();
            let part = part.trim_end().trim_end_matches(|ch| SCROLLBAR.contains(ch)).trim();
            if !part.is_empty() {
                out.push(part.to_owned());
            }
        }
    }
    out
}

// The footer.

/// Reads the last screen row as a Textual footer, unless a box covers it: each run in the first run's style is a
/// key, and the runs after it up to the next key are its label. Returns the bindings and whether the row is full.
fn find_footer(view: &View<'_>, mask: &mut Mask, boxes: &[Found]) -> (Vec<Binding>, bool) {
    let Some(r) = view.rows.checked_sub(1) else {
        return (Vec::new(), false);
    };
    if boxes.iter().any(|b| (b.rect.row..=b.rect.bottom()).contains(&r)) {
        return (Vec::new(), false);
    }
    for c in 0..view.cols {
        mask.claim(r, c);
    }
    let mut runs: Vec<(&Style, String)> = Vec::new();
    for c in 0..view.cols {
        let style = view.style(r, c);
        match runs.last_mut() {
            Some((s, text)) if *s == style => text.push_str(view.text(r, c)),
            _ => runs.push((style, view.text(r, c).to_owned())),
        }
    }
    let Some(key_style) = runs.iter().find(|(_, t)| !t.trim().is_empty()).map(|(s, _)| *s) else {
        return (Vec::new(), false);
    };
    let mut bindings: Vec<Binding> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for (style, text) in &runs {
        if *style == key_style && !text.trim().is_empty() {
            bindings.push(Binding {
                key: text.trim().to_owned(),
                label: String::new(),
            });
            labels.push(String::new());
        } else if let Some(label) = labels.last_mut() {
            label.push_str(text);
        }
    }
    for (binding, label) in bindings.iter_mut().zip(labels) {
        label.trim().clone_into(&mut binding.label);
    }
    let clipped = view.cols > 0 && !view.is_space(r, view.cols - 1);
    (bindings, clipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::test_grid;

    fn dark() -> ThemeColors {
        ThemeColors {
            name: DEFAULT_THEME.into(),
            focus: Some("#4ebf71".into()),
        }
    }

    fn extract_fixture(name: &str) -> Structure {
        extract(&test_grid::load(name), &dark())
    }

    fn titles(s: &Structure) -> Vec<&str> {
        s.boxes.iter().map(|b| b.title.as_str()).collect()
    }

    #[test]
    fn finds_every_border_family() {
        let s = extract_fixture("families.txt");
        assert_eq!(titles(&s), ["Light", "Rounded", "Heavy", "Double", "Ascii"]);
        assert_eq!(s.boxes[0].rect, [0, 0, 11, 4]);
        assert_eq!(s.boxes[4].rect, [48, 0, 11, 4]);
        for b in &s.boxes {
            assert_eq!(b.text, ["hi"], "{}", b.title);
            assert_eq!(b.parent, None);
        }
        assert_eq!(s.boxes[0].subtitle, "sub");
        assert_eq!(s.boxes[4].subtitle, "my-sub");
    }

    #[test]
    fn small_or_open_ascii_shapes_are_not_boxes() {
        let s = extract(
            &test_grid::parse("+-+  a + b\n|x|  C++\n+-+\n+--+\n|  |\n+--\n"),
            &dark(),
        );
        assert_eq!(titles(&s), [""]);
        assert_eq!(s.boxes[0].rect, [0, 0, 3, 3]);
        let s = extract(&test_grid::parse("++\n++\n"), &dark());
        assert!(s.boxes.is_empty());
    }

    #[test]
    fn nests_boxes_in_pre_order() {
        let s = extract_fixture("nested.txt");
        let shape: Vec<(&str, Option<usize>)> = s.boxes.iter().map(|b| (b.title.as_str(), b.parent)).collect();
        assert_eq!(
            shape,
            [
                ("Outer", None),
                ("Inner A", Some(0)),
                ("Deep", Some(1)),
                ("Inner B", Some(0)),
                ("Side", None)
            ]
        );
        assert_eq!(s.boxes[0].text, ["outer text"]);
        assert_eq!(s.boxes[2].text, ["deep"]);
        assert_eq!(s.loose_text, ["loose"]);
    }

    #[test]
    fn splits_a_title_at_border_runs() {
        let s = extract(
            &test_grid::parse("┌─ A ─── B ─┐\n│           │\n└── x ─ y ──┘\n"),
            &dark(),
        );
        assert_eq!(s.boxes[0].title, "A | B");
        assert_eq!(s.boxes[0].subtitle, "x | y");
    }

    #[test]
    fn rejects_a_corner_of_another_color() {
        let s = extract_fixture("bad-corner.txt");
        assert_eq!(titles(&s), ["Good"]);
        assert!(s.loose_text.iter().any(|t| t.contains("Bad")), "{:?}", s.loose_text);
    }

    #[test]
    fn marks_the_focus_colored_box_focused() {
        let s = extract_fixture("bad-corner.txt");
        assert!(s.boxes[0].focused);
        assert_eq!(s.focused, ["Good"]);
        let unknown = extract(
            &test_grid::load("bad-corner.txt"),
            &ThemeColors {
                name: "nord".into(),
                focus: None,
            },
        );
        assert!(unknown.focused.is_empty());
        assert_eq!(unknown.warnings.len(), 1);
        assert!(unknown.warnings[0].contains("nord"));
    }

    #[test]
    fn finds_tabs_and_the_active_one() {
        let s = extract_fixture("tabs.txt");
        let b = &s.boxes[0];
        assert_eq!(
            b.tabs,
            Some(Tabs {
                names: vec!["Info".into(), "Changed Files".into(), "Checks".into()],
                active: Some("Changed Files".into()),
            })
        );
        assert_eq!(b.text, ["tab body"]);
        assert!(b.tables.is_empty());
    }

    #[test]
    fn a_single_color_rule_is_not_a_tab_bar() {
        let s = extract(
            &test_grid::parse("┌──────────┐\n│ Info     │\n│━━━━━━━━━━│\n└──────────┘\n"),
            &dark(),
        );
        assert_eq!(s.boxes[0].tabs, None);
        assert_eq!(s.boxes[0].text, ["Info", "━━━━━━━━━━"]);
    }

    #[test]
    fn reads_a_table_with_cursor_and_scrollbar() {
        let s = extract_fixture("table.txt");
        let b = &s.boxes[0];
        assert_eq!(b.title, "[1] Repositories");
        assert_eq!(
            b.tables,
            [Table {
                headers: vec!["Owner".into(), "Name".into(), "Description".into()],
                rows: vec![
                    vec!["parity-bot".into(), "alpha".into(), "First repo".into()],
                    vec!["parity-bot".into(), "beta".into(), "Second one".into()],
                    vec!["gizmo".into(), "gamma".into(), String::new()],
                    vec!["gizmo".into(), "delta".into(), "Last".into()],
                ],
                cursor: Some(1),
            }]
        );
        assert_eq!(b.text, ["after the table"]);
    }

    #[test]
    fn a_bold_line_without_a_background_is_not_a_header() {
        let s = extract(
            &test_grid::parse(
                "┌──────────┐\n│ Heading  │\n│          │\n│ body     │\n└──────────┘\n--- styles\n\n..bbbbbbb\n--- legend\nb fg=#0178d4 bold\n",
            ),
            &dark(),
        );
        assert!(s.boxes[0].tables.is_empty());
        assert_eq!(s.boxes[0].text, ["Heading", "body"]);
    }

    #[test]
    fn finds_the_cursor_in_short_tables() {
        assert_eq!(
            find_cursor(&[Some("#1"), Some("#2"), Some("#2")], &[true, false, false]),
            Some(0)
        );
        assert_eq!(find_cursor(&[Some("#2"), Some("#2"), Some("#2")], &[false; 3]), None);
        assert_eq!(find_cursor(&[Some("#1")], &[true]), Some(0));
        assert_eq!(find_cursor(&[Some("#1")], &[false]), None);
        assert_eq!(find_cursor(&[Some("#2"), Some("#1")], &[false, true]), Some(1));
        assert_eq!(find_cursor(&[Some("#2"), Some("#2")], &[false, true]), None);
        assert_eq!(find_cursor(&[], &[]), None);
    }

    #[test]
    fn reads_footer_bindings_from_alternating_styles() {
        let s = extract_fixture("footer.txt");
        let footer: Vec<String> = s.footer.iter().map(Binding::display).collect();
        assert_eq!(footer, ["q Quit", "^p Command Pallete", "? Keybindings"]);
        assert!(!s.footer_clipped);
        assert_eq!(s.loose_text, ["status line"]);
    }

    #[test]
    fn an_empty_last_row_has_no_footer() {
        let s = extract(&test_grid::parse("┌─ A ─┐\n│     │\n└─────┘\n\n"), &dark());
        assert!(s.footer.is_empty());
        let covered = extract(&test_grid::parse("┌─ A ─┐\n│     │\n└─────┘\n"), &dark());
        assert!(covered.footer.is_empty());
        assert_eq!(covered.boxes.len(), 1);
    }

    #[test]
    fn themes_toml_parses_and_rejects_bad_colors() {
        let themes = Themes::parse("[textual-dark]\nfocus = \"#4EBF71\"\n").unwrap();
        assert_eq!(themes.colors("textual-dark").focus.as_deref(), Some("#4ebf71"));
        assert_eq!(themes.colors("nord").focus, None);
        assert!(Themes::parse("[x]\nfocus = \"green\"\n").is_err());
        assert!(Themes::parse("[x]\nfocus = \"#000000\"\nother = 1\n").is_err());
        let repo = Themes::load(&crate::parity::cli::repo_root().join("parity/themes.toml")).unwrap();
        assert_eq!(repo.colors(DEFAULT_THEME), dark());
    }

    fn snapshot(size: &str) -> String {
        let s = extract(&test_grid::load(&format!("python/startup-{size}.grid.json")), &dark());
        serde_json::to_string_pretty(&s).unwrap()
    }

    #[test]
    fn python_startup_160x48() {
        insta::assert_snapshot!(snapshot("160x48"));
    }

    #[test]
    fn python_startup_100x30() {
        insta::assert_snapshot!(snapshot("100x30"));
    }
}
