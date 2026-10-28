//! The multi-line editor: Textual's `TextArea` as LazyGithub uses it (`soft_wrap=True, tab_behavior="focus"`), in its
//! code-editor variant (line numbers, the monokai colors) and its plain one.
//!
//! `ratatui-textarea` holds the lines and does the edits. Everything Textual decides differently from that crate is
//! here: where each key moves the cursor ([`nav`], over the rows [`wrap`] breaks lines into), which range each delete
//! key removes, undo and redo with Textual's batching ([`history`]), the mouse, and the drawing ([`render`]). The
//! crate's own key handling, history, selection and widget are never used.

mod history;
mod nav;
mod render;
mod wrap;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

use tokio::time::Instant;

use self::history::{Edit, EditHistory};
use self::nav::Nav;
use self::wrap::WrapLayout;
use crate::components::scrollbar::VERTICAL_WIDTH;
use crate::components::text_input::printable;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

/// A place in the text: (line, column), both from 0, the column in chars. Ordered as Textual orders its locations.
pub type Location = (usize, usize);

/// The editor's bindings in Textual's merged order: `TextArea.BINDINGS` with the `ScrollView` keys they replace in
/// those keys' places, and `ScrollView`'s `ctrl+pageup` / `ctrl+pagedown`, which a soft-wrapped editor takes and does
/// nothing with.
pub const TEXT_EDITOR_BINDINGS: &[BindingId] = &[
    BindingId::TextualTextAreaCursorUp,
    BindingId::TextualTextAreaCursorDown,
    BindingId::TextualTextAreaCursorLeft,
    BindingId::TextualTextAreaCursorRight,
    BindingId::TextualTextAreaCursorLineStart,
    BindingId::TextualTextAreaCursorLineEnd,
    BindingId::TextualTextAreaCursorPageUp,
    BindingId::TextualTextAreaCursorPageDown,
    BindingId::TextualScrollPageLeft,
    BindingId::TextualScrollPageRight,
    BindingId::TextualTextAreaCursorWordLeft,
    BindingId::TextualTextAreaCursorWordRight,
    BindingId::TextualTextAreaCursorWordLeftSelect,
    BindingId::TextualTextAreaCursorWordRightSelect,
    BindingId::TextualTextAreaCursorLineStartSelect,
    BindingId::TextualTextAreaCursorLineEndSelect,
    BindingId::TextualTextAreaCursorUpSelect,
    BindingId::TextualTextAreaCursorDownSelect,
    BindingId::TextualTextAreaCursorLeftSelect,
    BindingId::TextualTextAreaCursorRightSelect,
    BindingId::TextualTextAreaSelectLine,
    BindingId::TextualTextAreaSelectAll,
    BindingId::TextualTextAreaDeleteLeft,
    BindingId::TextualTextAreaDeleteWordLeft,
    BindingId::TextualTextAreaDeleteRight,
    BindingId::TextualTextAreaDeleteWordRight,
    BindingId::TextualTextAreaCut,
    BindingId::TextualTextAreaCopy,
    BindingId::TextualTextAreaPaste,
    BindingId::TextualTextAreaDeleteToStartOfLine,
    BindingId::TextualTextAreaDeleteToEndOfLineOrDeleteLine,
    BindingId::TextualTextAreaDeleteLine,
    BindingId::TextualTextAreaUndo,
    BindingId::TextualTextAreaRedo,
];

/// Cells between each side of the border and the text area by default (`padding: 0 1`).
const PADDING: u16 = 1;

/// The padding inside the editor's border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorPadding {
    /// Rows above the text.
    pub top: u16,
    /// Columns each side of the text.
    pub horizontal: u16,
}

impl Default for EditorPadding {
    /// `TextArea`'s own `padding: 0 1`.
    fn default() -> Self {
        Self {
            top: 0,
            horizontal: PADDING,
        }
    }
}
/// Cells between the line numbers and the text (`gutter_margin`).
const GUTTER_MARGIN: usize = 2;
/// The column `wrap_width` keeps free for the cursor past a row's last char.
const CURSOR_WIDTH: usize = 1;

