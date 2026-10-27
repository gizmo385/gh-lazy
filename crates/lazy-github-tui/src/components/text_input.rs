//! A single-line text input: Textual's `Input` with its default look (a tall border, `padding: 0 2`), its editing and
//! selection keys, its clipboard keys, and mouse-drag selection.
//!
//! `tui-textarea` (PLAN §4) is for the multi-line editors. A one-line box is small enough to own, which lets it match
//! Textual's styling and word rules exactly; the M3 lookup modals can reuse it.

use std::sync::LazyLock;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use regex::Regex;

use crate::components::table::is_printable;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{AnsiColor, Paint, Theme};

/// Python's `\w` in a `str` pattern: `str.isalnum()` (letters and every kind of number) and the underscore.
static WORD: LazyLock<Regex> = LazyLock::new(|| {
    // A fixed pattern that's covered by the tests, so it always compiles.
    Regex::new(r"[\p{L}\p{N}_]").expect("valid pattern")
});

/// The input's bindings, in Textual's merged order: `left`, `right`, `home` and `end` keep the places of the
/// `ScrollView` keys they replace, then the rest of `Input.BINDINGS` follows. The inherited scroll keys come with #252.
pub const INPUT_BINDINGS: &[BindingId] = &[
    BindingId::TextualInputCursorLeft,
    BindingId::TextualInputCursorRight,
    BindingId::TextualInputHome,
    BindingId::TextualInputEnd,
    BindingId::TextualInputCursorLeftSelect,
    BindingId::TextualInputCursorLeftWord,
    BindingId::TextualInputCursorLeftWordSelect,
    BindingId::TextualInputCursorRightSelect,
    BindingId::TextualInputCursorRightWord,
    BindingId::TextualInputCursorRightWordSelect,
    BindingId::TextualInputDeleteLeft,
    BindingId::TextualInputSelectAll,
    BindingId::TextualInputHomeSelect,
    BindingId::TextualInputEndSelect,
    BindingId::TextualInputDeleteRight,
    BindingId::TextualInputSubmit,
    BindingId::TextualInputDeleteLeftWord,
    BindingId::TextualInputDeleteLeftAll,
    BindingId::TextualInputDeleteRightWord,
    BindingId::TextualInputDeleteRightAll,
    BindingId::TextualInputCut,
    BindingId::TextualInputCopy,
    BindingId::TextualInputPaste,
];

/// Rows the input takes: the text between the top and bottom border.
pub const HEIGHT: u16 = 3;

/// Cells between each side border and the text (`padding: 0 2`).
const PADDING: u16 = 2;

/// The opacity of an invalid, unfocused input's red border (`tall $error 60%`).
const INVALID_BLURRED_ALPHA: f64 = 0.6;

/// What a key did to the input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum InputOutcome {
    /// Taken: the text or the cursor may have changed.
    Changed,
    /// `enter`: the owner acts on [`TextInput::value`].
    Submitted,
    /// `ctrl+c` with a selection, or `ctrl+x` with or without one (which may have deleted it): the owner puts the
    /// text on the clipboard with [`Action::Copy`](crate::action::Action::Copy).
    Copied(String),
    /// Not the input's: the owner passes it on.
    Ignored,
}

/// The input's text, cursor and selection.
#[derive(Debug, Clone)]
pub struct TextInput {
    value: String,
    /// A char index into `value`, from 0 to its length. It's the selection's end, as in Textual.
    cursor: usize,
    /// Where the selection started, while there is one. An anchor equal to the cursor selects nothing. The select
    /// keys keep or set it; every other key but `enter` clears it.
    anchor: Option<usize>,
    /// Where the input was drawn when a left press landed on it, while the button is still down: drags move the
    /// cursor against it (Textual's `_selecting`).
    selecting: Option<Rect>,
    placeholder: &'static str,
    /// The first text cell shown, which follows the cursor.
    scroll: usize,
    /// The last validation failed: Textual's `-invalid` class, drawn as a red border.
    invalid: bool,
    /// What the text may be, as Textual's `restrict`: an edit whose result fails it is refused.
    filter: Option<fn(&str) -> bool>,
}

