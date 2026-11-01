//! The continuous diff body (#344, decision `diff-continuous-view`): every shown file of a [`Diff`] as one list of
//! rows, with one line cursor that crosses hunks and files.
//!
//! Per shown file, in diff order: a header row (its full path and counts), then unless the file is folded a
//! separator row per hunk followed by the hunk's lines, or for a deleted file one `file was removed` row. A
//! [`RowIndex`] maps row numbers to [`Row`]s from per-file and per-hunk row counts, so it's rebuilt in O(files) when a
//! file folds or the filter changes, and looked up in O(log n), with no list of every line. The cursor is kept as
//! what it's on ([`Cursor`]), never as a row number, so folding or filtering other files never moves it to another
//! line. Drawing lays out and colors only the rows on screen.
//!
//! The Diff tab (`DiffView`) owns it and scrolls it together with the pending comments and the review form below it:
//! [`DiffBody::fit`], [`DiffBody::set_scroll`] and [`DiffBody::draw`] are for that owner, and [`DiffBody::render`]
//! draws the body on its own, with its own scrollbar.

use std::ops::Range;

use lazy_github_core::diff::{ChangedFile, Counts, Diff, LineKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget as _};

use super::highlight::HunkCode;
use super::review::DiffLineRef;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::display;
use crate::keymap::BindingId;
use crate::theme::{Paint, Rgb, Theme};

/// The body's chain link, in footer order: `j Line down  k Line up  c Add Comment  J Next Hunk  K Previous Hunk  /
/// Filter Files`, then the hidden scroll keys. Add Comment and Filter Files are the Diff tab's to act on, so
/// [`DiffBody::handle`] passes on them.
pub const DIFF_BODY_BINDINGS: &[BindingId] = &[
    BindingId::DiffCursorDown,
    BindingId::DiffCursorUp,
    BindingId::DiffAddComment,
    BindingId::DiffNextHunk,
    BindingId::DiffPreviousHunk,
    BindingId::DiffFilterFiles,
    BindingId::TextualScrollPageDown,
    BindingId::TextualScrollPageUp,
    BindingId::TextualScrollHome,
    BindingId::TextualScrollEnd,
    BindingId::TextualScrollLeft,
    BindingId::TextualScrollRight,
];

/// A line row's gutter: `► ` or two spaces, the number in 4 cells, the flag cell (#390 and #391 mark comments in
/// it), then ` │ `.
pub const GUTTER_WIDTH: usize = 10;
/// How many rows the cursor keeps from the top and bottom edges, when there's room.
const SCROLL_MARGIN: usize = 3;
/// How many cells `left` and `right` pan the code.
const PAN_STEP: usize = 8;
/// What an unfolded deleted file shows in place of its hunks.
const FILE_REMOVED: &str = "file was removed";

/// Where the cursor is, by what it's on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// A file's header row, by the file's index in the diff.
    File(usize),
    /// A diff line.
    Line(DiffLineRef),
    /// An unfolded deleted file's `file was removed` row.
    Removed(usize),
}

impl Cursor {
    /// The file the cursor is in.
    pub fn file(self) -> usize {
        self.row().file()
    }

    /// The row the cursor is on.
    pub fn row(self) -> Row {
        match self {
            Cursor::File(f) => Row::FileHeader(f),
            Cursor::Line(line) => Row::Line(line),
            Cursor::Removed(f) => Row::Removed(f),
        }
    }
}

/// One row of the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A file's header.
    FileHeader(usize),
    /// The separator before a hunk: the file, then the hunk.
    HunkSeparator(usize, usize),
    /// A diff line.
    Line(DiffLineRef),
    /// An unfolded deleted file's `file was removed`.
    Removed(usize),
}

impl Row {
    /// The file the row belongs to.
    pub fn file(self) -> usize {
        match self {
            Row::FileHeader(f) | Row::HunkSeparator(f, _) | Row::Removed(f) => f,
            Row::Line(line) => line.file,
        }
    }

    /// The cursor on this row, when the row can hold it: every row but a hunk separator.
    pub fn cursor(self) -> Option<Cursor> {
        match self {
            Row::FileHeader(f) => Some(Cursor::File(f)),
            Row::HunkSeparator(..) => None,
            Row::Line(line) => Some(Cursor::Line(line)),
            Row::Removed(f) => Some(Cursor::Removed(f)),
        }
    }
}

/// What the position row under the body says: the cursor's file on the left, and on the right `Line: n/m` on a line
/// or `N hunks` on a header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorPosition {
    /// The cursor's file's path.
    pub path: String,
    /// The right-hand side; empty on a `file was removed` row.
    pub detail: String,
}