/// Which of LazyGithub's two editors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorVariant {
    /// `TextArea.code_editor`: line numbers and the monokai colors (New issue, Edit issue, the PR description).
    Code,
    /// A plain `TextArea`: no line numbers, the app theme's colors (New comment, the diff viewer's Add comment).
    Plain,
}

/// The editor's border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorBorder {
    /// `TextArea`'s own `tall` border, `$border-blurred` or `$border` while focused.
    Tall,
    /// `border: blank`: New comment's editor keeps the border's cells but draws nothing in them.
    Blank,
}

/// What a key or a paste did to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum EditorOutcome {
    /// The text changed: Textual's `TextArea.Changed`, which the owner reacts to (a preview, a draft save).
    Changed,
    /// `ctrl+c` with a selection or `ctrl+x`: the owner puts `text` on the clipboard with
    /// [`Action::Copy`](crate::action::Action::Copy). A cut also `changed` the text, which the owner reacts to as to
    /// [`Self::Changed`].
    Copied {
        /// What was copied or cut.
        text: String,
        /// Whether the text changed (a cut that removed something).
        changed: bool,
    },
    /// Only the cursor or the view moved.
    Moved,
    /// Nothing changed.
    Ignored,
}

/// The layout of the last draw, which movement and wrapping are measured against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct View {
    /// Where the editor was drawn, border included. Empty until the first draw, which leaves the text unwrapped.
    area: Rect,
    /// The rows of text it shows.
    height: usize,
    /// Whether the vertical scrollbar takes its two columns.
    scrollbar: bool,
}

/// A multi-line text editor. See the module docs.
#[derive(Debug, Clone)]
pub struct TextEditor {
    /// The lines and the cursor. The crate's selection and history are unused.
    buffer: TextArea<'static>,
    variant: EditorVariant,
    border: EditorBorder,
    padding: EditorPadding,
    /// The selection's start, which stays put while the cursor (its end) moves. Equal to the cursor when nothing is
    /// selected. The app keeps it rather than the crate, whose selection doesn't say which end is the anchor.
    anchor: Location,
    /// Undo and redo.
    history: EditHistory,
    /// A left press started a drag that hasn't been released (Textual's `_selecting`).
    selecting: bool,
    /// The wrapped rows at the last draw's width.
    layout: WrapLayout,
    /// Textual's navigator memory: the cursor's cell offset when it last moved sideways, which up and down aim for.
    last_x_offset: usize,
    /// The first wrapped row shown.
    scroll_y: usize,
    view: View,
}

impl TextEditor {
    /// An editor holding `text`, with the cursor at its start and Textual's own `tall` border.
    pub fn new(variant: EditorVariant, text: &str) -> Self {
        let mut editor = Self {
            buffer: TextArea::default(),
            variant,
            border: EditorBorder::Tall,
            padding: EditorPadding::default(),
            anchor: (0, 0),
            history: EditHistory::default(),
            selecting: false,
            layout: WrapLayout::default(),
            last_x_offset: 0,
            scroll_y: 0,
            view: View::default(),
        };
        editor.set_text(text);
        editor
    }

    /// The same editor with `border` instead of the `tall` one.
    #[must_use]
    pub fn with_border(mut self, border: EditorBorder) -> Self {
        self.border = border;
        self
    }

    /// The same editor with `padding` inside its border instead of `padding: 0 1`: New comment's
    /// `padding-top: 1`, which Textual stores as `padding: 1 0 0 0`.
    #[must_use]
    pub fn with_padding(mut self, padding: EditorPadding) -> Self {
        self.padding = padding;
        self
    }

    /// The padding inside the border.
    pub(super) fn padding(&self) -> EditorPadding {
        self.padding
    }

    /// The text, its lines joined with `\n`.
    pub fn text(&self) -> String {
        self.lines().join("\n")
    }