impl TextInput {
    /// An empty input showing `placeholder`.
    pub fn new(placeholder: &'static str) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            anchor: None,
            selecting: None,
            placeholder,
            scroll: 0,
            invalid: false,
            filter: None,
        }
    }

    /// Only lets the text become what `filter` accepts: a typed key, a paste or a delete whose result it refuses
    /// changes nothing, as Textual's `Input.restrict` and `type` do (e.g. a number lookup's input).
    #[must_use]
    pub fn with_filter(mut self, filter: fn(&str) -> bool) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Marks the input invalid or valid, as the owner's last check of its text said. An invalid input has a red
    /// border (Textual's `-invalid` class) until the owner marks it valid again.
    pub fn set_invalid(&mut self, invalid: bool) {
        self.invalid = invalid;
    }

    /// Whether the last check marked it invalid.
    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    /// The text.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The cursor, as a char index.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Replaces the text and puts the cursor at its end.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.len();
        self.anchor = None;
    }

    /// Selects all the text, with the cursor at its end: what Textual's `Input` does on focus
    /// (`select_on_focus`, on by default).
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.len();
    }

    /// The selection's fixed end: where it started, or the cursor when nothing is selected. With
    /// [`Self::cursor`], it's Textual's `Selection(start, end)`.
    pub fn anchor(&self) -> usize {
        self.anchor.unwrap_or(self.cursor)
    }

    /// Moves the cursor to char `index`, or the end when it's past it, and drops any selection.
    pub fn set_cursor(&mut self, index: usize) {
        self.cursor = index.min(self.len());
        self.anchor = None;
    }

    /// The selected chars as `(start, end)` with `start < end`, when anything is selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    /// The selected text, when anything is selected.
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection()?;
        Some(&self.value[self.byte_at(start)..self.byte_at(end)])
    }

    /// Whether the input takes `key` before any binding: printable characters, which Textual's `Input` consumes
    /// (`check_consume_key`), and its own bindings, which are innermost in the chain. Copy with nothing selected
    /// isn't the input's: Textual's `action_copy` skips, so the key goes on down the chain.
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        printable(key).is_some()
            || keymap
                .match_in(INPUT_BINDINGS, key)
                .is_some_and(|id| id != BindingId::TextualInputCopy || self.selection().is_some())
    }

    /// A key: a printable character is typed, an editing key edits, `enter` submits, copy and cut hand back the
    /// selected text, paste types `clipboard` (the in-app clipboard, [`Cx::clipboard`](crate::component::Cx::clipboard)),
    /// and anything else is [`InputOutcome::Ignored`].
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str) -> InputOutcome {
        if let Some(c) = printable(key) {
            self.replace_selection(c.encode_utf8(&mut [0; 4]));
            return InputOutcome::Changed;
        }
        match keymap.match_in(INPUT_BINDINGS, key) {
            Some(id) => self.on_binding(id, clipboard),
            None => InputOutcome::Ignored,
        }
    }

    fn on_binding(&mut self, id: BindingId, clipboard: &str) -> InputOutcome {
        use BindingId as B;
        let len = self.len();
        match id {
            B::TextualInputSubmit => return InputOutcome::Submitted,
            // Textual's `action_paste`: the whole in-app clipboard over the selection, line breaks and all (PLAN
            // §10.10). A filter may refuse the result.
            B::TextualInputPaste => {
                self.replace_selection(clipboard);
                return InputOutcome::Changed;
            }
            // `action_copy` copies the selection and keeps it; without one it skips, which `consumes_key` already
            // said.
            B::TextualInputCopy => {
                return self
                    .selected_text()
                    .map_or(InputOutcome::Ignored, |text| InputOutcome::Copied(text.to_owned()));
            }
            // `action_cut` copies even an empty selection, so the clipboard empties, and deletes what's selected.
            B::TextualInputCut => {
                let text = self.selected_text().unwrap_or_default().to_owned();
                if self.selection().is_some() {
                    self.replace_selection("");
                }
                return InputOutcome::Copied(text);
            }
            _ => {}
        }
        if self.select(id) {
            return InputOutcome::Changed;
        }
        // Every other key ends the selection. With one, left and right collapse it to its start or end and the delete
        // keys delete it; the other moves start from the cursor, which is the selection's end.
        let selection = self.selection();
        self.anchor = None;
        if let Some((start, end)) = selection {
            match id {
                B::TextualInputCursorLeft => {
                    self.cursor = start;
                    return InputOutcome::Changed;
                }
                B::TextualInputCursorRight => {
                    self.cursor = end;
                    return InputOutcome::Changed;
                }
                B::TextualInputDeleteLeft
                | B::TextualInputDeleteRight
                | B::TextualInputDeleteLeftWord
                | B::TextualInputDeleteRightWord
                | B::TextualInputDeleteLeftAll
                | B::TextualInputDeleteRightAll => {
                    self.delete(start, end);
                    return InputOutcome::Changed;
                }
                _ => {}
            }
        }
        match id {
            B::TextualInputCursorLeft => self.cursor = self.cursor.saturating_sub(1),
            B::TextualInputCursorRight => self.cursor = (self.cursor + 1).min(len),
            B::TextualInputCursorLeftWord => self.cursor = self.word_start_before(),
            B::TextualInputCursorRightWord => self.cursor = self.word_start_after().unwrap_or(len),
            B::TextualInputHome => self.cursor = 0,
            B::TextualInputEnd => self.cursor = len,
            B::TextualInputDeleteLeft => {
                if self.cursor > 0 {
                    self.delete(self.cursor - 1, self.cursor);
                }
            }
            B::TextualInputDeleteRight => self.delete(self.cursor, (self.cursor + 1).min(len)),
            B::TextualInputDeleteLeftWord => self.delete(self.word_start_before(), self.cursor),
            // Up to the start of the next word, or everything when there's none.
            B::TextualInputDeleteRightWord => self.delete(self.cursor, self.word_start_after().unwrap_or(len)),
            B::TextualInputDeleteLeftAll => self.delete(0, self.cursor),
            B::TextualInputDeleteRightAll => self.delete(self.cursor, len),
            _ => return InputOutcome::Ignored,
        }
        InputOutcome::Changed
    }

    /// The select keys: Textual's `action_*(select=True)` and `select_all`. A select move keeps the anchor (the
    /// cursor, when nothing was selected) and moves the cursor; `shift+home` and `shift+end` re-anchor at the cursor
    /// first (`Selection(cursor_position, 0)`). False for any other binding.
    fn select(&mut self, id: BindingId) -> bool {
        use BindingId as B;
        let len = self.len();
        let (anchor, cursor) = match id {
            B::TextualInputCursorLeftSelect => (self.anchor(), self.cursor.saturating_sub(1)),
            B::TextualInputCursorRightSelect => (self.anchor(), (self.cursor + 1).min(len)),
            B::TextualInputCursorLeftWordSelect => (self.anchor(), self.word_start_before()),
            B::TextualInputCursorRightWordSelect => (self.anchor(), self.word_start_after().unwrap_or(len)),
            B::TextualInputHomeSelect => (self.cursor, 0),
            B::TextualInputEndSelect => (self.cursor, len),
            B::TextualInputSelectAll => (0, len),
            _ => return false,
        };
        self.anchor = Some(anchor);
        self.cursor = cursor;
        true
    }

    /// A bracketed paste: only its first line goes in, as Textual's `Input._on_paste` does with `splitlines()[0]`.
    pub fn handle_paste(&mut self, text: &str) {
        let line = text.split(is_line_break).next().unwrap_or_default();
        self.replace_selection(line);
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    /// A left press at `column` in an input drawn in `area`: the cursor goes to the char under it, or the nearest end,
    /// any selection ends, and a drag starts there (Textual's `_on_mouse_down`, which runs after the focus that
    /// selected everything). The owner keeps the mouse for the input until [`Self::release`].
    pub fn press(&mut self, area: Rect, column: u16) {
        self.cursor = self.index_at(area, column);
        self.anchor = Some(self.cursor);
        self.selecting = Some(area);
    }

    /// The pointer moved to `column` with the button still down: the cursor follows it and the anchor stays where the
    /// press was (Textual's `_on_mouse_move`). The pointer may be outside the input: left of the text is the start
    /// and right of it the end, and the view scrolls to the cursor when it's next drawn. Nothing happens unless a
    /// press on the input started a drag.
    pub fn drag(&mut self, column: u16) {
        if let Some(area) = self.selecting {
            self.cursor = self.index_at(area, column);
        }
    }

    /// The left button went up: the drag ends and the selection stays (Textual's `_end_selecting`).
    pub fn release(&mut self) {
        self.selecting = None;
    }

    /// Whether a press started a drag that hasn't been released: the owner keeps the mouse for the input until then
    /// and hands it [`Self::captured_mouse`].
    pub fn is_selecting(&self) -> bool {
        self.selecting.is_some()
    }

    /// A mouse event while the input has the mouse, wherever the pointer is: a left drag moves the cursor
    /// ([`Self::drag`]) and the left release ends the drag ([`Self::release`]). Returns whether it took the event,
    /// which it never does without a drag.
    pub fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        if !self.is_selecting() {
            return false;
        }
        match event.kind {
            MouseEventKind::Drag(MouseButton::Left) => self.drag(event.column),
            MouseEventKind::Up(MouseButton::Left) => self.release(),
            _ => return false,
        }
        true
    }

    /// The char under `column` in an input drawn in `area`, scrolled as last drawn (Textual's
    /// `_cell_offset_to_index`). Columns left of the text are 0, and columns right of it are the end.
    fn index_at(&self, area: Rect, column: u16) -> usize {
        let text_x = usize::from(area.x) + 1 + usize::from(PADDING);
        // Left of the text, which starts `scroll` cells before the text area, the offset would be negative.
        let Some(offset) = (usize::from(column) + self.scroll).checked_sub(text_x) else {
            return 0;
        };
        let mut left = 0;
        for (index, c) in self.value.chars().enumerate() {
            let right = left + char_width(c);
            if (left..right).contains(&offset) {
                return index;
            }
            left = right;
        }
        offset.min(self.len())
    }

    fn byte_at(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map_or(self.value.len(), |(i, _)| i)
    }

    /// Types `text` over the selection, or at the cursor when there's none (Textual's `replace` and `insert`).
    fn replace_selection(&mut self, text: &str) {
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.anchor = None;
        self.replace(start, end, text);
    }

    /// Deletes the chars in `start..end` and leaves the cursor at `start`.
    fn delete(&mut self, start: usize, end: usize) {
        self.replace(start, end, "");
    }

    /// Replaces the chars in `start..end` with `text` and leaves the cursor after it, unless the filter refuses the
    /// result, which changes nothing (Textual's `Input.replace`, which every edit goes through).
    fn replace(&mut self, start: usize, end: usize, text: &str) {
        let (from, to) = (self.byte_at(start), self.byte_at(end));
        let mut value = String::with_capacity(self.value.len() + text.len());
        value.push_str(&self.value[..from]);
        value.push_str(text);
        value.push_str(&self.value[to..]);
        if self.filter.is_some_and(|allowed| !allowed(&value)) {
            return;
        }
        self.value = value;
        self.cursor = start + text.chars().count();
    }

    /// Textual's `_WORD_START`, `(?<=\W)\w`, over the chars before the cursor: the last word start, or 0.
    fn word_start_before(&self) -> usize {
        let chars: Vec<char> = self.value.chars().take(self.cursor).collect();
        (1..chars.len())
            .rev()
            .find(|&i| !is_word(chars[i - 1]) && is_word(chars[i]))
            .unwrap_or(0)
    }

    /// The first word start after the cursor. Textual searches the text from the cursor on as a new string, so a
    /// word starting right at the cursor never counts.
    fn word_start_after(&self) -> Option<usize> {
        let chars: Vec<char> = self.value.chars().collect();
        (self.cursor + 1..chars.len()).find(|&i| !is_word(chars[i - 1]) && is_word(chars[i]))
    }

    /// Draws the input into the top [`HEIGHT`] rows of `area`. `under` is the parent's background, which shows in the
    /// half-cell `▊`/`▎` edges.
    ///
    /// The cursor is always drawn while focused. Textual blinks it every 0.5 s and restarts the blink on each key;
    /// that isn't reproduced yet.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let area = Rect {
            height: area.height.min(HEIGHT),
            ..area
        };
        if area.is_empty() {
            return;
        }
        let styles = Styles::new(theme, under, focused, self.invalid);
        let border = TallBorder {
            left: styles.left,
            inner: styles.inner,
            right: styles.right,
            fill: styles.text,
        };
        border.draw(buf, area);

        // The text row, between the border and the padding.
        if area.height < 2 || area.width <= 2 * (PADDING + 1) {
            return;
        }
        let text = Rect {
            x: area.x + 1 + PADDING,
            y: area.y + 1,
            width: area.width - 2 * (PADDING + 1),
            height: 1,
        };
        let width = usize::from(text.width);
        let mut cells: Vec<(char, Style)> = Vec::new();
        if self.value.is_empty() {
            self.scroll = 0;
            cells.extend(self.placeholder.chars().map(|c| (c, styles.placeholder)));
            if focused {
                match cells.first_mut() {
                    Some(first) => first.1 = styles.cursor_over(first.1),
                    None => cells.push((' ', styles.cursor_over(styles.text))),
                }
            }
        } else {
            cells.extend(self.value.chars().map(|c| (c, styles.text)));
            if focused {
                // `stylize_before`: the cursor's style goes over the selection's.
                if let Some((start, end)) = self.selection() {
                    for cell in &mut cells[start..end] {
                        cell.1 = styles.selection;
                    }
                }
                if self.cursor == cells.len() {
                    cells.push((' ', styles.text));
                }
                if let Some(cell) = cells.get_mut(self.cursor) {
                    cell.1 = styles.cursor_over(cell.1);
                }
            }
            // Scroll so the cursor's cell is in view, as Textual's `scroll_to_region` does on every move.
            let before: usize = cells.iter().take(self.cursor).map(|&(c, _)| char_width(c)).sum();
            let at = cells.get(self.cursor).map_or(1, |&(c, _)| char_width(c));
            if before < self.scroll {
                self.scroll = before;
            } else if before + at > self.scroll + width {
                self.scroll = before + at - width;
            }
        }
        let line: Line<'_> = cells
            .into_iter()
            .map(|(c, style)| Span::styled(c.to_string(), style))
            .collect();
        Paragraph::new(line)
            .style(styles.text)
            .scroll((0, u16::try_from(self.scroll).unwrap_or(u16::MAX)))
            .render(text, buf);
    }
}

