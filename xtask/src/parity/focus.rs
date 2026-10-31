//! `wait.focused`: where a screen dump shows focus.
//!
//! Both apps draw focus the same handful of ways, and [`focused`] reads each from the screen alone, in the colors the
//! scenario's theme gives (`parity/themes.toml`):
//!
//! - **Pane**: a closed box outline (light, rounded, heavy, double, ascii, or Textual's heavy-dashed `┏╍┓╏┗┛`) whose
//!   border cells are all in the focus color. Labelled by its title, or by its interior text when it has none.
//! - **Field**: a Textual `tall` outline (`▊` left, `▔` top, `▁` bottom, `▎` right) all in the focused-border color.
//!   Labelled by its interior text, the text on the row above it and the text to its left.
//! - **Button**: a run of at least two non-blank cells that are bold and inverse. Labelled by its text.
//! - **Cursor**: a run of cells on the block-cursor background. Labelled by its text.
//!
//! The module is self-contained on purpose: it traces its own outlines rather than using `structure.rs`, so changes
//! to the structural extractor can't move capture timing. It's a reference input (`HARNESS_INPUTS`) for the same
//! reason. The rules and their limits are documented in `parity/README.md` (`wait.focused`).

use std::fmt;

use super::grid::{Grid, Style};

/// The theme colors [`focused`] reads. A kind whose color is `None` finds nothing; buttons need no color.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusColors {
    /// The focused pane's border, Textual's `$success` (`focus` in `themes.toml`).
    pub pane: Option<String>,
    /// A focused field's `tall` outline, Textual's `$border` (`border`).
    pub border: Option<String>,
    /// The focused widget's cursor background, Textual's `$block-cursor-background` (`block_cursor`).
    pub block_cursor: Option<String>,
}

/// The kinds of focus marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FocusKind {
    Pane,
    Field,
    Button,
    Cursor,
}

impl FocusKind {
    pub fn name(self) -> &'static str {
        match self {
            FocusKind::Pane => "pane",
            FocusKind::Field => "field",
            FocusKind::Button => "button",
            FocusKind::Cursor => "cursor",
        }
    }
}

/// One focus marker on screen and the labels a `wait.focused` string is matched against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focused {
    pub kind: FocusKind,
    /// Never empty, and no label is empty.
    pub labels: Vec<String>,
}

impl Focused {
    /// Whether any label contains `needle`.
    pub fn matches(&self, needle: &str) -> bool {
        self.labels.iter().any(|l| l.contains(needle))
    }
}

/// How long a label may get in [`describe`] before it's cut.
const DESCRIBE_LABEL_CHARS: usize = 40;

impl fmt::Display for Focused {
    /// `field "Issue number" / "Issue Number:"`: the kind and each label, long ones cut.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.name())?;
        for (i, label) in self.labels.iter().enumerate() {
            let sep = if i == 0 { " " } else { " / " };
            if label.chars().count() > DESCRIBE_LABEL_CHARS {
                let cut: String = label.chars().take(DESCRIBE_LABEL_CHARS).collect();
                write!(f, "{sep}{:?}", format!("{cut}…"))?;
            } else {
                write!(f, "{sep}{label:?}")?;
            }
        }
        Ok(())
    }
}