/// One file of the diff and how it shows.
#[derive(Debug, Clone)]
struct FileState {
    file: ChangedFile,
    folded: bool,
    /// Whether the filter keeps it.
    shown: bool,
    /// Each hunk's separator row, counted from the file's header (row 0) with the file unfolded. A hunk's lines
    /// follow its separator one row each; the inline comments of #390 will add rows inside a hunk, and then this
    /// and the line-to-row steps in [`FileState::local_row`] and [`FileState::local_of`] count them.
    hunk_starts: Vec<usize>,
    /// The rows the file takes unfolded, its header included.
    unfolded: usize,
    /// Each hunk's code, made the first time a row of it is drawn: highlighting is slow, and most hunks of a large
    /// diff are never on screen.
    code: Vec<Option<HunkCode>>,
    /// The widest line of the file's hunks, in cells, once panning has needed it.
    widest: Option<usize>,
}

impl FileState {
    /// Unfolded, but a deleted file folded, as files open today.
    fn new(file: ChangedFile) -> Self {
        let mut hunk_starts = Vec::with_capacity(file.hunks.len());
        let mut next = 1;
        for hunk in &file.hunks {
            hunk_starts.push(next);
            next += 1 + hunk.lines.len();
        }
        Self {
            folded: file.deleted,
            shown: true,
            unfolded: if file.deleted { 2 } else { next },
            code: vec![None; file.hunks.len()],
            widest: None,
            hunk_starts,
            file,
        }
    }

    /// The rows the file takes now.
    fn rows(&self) -> usize {
        if self.folded { 1 } else { self.unfolded }
    }

    /// The row `local` rows below the header of this file, file `f` of the diff. `local` is below [`Self::rows`].
    fn local_row(&self, f: usize, local: usize) -> Row {
        if local == 0 || self.folded {
            return Row::FileHeader(f);
        }
        if self.file.deleted {
            return Row::Removed(f);
        }
        // The last hunk whose separator is at or above `local`; the first separator is row 1.
        let h = self
            .hunk_starts
            .partition_point(|&start| start <= local)
            .saturating_sub(1);
        match local - self.hunk_starts.get(h).copied().unwrap_or(1) {
            0 => Row::HunkSeparator(f, h),
            k => Row::Line(DiffLineRef {
                file: f,
                hunk: h,
                line: k - 1,
            }),
        }
    }

    /// How many rows below the header `row` is, when this file shows it.
    fn local_of(&self, row: Row) -> Option<usize> {
        match row {
            Row::FileHeader(_) => Some(0),
            _ if self.folded => None,
            Row::Removed(_) => self.file.deleted.then_some(1),
            _ if self.file.deleted => None,
            Row::HunkSeparator(_, h) => self.hunk_starts.get(h).copied(),
            Row::Line(line) => {
                let hunk = self.file.hunks.get(line.hunk)?;
                (line.line < hunk.lines.len()).then(|| self.hunk_starts[line.hunk] + 1 + line.line)
            }
        }
    }

    /// The widest line of the file's hunks, in cells, as drawn.
    fn widest(&mut self) -> usize {
        if let Some(widest) = self.widest {
            return widest;
        }
        let file = &self.file;
        let widest = file
            .hunks
            .iter()
            .zip(&mut self.code)
            .map(|(hunk, code)| code.get_or_insert_with(|| HunkCode::new(file, hunk)).widest())
            .max()
            .unwrap_or(0);
        self.widest = Some(widest);
        widest
    }
}

/// Maps between row numbers and [`Row`]s, over the shown files.
#[derive(Debug, Clone, Default)]
struct RowIndex {
    /// The shown files in order: each one's header row, and its index in the diff.
    starts: Vec<(usize, usize)>,
    /// For each file of the diff, its place in `starts` when it's shown.
    slots: Vec<Option<usize>>,
    /// Every row.
    total: usize,
}

impl RowIndex {
    fn build(files: &[FileState]) -> Self {
        let mut index = Self {
            slots: vec![None; files.len()],
            ..Self::default()
        };
        for (f, file) in files.iter().enumerate().filter(|(_, file)| file.shown) {
            index.slots[f] = Some(index.starts.len());
            index.starts.push((index.total, f));
            index.total += file.rows();
        }
        index
    }

