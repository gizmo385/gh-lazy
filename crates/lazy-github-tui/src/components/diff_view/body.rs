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
//! The pull request's review threads (#390) take rows too: a file's comments on the whole file after its header, a
//! line's threads under it (collapsed to a summary row, or expanded to their comments, see `threads.rs`), and the
//! outdated ones under a separator after the file's last hunk. The cursor visits their rows like lines. The pending
//! comments added with Add Comment (#391) show under their lines too, after the line's threads, always expanded (see
//! `pending.rs`), and `x` removes the one under the cursor.
//!
//! Files fold one at a time (`enter` on a header, a click on its title) or all at once (`z`): everything folded is the
//! file list (decision `diff-file-navigation`). While the cursor's file header is scrolled above the rows shown, a
//! copy of it sticks over the top row ([`DiffBody::sticky_file`]).
//!
//! The Diff tab (`DiffView`) owns it and scrolls it together with the review form below it:
//! [`DiffBody::fit`], [`DiffBody::set_scroll`] and [`DiffBody::draw`] are for that owner, and [`DiffBody::render`]
//! draws the body on its own, with its own scrollbar.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use lazy_github_core::diff::{ChangedFile, Counts, Diff, LineKind};
use lazy_github_core::review_anchor::{Anchor, DiffThread, original_line};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget as _};

use super::highlight::HunkCode;
use super::pending::PendingLayout;
use super::review::{DiffLineRef, PendingComment};
use super::threads::{Place, ThreadStyles, ThreadView};
use crate::components::link_map::ScreenLink;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::display;
use crate::keymap::BindingId;
use crate::theme::{Paint, Rgb, Theme};

/// The body's chain link, in footer order: `j Line down  k Line up  c Add Comment  J Next Hunk  K Previous Hunk  ]
/// Next File  [ Previous File  enter Fold  z Fold Files  n Next Comment  N Previous Comment  x Remove Comment  / Filter
/// Files`, then the hidden keys: `g` and `G`, and the
/// scroll keys, of which `down` and `up` move the cursor as `j` and `k` do. Add Comment and Filter Files are the Diff
/// tab's to act on, so [`DiffBody::handle`] passes on them.
pub const DIFF_BODY_BINDINGS: &[BindingId] = &[
    BindingId::DiffCursorDown,
    BindingId::DiffCursorUp,
    BindingId::DiffAddComment,
    BindingId::DiffNextHunk,
    BindingId::DiffPreviousHunk,
    BindingId::DiffNextFile,
    BindingId::DiffPreviousFile,
    BindingId::DiffToggle,
    BindingId::DiffFoldAll,
    BindingId::DiffNextComment,
    BindingId::DiffPreviousComment,
    BindingId::DiffRemoveComment,
    BindingId::DiffFilterFiles,
    BindingId::DiffFirstLine,
    BindingId::DiffLastLine,
    BindingId::TextualScrollDown,
    BindingId::TextualScrollUp,
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
/// The sticky file header takes one of the body's rows only when more than this many show: on a short body, the row
/// is worth more than the reminder.
const STICKY_MIN_ROWS: usize = 8;

/// Where the cursor is, by what it's on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// A file's header row, by the file's index in the diff.
    File(usize),
    /// A diff line.
    Line(DiffLineRef),
    /// An unfolded deleted file's `file was removed` row.
    Removed(usize),
    /// Row `row` of a review thread (its summary is row 0): the file, and the thread's index in the body's threads.
    Thread { file: usize, thread: usize, row: usize },
    /// Row `row` of a pending comment (its title is row 0): the file, and the comment's index among the pending ones.
    Pending { file: usize, index: usize, row: usize },
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
            Cursor::Thread { file, thread, row } => Row::Thread { file, thread, row },
            Cursor::Pending { file, index, row } => Row::Pending { file, index, row },
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
    /// Row `row` of a review thread: the file, the thread's index in the body's threads, and the row in the thread.
    Thread { file: usize, thread: usize, row: usize },
    /// The separator before a file's outdated threads.
    OutdatedSeparator(usize),
    /// Row `row` of a pending comment: the file, the comment's index among the pending ones, and the row in it.
    Pending { file: usize, index: usize, row: usize },
}

impl Row {
    /// The file the row belongs to.
    pub fn file(self) -> usize {
        match self {
            Row::FileHeader(f) | Row::HunkSeparator(f, _) | Row::Removed(f) | Row::OutdatedSeparator(f) => f,
            Row::Thread { file, .. } | Row::Pending { file, .. } => file,
            Row::Line(line) => line.file,
        }
    }