    /// Replaces the text (Textual's `TextArea.text = ...`): the lines are split as Python's `str.splitlines` splits
    /// them, a trailing line break leaves an empty last line, the cursor and the view go back to the start, and the
    /// undo history is cleared (`load_text`).
    pub fn set_text(&mut self, text: &str) {
        let mut buffer = TextArea::new(insert_lines(text));
        buffer.set_max_histories(0);
        self.buffer = buffer;
        self.anchor = (0, 0);
        self.history.clear();
        self.selecting = false;
        self.last_x_offset = 0;
        self.scroll_y = 0;
        self.relayout();
    }

    /// The cursor: the selection's end.
    pub fn cursor(&self) -> Location {
        let cursor = self.buffer.cursor();
        (cursor.0, cursor.1)
    }

    /// The selection as Textual's `Selection(start, end)`: the anchor, then the cursor. Both are the cursor when
    /// nothing is selected.
    pub fn selection(&self) -> (Location, Location) {
        (self.anchor, self.cursor())
    }

    /// Moves the cursor to `location`, clamped into the text, and remembers its column for up and down, as
    /// Textual's `move_cursor` does.
    pub fn set_cursor(&mut self, location: Location) {
        let location = self.nav().clamp(location);
        self.move_cursor(location, false, true);
    }