/// The character a key types, when it's printable: no modifier but SHIFT (so not `ctrl+…`, `alt+…` or the kitty
/// protocol's `super+c`), and printable by Python's `str.isprintable`.
pub(crate) fn printable(key: &KeyChord) -> Option<char> {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Char(c) if key.mods.difference(KeyModifiers::SHIFT).is_empty() && is_printable(c) => Some(c),
        _ => None,
    }
}

pub(crate) fn is_word(c: char) -> bool {
    WORD.is_match(c.encode_utf8(&mut [0; 4]))
}

/// The line boundaries Python's `str.splitlines` splits on (`\r\n` splits at its `\r` first, which is enough for
/// taking the first line).
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn char_width(c: char) -> usize {
    Span::raw(c.encode_utf8(&mut [0; 4]) as &str).width()
}

/// The input's styles for one frame, from `Input.DEFAULT_CSS`.
struct Styles {
    /// The text and the padding: `$foreground` on `$surface`, tinted `$foreground 5%` while focused.
    text: Style,
    placeholder: Style,
    cursor: Style,
    /// `input--selection`: `$input-selection-foreground` on `$input-selection-background`.
    selection: Style,
    /// `▊` on the left: the parent's background over the border color, reversed (Textual's border location 2).
    left: Style,
    /// `▔`, `▁` and the blank between: the border color over the input's background (location 0).
    inner: Style,
    /// `▎` on the right: the border color over the parent's background (location 1).
    right: Style,
}