    fn row(&self, files: &[FileState], row: usize) -> Option<Row> {
        if row >= self.total {
            return None;
        }
        // The first file's header is row 0, so some file starts at or above any row.
        let i = self
            .starts
            .partition_point(|&(start, _)| start <= row)
            .saturating_sub(1);
        let &(start, f) = self.starts.get(i)?;
        Some(files[f].local_row(f, row - start))
    }

    fn row_of(&self, files: &[FileState], row: Row) -> Option<usize> {
        let f = row.file();
        let slot = (*self.slots.get(f)?)?;
        Some(self.starts[slot].0 + files[f].local_of(row)?)
    }
}

/// The continuous diff body: its files' fold and filter state, the cursor, and how far it's scrolled and panned.
#[derive(Debug, Clone)]
pub struct DiffBody {
    files: Vec<FileState>,
    index: RowIndex,
    cursor: Option<Cursor>,
    /// The first visible row.
    scroll: usize,
    /// How many code cells are panned off the left.
    pan: usize,
    /// The rows the body showed at the last draw (or [`Self::resize`]), which paging and the scroll margin go by.
    height: usize,
    /// The cells the code had at the last draw, which panning goes by.
    code_width: usize,
    /// The rows an owner draws after the position row, in the same scroll ([`Self::fit`]): the body may scroll that
    /// far past its own last row.
    trailing: usize,
}

impl DiffBody {
    /// The body of `diff`: every file unfolded but deleted ones, the cursor on the first file's header.
    pub fn new(diff: Diff) -> Self {
        let files: Vec<FileState> = diff.files.into_iter().map(FileState::new).collect();
        let mut body = Self {
            index: RowIndex::build(&files),
            files,
            cursor: None,
            scroll: 0,
            pan: 0,
            height: 0,
            code_width: 0,
            trailing: 0,
        };
        body.settle();
        body
    }

    /// Where the cursor is; `None` when no file is shown.
    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    /// The diff line under the cursor, which Add Comment comments on.
    pub fn cursor_line(&self) -> Option<DiffLineRef> {
        match self.cursor? {
            Cursor::Line(line) => Some(line),
            Cursor::File(_) | Cursor::Removed(_) => None,
        }
    }

    /// The first visible row.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// How many code cells are panned off the left.
    pub fn pan(&self) -> usize {
        self.pan
    }

    /// How many rows there are.
    pub fn rows(&self) -> usize {
        self.index.total
    }

    /// Row `row`, from 0.
    pub fn row(&self, row: usize) -> Option<Row> {
        self.index.row(&self.files, row)
    }

    /// The row number the cursor is on.
    pub fn cursor_row(&self) -> Option<usize> {
        self.index.row_of(&self.files, self.cursor?.row())
    }

    /// File `f` of the diff.
    pub fn file(&self, f: usize) -> Option<&ChangedFile> {
        self.files.get(f).map(|state| &state.file)
    }

    /// Whether file `f` is folded to its header.
    pub fn is_folded(&self, f: usize) -> bool {
        self.files.get(f).is_some_and(|state| state.folded)
    }

    /// Whether the filter shows file `f`.
    pub fn is_shown(&self, f: usize) -> bool {
        self.files.get(f).is_some_and(|state| state.shown)
    }

    /// Folds file `f` to its header row, or unfolds it. A cursor inside it moves to its header.
    pub fn set_folded(&mut self, f: usize, folded: bool) {
        let Some(state) = self.files.get_mut(f) else {
            return;
        };
        if state.folded != folded {
            state.folded = folded;
            self.reindex();
        }
    }

    /// Shows the files whose full path contains `needle`, ignoring case; every file when it's empty. A cursor in a
    /// file the filter hides moves to the first shown file's header.
    pub fn set_filter(&mut self, needle: &str) {
        let needle = needle.to_lowercase();
        for state in &mut self.files {
            state.shown = needle.is_empty() || state.file.path.to_lowercase().contains(&needle);
        }
        self.reindex();
    }

    /// Takes the size of the area [`Self::render`] is about to draw in, so the cursor keeps its margin and panning
    /// its limit as of that size: the position row takes the last row and the scrollbar the last two columns when
    /// the rows overflow.
    pub fn resize(&mut self, width: u16, height: u16) {
        let height = usize::from(height.saturating_sub(1));
        let bar = if self.index.total > height {
            usize::from(scrollbar::VERTICAL_WIDTH)
        } else {
            0
        };
        self.height = height;
        self.code_width = usize::from(width).saturating_sub(bar + GUTTER_WIDTH);
        self.trailing = 0;
        self.clamp_pan();
        self.follow();
    }