    /// The cursor on this row, when the row can hold it: every row but a separator.
    pub fn cursor(self) -> Option<Cursor> {
        match self {
            Row::FileHeader(f) => Some(Cursor::File(f)),
            Row::HunkSeparator(..) | Row::OutdatedSeparator(_) => None,
            Row::Thread { file, thread, row } => Some(Cursor::Thread { file, thread, row }),
            Row::Pending { file, index, row } => Some(Cursor::Pending { file, index, row }),
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
///
/// Its rows are numbered from its header (row 0) as if it were unfolded. Without threads ("base" rows) a hunk's lines
/// follow its separator one row each; the review threads of #390 and the pending comments of #391 come as
/// [`Insert`]s after a base row (the header, a line, the last row), each taking its items' rows, so a lookup steps
/// over the inserts above it.
#[derive(Debug, Clone)]
struct FileState {
    file: ChangedFile,
    folded: bool,
    /// Whether the filter keeps it.
    shown: bool,
    /// Each hunk's separator row, in base rows.
    hunk_starts: Vec<usize>,
    /// The base rows the file takes unfolded, its header included.
    base_rows: usize,
    /// The rows the file takes unfolded, its threads included.
    unfolded: usize,
    /// The threads' rows, in row order.
    inserts: Vec<Insert>,
    /// How many threads the file shows.
    threads: usize,
    /// How many pending comments the file shows.
    pending: usize,
    /// Each hunk's code, made the first time a row of it is drawn: highlighting is slow, and most hunks of a large
    /// diff are never on screen.
    code: Vec<Option<HunkCode>>,
    /// The widest line of the file's hunks, in cells, once panning has needed it.
    widest: Option<usize>,
}

/// Rows of threads after one base row of a file.
#[derive(Debug, Clone)]
struct Insert {
    /// The base row they follow.
    after: usize,
    kind: InsertKind,
    /// What they show, each with its rows.
    items: Vec<(Item, usize)>,
    /// The first of their rows, from the file's header, unfolded.
    start: usize,
    /// How many rows they take.
    rows: usize,
}

/// What an [`Insert`] follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertKind {
    /// The header: comments on the whole file.
    File,
    /// A line, or a deleted file's `file was removed` row: the threads on it, then its pending comments.
    Line,
    /// The file's last row: its outdated threads, under a separator.
    Outdated,
}

/// One part of an [`Insert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// The outdated group's separator row.
    Separator,
    /// A thread, by its index in the body's threads.
    Thread(usize),
    /// A pending comment, by its index among the pending ones.
    Pending(usize),
}

impl Item {
    /// The row of this item at `row` in it, in file `file`.
    fn row(self, file: usize, row: usize) -> Row {
        match self {
            Item::Separator => Row::OutdatedSeparator(file),
            Item::Thread(thread) => Row::Thread { file, thread, row },
            Item::Pending(index) => Row::Pending { file, index, row },
        }
    }

    /// Whether it's a comment `n` and `N` stop on: a thread or a pending comment.
    fn is_comment(self) -> bool {
        matches!(self, Item::Thread(_) | Item::Pending(_))
    }
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
        let base_rows = if file.deleted { 2 } else { next };
        Self {
            folded: file.deleted,
            shown: true,
            base_rows,
            unfolded: base_rows,
            inserts: Vec::new(),
            threads: 0,
            pending: 0,
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

    /// The base row of a line.
    fn line_base(&self, hunk: usize, line: usize) -> Option<usize> {
        let lines = self.file.hunks.get(hunk)?.lines.len();
        (line < lines).then(|| self.hunk_starts[hunk] + 1 + line)
    }

    /// The row `local` rows below the header of this file, file `f` of the diff. `local` is below [`Self::rows`].
    fn local_row(&self, f: usize, local: usize) -> Row {
        if local == 0 || self.folded {
            return Row::FileHeader(f);
        }
        // The last insert starting at or above `local`.
        let i = self.inserts.partition_point(|insert| insert.start <= local);
        let base = match i.checked_sub(1).map(|i| &self.inserts[i]) {
            Some(insert) if local < insert.start + insert.rows => return insert.row_at(f, local - insert.start),
            Some(insert) => local - (insert.start - insert.after - 1) - insert.rows,
            None => local,
        };
        self.base_row(f, base)
    }

    /// The row at base row `base` (not 0) of this file, file `f`.
    fn base_row(&self, f: usize, base: usize) -> Row {
        if self.file.deleted {
            return Row::Removed(f);
        }
        // The last hunk whose separator is at or above `base`; the first separator is row 1.
        let h = self
            .hunk_starts
            .partition_point(|&start| start <= base)
            .saturating_sub(1);
        match base - self.hunk_starts.get(h).copied().unwrap_or(1) {
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
            _ => self.unfolded_local_of(row),
        }
    }

    /// How many rows below the header `row` is with the file unfolded, when the file has it.
    fn unfolded_local_of(&self, row: Row) -> Option<usize> {
        let base = match row {
            Row::FileHeader(_) => return Some(0),
            Row::Thread { thread, row, .. } => return self.item_local(Item::Thread(thread), row),
            Row::Pending { index, row, .. } => return self.item_local(Item::Pending(index), row),
            Row::OutdatedSeparator(_) => {
                return self
                    .inserts
                    .iter()
                    .find(|insert| insert.kind == InsertKind::Outdated)
                    .map(|insert| insert.start);
            }
            Row::Removed(_) => self.file.deleted.then_some(1)?,
            _ if self.file.deleted => return None,
            Row::HunkSeparator(_, h) => self.hunk_starts.get(h).copied()?,
            Row::Line(line) => self.line_base(line.hunk, line.line)?,
        };
        Some(base + self.rows_inserted_before(base))
    }

    /// How many rows below the header row `row` of `item` is, unfolded, when the file has it.
    fn item_local(&self, item: Item, row: usize) -> Option<usize> {
        self.inserts.iter().find_map(|insert| {
            let mut at = insert.start;
            for &(other, rows) in &insert.items {
                if other == item {
                    return (row < rows).then_some(at + row);
                }
                at += rows;
            }
            None
        })
    }

    /// The thread rows above base row `base`.
    fn rows_inserted_before(&self, base: usize) -> usize {
        let i = self.inserts.partition_point(|insert| insert.after < base);
        i.checked_sub(1).map_or(0, |i| {
            let insert = &self.inserts[i];
            insert.start + insert.rows - insert.after - 1
        })
    }

    /// What's under base row `base`: a line's (or a deleted file's `file was removed` row's) threads, then its
    /// pending comments.
    fn items_on(&self, base: usize) -> impl Iterator<Item = Item> + '_ {
        let i = self.inserts.partition_point(|insert| insert.after < base);
        self.inserts[i..]
            .iter()
            .take_while(move |insert| insert.after == base)
            .filter(|insert| insert.kind == InsertKind::Line)
            .flat_map(|insert| insert.items.iter().map(|&(item, _)| item))
    }