impl Styles {
    fn new(theme: &Theme, under: Paint, focused: bool, invalid: bool) -> Self {
        let p = theme.palette;
        let selection = |bg: Paint| {
            let selected = p.input_selection_background.over(bg);
            Style::new()
                .fg(theme.color(p.input_selection_foreground, selected))
                .bg(theme.paint(selected))
        };
        if theme.ansi {
            // `&:ansi`: the terminal's own colors.
            let default = theme.paint(Paint::Ansi(AnsiColor::Default));
            let text = Style::new().fg(default).bg(default);
            // `Input.-invalid:ansi`: `ansi_red`, focused or not.
            let border = if invalid {
                theme.paint(Paint::Ansi(AnsiColor::Red))
            } else {
                theme.color(if focused { p.border } else { p.border_blurred }, under)
            };
            return Self {
                text,
                placeholder: text.add_modifier(Modifier::DIM),
                cursor: Style::new()
                    .fg(theme.paint(Paint::Ansi(AnsiColor::Black)))
                    .bg(theme.paint(Paint::Ansi(AnsiColor::White))),
                selection: selection(Paint::Ansi(AnsiColor::Default)),
                left: Style::new().fg(border).bg(default).add_modifier(Modifier::REVERSED),
                inner: Style::new().fg(border).bg(default),
                right: Style::new().fg(border).bg(default),
            };
        }
        let surface = p.surface.over(under);
        let bg = if focused {
            theme.tint(surface, p.foreground, 5.0)
        } else {
            surface
        };
        let text = Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg));
        let border = match (invalid, focused) {
            // `Input.-invalid { border: tall $error 60% }`, and `$error` while focused. The translucent red is
            // blended over the parent's background, as Textual draws it.
            (true, true) => theme.color(p.error, under),
            (true, false) => theme.color(p.error.multiply_alpha(INVALID_BLURRED_ALPHA), under),
            (false, true) => theme.color(p.border, bg),
            (false, false) => theme.color(p.border_blurred, bg),
        };
        let cursor_bg = p.input_cursor_background.over(bg);
        let cursor = p.input_cursor_text_style.apply(
            Style::new()
                .fg(theme.color(p.input_cursor_foreground, cursor_bg))
                .bg(theme.paint(cursor_bg)),
        );
        Self {
            text,
            placeholder: text.fg(theme.color(p.text_disabled, bg)),
            cursor,
            selection: selection(bg),
            left: Style::new()
                .fg(border)
                .bg(theme.paint(under))
                .add_modifier(Modifier::REVERSED),
            inner: Style::new().fg(border).bg(theme.paint(bg)),
            right: Style::new().fg(border).bg(theme.paint(under)),
        }
    }

    /// The cursor drawn over a cell: Textual stylizes the cursor's cell with `input--cursor`, whose colors win.
    fn cursor_over(&self, cell: Style) -> Style {
        cell.patch(self.cursor)
    }
}

/// Textual's `tall` border, which the text input and the text editor draw: `▊▔▎` / `▊ ▎` / `▊▁▎`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TallBorder {
    /// `▊` on the left: the parent's background over the border color, reversed (Textual's border location 2).
    pub(crate) left: Style,
    /// `▔` and `▁`: the border color over the widget's background (location 0).
    pub(crate) inner: Style,
    /// `▎` on the right: the border color over the parent's background (location 1).
    pub(crate) right: Style,
    /// The inside.
    pub(crate) fill: Style,
}