    /// Takes the size an owner that scrolls the body along with other content draws it at: `width` the cells its
    /// rows get (the owner's scrollbar left out), `rows` how many of them show at most (the position row not
    /// counted), and `trailing` the rows the owner draws after the position row in the same scroll. Unlike
    /// [`Self::resize`] it doesn't scroll: the owner says when the cursor is revealed ([`Self::reveal_cursor`]).
    pub fn fit(&mut self, width: u16, rows: usize, trailing: usize) {
        self.height = rows;
        self.code_width = usize::from(width).saturating_sub(GUTTER_WIDTH);
        self.trailing = trailing;
        self.clamp_pan();
    }

    /// Scrolls so row `row` is the first one an owner's scroll shows, without moving the cursor. It may be past the
    /// last row when the owner shows what follows the body ([`Self::fit`]'s `trailing`).
    pub fn set_scroll(&mut self, row: usize) {
        self.scroll = row;
    }

    /// Scrolls as little as possible to bring the cursor's row inside its margin, as a cursor move does.
    pub fn reveal_cursor(&mut self) {
        self.follow();
    }

    /// Moves the cursor, without scrolling, to the row nearest it among `rows` that can hold it, when it's outside
    /// them: what the wheel does once the cursor's row has scrolled out of view, as `less` and vim do.
    pub fn keep_cursor_in(&mut self, rows: Range<usize>) {
        let Some(row) = self.cursor_row() else {
            return;
        };
        if rows.is_empty() || rows.contains(&row) {
            return;
        }
        let last = rows.end - 1;
        let target = if row < rows.start {
            self.cursor_from(rows.start, true).filter(|&r| r <= last)
        } else {
            self.cursor_from(last, false).filter(|&r| r >= rows.start)
        };
        if let Some(cursor) = target.and_then(|r| self.row(r)).and_then(Row::cursor) {
            self.cursor = Some(cursor);
        }
    }

    /// A click on row `row`: the cursor moves to it, or from a hunk separator to the hunk's first line, without
    /// scrolling. Says whether the row was there.
    pub fn click_row(&mut self, row: usize) -> bool {
        let Some(target) = self.cursor_from(row, true) else {
            return false;
        };
        if let Some(cursor) = self.row(target).and_then(Row::cursor) {
            self.cursor = Some(cursor);
        }
        true
    }

    /// The first shown file's header takes the cursor, scrolled into view.
    pub fn cursor_to_first_file(&mut self) {
        if let Some(row) = self.cursor_from(0, true) {
            self.move_to(row);
        }
    }

    /// A binding of [`DIFF_BODY_BINDINGS`]. Says whether it's the body's to handle: every one but Add Comment and
    /// Filter Files, whether or not the cursor could move.
    pub fn handle(&mut self, id: BindingId) -> bool {
        match id {
            BindingId::DiffCursorDown => self.step(true),
            BindingId::DiffCursorUp => self.step(false),
            BindingId::DiffNextHunk => self.next_hunk(),
            BindingId::DiffPreviousHunk => self.previous_hunk(),
            BindingId::TextualScrollPageDown => self.page(true),
            BindingId::TextualScrollPageUp => self.page(false),
            BindingId::TextualScrollHome => {
                if let Some(row) = self.cursor_from(0, true) {
                    self.move_to(row);
                }
            }
            BindingId::TextualScrollEnd => {
                if let Some(row) = self
                    .index
                    .total
                    .checked_sub(1)
                    .and_then(|last| self.cursor_from(last, false))
                {
                    self.move_to(row);
                }
            }
            BindingId::TextualScrollLeft => self.pan = self.pan.saturating_sub(PAN_STEP),
            BindingId::TextualScrollRight => self.pan = (self.pan + PAN_STEP).min(self.max_pan()),
            _ => return false,
        }
        true
    }

    /// What the position row says, when there's a cursor.
    pub fn position(&self) -> Option<CursorPosition> {
        let cursor = self.cursor?;
        let file = &self.files.get(cursor.file())?.file;
        let detail = match cursor {
            Cursor::Line(line) => {
                let len = file.hunks.get(line.hunk).map_or(0, |hunk| hunk.lines.len());
                format!("Line: {}/{len}", line.line + 1)
            }
            Cursor::File(_) => match file.hunks.len() {
                1 => "1 hunk".to_owned(),
                n => format!("{n} hunks"),
            },
            Cursor::Removed(_) => String::new(),
        };
        Some(CursorPosition {
            path: display::sanitize(&file.path).into_owned(),
            detail,
        })
    }