    /// The first wrapped row shown.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_y
    }

    /// Whether the editor takes `key` before any binding: printable characters (Textual's `check_consume_key`),
    /// `enter`, which inserts a line break, and every key in [`TEXT_EDITOR_BINDINGS`], except copy with nothing
    /// selected, which Textual skips so the key goes on. Never `tab` or `shift+tab`, which move focus
    /// (`tab_behavior="focus"`).
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        printable(key).is_some()
            || is_enter(key)
            || keymap
                .match_in(TEXT_EDITOR_BINDINGS, key)
                .is_some_and(|id| id != BindingId::TextualTextAreaCopy || self.has_selection())
    }

    /// A key at `now` (which batches undo): a printable character is typed over the selection, `enter` inserts a
    /// line break, `ctrl+v` pastes `clipboard` (the in-app clipboard, [`Cx::clipboard`](crate::component::Cx)),
    /// and the other keys do what Textual's do.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str, now: Instant) -> EditorOutcome {
        if let Some(c) = printable(key) {
            return self.replace_selection(c.encode_utf8(&mut [0; 4]), now).0;
        }
        if is_enter(key) {
            return self.replace_selection("\n", now).0;
        }
        match keymap.match_in(TEXT_EDITOR_BINDINGS, key) {
            Some(id) => self.on_binding(id, clipboard, now),
            None => EditorOutcome::Ignored,
        }
    }

    /// A bracketed paste at `now`: the whole text, every line of it, over the selection, the cursor after it.
    pub fn handle_paste(&mut self, text: &str, now: Instant) -> EditorOutcome {
        self.paste(text, now)
    }

    /// The editor gained focus: the next edit starts a new undo batch, as Textual's `_watch_has_focus` does.
    pub fn focus_gained(&mut self) {
        self.history.checkpoint();
    }

    fn has_selection(&self) -> bool {
        self.anchor != self.cursor()
    }

    /// The selection as (top, bottom), in text order.
    pub(super) fn selection_range(&self) -> (Location, Location) {
        let cursor = self.cursor();
        (self.anchor.min(cursor), self.anchor.max(cursor))
    }

    /// The selected text, lines joined with `\n`; empty when nothing is selected.
    pub fn selected_text(&self) -> String {
        let (top, bottom) = self.selection_range();
        self.text_between(top, bottom)
    }

    fn on_binding(&mut self, id: BindingId, clipboard: &str, now: Instant) -> EditorOutcome {
        use BindingId as B;
        let cursor = self.cursor();
        let nav = self.nav();
        let (start, end) = self.selection_range();
        let selected = start != end;
        let last_x = self.last_x_offset;
        match id {
            B::TextualTextAreaCursorLeft => self.move_key(if selected { start } else { nav.left(cursor) }, false, true),
            B::TextualTextAreaCursorLeftSelect => self.move_key(nav.left(cursor), true, true),
            B::TextualTextAreaCursorRight => self.move_key(if selected { end } else { nav.right(cursor) }, false, true),
            B::TextualTextAreaCursorRightSelect => self.move_key(nav.right(cursor), true, true),
            B::TextualTextAreaCursorUp | B::TextualTextAreaCursorUpSelect => {
                self.move_key(nav.above(cursor, last_x), id == B::TextualTextAreaCursorUpSelect, false)
            }
            B::TextualTextAreaCursorDown | B::TextualTextAreaCursorDownSelect => self.move_key(
                nav.below(cursor, last_x),
                id == B::TextualTextAreaCursorDownSelect,
                false,
            ),
            B::TextualTextAreaCursorWordLeft | B::TextualTextAreaCursorWordLeftSelect if cursor != (0, 0) => self
                .move_key(
                    nav.word_left(cursor),
                    id == B::TextualTextAreaCursorWordLeftSelect,
                    true,
                ),
            B::TextualTextAreaCursorWordRight | B::TextualTextAreaCursorWordRightSelect if !nav.is_end(cursor) => self
                .move_key(
                    nav.word_right(cursor),
                    id == B::TextualTextAreaCursorWordRightSelect,
                    true,
                ),
            B::TextualTextAreaCursorLineStart | B::TextualTextAreaCursorLineStartSelect => self.move_key(
                nav.line_start(cursor, true),
                id == B::TextualTextAreaCursorLineStartSelect,
                true,
            ),
            B::TextualTextAreaCursorLineEnd | B::TextualTextAreaCursorLineEndSelect => {
                self.move_key(nav.line_end(cursor), id == B::TextualTextAreaCursorLineEndSelect, true)
            }
            B::TextualTextAreaCursorPageUp => self.page(-1),
            B::TextualTextAreaCursorPageDown => self.page(1),
            // `select_line` and `select_all` set the selection without ending the undo batch.
            B::TextualTextAreaSelectLine => {
                let row = cursor.0;
                self.select(((row, 0), (row, nav.len(row))))
            }
            B::TextualTextAreaSelectAll => self.select(((0, 0), nav.end_of_text())),
            B::TextualTextAreaDeleteLeft => {
                let to = if selected { self.anchor } else { nav.left(cursor) };
                self.edit(cursor, to, "", now).0
            }
            B::TextualTextAreaDeleteRight => {
                let to = if selected { self.anchor } else { nav.right(cursor) };
                self.edit(cursor, to, "", now).0
            }
            B::TextualTextAreaDeleteWordLeft if cursor != (0, 0) => {
                let to = if selected { self.anchor } else { nav.word_left(cursor) };
                self.edit(cursor, to, "", now).0
            }
            B::TextualTextAreaDeleteWordRight if !nav.is_end(cursor) => {
                let to = if selected {
                    self.anchor
                } else {
                    nav.delete_word_right_target(cursor)
                };
                self.edit(cursor, to, "", now).0
            }
            B::TextualTextAreaDeleteToStartOfLine => self.edit(cursor, nav.line_start(cursor, false), "", now).0,
            B::TextualTextAreaDeleteToEndOfLineOrDeleteLine => {
                let line_end = nav.line_end(cursor);
                if nav.line_start(cursor, false) == line_end {
                    self.delete_lines(now).0
                } else if !selected && cursor == line_end {
                    self.edit(cursor, nav.right(cursor), "", now).0
                } else {
                    self.edit(cursor, line_end, "", now).0
                }
            }
            B::TextualTextAreaDeleteLine => self.delete_lines(now).0,
            // `action_cut`: the selection, or the cursor's whole line with its line break.
            B::TextualTextAreaCut => {
                let (outcome, text) = if selected {
                    self.edit(self.anchor, cursor, "", now)
                } else {
                    self.delete_lines(now)
                };
                EditorOutcome::Copied {
                    text,
                    changed: outcome == EditorOutcome::Changed,
                }
            }
            // `action_copy`: without a selection it skips, which `consumes_key` already said.
            B::TextualTextAreaCopy if selected => EditorOutcome::Copied {
                text: self.selected_text(),
                changed: false,
            },
            B::TextualTextAreaPaste => self.paste(clipboard, now),
            B::TextualTextAreaUndo => self.undo(),
            B::TextualTextAreaRedo => self.redo(),
            // The word keys at the text's ends, copy without a selection, and `ctrl+pageup` / `ctrl+pagedown`
            // (`page_left` / `page_right`, with nothing to scroll sideways) do nothing, as in Textual.
            _ => EditorOutcome::Ignored,
        }
    }

    fn nav(&self) -> Nav<'_> {
        Nav {
            lines: self.lines(),
            wrap: &self.layout,
        }
    }

    fn lines(&self) -> &[String] {
        self.buffer.lines()
    }

    /// Compares what a change did to the selection and the view.
    fn outcome_since(&self, before: ((Location, Location), usize)) -> EditorOutcome {
        if (self.selection(), self.scroll_y) == before {
            EditorOutcome::Ignored
        } else {
            EditorOutcome::Moved
        }
    }

    /// A movement key's move. `select` keeps the anchor; `record` is false for up and down, which keep aiming for
    /// the remembered column.
    fn move_key(&mut self, target: Location, select: bool, record: bool) -> EditorOutcome {
        let before = (self.selection(), self.scroll_y);
        self.move_cursor(target, select, record);
        self.outcome_since(before)
    }

    /// Textual's `move_cursor`: the cursor goes to `target`, the anchor with it unless `select`, its column is
    /// remembered if `record`, and the undo batch ends.
    fn move_cursor(&mut self, target: Location, select: bool, record: bool) {
        let anchor = if select { self.anchor } else { target };
        self.set_selection((anchor, target));
        if record {
            self.record_cursor_width();
        }
        self.history.checkpoint();
    }

    /// Sets the selection directly, remembering the cursor's column (`select_line`, `select_all`).
    fn select(&mut self, selection: (Location, Location)) -> EditorOutcome {
        let before = (self.selection(), self.scroll_y);
        self.set_selection(selection);
        self.record_cursor_width();
        self.outcome_since(before)
    }

    /// Sets the selection as (anchor, cursor) and scrolls the cursor into view, as Textual's `selection` watcher
    /// does.
    fn set_selection(&mut self, (anchor, (row, column)): (Location, Location)) {
        // `Jump` takes u16s; no line in an issue body is that long, and the crate clamps to the text anyway.
        let jump = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        self.buffer.move_cursor(CursorMove::Jump(jump(row), jump(column)));
        self.anchor = anchor;
        self.scroll_cursor_visible();
    }

    /// `record_cursor_width`: what up and down aim for from now on.
    fn record_cursor_width(&mut self) {
        self.last_x_offset = self.layout.location_to_offset(self.lines(), self.cursor()).0;
    }

    /// `pageup` / `pagedown`: the view scrolls a page and the cursor moves a page of wrapped rows.
    fn page(&mut self, direction: isize) -> EditorOutcome {
        let height = self.view.height;
        let rows = direction * isize::try_from(height).unwrap_or(isize::MAX);
        let target = self.nav().at_row_offset(self.cursor(), rows);
        let before = (self.selection(), self.scroll_y);
        self.scroll_y = if direction < 0 {
            self.scroll_y.saturating_sub(height)
        } else {
            (self.scroll_y + height).min(self.max_scroll())
        };
        self.move_cursor(target, false, true);
        self.outcome_since(before)
    }

    /// `_delete_cursor_line`: the lines the selection touches (the cursor's line) go, line breaks and all, and the
    /// cursor keeps the selection end's column where the line that takes their place is long enough. A selection
    /// ending at a line's start leaves that line. Also returns the deleted text.
    fn delete_lines(&mut self, now: Instant) -> (EditorOutcome, String) {
        let (start, (mut end_row, end_column)) = self.selection_range();
        if start.0 != end_row && end_column == 0 {
            end_row -= 1;
        }
        let result = self.edit((start.0, 0), (end_row + 1, 0), "", now);
        // `move_cursor_relative(columns=…, record_width=False)` after the edit, which recorded column 0.
        let (row, column) = self.cursor();
        let target = self.nav().clamp((row, column + end_column));
        self.move_cursor(target, false, false);
        result
    }

    fn replace_selection(&mut self, text: &str, now: Instant) -> (EditorOutcome, String) {
        self.edit(self.anchor, self.cursor(), text, now)
    }

    /// `action_paste` and `_on_paste`: `text` over the selection, then the cursor is moved to its end, which ends the
    /// undo batch.
    fn paste(&mut self, text: &str, now: Instant) -> EditorOutcome {
        let (outcome, _) = self.replace_selection(text, now);
        let cursor = self.cursor();
        self.move_cursor(cursor, false, true);
        outcome
    }

    /// An edit through the keyboard (Textual's `edit` with `maintain_selection_offset=False`): `text` replaces the
    /// text between `from` and `to` (either order), the cursor goes after it with nothing selected and its column is
    /// remembered, and the edit is recorded for undo. Returns the replaced text too.
    fn edit(&mut self, from: Location, to: Location, text: &str, now: Instant) -> (EditorOutcome, String) {
        let selection_before = self.selection();
        let (top, bottom) = self.clamp_range(from, to);
        let (end, replaced) = self.apply(top, bottom, text);
        let changed = !replaced.is_empty() || !text.is_empty();
        if changed {
            self.relayout();
        }
        self.set_selection((end, end));
        self.record_cursor_width();
        let outcome = if changed {
            EditorOutcome::Changed
        } else if self.selection() != selection_before {
            EditorOutcome::Moved
        } else {
            EditorOutcome::Ignored
        };
        self.history.record(
            Edit {
                top,
                bottom,
                text: text.to_owned(),
                replaced: replaced.clone(),
                end,
                selection_before,
            },
            now,
        );
        (outcome, replaced)
    }

    /// `from` and `to` in text order and moved into the text. Past the last line (deleting the last line) is the
    /// text's end, as Python's slicing makes it.
    fn clamp_range(&self, from: Location, to: Location) -> (Location, Location) {
        let nav = self.nav();
        let clamp = |location: Location| {
            if location.0 >= self.lines().len() {
                nav.end_of_text()
            } else {
                nav.clamp(location)
            }
        };
        (clamp(from.min(to)), clamp(from.max(to)))
    }

    /// `Document.replace_range` on the buffer: `text` replaces `top..bottom`, split into lines as Textual splits it.
    /// Returns where the inserted text ends and what was replaced. Nothing else changes: no layout, no selection.
    fn apply(&mut self, top: Location, bottom: Location, text: &str) -> (Location, String) {
        let replaced = self.text_between(top, bottom);
        let removed = self.chars_between(top, bottom);
        let inserted = insert_lines(text);
        self.buffer.move_cursor(CursorMove::Jump(
            u16::try_from(top.0).unwrap_or(u16::MAX),
            u16::try_from(top.1).unwrap_or(u16::MAX),
        ));
        self.buffer.delete_str(removed);
        for (i, line) in inserted.iter().enumerate() {
            if i > 0 {
                self.buffer.insert_newline();
            }
            self.buffer.insert_str(line);
        }
        let end = match inserted.as_slice() {
            [only] => (top.0, top.1 + only.chars().count()),
            [.., last] => (top.0 + inserted.len() - 1, last.chars().count()),
            [] => top,
        };
        (end, replaced)
    }

    /// Undoes the newest batch: its edits are reversed newest first, and the selection goes back to what it was
    /// before the batch.
    fn undo(&mut self) -> EditorOutcome {
        let Some(batch) = self.history.pop_undo() else {
            return EditorOutcome::Ignored;
        };
        for edit in batch.iter().rev() {
            let _ = self.apply(edit.top, edit.end, &edit.replaced);
        }
        self.relayout();
        if let Some(first) = batch.first() {
            self.set_selection(first.selection_before);
        }
        self.record_cursor_width();
        EditorOutcome::Changed
    }

    /// Redoes the newest undone batch, in order; the cursor ends after its last edit.
    fn redo(&mut self) -> EditorOutcome {
        let Some(batch) = self.history.pop_redo() else {
            return EditorOutcome::Ignored;
        };
        let mut end = self.cursor();
        for edit in &batch {
            end = self.apply(edit.top, edit.bottom, &edit.text).0;
        }
        self.relayout();
        self.set_selection((end, end));
        self.record_cursor_width();
        EditorOutcome::Changed
    }

    /// The text between two locations in text order, lines joined with `\n` (`get_text_range`).
    fn text_between(&self, top: Location, bottom: Location) -> String {
        let lines = self.lines();
        let slice = |line: &str, from: usize, to: Option<usize>| -> String {
            let chars = line.chars().skip(from);
            match to {
                Some(to) => chars.take(to.saturating_sub(from)).collect(),
                None => chars.collect(),
            }
        };
        if top.0 == bottom.0 {
            return lines
                .get(top.0)
                .map_or(String::new(), |line| slice(line, top.1, Some(bottom.1)));
        }
        let mut text = slice(&lines[top.0], top.1, None);
        for line in &lines[top.0 + 1..bottom.0] {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text.push_str(&slice(&lines[bottom.0], 0, Some(bottom.1)));
        text
    }

    // The mouse.

    /// The location drawn at a screen cell, against the last draw (`get_target_document_location`): left of the
    /// text (the gutter, the padding) is the row's start, past a row's end its nearest position, below the text its
    /// last row.
    fn location_at(&self, column: u16, row: u16) -> Location {
        let area = self.view.area;
        let text_x = i64::from(area.x) + 1 + i64::from(self.padding.horizontal);
        let gutter = i64::try_from(self.gutter_width()).unwrap_or(0);
        let scroll = i64::try_from(self.scroll_y).unwrap_or(0);
        let x = i64::from(column) - text_x - gutter;
        let y = i64::from(row) - i64::from(area.y) - 1 - i64::from(self.padding.top) + scroll;
        let clamp = |n: i64| isize::try_from(n).unwrap_or(0);
        self.layout.offset_to_location(self.lines(), clamp(x), clamp(y))
    }

    /// A left press at a screen cell: the cursor goes to the char under it with nothing selected, a drag starts
    /// there, and the undo batch ends (Textual's `_on_mouse_down`). The owner focuses the editor and keeps the mouse
    /// for it until [`Self::release`].
    pub fn press(&mut self, column: u16, row: u16) {
        let target = self.location_at(column, row);
        self.set_selection((target, target));
        self.selecting = true;
        self.history.checkpoint();
    }

    /// The pointer moved to a cell with the button down: the selection runs from the press to the char under it,
    /// even outside the editor (`_on_mouse_move`). Nothing happens without a press.
    pub fn drag(&mut self, column: u16, row: u16) {
        if self.selecting {
            let target = self.location_at(column, row);
            self.set_selection((self.anchor, target));
        }
    }

    /// The button went up: the drag ends, the selection stays, and the cursor's column is remembered
    /// (`_end_mouse_selection`).
    pub fn release(&mut self) {
        if self.selecting {
            self.selecting = false;
            self.record_cursor_width();
        }
    }

    /// Whether a press started a drag that hasn't been released: the owner routes the mouse to the editor until then.
    pub fn is_selecting(&self) -> bool {
        self.selecting
    }

    /// A mouse event while the editor has the mouse, wherever the pointer is: a left drag selects
    /// ([`Self::drag`]) and the left release ends it ([`Self::release`]). Returns whether it took the event, which
    /// it never does without a drag.
    pub fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        if !self.selecting {
            return false;
        }
        match event.kind {
            MouseEventKind::Drag(MouseButton::Left) => self.drag(event.column, event.row),
            MouseEventKind::Up(MouseButton::Left) => self.release(),
            _ => return false,
        }
        true
    }

    /// The mouse wheel: the view scrolls `rows` wrapped rows (negative is up), and the cursor stays where it is.
    /// Returns whether the view moved: when it can't, Textual lets the wheel go on to the container around it.
    pub fn scroll(&mut self, rows: isize) -> bool {
        let before = self.scroll_y;
        self.scroll_y = self.scroll_y.saturating_add_signed(rows).min(self.max_scroll());
        self.scroll_y != before
    }

    /// How many chars `delete_str` removes to go from `top` to `bottom`, a line break counting as one.
    fn chars_between(&self, top: Location, bottom: Location) -> usize {
        if top.0 == bottom.0 {
            return bottom.1.saturating_sub(top.1);
        }
        let lines = self.lines();
        let first = lines[top.0].chars().count() - top.1;
        let middle: usize = lines[top.0 + 1..bottom.0]
            .iter()
            .map(|line| line.chars().count() + 1)
            .sum();
        first + 1 + middle + bottom.1
    }

    // Layout.

    /// The line numbers' width, margin included, in the code editor (`gutter_width`).
    fn gutter_width(&self) -> usize {
        match self.variant {
            EditorVariant::Code => self.lines().len().to_string().len() + GUTTER_MARGIN,
            EditorVariant::Plain => 0,
        }
    }

    /// The text area's width inside the border and the padding, scrollbar included.
    fn inner_width(&self) -> usize {
        usize::from(self.view.area.width.saturating_sub(2 + 2 * self.padding.horizontal))
    }

    /// Rewraps the text for the last draw's size: Textual's `wrap_width`, which leaves out the gutter, a column for
    /// the cursor, and the scrollbar once the wrapped text is taller than the view.
    fn relayout(&mut self) {
        let wrap_width = |scrollbar: bool| {
            let scrollbar = if scrollbar { usize::from(VERTICAL_WIDTH) } else { 0 };
            self.inner_width()
                .saturating_sub(scrollbar + self.gutter_width() + CURSOR_WIDTH)
        };
        let unwrapped = self.view.area.is_empty();
        let mut layout = WrapLayout::new(self.lines(), if unwrapped { 0 } else { wrap_width(false) });
        let scrollbar = !unwrapped && layout.height() > self.view.height;
        if scrollbar {
            layout = WrapLayout::new(self.lines(), wrap_width(true));
        }
        self.layout = layout;
        self.view.scrollbar = scrollbar;
        self.scroll_y = self.scroll_y.min(self.max_scroll());
    }

    fn max_scroll(&self) -> usize {
        self.layout.height().saturating_sub(self.view.height)
    }

    /// Scrolls so the cursor's row is in view (`scroll_cursor_visible`).
    fn scroll_cursor_visible(&mut self) {
        let height = self.view.height;
        if height == 0 {
            return;
        }
        let (_, y) = self.layout.location_to_offset(self.lines(), self.cursor());
        if y < self.scroll_y {
            self.scroll_y = y;
        } else if y >= self.scroll_y + height {
            self.scroll_y = y + 1 - height;
        }
        self.scroll_y = self.scroll_y.min(self.max_scroll());
    }

    /// Draws the editor into `area`: the border, the text with its line numbers (the code editor), the cursor's
    /// line highlighted, the cursor itself while `focused`, and a scrollbar when the text is taller than the view.
    /// `under` is the parent's background, which shows in the border's half-cell edges.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        if area != self.view.area {
            self.view.area = area;
            self.view.height = usize::from(area.height.saturating_sub(2 + self.padding.top));
            self.relayout();
            self.scroll_cursor_visible();
        }
        render::draw(self, buf, theme, under, focused);
    }
}

/// `enter`, which inserts a line break: not a binding in Textual, but handled by `TextArea._on_key`.
fn is_enter(key: &KeyChord) -> bool {
    *key == KeyChord::new(crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE)
}

/// How Textual's `Document` splits text into lines: Python's `str.splitlines`, plus an empty last line after a
/// trailing `\r\n`, `\n` or `\r`. An empty text is one empty line.
fn insert_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(is_line_break) {
        lines.push(rest[..at].to_owned());
        let skip = if rest[at..].starts_with("\r\n") {
            2
        } else {
            rest[at..].chars().next().map_or(1, char::len_utf8)
        };
        rest = &rest[at + skip..];
    }
    if !rest.is_empty() {
        lines.push(rest.to_owned());
    }
    if text.is_empty() || text.ends_with(['\n', '\r']) {
        lines.push(String::new());
    }
    lines
}

/// The line boundaries Python's `str.splitlines` splits on.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

#[cfg(test)]
mod tests;