/// Every marker in `found`, comma-separated, or `nothing`: what a timed-out `wait.focused` reports.
pub fn describe(found: &[Focused]) -> String {
    if found.is_empty() {
        return "nothing".to_owned();
    }
    found.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

/// Every focus marker on `grid`: panes, then fields, buttons and cursors, each top to bottom. Pure: no I/O.
pub fn focused(grid: &Grid, colors: &FocusColors) -> Vec<Focused> {
    let view = View::new(grid);
    let mut found = Vec::new();
    if let Some(color) = &colors.pane {
        found.extend(panes(&view, color));
    }
    if let Some(color) = &colors.border {
        found.extend(fields(&view, color));
    }
    found.extend(buttons(&view));
    if let Some(color) = &colors.block_cursor {
        found.extend(cursors(&view, color));
    }
    found
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

struct View<'a> {
    rows: usize,
    cols: usize,
    cells: Vec<&'a [String]>,
    styles: Vec<Vec<&'a Style>>,
}

impl<'a> View<'a> {
    fn new(grid: &'a Grid) -> Self {
        let cols = usize::from(grid.cols);
        let styles = grid
            .lines
            .iter()
            .map(|line| {
                let mut row = vec![&BLANK; cols];
                for run in &line.runs {
                    let start = usize::from(run.col);
                    for slot in row.iter_mut().skip(start).take(usize::from(run.len)) {
                        *slot = &run.style;
                    }
                }
                row
            })
            .collect();
        Self {
            rows: grid.lines.len(),
            cols,
            cells: grid.lines.iter().map(|l| l.cells.as_slice()).collect(),
            styles,
        }
    }

    fn text(&self, r: usize, c: usize) -> &str {
        self.cells[r].get(c).map_or(" ", String::as_str)
    }

    /// The cell's character, when it holds exactly one.
    fn ch(&self, r: usize, c: usize) -> Option<char> {
        let mut chars = self.text(r, c).chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    }

    fn style(&self, r: usize, c: usize) -> &Style {
        self.styles[r].get(c).copied().unwrap_or(&BLANK)
    }

    fn fg_is(&self, r: usize, c: usize, color: &str) -> bool {
        self.style(r, c).fg.as_deref() == Some(color)
    }

    /// Columns `from..to` of row `r` as text.
    fn span(&self, r: usize, from: usize, to: usize) -> String {
        (from..to.min(self.cols)).map(|c| self.text(r, c)).collect()
    }
}

/// Box-drawing and block-element characters: outlines, never label text.
fn is_outline_glyph(ch: char) -> bool {
    ('\u{2500}'..='\u{259f}').contains(&ch)
}

/// A label as matched: outline glyphs dropped, ends trimmed. `None` when nothing is left.
fn label(text: &str) -> Option<String> {
    let cleaned: String = text.chars().filter(|&ch| !is_outline_glyph(ch)).collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Rows `top..bottom`, columns `left..right`, each row trimmed and the non-blank ones joined by a space.
fn interior(view: &View<'_>, top: usize, bottom: usize, left: usize, right: usize) -> Option<String> {
    let rows: Vec<String> = (top..bottom)
        .filter_map(|r| label(&view.span(r, left, right)))
        .collect();
    label(&rows.join(" "))
}

// Panes.

/// A box family. Heavy includes Textual's heavy-dashed edges (`╍`, `╏`), which share its corners.
struct Family {
    /// Top-left, top-right, bottom-left, bottom-right.
    corners: [char; 4],
    horizontals: &'static [char],
    verticals: &'static [char],
}

const TL: usize = 0;
const TR: usize = 1;
const BL: usize = 2;
const BR: usize = 3;

const FAMILIES: [Family; 5] = [
    Family {
        corners: ['┌', '┐', '└', '┘'],
        horizontals: &['─'],
        verticals: &['│'],
    },
    Family {
        corners: ['╭', '╮', '╰', '╯'],
        horizontals: &['─'],
        verticals: &['│'],
    },
    Family {
        corners: ['┏', '┓', '┗', '┛'],
        horizontals: &['━', '╍'],
        verticals: &['┃', '╏'],
    },
    Family {
        corners: ['╔', '╗', '╚', '╝'],
        horizontals: &['═'],
        verticals: &['║'],
    },
    Family {
        corners: ['+', '+', '+', '+'],
        horizontals: &['-'],
        verticals: &['|'],
    },
];

impl Family {
    fn is_ascii(&self) -> bool {
        self.horizontals == ['-']
    }

    /// Whether `ch` can close this family's box at `corner`. Box-drawing families accept each other's corners, as
    /// Textual draws where outlines meet; ascii only `+`.
    fn corner(&self, corner: usize, ch: char) -> bool {
        if self.is_ascii() {
            ch == '+'
        } else {
            FAMILIES.iter().any(|f| !f.is_ascii() && f.corners[corner] == ch)
        }
    }

    /// A character of some border, which can't be title text.
    fn is_border(&self, ch: char) -> bool {
        if self.is_ascii() {
            matches!(ch, '+' | '-' | '|')
        } else {
            ('\u{2500}'..='\u{257f}').contains(&ch)
        }
    }

    fn min_size(&self) -> usize {
        if self.is_ascii() { 3 } else { 2 }
    }
}

fn panes(view: &View<'_>, color: &str) -> Vec<Focused> {
    let mut found = Vec::new();
    for r in 0..view.rows {
        for c in 0..view.cols {
            let Some(ch) = view.ch(r, c) else { continue };
            if !view.fg_is(r, c, color) {
                continue;
            }
            let Some((family, right, bottom)) = FAMILIES
                .iter()
                .filter(|f| f.corners[TL] == ch)
                .find_map(|f| trace(view, f, color, r, c).map(|(right, bottom)| (f, right, bottom)))
            else {
                continue;
            };
            let labels = match label(&edge_title(view, family, r, c + 1, right)) {
                Some(title) => vec![title],
                None => interior(view, r + 1, bottom, c + 1, right).into_iter().collect(),
            };
            if !labels.is_empty() {
                found.push(Focused {
                    kind: FocusKind::Pane,
                    labels,
                });
            }
        }
    }
    found
}

/// Follows a box outline from its top-left corner at `(r, c)`, requiring every border cell in `color`. Returns the
/// right column and bottom row; `None` when it isn't a closed box all in `color`.
fn trace(view: &View<'_>, f: &Family, color: &str, r: usize, c: usize) -> Option<(usize, usize)> {
    let on = |y: usize, x: usize| view.fg_is(y, x, color);

    // Top edge: horizontals and title text up to the first top-right corner.
    let mut right = None;
    for x in c + 1..view.cols {
        match view.ch(r, x) {
            Some(ch) if f.corner(TR, ch) => {
                right = Some(x);
                break;
            }
            Some(ch) if f.horizontals.contains(&ch) => {
                if !on(r, x) {
                    return None;
                }
            }
            Some(ch) if f.is_border(ch) => return None,
            _ => {}
        }
    }
    let right = right.filter(|&x| on(r, x))?;

    // Left side down to a bottom-left corner.
    let mut bottom = None;
    for y in r + 1..view.rows {
        match view.ch(y, c) {
            Some(ch) if f.corner(BL, ch) && on(y, c) => {
                bottom = Some(y);
                break;
            }
            Some(ch) if f.verticals.contains(&ch) && on(y, c) => {}
            _ => return None,
        }
    }
    let bottom = bottom?;

    // Right side, bottom-right corner, bottom edge (horizontals and subtitle text).
    for y in r + 1..bottom {
        if !view.ch(y, right).is_some_and(|ch| f.verticals.contains(&ch)) || !on(y, right) {
            return None;
        }
    }
    if !view.ch(bottom, right).is_some_and(|ch| f.corner(BR, ch)) || !on(bottom, right) {
        return None;
    }
    for x in c + 1..right {
        match view.ch(bottom, x) {
            Some(ch) if f.horizontals.contains(&ch) => {
                if !on(bottom, x) {
                    return None;
                }
            }
            Some(ch) if f.is_border(ch) => return None,
            _ => {}
        }
    }
    let (width, height) = (right - c + 1, bottom - r + 1);
    (width >= f.min_size() && height >= f.min_size()).then_some((right, bottom))
}

/// A top edge's text between the corners, split into segments by runs of the horizontal border and joined with
/// `" | "`. In an ascii box a single `-` inside text (`my-repo`) isn't a separator.
fn edge_title(view: &View<'_>, f: &Family, row: usize, from: usize, to: usize) -> String {
    let cells: Vec<&str> = (from..to).map(|x| view.text(row, x)).collect();
    let is_horizontal = |s: &str| {
        let mut chars = s.chars();
        matches!((chars.next(), chars.next()), (Some(ch), None) if f.horizontals.contains(&ch))
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

// Fields.

const TALL_LEFT: char = '▊';
const TALL_RIGHT: char = '▎';
const TALL_TOP: char = '▔';
const TALL_BOTTOM: char = '▁';

fn fields(view: &View<'_>, color: &str) -> Vec<Focused> {
    let mut found = Vec::new();
    for r in 0..view.rows {
        for c in 0..view.cols {
            let Some((right, bottom)) = trace_tall(view, color, r, c) else {
                continue;
            };
            let mut labels = Vec::new();
            labels.extend(interior(view, r + 1, bottom, c + 1, right));
            if r > 0 {
                labels.extend(label(&view.span(r - 1, c, right + 1)));
            }
            for y in r..=bottom {
                let start = (0..c)
                    .rev()
                    .find(|&x| view.ch(y, x).is_some_and(is_outline_glyph))
                    .map_or(0, |x| x + 1);
                labels.extend(label(&view.span(y, start, c)));
            }
            labels.dedup();
            if !labels.is_empty() {
                found.push(Focused {
                    kind: FocusKind::Field,
                    labels,
                });
            }
        }
    }
    found
}

/// Follows a `tall` outline from its top-left `▊` at `(r, c)`, every cell in `color`. Returns the right column and
/// bottom row.
fn trace_tall(view: &View<'_>, color: &str, r: usize, c: usize) -> Option<(usize, usize)> {
    let is = |y: usize, x: usize, glyph: char| view.ch(y, x) == Some(glyph) && view.fg_is(y, x, color);
    if !is(r, c, TALL_LEFT) {
        return None;
    }
    let right = (c + 1..view.cols).find(|&x| !is(r, x, TALL_TOP))?;
    if right == c + 1 || !is(r, right, TALL_RIGHT) {
        return None;
    }
    for y in r + 1..view.rows {
        if !is(y, c, TALL_LEFT) || !is(y, right, TALL_RIGHT) {
            return None;
        }
        if y > r + 1 && (c + 1..right).all(|x| is(y, x, TALL_BOTTOM)) {
            return Some((right, y));
        }
    }
    None
}

// Buttons and cursors: runs of cells on one row.

/// The runs of cells on each row for which `inside` holds, as their trimmed text, with the number of non-blank
/// cells in each.
fn runs(view: &View<'_>, inside: impl Fn(&Style) -> bool) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for r in 0..view.rows {
        let mut text = String::new();
        let mut filled = 0;
        for c in 0..=view.cols {
            if c < view.cols && inside(view.style(r, c)) {
                let cell = view.text(r, c);
                text.push_str(cell);
                filled += usize::from(!cell.trim().is_empty());
            } else if !text.is_empty() {
                found.push((std::mem::take(&mut text), std::mem::take(&mut filled)));
            }
        }
    }
    found
}

/// A focused button: Textual draws its label in `$button-focus-text-style`, bold reverse in the built-in themes.
/// Single cells are left out, so a reversed cursor cell in bold text isn't a button.
fn buttons(view: &View<'_>) -> Vec<Focused> {
    runs(view, |s| s.bold && s.inverse)
        .into_iter()
        .filter(|(_, filled)| *filled >= 2)
        .filter_map(|(text, _)| label(&text))
        .map(|l| Focused {
            kind: FocusKind::Button,
            labels: vec![l],
        })
        .collect()
}

fn cursors(view: &View<'_>, color: &str) -> Vec<Focused> {
    runs(view, |s| s.bg.as_deref() == Some(color))
        .into_iter()
        .filter_map(|(text, _)| label(&text))
        .map(|l| Focused {
            kind: FocusKind::Cursor,
            labels: vec![l],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::structure::{self, ThemeColors};
    use super::super::test_grid;
    use super::*;

    fn dark() -> FocusColors {
        FocusColors {
            pane: Some("#4ebf71".into()),
            border: Some("#0178d4".into()),
            block_cursor: Some("#0178d4".into()),
        }
    }

    fn python(name: &str) -> Vec<Focused> {
        focused(&test_grid::load(&format!("python/{name}.grid.json")), &dark())
    }

    fn of_kind(found: &[Focused], kind: FocusKind) -> Vec<&Focused> {
        found.iter().filter(|f| f.kind == kind).collect()
    }

    fn marker(kind: FocusKind, labels: &[&str]) -> Focused {
        Focused {
            kind,
            labels: labels.iter().map(|&l| l.to_owned()).collect(),
        }
    }

    #[test]
    fn python_startup_has_the_focused_pane_structure_finds_and_its_table_cursor() {
        for size in ["160x48", "100x30"] {
            let grid = test_grid::load(&format!("python/startup-{size}.grid.json"));
            let found = focused(&grid, &dark());
            let panes: Vec<&str> = of_kind(&found, FocusKind::Pane)
                .iter()
                .map(|f| f.labels[0].as_str())
                .collect();
            assert_eq!(panes, ["[1] Repositories"], "{size}");
            let theme = ThemeColors {
                name: "textual-dark".into(),
                focus: Some("#4ebf71".into()),
            };
            assert_eq!(panes, structure::extract(&grid, &theme).focused, "{size}");
            // The focused table's highlighted row is the other marker.
            assert!(of_kind(&found, FocusKind::Field).is_empty());
            assert!(of_kind(&found, FocusKind::Button).is_empty());
            let cursors = of_kind(&found, FocusKind::Cursor);
            assert_eq!(cursors.len(), 1, "{size}: {found:?}");
            assert!(cursors[0].matches("parity-bot  dotfiles"), "{size}: {found:?}");
        }
    }

    #[test]
    fn python_issue_new_empty_title_focuses_save() {
        let found = python("issue-new-empty-title-160x48");
        assert_eq!(found, [marker(FocusKind::Button, &["Save"])]);
        assert_eq!(describe(&found), "button \"Save\"");
    }

    #[test]
    fn python_issue_new_typed_focuses_the_description() {
        let found = python("issue-new-typed-160x48");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, FocusKind::Field);
        assert_eq!(found[0].labels[1], "Description");
        assert!(found[0].matches("First line"));
        assert!(!found[0].matches("Parity title"));
    }

    #[test]
    fn python_issue_lookup_open_focuses_the_number_field() {
        let found = python("issue-lookup-open-160x48");
        assert_eq!(found, [marker(FocusKind::Field, &["Issue number", "Issue Number:"])]);
    }

    #[test]
    fn python_settings_save_edited_focuses_a_switch_labelled_on_its_left() {
        let found = python("settings-save-edited-160x48");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, FocusKind::Field);
        assert!(found[0].labels.iter().any(|l| l == "Enabled:"), "{found:?}");
        assert!(!found[0].matches("Mark Notification"));
    }

    #[test]
    fn python_pr_maximize_reviews_title_is_a_cursor() {
        let found = python("pr-maximize-reviews-title-160x48");
        assert_eq!(
            found,
            [marker(FocusKind::Cursor, &["▶ PR Reviews: ✘ Changes Requested"])]
        );
    }

    #[test]
    fn python_issue_details_comment_focused_is_a_dashed_pane() {
        let found = python("issue-details-comment-focused-160x48");
        let panes = of_kind(&found, FocusKind::Pane);
        assert_eq!(panes.len(), 2, "{found:?}");
        assert_eq!(panes[0].labels, ["[5] Issue #12 Details"]);
        assert!(panes[1].matches("Thanks @parity-friend"), "{found:?}");
        assert!(!panes[1].matches("deleted account"), "{found:?}");
    }

    #[test]
    fn an_outline_of_mixed_colors_is_not_focused() {
        let grid = test_grid::parse(
            "┌─ Pane ─┐\n│ inside │\n└────────┘\n▊▔▔▔▔▎\n▊ ab ▎\n▊▁▁▁▁▎\n\
             --- styles\ngggggggggg\ng........g\nggggxggggg\nbbbbbb\nb....b\nbbbxbb\n\
             --- legend\ng fg=#4ebf71\nb fg=#0178d4\nx fg=#ffffff\n",
        );
        assert_eq!(focused(&grid, &dark()), []);
        // The same outlines all in one color are.
        let grid = test_grid::parse(
            "┌─ Pane ─┐\n│ inside │\n└────────┘\n▊▔▔▔▔▎\n▊ ab ▎\n▊▁▁▁▁▎\n\
             --- styles\ngggggggggg\ng........g\ngggggggggg\nbbbbbb\nb....b\nbbbbbb\n\
             --- legend\ng fg=#4ebf71\nb fg=#0178d4\n",
        );
        assert_eq!(
            focused(&grid, &dark()),
            [marker(FocusKind::Pane, &["Pane"]), marker(FocusKind::Field, &["ab"])]
        );
    }

    #[test]
    fn a_single_bold_inverse_cell_is_not_a_button() {
        let grid = test_grid::parse("x OK y\n--- styles\ni.ii.i\n--- legend\ni bold inverse\n");
        let found = focused(&grid, &dark());
        assert_eq!(found, [marker(FocusKind::Button, &["OK"])]);
        let grid = test_grid::parse(" x \n--- styles\niii\n--- legend\ni bold inverse\n");
        assert_eq!(focused(&grid, &dark()), []);
    }

    #[test]
    fn a_missing_color_finds_nothing_for_its_kind() {
        let grid = test_grid::parse(
            "┌─ Pane ─┐\n└────────┘\n▊▔▔▔▔▎\n▊ ab ▎\n▊▁▁▁▁▎\n Tab \n\
             --- styles\ngggggggggg\ngggggggggg\nbbbbbb\nb....b\nbbbbbb\nccccc\n\
             --- legend\ng fg=#4ebf71\nb fg=#0178d4\nc bg=#0178d4\n",
        );
        assert_eq!(focused(&grid, &dark()).len(), 3);
        let only = |colors: FocusColors| -> Vec<FocusKind> { focused(&grid, &colors).iter().map(|f| f.kind).collect() };
        assert_eq!(
            only(FocusColors { pane: None, ..dark() }),
            [FocusKind::Field, FocusKind::Cursor]
        );
        assert_eq!(
            only(FocusColors { border: None, ..dark() }),
            [FocusKind::Pane, FocusKind::Cursor]
        );
        assert_eq!(
            only(FocusColors {
                block_cursor: None,
                ..dark()
            }),
            [FocusKind::Pane, FocusKind::Field]
        );
        assert_eq!(only(FocusColors::default()), []);
    }

    #[test]
    fn describes_what_is_focused() {
        assert_eq!(describe(&[]), "nothing");
        let long = "x".repeat(50);
        assert_eq!(
            describe(&[
                marker(FocusKind::Field, &["a", "b"]),
                marker(FocusKind::Pane, &["[3] Issues"]),
                marker(FocusKind::Cursor, &[&long]),
            ]),
            format!(
                "field \"a\" / \"b\", pane \"[3] Issues\", cursor \"{}…\"",
                "x".repeat(40)
            )
        );
    }
}