    /// Rebuilds the row index after a fold or filter change, and keeps the cursor and the view in step.
    fn reindex(&mut self) {
        self.index = RowIndex::build(&self.files);
        self.settle();
        self.clamp_pan();
        self.follow();
    }

    /// Keeps the cursor on a shown row: a cursor inside a folded file goes to its header, one in a hidden file to the
    /// first shown file's header, and with no cursor yet, the first row takes it.
    fn settle(&mut self) {
        self.cursor = match self.cursor {
            Some(cursor) if self.index.row_of(&self.files, cursor.row()).is_some() => Some(cursor),
            Some(cursor) if self.is_shown(cursor.file()) => Some(Cursor::File(cursor.file())),
            _ => self.row(0).and_then(Row::cursor),
        };
    }

    /// The first row from `from` (itself included), going down or up, that can hold the cursor.
    fn cursor_from(&self, from: usize, down: bool) -> Option<usize> {
        let mut row = from;
        loop {
            if self.row(row)?.cursor().is_some() {
                return Some(row);
            }
            row = if down { row + 1 } else { row.checked_sub(1)? };
        }
    }

    /// Puts the cursor on row `row`, which can hold it, and scrolls to keep it in its margin.
    fn move_to(&mut self, row: usize) {
        if let Some(cursor) = self.row(row).and_then(Row::cursor) {
            self.cursor = Some(cursor);
            self.follow();
        }
    }

    /// `j` / `k`: the next or previous row that can hold the cursor, stopping at the ends.
    fn step(&mut self, down: bool) {
        let Some(row) = self.cursor_row() else {
            return;
        };
        let next = if down { Some(row + 1) } else { row.checked_sub(1) };
        if let Some(target) = next.and_then(|next| self.cursor_from(next, down)) {
            self.move_to(target);
        }
    }

    /// `pagedown` / `pageup`: the cursor moves by the visible height less one row, to the nearest row that can hold
    /// it, stopping at the ends.
    fn page(&mut self, down: bool) {
        let (Some(row), Some(last)) = (self.cursor_row(), self.index.total.checked_sub(1)) else {
            return;
        };
        let by = self.height.saturating_sub(1).max(1);
        let target = if down {
            (row + by).min(last)
        } else {
            row.saturating_sub(by)
        };
        if let Some(target) = self
            .cursor_from(target, down)
            .or_else(|| self.cursor_from(target, !down))
        {
            self.move_to(target);
        }
    }

    /// The hunks the cursor can jump to, in order: those with lines, of the shown files that aren't deleted, folded
    /// or not.
    fn hunks(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.files
            .iter()
            .enumerate()
            .filter(|(_, state)| state.shown && !state.file.deleted)
            .flat_map(|(f, state)| {
                state
                    .file
                    .hunks
                    .iter()
                    .enumerate()
                    .filter(|(_, hunk)| !hunk.lines.is_empty())
                    .map(move |(h, _)| (f, h))
            })
    }

    /// `J`: the next hunk's first line; from a header, that file's first hunk.
    fn next_hunk(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let after = match cursor {
            Cursor::Line(line) => (line.file, line.hunk + 1),
            Cursor::File(f) => (f, 0),
            Cursor::Removed(f) => (f + 1, 0),
        };
        let target = self.hunks().find(|&hunk| hunk >= after);
        if let Some(target) = target {
            self.jump_to(target);
        }
    }

    /// `K`: the first line of the cursor's hunk when the cursor is below it, else the previous hunk's first line;
    /// from a header, the last hunk before its file.
    fn previous_hunk(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let target = match cursor {
            Cursor::Line(line) if line.line > 0 => Some((line.file, line.hunk)),
            Cursor::Line(line) => self.hunks().take_while(|&hunk| hunk < (line.file, line.hunk)).last(),
            Cursor::File(f) | Cursor::Removed(f) => self.hunks().take_while(|&(g, _)| g < f).last(),
        };
        if let Some(target) = target {
            self.jump_to(target);
        }
    }

    /// The cursor on a hunk's first line, unfolding its file.
    fn jump_to(&mut self, (file, hunk): (usize, usize)) {
        self.set_folded(file, false);
        self.cursor = Some(Cursor::Line(DiffLineRef { file, hunk, line: 0 }));
        self.follow();
    }

    /// The most the code can pan: until the widest shown line's end is in view.
    fn max_pan(&mut self) -> usize {
        let widest = self
            .files
            .iter_mut()
            .filter(|state| state.shown && !state.folded && !state.file.deleted)
            .map(FileState::widest)
            .max()
            .unwrap_or(0);
        widest.saturating_sub(self.code_width)
    }

