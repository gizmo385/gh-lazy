//! The multi-line editor: Textual's `TextArea` as LazyGithub uses it (`soft_wrap=True, tab_behavior="focus"`), in its
//! code-editor variant (line numbers, the monokai colors) and its plain one.
//!
//! `ratatui-textarea` holds the lines and does the edits. Everything Textual decides differently from that crate is
//! here: where each key moves the cursor ([`nav`], over the rows [`wrap`] breaks lines into), which range each delete
//! key removes, and the drawing ([`render`]). The crate's own key handling, history and widget are never used.
//!
//! Selection keys, undo and redo, the clipboard keys and the mouse come with #257. The selection is already an
//! anchor and a cursor, as in Textual, so those keys only move the anchor.

mod nav;
mod render;
mod wrap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

use self::nav::Nav;
use self::wrap::WrapLayout;
use crate::components::scrollbar::VERTICAL_WIDTH;
use crate::components::text_input::printable;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

/// A place in the text: (line, column), both from 0, the column in chars. Ordered as Textual orders its locations.
pub type Location = (usize, usize);

/// `TextArea.BINDINGS`, in Textual's order. The selection, clipboard, undo and redo keys are taken but do nothing
/// until #257.
pub const TEXT_EDITOR_BINDINGS: &[BindingId] = &[
    BindingId::TextualTextAreaCursorUp,
    BindingId::TextualTextAreaCursorDown,
    BindingId::TextualTextAreaCursorLeft,
    BindingId::TextualTextAreaCursorRight,
    BindingId::TextualTextAreaCursorWordLeft,
    BindingId::TextualTextAreaCursorWordRight,
    BindingId::TextualTextAreaCursorLineStart,
    BindingId::TextualTextAreaCursorLineEnd,
    BindingId::TextualTextAreaCursorPageUp,
    BindingId::TextualTextAreaCursorPageDown,
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

/// Cells between each side of the border and the text area (`padding: 0 1`).
const PADDING: u16 = 1;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum EditorOutcome {
    /// The text changed: Textual's `TextArea.Changed`, which the owner reacts to (a preview, a draft save).
    Changed,
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
    /// The selection's start, which stays put while the cursor (its end) moves. Equal to the cursor when nothing is
    /// selected; only #257's select keys and the mouse will set it apart.
    anchor: Location,
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
            anchor: (0, 0),
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

    /// The text, its lines joined with `\n`.
    pub fn text(&self) -> String {
        self.lines().join("\n")
    }

    /// Replaces the text (Textual's `TextArea.text = ...`): the lines are split as Python's `str.splitlines` splits
    /// them, a trailing line break leaves an empty last line, and the cursor and the view go back to the start.
    pub fn set_text(&mut self, text: &str) {
        let mut buffer = TextArea::new(insert_lines(text));
        buffer.set_max_histories(0);
        self.buffer = buffer;
        self.anchor = (0, 0);
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
        self.move_to(location, true);
    }

    /// The first wrapped row shown.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_y
    }

    /// Whether the editor takes `key` before any binding: printable characters (Textual's `check_consume_key`),
    /// `enter`, which inserts a line break, and every key in [`TEXT_EDITOR_BINDINGS`]. Never `tab` or `shift+tab`,
    /// which move focus (`tab_behavior="focus"`).
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        printable(key).is_some() || is_enter(key) || keymap.match_in(TEXT_EDITOR_BINDINGS, key).is_some()
    }

    /// A key: a printable character is typed over the selection, `enter` inserts a line break, and the movement and
    /// delete keys do what Textual's do.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap) -> EditorOutcome {
        if let Some(c) = printable(key) {
            return self.replace_selection(c.encode_utf8(&mut [0; 4]));
        }
        if is_enter(key) {
            return self.replace_selection("\n");
        }
        match keymap.match_in(TEXT_EDITOR_BINDINGS, key) {
            Some(id) => self.on_binding(id),
            None => EditorOutcome::Ignored,
        }
    }

    /// A bracketed paste: the whole text, every line of it, over the selection, the cursor after it.
    pub fn handle_paste(&mut self, text: &str) -> EditorOutcome {
        self.replace_selection(text)
    }

    fn on_binding(&mut self, id: BindingId) -> EditorOutcome {
        use BindingId as B;
        let cursor = self.cursor();
        let nav = self.nav();
        let (start, end) = (self.anchor.min(cursor), self.anchor.max(cursor));
        let selected = start != end;
        match id {
            B::TextualTextAreaCursorLeft => {
                let target = if selected { start } else { nav.left(cursor) };
                self.move_key(target, true)
            }
            B::TextualTextAreaCursorRight => {
                let target = if selected { end } else { nav.right(cursor) };
                self.move_key(target, true)
            }
            B::TextualTextAreaCursorUp => self.move_key(nav.above(cursor, self.last_x_offset), false),
            B::TextualTextAreaCursorDown => self.move_key(nav.below(cursor, self.last_x_offset), false),
            B::TextualTextAreaCursorWordLeft if cursor != (0, 0) => self.move_key(nav.word_left(cursor), true),
            B::TextualTextAreaCursorWordRight if !nav.is_end(cursor) => self.move_key(nav.word_right(cursor), true),
            B::TextualTextAreaCursorLineStart => self.move_key(nav.line_start(cursor, true), true),
            B::TextualTextAreaCursorLineEnd => self.move_key(nav.line_end(cursor), true),
            B::TextualTextAreaCursorPageUp => self.page(-1),
            B::TextualTextAreaCursorPageDown => self.page(1),
            B::TextualTextAreaDeleteLeft => {
                let to = if selected { self.anchor } else { nav.left(cursor) };
                self.delete(cursor, to)
            }
            B::TextualTextAreaDeleteRight => {
                let to = if selected { self.anchor } else { nav.right(cursor) };
                self.delete(cursor, to)
            }
            B::TextualTextAreaDeleteWordLeft if cursor != (0, 0) => {
                let to = if selected { self.anchor } else { nav.word_left(cursor) };
                self.delete(cursor, to)
            }
            B::TextualTextAreaDeleteWordRight if !nav.is_end(cursor) => {
                let to = if selected {
                    self.anchor
                } else {
                    nav.delete_word_right_target(cursor)
                };
                self.delete(cursor, to)
            }
            B::TextualTextAreaDeleteToStartOfLine => self.delete(cursor, nav.line_start(cursor, false)),
            B::TextualTextAreaDeleteToEndOfLineOrDeleteLine => {
                let line_end = nav.line_end(cursor);
                if nav.line_start(cursor, false) == line_end {
                    self.delete_lines()
                } else if !selected && cursor == line_end {
                    self.delete(cursor, nav.right(cursor))
                } else {
                    self.delete(cursor, line_end)
                }
            }
            B::TextualTextAreaDeleteLine => self.delete_lines(),
            // The select, clipboard, undo and redo keys: #257. The word keys at the text's ends do nothing either,
            // as in Textual.
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

    /// A movement key's move. `record` is false for up and down, which keep aiming for the remembered column.
    fn move_key(&mut self, target: Location, record: bool) -> EditorOutcome {
        let before = (self.selection(), self.scroll_y);
        self.move_to(target, record);
        if (self.selection(), self.scroll_y) == before {
            EditorOutcome::Ignored
        } else {
            EditorOutcome::Moved
        }
    }

    /// Puts the cursor at `target` with nothing selected, and scrolls it into view (Textual's `move_cursor`).
    fn move_to(&mut self, (row, column): Location, record: bool) {
        // `Jump` takes u16s; no line in an issue body is that long, and the crate clamps to the text anyway.
        let jump = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        self.buffer.move_cursor(CursorMove::Jump(jump(row), jump(column)));
        self.anchor = self.cursor();
        if record {
            self.record_cursor_width();
        }
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
        self.move_to(target, true);
        if (self.selection(), self.scroll_y) == before {
            EditorOutcome::Ignored
        } else {
            EditorOutcome::Moved
        }
    }

    /// `_delete_cursor_line`: the cursor's line goes, line break and all, and the cursor keeps its column where the
    /// line that takes its place is long enough.
    fn delete_lines(&mut self) -> EditorOutcome {
        let (row, column) = self.cursor();
        let outcome = self.delete((row, 0), (row + 1, 0));
        // `move_cursor_relative(columns=…, record_width=False)` after the edit, which recorded column 0.
        let target = self.nav().clamp((self.cursor().0, self.cursor().1 + column));
        self.move_to(target, false);
        outcome
    }

    fn delete(&mut self, from: Location, to: Location) -> EditorOutcome {
        self.replace(from, to, "")
    }

    fn replace_selection(&mut self, text: &str) -> EditorOutcome {
        self.replace(self.anchor, self.cursor(), text)
    }

    /// Textual's `Document.replace_range` through the keyboard (`maintain_selection_offset=False`): `text` replaces
    /// the text between `from` and `to` (either order), the cursor goes after it with nothing selected, and its
    /// column is remembered.
    fn replace(&mut self, from: Location, to: Location, text: &str) -> EditorOutcome {
        let nav = self.nav();
        // A range past the text's last line (deleting the last line) stops at the text's end, as Python's slicing
        // does.
        let past_end = |location: Location| {
            if location.0 >= self.lines().len() {
                nav.end_of_text()
            } else {
                nav.clamp(location)
            }
        };
        let (top, bottom) = (past_end(from.min(to)), past_end(from.max(to)));
        let removed = self.chars_between(top, bottom);
        let inserted = insert_lines(text);
        let before = self.selection();

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
        let changed = removed > 0 || !text.is_empty();
        if changed {
            self.relayout();
        }
        self.move_to(end, true);
        if changed {
            EditorOutcome::Changed
        } else if self.selection() != before {
            EditorOutcome::Moved
        } else {
            EditorOutcome::Ignored
        }
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
        usize::from(self.view.area.width.saturating_sub(2 + 2 * PADDING))
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
            self.view.height = usize::from(area.height.saturating_sub(2));
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