    /// The threads on base row `base`: a line's, or a deleted file's `file was removed` row's.
    fn threads_on(&self, base: usize) -> impl Iterator<Item = usize> + '_ {
        self.items_on(base).filter_map(|item| match item {
            Item::Thread(t) => Some(t),
            Item::Separator | Item::Pending(_) => None,
        })
    }

    /// The file's comments (threads and pending comments) in row order.
    fn comment_order(&self) -> impl Iterator<Item = Item> + '_ {
        self.inserts
            .iter()
            .flat_map(|insert| insert.items.iter().map(|&(item, _)| item))
            .filter(|item| item.is_comment())
    }

    /// Sets the file's inserts, unsized, from its threads (`threads`, each with where it goes, in creation order) and
    /// its pending comments (`pending`, each with its line, in the order they were added).
    fn place(&mut self, threads: &[(usize, Place)], pending: &[(usize, DiffLineRef)]) {
        let mut file_level = Vec::new();
        let mut lines: BTreeMap<usize, Vec<Item>> = BTreeMap::new();
        let mut outdated = Vec::new();
        for &(t, place) in threads {
            match place {
                Place::File => file_level.push(t),
                Place::Line { hunk, line } => match self.line_base(hunk, line) {
                    Some(base) => lines.entry(base).or_default().push(Item::Thread(t)),
                    None => outdated.push(t),
                },
                Place::Removed => lines.entry(1).or_default().push(Item::Thread(t)),
                Place::Outdated => outdated.push(t),
            }
        }
        // After the line's threads, in the order they were added. A comment is made on a line of this diff, so its
        // line is there.
        for &(p, line) in pending {
            if let Some(base) = self.line_base(line.hunk, line.line) {
                lines.entry(base).or_default().push(Item::Pending(p));
            }
        }
        let mut inserts = Vec::new();
        let insert = |after, kind, items: Vec<Item>| Insert {
            after,
            kind,
            items: items.into_iter().map(|item| (item, 1)).collect(),
            start: 0,
            rows: 0,
        };
        if !file_level.is_empty() {
            inserts.push(insert(
                0,
                InsertKind::File,
                file_level.into_iter().map(Item::Thread).collect(),
            ));
        }
        for (base, items) in lines {
            inserts.push(insert(base, InsertKind::Line, items));
        }
        if !outdated.is_empty() {
            let items = std::iter::once(Item::Separator)
                .chain(outdated.into_iter().map(Item::Thread))
                .collect();
            inserts.push(insert(self.base_rows - 1, InsertKind::Outdated, items));
        }
        self.threads = threads.len();
        self.pending = pending.len();
        self.inserts = inserts;
    }

    /// Sizes the inserts from their items' rows, which `rows_of` gives for threads and pending comments.
    fn size_items(&mut self, mut rows_of: impl FnMut(Item) -> usize) {
        let mut inserted = 0;
        for insert in &mut self.inserts {
            for (item, rows) in &mut insert.items {
                *rows = match *item {
                    Item::Separator => 1,
                    other => rows_of(other),
                };
            }
            insert.rows = insert.items.iter().map(|&(_, rows)| rows).sum();
            insert.start = insert.after + 1 + inserted;
            inserted += insert.rows;
        }
        self.unfolded = self.base_rows + inserted;
    }

    /// The outdated group's size: how many threads it lists.
    fn outdated_count(&self) -> usize {
        self.inserts
            .iter()
            .filter(|insert| insert.kind == InsertKind::Outdated)
            .map(|insert| insert.items.len() - 1)
            .sum()
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

impl Insert {
    /// The row `offset` rows into the insert, in file `f`.
    fn row_at(&self, f: usize, offset: usize) -> Row {
        let mut at = 0;
        for &(item, rows) in &self.items {
            if offset < at + rows {
                return item.row(f, offset - at);
            }
            at += rows;
        }
        // `offset` is below the insert's rows, which its items add up to.
        Row::FileHeader(f)
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
    /// The review threads of the shown diff's files, in creation order.
    threads: Vec<ThreadView>,
    /// The threads expanded, by their first comment's id: kept when the threads are set again.
    expanded: HashSet<u64>,
    /// The links the last draw put on screen, in expanded threads.
    links: Vec<ScreenLink>,
    /// The comments added with Add Comment, in the order they were added.
    pending: Vec<PendingComment>,
    /// Each pending comment's rows, as `pending` has them.
    pending_layouts: Vec<PendingLayout>,
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
            threads: Vec::new(),
            expanded: HashSet::new(),
            links: Vec::new(),
            pending: Vec::new(),
            pending_layouts: Vec::new(),
        };
        body.settle();
        body
    }

    /// Shows `threads` under their lines, replacing any shown: a thread on a file the diff doesn't have is left out,
    /// one whose position matches no line of its file shows as outdated. Expanded threads stay expanded, and the
    /// cursor stays where it was (on a thread, the same thread when it's still there, else its file's header).
    /// Bodies' `#N` references point into `repo_full_name`.
    pub fn set_threads(&mut self, threads: Vec<DiffThread>, repo_full_name: &str) {
        let kept = match self.cursor {
            Some(Cursor::Thread { file, thread, row }) => {
                self.threads.get(thread).map(|view| (file, view.root_id(), row))
            }
            _ => None,
        };
        self.threads = threads
            .into_iter()
            .filter_map(|thread| {
                let f = self.files.iter().position(|state| state.file.path == thread.path)?;
                let file = &self.files[f].file;
                let original = || original_line(&thread.root().diff_hunk);
                let (place, original) = match thread.anchor {
                    Anchor::File => (Place::File, None),
                    Anchor::Position(position) => match file.line_at(position) {
                        Some(_) if file.deleted => (Place::Removed, None),
                        Some((hunk, line)) => (Place::Line { hunk, line }, None),
                        None => (Place::Outdated, original()),
                    },
                    Anchor::Outdated { ref original } => (Place::Outdated, original.clone()),
                };
                let mut view = ThreadView::new(thread, f, place, original, repo_full_name);
                view.expanded = self.expanded.contains(&view.root_id());
                Some(view)
            })
            .collect();
        let width = self.thread_width();
        self.place_all();
        if let Some((file, root, row)) = kept {
            self.cursor = Some(match self.threads.iter().position(|view| view.root_id() == root) {
                Some(thread) => {
                    let view = &mut self.threads[thread];
                    let row = row.min(view.rows(width).saturating_sub(1));
                    Cursor::Thread {
                        file: view.file,
                        thread,
                        row,
                    }
                }
                None => Cursor::File(file),
            });
        }
        self.reindex();
    }

    /// Sets every file's inserts from the threads and the pending comments, and sizes them.
    fn place_all(&mut self) {
        for f in 0..self.files.len() {
            let threads: Vec<(usize, Place)> = (self.threads.iter().enumerate())
                .filter(|(_, view)| view.file == f)
                .map(|(t, view)| (t, view.place))
                .collect();
            let pending: Vec<(usize, DiffLineRef)> = (self.pending.iter().enumerate())
                .filter(|(_, comment)| comment.line.file == f)
                .map(|(p, comment)| (p, comment.line))
                .collect();
            self.files[f].place(&threads, &pending);
            self.size_file(f);
        }
    }

    /// Sizes file `f`'s inserts at the code column's width.
    fn size_file(&mut self, f: usize) {
        let width = self.thread_width();
        let (threads, pending, layouts) = (&mut self.threads, &self.pending, &mut self.pending_layouts);
        let Some(state) = self.files.get_mut(f) else {
            return;
        };
        state.size_items(|item| match item {
            Item::Thread(t) => threads.get_mut(t).map_or(1, |view| view.rows(width)),
            Item::Pending(p) => match (pending.get(p), layouts.get_mut(p)) {
                (Some(comment), Some(layout)) => layout.rows(comment, width),
                _ => 1,
            },
            Item::Separator => 1,
        });
    }

    /// How many review threads the shown files show.
    pub fn thread_count(&self) -> usize {
        self.files
            .iter()
            .filter(|state| state.shown)
            .map(|state| state.threads)
            .sum()
    }

    /// How many comments `n` and `N` can stop on: the shown files' threads and pending comments.
    pub fn comment_count(&self) -> usize {
        self.files
            .iter()
            .filter(|state| state.shown)
            .map(|state| state.threads + state.pending)
            .sum()
    }

    /// The pending comments, in the order they were added, the files the filter hides included.
    pub fn pending(&self) -> &[PendingComment] {
        &self.pending
    }

    /// Adds `comment` after the others, under its line after the line's threads. The cursor stays where it is.
    pub fn add_pending(&mut self, comment: PendingComment) {
        self.pending.push(comment);
        self.pending_layouts.push(PendingLayout::default());
        self.place_all();
        self.reindex();
    }

    /// Drops pending comment `index`. A cursor on it goes to its line; one on a later comment stays on that comment.
    pub fn remove_pending(&mut self, index: usize) {
        let Some(comment) = self.pending.get(index) else {
            return;
        };
        let line = comment.line;
        self.cursor = match self.cursor {
            Some(Cursor::Pending { index: i, .. }) if i == index => Some(Cursor::Line(line)),
            Some(Cursor::Pending { file, index: i, row }) if i > index => Some(Cursor::Pending {
                file,
                index: i - 1,
                row,
            }),
            other => other,
        };
        self.pending.remove(index);
        self.pending_layouts.remove(index);
        self.place_all();
        self.reindex();
    }

    /// Whether thread `t` is expanded.
    pub fn is_expanded(&self, t: usize) -> bool {
        self.threads.get(t).is_some_and(|view| view.expanded)
    }

    /// The first comment's id of thread `t`.
    pub fn thread_root(&self, t: usize) -> Option<u64> {
        self.threads.get(t).map(ThreadView::root_id)
    }

    /// The links the last draw put on screen, in expanded threads' bodies.
    pub fn links(&self) -> &[ScreenLink] {
        &self.links
    }

    /// What `enter` does at the cursor, for the footer: `Some(true)` expands (a collapsed thread, or a line with a
    /// collapsed thread), `Some(false)` collapses, `None` when the cursor isn't on a thread or a line with threads.
    pub fn toggle_expands(&self) -> Option<bool> {
        match self.cursor? {
            Cursor::Thread { thread, .. } => Some(!self.is_expanded(thread)),
            cursor => {
                let on = self.threads_at(cursor);
                (!on.is_empty()).then(|| on.iter().any(|&t| !self.is_expanded(t)))
            }
        }
    }

    /// The threads under the cursor's line (or a deleted file's `file was removed` row).
    fn threads_at(&self, cursor: Cursor) -> Vec<usize> {
        let Some(state) = self.files.get(cursor.file()) else {
            return Vec::new();
        };
        let base = match cursor {
            Cursor::Line(line) => state.line_base(line.hunk, line.line),
            Cursor::Removed(_) => Some(1),
            Cursor::File(_) | Cursor::Thread { .. } | Cursor::Pending { .. } => None,
        };
        base.map_or_else(Vec::new, |base| state.threads_on(base).collect())
    }

    /// The cells a thread's rows get after the gutter.
    fn thread_width(&self) -> u16 {
        u16::try_from(self.code_width).unwrap_or(u16::MAX)
    }

    /// Expands or collapses `threads` and resizes their files' rows. A cursor inside a thread that collapses goes to
    /// its summary row.
    fn set_expanded(&mut self, threads: &[usize], expanded: bool) {
        let mut files = Vec::new();
        for &t in threads {
            let Some(view) = self.threads.get_mut(t) else {
                continue;
            };
            view.expanded = expanded;
            if expanded {
                self.expanded.insert(view.root_id());
            } else {
                self.expanded.remove(&view.root_id());
            }
            files.push(view.file);
            if let Some(Cursor::Thread { file, thread, .. }) = self.cursor
                && thread == t
                && !expanded
            {
                self.cursor = Some(Cursor::Thread { file, thread, row: 0 });
            }
        }
        files.dedup();
        for f in files {
            self.size_file(f);
        }
        self.reindex();
    }

    /// `enter` off a header: a thread row expands or collapses its thread; a line with threads expands them all when
    /// any is collapsed, else collapses them all.
    fn toggle_threads(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        match cursor {
            Cursor::Thread { thread, .. } => self.set_expanded(&[thread], !self.is_expanded(thread)),
            cursor => {
                let on = self.threads_at(cursor);
                let expand = on.iter().any(|&t| !self.is_expanded(t));
                self.set_expanded(&on, expand);
            }
        }
        self.reveal_threads_below();
    }

    /// After `enter` expanded threads at the cursor: scrolls down as far as needed to show the last of them, as long
    /// as the cursor stays in view.
    fn reveal_threads_below(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let threads = match cursor {
            Cursor::Thread { thread, .. } => vec![thread],
            cursor => self.threads_at(cursor),
        };
        let width = self.thread_width();
        let Some(&last) = threads.last() else {
            return;
        };
        if !self.is_expanded(last) || self.height == 0 {
            return;
        }
        let file = self.threads[last].file;
        let rows = self.threads[last].rows(width);
        let (Some(end), Some(at)) = (
            self.index.row_of(
                &self.files,
                Row::Thread {
                    file,
                    thread: last,
                    row: rows - 1,
                },
            ),
            self.cursor_row(),
        ) else {
            return;
        };
        // The cursor keeps its margin, which a later reveal of the cursor would restore anyway.
        let margin = SCROLL_MARGIN.min(self.height.saturating_sub(1) / 2);
        let wanted = (end + 1).saturating_sub(self.height);
        self.scroll = self.scroll.max(wanted.min(at.saturating_sub(margin)));
    }

    /// The cursor's place for `n` and `N`: its file, and its row in the file as if unfolded.
    fn cursor_place(&self) -> Option<(usize, usize)> {
        let cursor = self.cursor?;
        let f = cursor.file();
        Some((f, self.files.get(f)?.unfolded_local_of(cursor.row())?))
    }

    /// The comments (threads and pending comments) of the shown files in diff order, each as its first row, with its
    /// file and that row's place in its file unfolded.
    fn comment_places(&self) -> impl Iterator<Item = (usize, Row, usize)> + '_ {
        self.files
            .iter()
            .enumerate()
            .filter(|(_, state)| state.shown)
            .flat_map(|(f, state)| {
                state.comment_order().filter_map(move |item| {
                    let first = item.row(f, 0);
                    Some((f, first, state.unfolded_local_of(first)?))
                })
            })
    }

    /// `n`: the next comment's first row in diff order (a thread's summary, a pending comment's title), unfolding its
    /// file; `N`: the previous one, or the cursor's own comment's when the cursor is below its first row. Nothing past
    /// either end.
    fn jump_comment(&mut self, forward: bool) {
        let Some(at) = self.cursor_place() else {
            return;
        };
        let target = if forward {
            self.comment_places().find(|&(f, _, row)| (f, row) > at)
        } else {
            self.comment_places().take_while(|&(f, _, row)| (f, row) < at).last()
        };
        if let Some((file, row, _)) = target {
            self.set_folded(file, false);
            self.cursor = row.cursor();
            self.follow();
        }
    }

    /// Where the cursor is; `None` when no file is shown.
    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    /// The diff line under the cursor, which Add Comment comments on.
    pub fn cursor_line(&self) -> Option<DiffLineRef> {
        match self.cursor? {
            Cursor::Line(line) => Some(line),
            Cursor::File(_) | Cursor::Removed(_) | Cursor::Thread { .. } | Cursor::Pending { .. } => None,
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
        self.set_code_width(usize::from(width).saturating_sub(bar + GUTTER_WIDTH));
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
        self.set_code_width(usize::from(width).saturating_sub(GUTTER_WIDTH));
        self.trailing = trailing;
        self.clamp_pan();
    }

    /// Takes the code column's width: expanded threads, whose rows wrap to it, are sized again when it changed.
    fn set_code_width(&mut self, width: usize) {
        if self.code_width == width {
            return;
        }
        self.code_width = width;
        if !self.pending.is_empty() || self.threads.iter().any(|view| view.expanded) {
            for f in 0..self.files.len() {
                self.size_file(f);
            }
            self.index = RowIndex::build(&self.files);
            self.settle();
        }
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

    /// A click on row `row`, `x` cells from the body's left edge: the cursor moves to it, or from a hunk separator to
    /// the hunk's first line, without scrolling. A click on a file header's title (its glyph, path and counts) also
    /// folds or unfolds the file, as a collapsible's title does, and one on a thread's summary row expands or
    /// collapses the thread. A pending comment's rows only take the cursor. Says whether the row was there.
    pub fn click_row(&mut self, row: usize, x: usize) -> bool {
        let Some(target) = self.cursor_from(row, true) else {
            return false;
        };
        if let Some(cursor) = self.row(target).and_then(Row::cursor) {
            self.cursor = Some(cursor);
        }
        if let Some(Row::FileHeader(f)) = self.row(row)
            && x < self.header_width(f)
        {
            self.set_folded(f, !self.is_folded(f));
        }
        // A thread's summary row toggles the thread, as a file header's title folds the file.
        if let Some(Row::Thread { thread, row: 0, .. }) = self.row(row) {
            self.set_expanded(&[thread], !self.is_expanded(thread));
        }
        true
    }

    /// The cursor goes to file `f`'s header, when the file is shown, scrolled into view.
    pub fn cursor_to_file(&mut self, f: usize) {
        if self.is_shown(f) {
            self.cursor = Some(Cursor::File(f));
            self.follow();
        }
    }

    /// Whether every shown file is folded to its header, so `z` unfolds them: the file list.
    pub fn all_folded(&self) -> bool {
        self.files.iter().filter(|state| state.shown).all(|state| state.folded)
    }

    /// The file whose header sticks to the body's top row while the body's `rows` show: the cursor's file, once its
    /// header has scrolled above them, when more than [`STICKY_MIN_ROWS`] show. None while the cursor itself is out
    /// of them (focus on the review form below, say): the header would name a file the rows shown aren't in.
    pub fn sticky_file(&self, rows: Range<usize>) -> Option<usize> {
        if rows.len() <= STICKY_MIN_ROWS || !rows.contains(&self.cursor_row()?) {
            return None;
        }
        let f = self.cursor?.file();
        let header = self.index.row_of(&self.files, Row::FileHeader(f))?;
        (header < rows.start).then_some(f)
    }

    /// The cells file `f`'s header title takes: its glyph, path and counts.
    fn header_width(&self, f: usize) -> usize {
        self.files.get(f).map_or(0, |state| {
            header_spans(state, Style::new(), Color::Reset, Color::Reset)
                .iter()
                .map(|span| display::width(&span.content))
                .sum()
        })
    }

    /// The first shown file's header takes the cursor, scrolled into view.
    pub fn cursor_to_first_file(&mut self) {
        if let Some(row) = self.cursor_from(0, true) {
            self.move_to(row);
        }
    }

    /// A binding of [`DIFF_BODY_BINDINGS`]. Says whether it's the body's to handle: every one but Add Comment and
    /// Filter Files, whether or not the cursor could move, and Remove Comment only on a pending comment's row. `enter`
    /// off a header does nothing, but is still the body's.
    pub fn handle(&mut self, id: BindingId) -> bool {
        match id {
            BindingId::DiffCursorDown | BindingId::TextualScrollDown => self.step(true),
            BindingId::DiffCursorUp | BindingId::TextualScrollUp => self.step(false),
            BindingId::DiffNextHunk => self.next_hunk(),
            BindingId::DiffPreviousHunk => self.previous_hunk(),
            BindingId::DiffNextFile => self.next_file(),
            BindingId::DiffPreviousFile => self.previous_file(),
            BindingId::DiffToggle => match self.cursor {
                Some(Cursor::File(f)) => self.set_folded(f, !self.is_folded(f)),
                Some(_) => self.toggle_threads(),
                None => {}
            },
            BindingId::DiffFoldAll => self.fold_all(),
            BindingId::DiffNextComment => self.jump_comment(true),
            BindingId::DiffPreviousComment => self.jump_comment(false),
            BindingId::DiffRemoveComment => match self.cursor {
                Some(Cursor::Pending { index, .. }) => self.remove_pending(index),
                _ => return false,
            },
            BindingId::TextualScrollPageDown => self.page(true),
            BindingId::TextualScrollPageUp => self.page(false),
            BindingId::DiffFirstLine | BindingId::TextualScrollHome => {
                if let Some(row) = self.cursor_from(0, true) {
                    self.move_to(row);
                }
            }
            BindingId::DiffLastLine | BindingId::TextualScrollEnd => {
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
            Cursor::Thread { thread, .. } => self.comment_position(cursor.file(), Item::Thread(thread)),
            Cursor::Pending { index, .. } => self.comment_position(cursor.file(), Item::Pending(index)),
        };
        Some(CursorPosition {
            path: display::sanitize(&file.path).into_owned(),
            detail,
        })
    }

    /// `Comment n/m`: `item`'s place among file `f`'s comments, threads and pending comments together.
    fn comment_position(&self, f: usize, item: Item) -> String {
        let state = &self.files[f];
        let at = state
            .comment_order()
            .position(|other| other == item)
            .map_or(0, |i| i + 1);
        format!("Comment {at}/{}", state.threads + state.pending)
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
        let after = match self.place_for_hunks(cursor) {
            Cursor::Line(line) => (line.file, line.hunk + 1),
            Cursor::File(f) => (f, 0),
            Cursor::Removed(f) | Cursor::Thread { file: f, .. } | Cursor::Pending { file: f, .. } => (f + 1, 0),
        };
        let target = self.hunks().find(|&hunk| hunk >= after);
        if let Some(target) = target {
            self.jump_to(target);
        }
    }

    /// Where `J` goes from as a row that isn't a comment's: a thread or a pending comment under a line as the line, a
    /// thread on the file as the header; an outdated thread, or one under `file was removed`, is past the file's hunks.
    fn place_for_hunks(&self, cursor: Cursor) -> Cursor {
        if let Cursor::Pending { index, .. } = cursor {
            return self
                .pending
                .get(index)
                .map_or(cursor, |comment| Cursor::Line(comment.line));
        }
        let Cursor::Thread { file, thread, .. } = cursor else {
            return cursor;
        };
        match self.threads.get(thread).map(|view| view.place) {
            Some(Place::Line { hunk, line }) => Cursor::Line(DiffLineRef { file, hunk, line }),
            Some(Place::File) => Cursor::File(file),
            Some(Place::Removed | Place::Outdated) | None => cursor,
        }
    }

    /// `K`: the first line of the cursor's hunk when the cursor is below it, else the previous hunk's first line;
    /// from a header, the last hunk before its file.
    fn previous_hunk(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let place = match cursor {
            Cursor::Thread { thread, .. } => self.threads.get(thread).map(|view| view.place),
            _ => None,
        };
        let target = match (cursor, place) {
            // A thread or a pending comment under a line is below the line's hunk's first line.
            (Cursor::Thread { file, .. }, Some(Place::Line { hunk, .. })) => Some((file, hunk)),
            (Cursor::Pending { file, index, .. }, _) => match self.pending.get(index) {
                Some(comment) => Some((file, comment.line.hunk)),
                None => self.hunks().take_while(|&(g, _)| g < file).last(),
            },
            // An outdated thread is after the file's hunks.
            (Cursor::Thread { file, .. }, Some(Place::Outdated)) => self.hunks().take_while(|&(g, _)| g <= file).last(),
            (Cursor::Line(line), _) if line.line > 0 => Some((line.file, line.hunk)),
            (Cursor::Line(line), _) => self.hunks().take_while(|&hunk| hunk < (line.file, line.hunk)).last(),
            (Cursor::File(f) | Cursor::Removed(f) | Cursor::Thread { file: f, .. }, _) => {
                self.hunks().take_while(|&(g, _)| g < f).last()
            }
        };
        if let Some(target) = target {
            self.jump_to(target);
        }
    }

    /// `]`: the next shown file's header, folded or not.
    fn next_file(&mut self) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let next = (cursor.file() + 1..self.files.len()).find(|&f| self.is_shown(f));
        if let Some(f) = next {
            self.cursor_to_file(f);
        }
    }

    /// `[`: the cursor's file's header when the cursor is below it, else the previous shown file's header.
    fn previous_file(&mut self) {
        let target = match self.cursor {
            None => None,
            Some(Cursor::File(f)) => (0..f).rev().find(|&g| self.is_shown(g)),
            Some(cursor) => Some(cursor.file()),
        };
        if let Some(f) = target {
            self.cursor_to_file(f);
        }
    }

    /// `z`: folds every shown file while any is unfolded, else unfolds them all; the cursor goes to its file's header.
    fn fold_all(&mut self) {
        let fold = !self.all_folded();
        for state in self.files.iter_mut().filter(|state| state.shown) {
            state.folded = fold;
        }
        if let Some(cursor) = self.cursor {
            self.cursor = Some(Cursor::File(cursor.file()));
        }
        self.reindex();
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

    /// The rows from the scroll position, as many as fit in `area`, and the links of the thread rows among them.
    fn draw_rows(&mut self, buf: &mut Buffer, area: Rect, focused: bool, theme: &Theme, styles: &Styles) {
        self.links.clear();
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
            if let Row::Thread { thread, row, .. } = row {
                self.record_thread_links(thread, row, rect, theme);
            }
        }
        // Over the top row: the header of the file the cursor is in, scrolled out of view above. It never holds the
        // cursor, so it's drawn as a header the cursor isn't on. The row under it is cleared first, as a header shorter
        // than that row would leave its tail showing.
        if let Some(f) = self.sticky_file(self.scroll..self.scroll + visible) {
            let rect = Rect::new(area.x, area.y, area.width, 1);
            self.links.retain(|link| link.y != rect.y);
            Clear.render(rect, buf);
            let spans = header_spans(&self.files[f], styles.header, styles.stat_added, styles.stat_removed);
            put(buf, rect, spans, styles.header);
        }
    }

    /// Records the links of thread `thread`'s row `row`, drawn in `rect`.
    fn record_thread_links(&mut self, thread: usize, row: usize, rect: Rect, theme: &Theme) {
        let width = self.thread_width();
        let Some(view) = self.threads.get_mut(thread) else {
            return;
        };
        let Some(index) = row.checked_sub(1 + usize::from(view.original.is_some())) else {
            return;
        };
        if !view.expanded {
            return;
        }
        let left = rect.x.saturating_add(u16::try_from(GUTTER_WIDTH).unwrap_or(u16::MAX));
        let right = rect.right();
        for span in view
            .comment_rows(width, theme)
            .links
            .iter()
            .filter(|span| span.row == index)
        {
            let start = left.saturating_add(span.columns.start);
            let end = left.saturating_add(span.columns.end).min(right);
            if start < end {
                self.links.push(ScreenLink {
                    x: start,
                    y: rect.y,
                    width: end - start,
                    target: span.target.clone(),
                });
            }
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
                (
                    header_spans(state, style, styles.stat_added, styles.stat_removed),
                    style,
                )
            }
            Row::HunkSeparator(f, h) => (self.separator_spans(f, h, styles), styles.separator),
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
                let flag = self.line_flag(line, theme, styles);
                let mut spans = vec![
                    Span::styled(format!("{marker}{number}"), styles.gutter),
                    flag,
                    Span::styled(" │ ", styles.gutter),
                ];
                spans.extend(code_spans);
                (spans, code_style)
            }
            Row::OutdatedSeparator(f) => (
                vec![Span::styled(
                    format!("  Outdated comments ({})", self.files[f].outdated_count()),
                    styles.separator,
                )],
                styles.separator,
            ),
            Row::Thread { thread, row, .. } => {
                self.thread_row_spans(thread, row, current, focused, width, theme, styles)
            }
            Row::Pending { index, row, .. } => {
                self.pending_row_spans(index, row, current && focused, width, theme, styles)
            }
        }
    }

    /// Hunk `h` of file `f`'s separator row: its label, with the counts colored.
    fn separator_spans(&self, f: usize, h: usize, styles: &Styles) -> Vec<Span<'static>> {
        let label = self.files[f].file.hunks[h].label();
        let mut spans = vec![Span::styled("  ", styles.separator)];
        spans.push(Span::styled(
            display::sanitize(&label.text).into_owned(),
            styles.separator,
        ));
        spans.extend(count_spans(
            label.counts,
            styles.separator,
            styles.stat_added,
            styles.stat_removed,
        ));
        spans.push(Span::styled(
            display::sanitize(&label.suffix).into_owned(),
            styles.separator,
        ));
        spans
    }

    /// A line's flag cell: `●` when review threads or pending comments are under it, in `$warning` when a pending
    /// comment is (it's what will be sent), else in `$accent`.
    fn line_flag(&self, line: DiffLineRef, theme: &Theme, styles: &Styles) -> Span<'static> {
        let state = &self.files[line.file];
        let (mut threads, mut pending) = (false, false);
        if let Some(base) = state.line_base(line.hunk, line.line) {
            for item in state.items_on(base) {
                threads |= matches!(item, Item::Thread(_));
                pending |= matches!(item, Item::Pending(_));
            }
        }
        let thread_styles = ThreadStyles::new(theme);
        if pending {
            Span::styled("●", thread_styles.flag_outdated.bg(theme.paint(styles.base)))
        } else if threads {
            Span::styled("●", thread_styles.flag)
        } else {
            Span::styled(" ", styles.gutter)
        }
    }

    /// Row `row` of pending comment `index`, as [`Self::row_spans`] gives it; `on_cursor` puts the `►` in its gutter.
    fn pending_row_spans(
        &mut self,
        index: usize,
        row: usize,
        on_cursor: bool,
        width: usize,
        theme: &Theme,
        styles: &Styles,
    ) -> (Vec<Span<'static>>, Style) {
        let fill = ThreadStyles::new(theme).fill;
        let layout_width = self.thread_width();
        let (Some(comment), Some(layout)) = (self.pending.get(index), self.pending_layouts.get_mut(index)) else {
            return (Vec::new(), styles.gutter);
        };
        let line = layout
            .lines(comment, layout_width, theme)
            .lines
            .get(row)
            .cloned()
            .unwrap_or_default();
        let marker = if on_cursor { "► " } else { "  " };
        let mut spans = vec![Span::styled(
            format!("{marker}{}", " ".repeat(GUTTER_WIDTH - 2)),
            styles.gutter,
        )];
        spans.extend(display::cut_spans(line.spans, width.saturating_sub(GUTTER_WIDTH)).0);
        (spans, fill)
    }

    /// Row `row` of thread `thread`, as [`Self::row_spans`] gives it.
    #[allow(clippy::too_many_arguments)] // The row's own arguments, then the draw's.
    fn thread_row_spans(
        &mut self,
        thread: usize,
        row: usize,
        current: bool,
        focused: bool,
        width: usize,
        theme: &Theme,
        styles: &Styles,
    ) -> (Vec<Span<'static>>, Style) {
        let thread_styles = ThreadStyles::new(theme);
        let code_width = width.saturating_sub(GUTTER_WIDTH);
        let layout_width = self.thread_width();
        let marker = if current && focused { "► " } else { "  " };
        let Some(view) = self.threads.get_mut(thread) else {
            return (Vec::new(), styles.gutter);
        };
        let blank_gutter = || Span::styled(format!("{marker}{}", " ".repeat(GUTTER_WIDTH - 2)), styles.gutter);
        if row == 0 {
            let highlight = (current && focused).then_some(styles.header_cursor);
            let mut spans = vec![blank_gutter()];
            spans.extend(view.summary_spans(code_width, &thread_styles, highlight));
            return (spans, thread_styles.fill);
        }
        if row == 1
            && let Some(original) = &view.original
        {
            let background = line_background(original.kind, current, theme).unwrap_or_else(|| theme.paint(styles.base));
            let text_style = thread_styles.muted_fg.bg(background);
            let text = display::sanitize(&original.text).into_owned();
            let (mut code, left) = display::cut_spans([Span::styled(text, text_style)], code_width);
            if left > 0 {
                code.push(Span::styled(" ".repeat(left), text_style));
            }
            let mut spans = vec![
                Span::styled(format!("{marker}{:>4}", original.number), styles.gutter),
                Span::styled("●", thread_styles.flag_outdated.bg(theme.paint(styles.base))),
                Span::styled(" │ ", styles.gutter),
            ];
            spans.extend(code);
            return (spans, text_style);
        }
        let index = row - 1 - usize::from(view.original.is_some());
        let line = view
            .comment_rows(layout_width, theme)
            .lines
            .get(index)
            .cloned()
            .unwrap_or_default();
        let mut spans = vec![blank_gutter()];
        spans.extend(display::cut_spans(line.spans, code_width).0);
        (spans, thread_styles.fill)
    }
}