    /// Pulls the pan back after the lines or the width changed. Not panned, there's nothing to measure, which keeps
    /// a large diff from highlighting every hunk to find its widest line.
    fn clamp_pan(&mut self) {
        if self.pan > 0 {
            self.pan = self.pan.min(self.max_pan());
        }
    }

    /// Scrolls as little as possible to keep the cursor's row [`SCROLL_MARGIN`] rows from the top and bottom edges
    /// (fewer in a body too short for both), and never past the last row.
    fn follow(&mut self) {
        let height = self.height;
        if height == 0 {
            return;
        }
        if let Some(row) = self.cursor_row() {
            let margin = SCROLL_MARGIN.min(height.saturating_sub(1) / 2);
            if row < self.scroll + margin {
                self.scroll = row.saturating_sub(margin);
            } else if row + margin >= self.scroll + height {
                self.scroll = row + margin + 1 - height;
            }
        }
        self.scroll = self
            .scroll
            .min((self.index.total + self.trailing).saturating_sub(height));
    }

    /// Draws the visible rows in `area` less its last row, with a scrollbar when they overflow, then the position
    /// row in the last row. With no file shown, the area is left blank. The `►` and the block cursor on a header
    /// show only while `focused`; the cursor's line keeps its brighter background either way.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, focused: bool) {
        let area = area.intersection(buf.area);
        let styles = Styles::new(theme);
        Clear.render(area, buf);
        buf.set_style(area, Style::new().bg(theme.paint(styles.base)));
        if self.index.total == 0 || area.height == 0 {
            return;
        }
        self.resize(area.width, area.height);
        let height = self.height;
        let bar = self.index.total > height;
        let text_width = if bar {
            area.width.saturating_sub(scrollbar::VERTICAL_WIDTH)
        } else {
            area.width
        };
        let rows = Rect::new(area.x, area.y, text_width, area.height - 1);
        self.draw_rows(buf, rows, focused, theme, &styles);
        if bar {
            let rect = Rect::new(area.x + text_width, area.y, area.width - text_width, area.height - 1);
            let state = ScrollbarState {
                content: self.index.total,
                window: height,
                position: self.scroll,
            };
            scrollbar::render(buf, rect, Axis::Vertical, state, styles.scrollbar);
        }
        self.draw_position(buf, area, &styles);
    }

    /// Draws the rows from the scroll position in `area` less its last row, and the position row in the last row,
    /// with no scrollbar and without scrolling or taking the area's size: for an owner that scrolls the body along
    /// with other content ([`Self::fit`]), which gives it only the rows of the body it shows. With no file shown,
    /// the area is left blank.
    pub fn draw(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, focused: bool) {
        let area = area.intersection(buf.area);
        let styles = Styles::new(theme);
        Clear.render(area, buf);
        buf.set_style(area, Style::new().bg(theme.paint(styles.base)));
        if self.index.total == 0 || area.height == 0 {
            return;
        }
        let rows = Rect::new(area.x, area.y, area.width, area.height - 1);
        self.draw_rows(buf, rows, focused, theme, &styles);
        self.draw_position(buf, area, &styles);
    }

    /// The colors of the scrollbar beside the body, for an owner that draws its own.
    pub fn scrollbar_colors(theme: &Theme) -> ScrollbarColors {
        Styles::new(theme).scrollbar
    }

    /// The rows from the scroll position, as many as fit in `area`.
    fn draw_rows(&mut self, buf: &mut Buffer, area: Rect, focused: bool, theme: &Theme, styles: &Styles) {
        let cursor = self.cursor;
        let visible = self
            .index
            .total
            .saturating_sub(self.scroll)
            .min(usize::from(area.height));
        for dy in 0..visible {
            let Some(row) = self.row(self.scroll + dy) else {
                break;
            };
            // `dy` is below the area's height, a u16.
            let y = area.y + u16::try_from(dy).unwrap_or(u16::MAX);
            let rect = Rect::new(area.x, y, area.width, 1);
            let current = cursor.is_some_and(|cursor| cursor.row() == row);
            let (spans, fill) = self.row_spans(row, current, focused, usize::from(area.width), theme, styles);
            put(buf, rect, spans, fill);
        }
    }

    /// The position row, in `area`'s last row.
    fn draw_position(&self, buf: &mut Buffer, area: Rect, styles: &Styles) {
        if let Some(position) = self.position() {
            let rect = Rect::new(area.x, area.bottom() - 1, area.width, 1);
            put(
                buf,
                rect,
                position_spans(&position, usize::from(area.width), styles.position),
                styles.position,
            );
        }
    }

    /// Row `row`, `width` cells wide, as spans and the style the rest of the row is filled in.
    fn row_spans(
        &mut self,
        row: Row,
        current: bool,
        focused: bool,
        width: usize,
        theme: &Theme,
        styles: &Styles,
    ) -> (Vec<Span<'static>>, Style) {
        match row {
            Row::FileHeader(f) => {
                let state = &self.files[f];
                let style = if current && focused {
                    styles.header_cursor
                } else {
                    styles.header
                };
                (header_spans(&state.file, state.folded, style, styles), style)
            }
            Row::HunkSeparator(f, h) => {
                let label = self.files[f].file.hunks[h].label();
                let mut spans = vec![Span::styled("  ", styles.separator)];
                spans.push(Span::styled(
                    display::sanitize(&label.text).into_owned(),
                    styles.separator,
                ));
                spans.extend(count_spans(label.counts, styles.separator, styles));
                spans.push(Span::styled(
                    display::sanitize(&label.suffix).into_owned(),
                    styles.separator,
                ));
                (spans, styles.separator)
            }
            Row::Removed(_) => {
                let marker = if current && focused { "► " } else { "  " };
                (
                    vec![
                        Span::styled(marker, styles.gutter),
                        Span::styled(FILE_REMOVED, styles.removed),
                    ],
                    styles.gutter,
                )
            }
            Row::Line(line) => {
                let pan = self.pan;
                let state = &mut self.files[line.file];
                let file = &state.file;
                let hunk = &file.hunks[line.hunk];
                let code = state.code[line.hunk].get_or_insert_with(|| HunkCode::new(file, hunk));
                let code = code.lines(line.line + 1).get(line.line);
                let Some(diff_line) = hunk.lines.get(line.line) else {
                    return (Vec::new(), styles.gutter);
                };
                let marker = if current && focused { "► " } else { "  " };
                // Py bug B22: a removed line shows its old-file number, every other line its new-file number.
                let number = match diff_line.kind {
                    LineKind::Removed => diff_line.old,
                    LineKind::Context | LineKind::Added | LineKind::NoNewline => diff_line.new,
                }
                .map_or_else(|| " ".repeat(4), |n| format!("{n:>4}"));
                let background = line_background(diff_line.kind, current, theme);
                // A context line keeps the highlighter's background (monokai's), as Python's hunk panes do.
                let code_style = styles.code.bg(background
                    .or_else(|| code.and_then(|code| code.style.bg).map(|bg| reduce(bg, theme)))
                    .unwrap_or_else(|| theme.paint(styles.base)));
                let code_spans = code.map_or_else(Vec::new, |code| {
                    code.spans
                        .iter()
                        .map(|span| {
                            let mut style = code_style.patch(span.style);
                            style.fg = style.fg.map(|fg| reduce(fg, theme));
                            // The line's own background wins over any the highlighter gives.
                            style.bg = background.or_else(|| style.bg.map(|bg| reduce(bg, theme)));
                            Span::styled(span.content.clone(), style)
                        })
                        .collect()
                });
                let (mut code_spans, left) =
                    display::cut_spans(display::skip_spans(code_spans, pan), width.saturating_sub(GUTTER_WIDTH));
                if left > 0 {
                    code_spans.push(Span::styled(" ".repeat(left), code_style));
                }
                let mut spans = vec![Span::styled(format!("{marker}{number}  │ "), styles.gutter)];
                spans.extend(code_spans);
                (spans, code_style)
            }
        }
    }
}