impl TallBorder {
    /// Draws the border around `area`, the inside filled with spaces in [`Self::fill`]. The bottom edge is drawn on
    /// the last row once `area` is at least [`HEIGHT`] tall; a shorter area is cut off before it.
    pub(crate) fn draw(&self, buf: &mut Buffer, area: Rect) {
        let last_row = area.height - 1;
        for dy in 0..area.height {
            let y = area.y + dy;
            let fill = if dy == 0 {
                "▔"
            } else if dy == last_row && area.height >= HEIGHT {
                "▁"
            } else {
                " "
            };
            for dx in 0..area.width {
                let x = area.x + dx;
                let (symbol, style) = if dx == 0 {
                    ("▊", self.left)
                } else if dx == area.width - 1 {
                    ("▎", self.right)
                } else if fill == " " {
                    (" ", self.fill)
                } else {
                    (fill, self.inner)
                };
                buf[(x, y)].set_symbol(symbol).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::keymap::KeySpec;

    fn chord(key: &str) -> KeyChord {
        match KeySpec::parse(key) {
            KeySpec::Chord(chord) => chord,
            KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
        }
    }

    /// An input holding `value` with the cursor at char `cursor`.
    fn input(value: &str, cursor: usize) -> TextInput {
        let mut input = TextInput::new("Search...");
        input.set_value(value);
        input.cursor = cursor;
        input
    }

    fn press(input: &mut TextInput, key: &str) -> InputOutcome {
        input.handle_key(&chord(key), &Keymap::defaults(), "")
    }

    #[test]
    fn printable_keys_type_at_the_cursor() {
        let mut input = TextInput::new("Search...");
        for key in ["q", "j", "slash", "space", "J", "question_mark"] {
            assert_eq!(press(&mut input, key), InputOutcome::Changed);
        }
        assert_eq!(input.value(), "qj/ J?");
        assert_eq!(input.cursor(), 6);
        let mut middle = self::input("ac", 1);
        let _ = press(&mut middle, "b");
        assert_eq!((middle.value(), middle.cursor()), ("abc", 2));
    }

    #[test]
    fn non_text_keys_are_ignored() {
        let mut input = input("abc", 1);
        for key in ["tab", "shift+tab", "escape", "ctrl+p", "f1", "up", "down", "alt+x"] {
            assert_eq!(press(&mut input, key), InputOutcome::Ignored, "{key}");
        }
        assert_eq!((input.value(), input.cursor()), ("abc", 1));
        let keymap = Keymap::defaults();
        assert!(!input.consumes_key(&chord("tab"), &keymap));
        assert!(input.consumes_key(&chord("q"), &keymap));
        assert!(input.consumes_key(&chord("ctrl+f"), &keymap));
        assert!(input.consumes_key(&chord("enter"), &keymap));
    }

    /// Presses `key` with `clipboard` as the in-app clipboard.
    fn press_with(input: &mut TextInput, key: &str, clipboard: &str) -> InputOutcome {
        input.handle_key(&chord(key), &Keymap::defaults(), clipboard)
    }

    /// `foo-[bar] baz`, the cursor after the `r`.
    fn bar_selected() -> TextInput {
        let mut input = input("foo-bar baz", 4);
        press_all(&mut input, &["shift+right", "shift+right", "shift+right"]);
        assert_eq!(input.selected_text(), Some("bar"));
        input
    }

    #[test]
    fn cut_copies_and_deletes_the_selection() {
        let mut input = bar_selected();
        assert_eq!(press(&mut input, "ctrl+x"), InputOutcome::Copied("bar".into()));
        assert_eq!(
            (input.value(), input.cursor(), input.selection()),
            ("foo- baz", 4, None)
        );
    }

    #[test]
    fn cut_without_a_selection_copies_nothing_and_deletes_nothing() {
        let mut input = input("foo-bar baz", 4);
        assert!(input.consumes_key(&chord("ctrl+x"), &Keymap::defaults()));
        assert_eq!(press(&mut input, "ctrl+x"), InputOutcome::Copied(String::new()));
        assert_eq!((input.value(), input.cursor()), ("foo-bar baz", 4));
    }

    #[rstest]
    fn copy_copies_the_selection_and_keeps_it(#[values("ctrl+c", "super+c")] key: &str) {
        let mut input = bar_selected();
        assert!(input.consumes_key(&chord(key), &Keymap::defaults()));
        assert_eq!(press(&mut input, key), InputOutcome::Copied("bar".into()));
        assert_eq!((input.value(), input.selection()), ("foo-bar baz", Some((4, 7))));
    }

    #[rstest]
    fn copy_without_a_selection_isnt_the_inputs(#[values("ctrl+c", "super+c")] key: &str) {
        let mut input = input("foo-bar baz", 4);
        assert!(!input.consumes_key(&chord(key), &Keymap::defaults()));
        assert_eq!(press(&mut input, key), InputOutcome::Ignored);
        assert_eq!((input.value(), input.cursor()), ("foo-bar baz", 4));
    }

    #[test]
    fn paste_types_the_clipboard_over_the_selection() {
        let mut input = bar_selected();
        assert_eq!(press_with(&mut input, "ctrl+v", "lazy"), InputOutcome::Changed);
        assert_eq!(
            (input.value(), input.cursor(), input.selection()),
            ("foo-lazy baz", 8, None)
        );
        let _ = press_with(&mut input, "ctrl+v", "é");
        assert_eq!((input.value(), input.cursor()), ("foo-lazyé baz", 9));
    }

    #[test]
    fn paste_keeps_the_clipboards_line_breaks() {
        let mut input = input("xy", 1);
        let _ = press_with(&mut input, "ctrl+v", "a\nb");
        assert_eq!((input.value(), input.cursor()), ("xa\nby", 4));
    }

    #[test]
    fn a_filter_refuses_a_paste_that_cant_make_a_number() {
        let mut input = numeric("12");
        assert_eq!(press_with(&mut input, "ctrl+v", "3a"), InputOutcome::Changed);
        assert_eq!((input.value(), input.cursor()), ("12", 2));
        let _ = press_with(&mut input, "ctrl+v", "34");
        assert_eq!((input.value(), input.cursor()), ("1234", 4));
    }

    #[test]
    fn ctrl_v_pastes_the_empty_in_app_clipboard() {
        let keymap = Keymap::defaults();
        let mut plain = input("abc", 1);
        assert!(plain.consumes_key(&chord("ctrl+v"), &keymap));
        assert_eq!(press(&mut plain, "ctrl+v"), InputOutcome::Changed);
        assert_eq!((plain.value(), plain.cursor()), ("abc", 1));
        // Over a selection, the empty paste deletes it.
        let mut selected = input("abc", 1);
        selected.select_all();
        let _ = press(&mut selected, "ctrl+v");
        assert_eq!((selected.value(), selected.cursor()), ("", 0));
    }

    #[test]
    fn enter_submits_without_changing_the_text() {
        let mut input = input("abc", 1);
        assert_eq!(press(&mut input, "enter"), InputOutcome::Submitted);
        assert_eq!((input.value(), input.cursor()), ("abc", 1));
    }

    #[rstest]
    #[case::left("left", 3, 2)]
    #[case::left_at_start("left", 0, 0)]
    #[case::right("right", 3, 4)]
    #[case::right_at_end("right", 11, 11)]
    #[case::home("home", 5, 0)]
    #[case::ctrl_a("ctrl+a", 5, 0)]
    #[case::end("end", 5, 11)]
    #[case::ctrl_e("ctrl+e", 5, 11)]
    // `foo-bar baz`: words start at 0, 4 and 8.
    #[case::word_left_from_end("ctrl+left", 11, 8)]
    #[case::word_left_from_word_start("ctrl+left", 8, 4)]
    #[case::word_left_inside_word("ctrl+left", 6, 4)]
    #[case::word_left_to_start("ctrl+left", 4, 0)]
    #[case::word_left_at_start("ctrl+left", 0, 0)]
    #[case::word_right_from_start("ctrl+right", 0, 4)]
    #[case::word_right_from_dash("ctrl+right", 3, 4)]
    #[case::word_right_from_word_start("ctrl+right", 4, 8)]
    #[case::word_right_to_end("ctrl+right", 8, 11)]
    fn cursor_keys_move(#[case] key: &str, #[case] from: usize, #[case] to: usize) {
        let mut input = input("foo-bar baz", from);
        assert_eq!(press(&mut input, key), InputOutcome::Changed);
        assert_eq!(input.cursor(), to);
        assert_eq!(input.value(), "foo-bar baz");
    }

    #[rstest]
    #[case::backspace("backspace", 4, "foobar baz", 3)]
    #[case::backspace_at_start("backspace", 0, "foo-bar baz", 0)]
    #[case::delete("delete", 3, "foobar baz", 3)]
    #[case::ctrl_d("ctrl+d", 3, "foobar baz", 3)]
    #[case::delete_at_end("delete", 11, "foo-bar baz", 11)]
    #[case::word_left("ctrl+w", 11, "foo-bar ", 8)]
    #[case::word_left_inside_word("ctrl+w", 6, "foo-r baz", 4)]
    #[case::word_left_from_word_start("ctrl+w", 8, "foo-baz", 4)]
    #[case::word_left_to_start("ctrl+w", 3, "-bar baz", 0)]
    #[case::word_right("ctrl+f", 0, "bar baz", 0)]
    #[case::word_right_inside_word("ctrl+f", 5, "foo-bbaz", 5)]
    #[case::word_right_from_word_start("ctrl+f", 4, "foo-baz", 4)]
    #[case::word_right_without_next_word("ctrl+f", 9, "foo-bar b", 9)]
    #[case::all_left("ctrl+u", 4, "bar baz", 0)]
    #[case::all_right("ctrl+k", 4, "foo-", 4)]
    fn delete_keys_delete(#[case] key: &str, #[case] from: usize, #[case] value: &str, #[case] cursor: usize) {
        let mut input = input("foo-bar baz", from);
        assert_eq!(press(&mut input, key), InputOutcome::Changed);
        assert_eq!((input.value(), input.cursor()), (value, cursor));
    }

    #[test]
    fn editing_counts_chars_not_bytes() {
        let mut input = input("héllo wörld", 11);
        let _ = press(&mut input, "ctrl+w");
        assert_eq!((input.value(), input.cursor()), ("héllo ", 6));
        let _ = press(&mut input, "left");
        let _ = press(&mut input, "backspace");
        assert_eq!((input.value(), input.cursor()), ("héll ", 4));
    }

    #[rstest]
    #[case::one_line("lazy", "xlazyy")]
    #[case::multi_line("lazy\ngithub\nmore", "xlazyy")]
    #[case::crlf("lazy\r\ngithub", "xlazyy")]
    #[case::leading_newline("\nlazy", "xy")]
    #[case::unicode_line_separator("lazy\u{2028}github", "xlazyy")]
    fn paste_inserts_only_the_first_line(#[case] pasted: &str, #[case] value: &str) {
        let mut input = input("xy", 1);
        input.handle_paste(pasted);
        assert_eq!(input.value(), value);
        assert_eq!(input.cursor(), value.chars().count() - 1);
    }

    /// `foo-bar baz` with everything selected, as focusing the box leaves it.
    fn selected() -> TextInput {
        let mut input = input("foo-bar baz", 3);
        input.select_all();
        assert_eq!(input.selection(), Some((0, 11)));
        assert_eq!(input.cursor(), 11);
        input
    }

    #[test]
    fn typing_and_pasting_replace_the_selection() {
        let mut input = selected();
        let _ = press(&mut input, "x");
        assert_eq!((input.value(), input.cursor(), input.selection()), ("x", 1, None));
        let mut input = selected();
        input.handle_paste("lazy\ngit");
        assert_eq!((input.value(), input.cursor(), input.selection()), ("lazy", 4, None));
    }

    #[rstest]
    fn delete_keys_delete_the_selection(
        #[values("backspace", "delete", "ctrl+d", "ctrl+w", "ctrl+f", "ctrl+u", "ctrl+k")] key: &str,
    ) {
        let mut input = selected();
        assert_eq!(press(&mut input, key), InputOutcome::Changed);
        assert_eq!((input.value(), input.cursor(), input.selection()), ("", 0, None));
    }

    #[rstest]
    // left and right collapse the selection to its start or end.
    #[case::left("left", 0)]
    #[case::right("right", 11)]
    // home and end just set the cursor.
    #[case::home("home", 0)]
    #[case::end("end", 11)]
    // Word moves start from the cursor, the selection's end.
    #[case::word_left("ctrl+left", 8)]
    #[case::word_right("ctrl+right", 11)]
    fn moves_end_the_selection(#[case] key: &str, #[case] cursor: usize) {
        let mut input = selected();
        assert_eq!(press(&mut input, key), InputOutcome::Changed);
        assert_eq!(
            (input.value(), input.cursor(), input.selection()),
            ("foo-bar baz", cursor, None)
        );
        let _ = press(&mut input, "x");
        assert_eq!(
            input.value().chars().count(),
            12,
            "typing inserts once the selection is gone"
        );
    }

    #[test]
    fn enter_keeps_the_selection() {
        let mut input = selected();
        assert_eq!(press(&mut input, "enter"), InputOutcome::Submitted);
        assert_eq!(input.selection(), Some((0, 11)));
    }

    #[test]
    fn a_press_places_the_cursor_and_ends_the_selection() {
        // The text starts 3 cells in: the left border and the padding.
        let area = Rect::new(10, 0, 30, HEIGHT);
        for (column, cursor) in [(13, 0), (16, 3), (11, 0), (35, 11)] {
            let mut input = selected();
            input.press(area, column);
            assert_eq!((input.cursor(), input.selection()), (cursor, None), "column {column}");
        }
    }

    /// Presses each key in `keys`, all of which the input takes.
    fn press_all(input: &mut TextInput, keys: &[&str]) {
        for key in keys {
            assert_eq!(press(input, key), InputOutcome::Changed, "{key}");
        }
    }

    // `foo-bar baz`: words start at 0, 4 and 8. Each case is the keys, the cursor before them, and the anchor and
    // cursor after: Textual's `Selection(start, end)`.
    #[rstest]
    #[case::shift_left(&["shift+left"], 5, (5, 4))]
    #[case::shift_left_repeated(&["shift+left", "shift+left", "shift+left"], 5, (5, 2))]
    #[case::shift_left_at_start(&["shift+left"], 0, (0, 0))]
    #[case::shift_left_past_start(&["shift+left", "shift+left"], 1, (1, 0))]
    #[case::shift_right(&["shift+right"], 5, (5, 6))]
    #[case::shift_right_repeated(&["shift+right", "shift+right", "shift+right"], 5, (5, 8))]
    #[case::shift_right_at_end(&["shift+right"], 11, (11, 11))]
    #[case::shift_right_past_end(&["shift+right", "shift+right"], 10, (10, 11))]
    #[case::shift_right_back_across_the_anchor(&["shift+left", "shift+right", "shift+right"], 5, (5, 6))]
    #[case::word_left_select(&["ctrl+shift+left", "ctrl+shift+left"], 11, (11, 4))]
    #[case::word_left_select_from_inside(&["ctrl+shift+left", "ctrl+shift+left"], 6, (6, 0))]
    #[case::word_right_select(&["ctrl+shift+right", "ctrl+shift+right"], 0, (0, 8))]
    #[case::word_right_select_to_end(&["ctrl+shift+right", "ctrl+shift+right"], 6, (6, 11))]
    #[case::word_moves_select_both_ways(&["ctrl+shift+left", "ctrl+shift+right"], 6, (6, 8))]
    // `shift+home` and `shift+end` select from the cursor, whatever the anchor was.
    #[case::shift_home(&["shift+home"], 5, (5, 0))]
    #[case::shift_home_reanchors(&["shift+left", "shift+home"], 5, (4, 0))]
    #[case::shift_end(&["shift+end"], 5, (5, 11))]
    #[case::shift_end_reanchors(&["shift+right", "shift+end"], 5, (6, 11))]
    #[case::shift_end_after_shift_left(&["shift+left", "shift+end"], 5, (4, 11))]
    #[case::select_all(&["ctrl+shift+a"], 5, (0, 11))]
    #[case::select_all_then_shift_left(&["ctrl+shift+a", "shift+left"], 5, (0, 10))]
    fn select_keys_move_the_cursor_and_keep_the_anchor(
        #[case] keys: &[&str],
        #[case] from: usize,
        #[case] selection: (usize, usize),
    ) {
        let mut input = input("foo-bar baz", from);
        press_all(&mut input, keys);
        assert_eq!((input.anchor(), input.cursor()), selection);
        assert_eq!(input.value(), "foo-bar baz");
    }

    #[rstest]
    fn keys_after_a_select_key_act_on_its_selection(
        #[values(
            &["shift+left", "shift+left"][..],
            &["shift+right", "shift+right"][..],
            &["ctrl+shift+left"][..],
            &["ctrl+shift+right"][..],
            &["shift+left", "shift+home"][..],
            &["shift+right", "shift+end"][..],
            &["ctrl+shift+a"][..],
        )]
        keys: &[&str],
    ) {
        let selected = || {
            let mut input = input("foo-bar baz", 6);
            press_all(&mut input, keys);
            input
        };
        let (start, end) = selected().selection().expect("the keys select something");
        let mut left = selected();
        let _ = press(&mut left, "left");
        assert_eq!((left.cursor(), left.selection()), (start, None), "left");
        let mut right = selected();
        let _ = press(&mut right, "right");
        assert_eq!((right.cursor(), right.selection()), (end, None), "right");
        let mut typed = selected();
        let _ = press(&mut typed, "x");
        let chars: Vec<char> = "foo-bar baz".chars().collect();
        let expected: String = chars[..start].iter().chain(&['x']).chain(&chars[end..]).collect();
        assert_eq!(
            (typed.value(), typed.cursor()),
            (expected.as_str(), start + 1),
            "typing"
        );
    }

    #[test]
    fn select_keys_count_chars_not_bytes() {
        let mut input = input("héllo wörld", 11);
        press_all(&mut input, &["ctrl+shift+left", "shift+left"]);
        assert_eq!(input.selection(), Some((5, 11)));
        let _ = press(&mut input, "x");
        assert_eq!((input.value(), input.cursor()), ("héllox", 6));
        press_all(&mut input, &["shift+home", "shift+right", "shift+right"]);
        assert_eq!((input.anchor(), input.cursor()), (6, 2));
        let _ = press(&mut input, "y");
        assert_eq!((input.value(), input.cursor()), ("héy", 3));
    }

    /// `foo-bar baz` in a 30-wide input at x = 10, so the text starts at column 13.
    const AREA: Rect = Rect::new(10, 0, 30, HEIGHT);

    #[rstest]
    #[case::right(17, 20, (4, 7))]
    #[case::left(17, 14, (4, 1))]
    #[case::onto_the_press(17, 17, (4, 4))]
    // Past either end of the text: the border, beyond the input, the end of the text, the right border.
    #[case::past_the_start(17, 11, (4, 0))]
    #[case::left_of_the_input(17, 2, (4, 0))]
    #[case::past_the_end(17, 30, (4, 11))]
    #[case::right_of_the_input(17, 60, (4, 11))]
    fn a_drag_selects_from_the_press(#[case] at: u16, #[case] to: u16, #[case] selection: (usize, usize)) {
        let mut input = selected();
        input.press(AREA, at);
        assert!(input.is_selecting());
        input.drag(to);
        assert_eq!((input.anchor(), input.cursor()), selection);
        input.release();
        assert!(!input.is_selecting());
        assert_eq!(
            (input.anchor(), input.cursor()),
            selection,
            "the release keeps the selection"
        );
    }

    #[test]
    fn a_drag_in_a_scrolled_view_counts_the_scroll() {
        let mut input = input("abcdefghijklmnopqrstuvwxyz0123456789", 30);
        input.scroll = 10;
        // The first text cell shows `k`.
        input.press(AREA, 13);
        assert_eq!(input.cursor(), 10);
        // Left of the text area, the scrolled-off text is still there: two cells left is `i`.
        input.drag(11);
        assert_eq!((input.anchor(), input.cursor()), (10, 8));
        input.drag(0);
        assert_eq!(input.cursor(), 0);
        input.drag(20);
        assert_eq!((input.anchor(), input.cursor()), (10, 17));
        input.drag(70);
        assert_eq!(input.cursor(), 36);
    }

    #[test]
    fn a_press_without_a_drag_selects_nothing() {
        let mut input = selected();
        input.press(AREA, 17);
        input.release();
        assert_eq!((input.cursor(), input.selection()), (4, None));
        // Without a press, a drag does nothing.
        input.drag(20);
        assert_eq!((input.cursor(), input.selection()), (4, None));
        // A select key extends from the press.
        let _ = press(&mut input, "shift+right");
        assert_eq!(input.selection(), Some((4, 5)));
    }

    #[test]
    fn the_captured_mouse_drags_and_releases() {
        use crossterm::event::KeyModifiers;
        let event = |kind, column| MouseEvent {
            kind,
            column,
            row: 40,
            modifiers: KeyModifiers::NONE,
        };
        let mut input = selected();
        assert!(!input.captured_mouse(event(MouseEventKind::Drag(MouseButton::Left), 20)));
        input.press(AREA, 17);
        assert!(input.captured_mouse(event(MouseEventKind::Drag(MouseButton::Left), 20)));
        assert!(!input.captured_mouse(event(MouseEventKind::ScrollDown, 20)));
        assert!(input.captured_mouse(event(MouseEventKind::Up(MouseButton::Left), 25)));
        assert_eq!((input.selection(), input.is_selecting()), (Some((4, 7)), false));
    }

    /// An input that only holds what a number input may.
    fn numeric(value: &str) -> TextInput {
        let mut input = TextInput::new("Pull request number").with_filter(crate::modal::number::can_become_number);
        input.set_value(value);
        input
    }

    #[test]
    fn a_filter_refuses_typed_keys_that_cant_make_a_number() {
        let mut input = numeric("");
        for key in ["a", "b", "1", "full_stop", "x", "5", "e", "space", "minus", "2"] {
            // Refused or not, a printable key is the input's.
            assert_eq!(press(&mut input, key), InputOutcome::Changed, "{key}");
        }
        assert_eq!((input.value(), input.cursor()), ("1.5e-2", 6));
        // A second point is refused wherever it goes.
        let _ = press(&mut input, "home");
        let _ = press(&mut input, "full_stop");
        assert_eq!((input.value(), input.cursor()), ("1.5e-2", 0));
    }

    #[test]
    fn a_filter_refuses_pastes_that_cant_make_a_number() {
        let mut input = numeric("12");
        input.handle_paste("3a");
        assert_eq!((input.value(), input.cursor()), ("12", 2));
        input.handle_paste("34\nabc");
        assert_eq!((input.value(), input.cursor()), ("1234", 4));
        // Over a selection: the result is what's checked.
        input.select_all();
        input.handle_paste("-9");
        assert_eq!(input.value(), "-9");
    }

    #[test]
    fn a_filter_refuses_deletes_that_leave_no_number() {
        // `1_2` without its 1 would start with `_`, which Textual refuses too.
        let mut input = numeric("1_2");
        input.cursor = 1;
        let _ = press(&mut input, "backspace");
        assert_eq!((input.value(), input.cursor()), ("1_2", 1));
        let _ = press(&mut input, "delete");
        assert_eq!((input.value(), input.cursor()), ("12", 1));
    }

    #[test]
    fn word_chars_follow_python() {
        // Letters and numbers of every script, and the underscore.
        for c in ['a', 'é', '中', '٣', '½', '_'] {
            assert!(is_word(c), "{c:?}");
        }
        // Not combining marks, even the `Alphabetic` ones Rust's `is_alphanumeric` takes (a Devanagari vowel sign).
        for c in ['-', ' ', '\u{93f}', '\u{345}', '\u{301}'] {
            assert!(!is_word(c), "{c:?}");
        }
        // So `x` after the vowel sign starts a word, as in Python.
        let mut input = input("किx", 3);
        let _ = press(&mut input, "ctrl+left");
        assert_eq!(input.cursor(), 2);
    }
}