/// A file's header: `▼` or `▶`, then `📄 <path>` and its counts, or `🗑️  <path> (deleted)`; then ` · N comments`
/// when it has review threads or pending comments.
fn header_spans(state: &FileState, style: Style, added: Color, removed: Color) -> Vec<Span<'static>> {
    let file = &state.file;
    let glyph = if state.folded { "▶" } else { "▼" };
    let path = display::sanitize(&file.path);
    let mut spans = if file.deleted {
        vec![Span::styled(format!("{glyph} 🗑️  {path} (deleted)"), style)]
    } else {
        let mut spans = vec![Span::styled(format!("{glyph} 📄 {path}"), style)];
        spans.extend(count_spans(file.counts(), style, added, removed));
        spans
    };
    match state.threads + state.pending {
        0 => {}
        1 => spans.push(Span::styled(" · 1 comment", style)),
        n => spans.push(Span::styled(format!(" · {n} comments"), style)),
    }
    spans
}

/// ` (+A -D)` in `style`, the counts in `added` and `removed`; a zero count left out, nothing when both are.
fn count_spans(counts: Counts, style: Style, added: Color, removed: Color) -> Vec<Span<'static>> {
    let Counts { additions, deletions } = counts;
    if counts.is_empty() {
        return Vec::new();
    }
    let mut spans = vec![Span::styled(" (", style)];
    if additions > 0 {
        spans.push(Span::styled(format!("+{additions}"), style.fg(added)));
    }
    if additions > 0 && deletions > 0 {
        spans.push(Span::styled(" ", style));
    }
    if deletions > 0 {
        spans.push(Span::styled(format!("-{deletions}"), style.fg(removed)));
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