/// A file's header: `▼` or `▶`, then `📄 <path>` and its counts, or `🗑️  <path> (deleted)`.
fn header_spans(file: &ChangedFile, folded: bool, style: Style, styles: &Styles) -> Vec<Span<'static>> {
    let glyph = if folded { "▶" } else { "▼" };
    let path = display::sanitize(&file.path);
    if file.deleted {
        return vec![Span::styled(format!("{glyph} 🗑️  {path} (deleted)"), style)];
    }
    let mut spans = vec![Span::styled(format!("{glyph} 📄 {path}"), style)];
    spans.extend(count_spans(file.counts(), style, styles));
    spans
}

/// ` (+A -D)` in `style`, the counts green and red; a zero count left out, nothing when both are.
fn count_spans(counts: Counts, style: Style, styles: &Styles) -> Vec<Span<'static>> {
    let Counts { additions, deletions } = counts;
    if counts.is_empty() {
        return Vec::new();
    }
    let mut spans = vec![Span::styled(" (", style)];
    if additions > 0 {
        spans.push(Span::styled(format!("+{additions}"), style.fg(styles.stat_added)));
    }
    if additions > 0 && deletions > 0 {
        spans.push(Span::styled(" ", style));
    }
    if deletions > 0 {
        spans.push(Span::styled(format!("-{deletions}"), style.fg(styles.stat_removed)));
    }
    spans.push(Span::styled(")", style));
    spans
}

/// The position row: the path on the left, cut to leave room for the detail on the right, a cell of padding each
/// side.
fn position_spans(position: &CursorPosition, width: usize, style: Style) -> Vec<Span<'static>> {
    let right = if position.detail.is_empty() {
        String::new()
    } else {
        format!("{} ", position.detail)
    };
    let right_width = display::width(&right);
    let room = width.saturating_sub(right_width + 1);
    let path = display::truncate(&position.path, room);
    let gap = room.saturating_sub(display::width(&path));
    vec![Span::styled(format!(" {path}{}{right}", " ".repeat(gap)), style)]
}

/// Draws `spans` across `rect`, a row, over `fill`.
fn put(buf: &mut Buffer, rect: Rect, spans: Vec<Span<'static>>, fill: Style) {
    buf.set_style(rect, fill);
    buf.set_line(rect.x, rect.y, &Line::from(spans), rect.width);
}

/// A line's own background: added and removed lines are tinted green and red, brighter on the cursor's line; the
/// cursor's context line is yellow, and other context lines have none.
fn line_background(kind: LineKind, current: bool, theme: &Theme) -> Option<Color> {
    let diff = &theme.roles().diff;
    let color = match (kind, current) {
        (LineKind::Removed, false) => diff.removed_bg,
        (LineKind::Added, false) => diff.added_bg,
        (LineKind::Removed, true) => diff.removed_current_bg,
        (LineKind::Added, true) => diff.added_current_bg,
        (LineKind::Context | LineKind::NoNewline, true) => diff.context_current_bg,
        (LineKind::Context | LineKind::NoNewline, false) => return None,
    };
    Some(theme.color(color, theme.background()))
}

/// A highlighter's exact color reduced to what the terminal shows, as the theme's own colors are.
fn reduce(color: Color, theme: &Theme) -> Color {
    match color {
        Color::Rgb(r, g, b) => theme.paint(Rgb::new(r, g, b).into()),
        other => other,
    }
}

/// The body's styles for one draw, from the theme's roles.
struct Styles {
    /// What the rows are drawn on (`$surface`).
    base: Paint,
    /// A line's gutter and the `file was removed` row's marker.
    gutter: Style,
    /// A line's code, before its background.
    code: Style,
    /// A file header: bold `$text` on `$boost`.
    header: Style,
    /// The header under the focused cursor: the block cursor.
    header_cursor: Style,
    /// A hunk separator (`$text-muted`).
    separator: Style,
    /// `file was removed` (`$error`).
    removed: Style,
    /// The position row (`$text` on `$accent`).
    position: Style,
    stat_added: Color,
    stat_removed: Color,
    scrollbar: ScrollbarColors,
}

impl Styles {
    fn new(theme: &Theme) -> Self {
        let roles = theme.roles();
        let base = roles.surface.pane.over(theme.background());
        let boost = roles.surface.boost.over(base);
        let cursor = &roles.cursor.focused;
        let track = roles.scrollbar.track.over(base);
        let plain = Style::new()
            .fg(theme.color(roles.text.plain, base))
            .bg(theme.paint(base));
        Self {
            base,
            gutter: plain,
            code: plain,
            header: Style::new()
                .fg(theme.color(roles.text.normal, boost))
                .bg(theme.paint(boost))
                .add_modifier(Modifier::BOLD),
            header_cursor: cursor.style.apply(theme.fg_on(cursor.fg, cursor.bg, base)),
            separator: Style::new()
                .fg(theme.color(roles.text.muted, base))
                .bg(theme.paint(base)),
            removed: Style::new().fg(theme.color(roles.status.error, base)),
            position: theme.fg_on(roles.diff.indicator.fg, roles.diff.indicator.bg, base),
            stat_added: theme.color(roles.diff.stat_added, base),
            stat_removed: theme.color(roles.diff.stat_removed, base),
            scrollbar: ScrollbarColors {
                bar: theme.color(roles.scrollbar.thumb, track),
                back: theme.paint(track),
                fg: theme.color(roles.text.plain, base),
                bg: theme.paint(base),
            },
        }
    }
}

#[cfg(test)]
mod tests;
