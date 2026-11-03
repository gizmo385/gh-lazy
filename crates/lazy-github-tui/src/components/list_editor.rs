//! An editable list of strings in a `solid` box: one line per item, then a dim "+ Add item..." line. The arrows move
//! a highlight; `enter` opens the highlighted item for editing in place (or starts a new one on the add line), `a`
//! adds, `d` or `delete` removes, and `shift+up` / `shift+down` move an item. The Settings list fields use it (#343,
//! decision settings-list-editor); nothing in it is settings-specific.
//!
//! Like the other lists, the editor doesn't know whether it has focus: its owner puts [`LIST_EDITOR_BINDINGS`] in its
//! chain while it has and isn't editing, and [`LIST_EDITOR_INPUT_BINDINGS`] while its input is open.

use std::ops::Range;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use super::command_log::{WHEEL_ROWS, Wheel};
use super::option_nav;
use super::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use super::selection_list::draw_solid_box;
use super::table::py_strip;
use super::text_input::{InputOutcome, TextInput};
use crate::display;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

/// The editor's bindings while it has focus and no input is open: `OptionList`'s moves (hidden), then edit, add,
/// remove and the two moves, in the footer's order.
pub const LIST_EDITOR_BINDINGS: &[BindingId] = &[
    BindingId::TextualOptionListCursorDown,
    BindingId::TextualOptionListLast,
    BindingId::TextualOptionListFirst,
    BindingId::TextualOptionListPageDown,
    BindingId::TextualOptionListPageUp,
    BindingId::TextualOptionListCursorUp,
    BindingId::SettingsListEdit,
    BindingId::SettingsListAdd,
    BindingId::SettingsListRemove,
    BindingId::SettingsListMoveUp,
    BindingId::SettingsListMoveDown,
];

/// The editor's bindings while its input is open: commit and cancel, then the input's own
/// ([`super::text_input::INPUT_BINDINGS`]) but its submit, which commit replaces. `escape` cancels the edit rather than
/// closing what the editor is in.
pub const LIST_EDITOR_INPUT_BINDINGS: &[BindingId] = &[
    BindingId::SettingsListCommit,
    BindingId::SettingsListCancel,
    BindingId::TextualScrollUp,
    BindingId::TextualScrollDown,
    BindingId::TextualInputCursorLeft,
    BindingId::TextualInputCursorRight,
    BindingId::TextualInputHome,
    BindingId::TextualInputEnd,
    BindingId::TextualScrollPageUp,
    BindingId::TextualScrollPageDown,
    BindingId::TextualScrollPageLeft,
    BindingId::TextualScrollPageRight,
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
    BindingId::TextualInputDeleteLeftWord,
    BindingId::TextualInputDeleteLeftAll,
    BindingId::TextualInputDeleteRightWord,
    BindingId::TextualInputDeleteRightAll,
    BindingId::TextualInputCut,
    BindingId::TextualInputCopy,
    BindingId::TextualInputPaste,
];

/// Commit and cancel, which the open input leaves to the chain.
const EDIT_KEYS: &[BindingId] = &[BindingId::SettingsListCommit, BindingId::SettingsListCancel];

/// The box's widest: an `Input`'s `width: 70`.
pub const MAX_WIDTH: u16 = 70;

/// The most lines in view before the box scrolls: Python's checklist's `max-height: 10`, less its border.
pub const MAX_LINES: usize = 8;

/// The box's border, each side.
const BORDER: u16 = 1;

/// The blank cell between the border and the lines, each side (`OptionList`'s `padding: 0 1`).
const PADDING: u16 = 1;

/// The last line's text.
const ADD_LINE: &str = "+ Add item...";

/// What the open input is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    /// Item `index`, edited in place.
    Item(usize),
    /// A new item, on the add line. `escape` puts the highlight back on line `from`, where adding started.
    New { from: usize },
}

/// The open input and what it's for.
#[derive(Debug, Clone)]
struct Edit {
    target: EditTarget,
    input: TextInput,
}

/// How an open input closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// `enter`: refused text keeps the input open, and an added item opens a fresh add input.
    Enter,
    /// Focus left, or the owner is saving: refused text is dropped and the input closes either way.
    Blur,
}

/// What a key, a binding, a paste or a press did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum ListEditorOutcome {
    /// The highlight or the scroll may have moved; the items are as they were.
    Moved,
    /// The items changed.
    Changed,
    /// The input opened, or took the key.
    Editing,
    /// The input closed with nothing changed.
    Cancelled,
    /// `text` is listed already, so it wasn't taken. The owner tells the user, naming the list.
    Refused {
        /// The stripped text.
        text: String,
    },
    /// The input's copy or cut: the owner puts the text on the clipboard.
    Copied(String),
    /// Not the editor's.
    Ignored,
}

/// The editor's state.
#[derive(Debug, Clone)]
pub struct ListEditor {
    items: Vec<String>,
    /// The highlighted line, from 0 to `items.len()`, which is the add line.
    highlighted: usize,
    /// The first line in view.
    scroll: usize,
    edit: Option<Edit>,
    /// The add input's placeholder.
    placeholder: &'static str,
    /// Where the lines were drawn last, in the coordinates of the buffer they went to.
    lines_area: Rect,
    /// Where the box was drawn last.
    box_area: Rect,
}

impl ListEditor {
    /// An editor over `items`, with the first line highlighted; the add input shows `placeholder`.
    pub fn new(items: Vec<String>, placeholder: &'static str) -> Self {
        Self {
            items,
            highlighted: 0,
            scroll: 0,
            edit: None,
            placeholder,
            lines_area: Rect::default(),
            box_area: Rect::default(),
        }
    }

    /// The items, in the order shown.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The highlighted line: an item's index, or `items().len()` for the add line.
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// Whether an input is open.
    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    /// The open input's text, while there is one.
    pub fn input_value(&self) -> Option<&str> {
        self.edit.as_ref().map(|edit| edit.input.value())
    }

    /// The lines: every item and the add line.
    fn lines(&self) -> usize {
        self.items.len() + 1
    }

    /// The lines in view.
    fn view(&self) -> usize {
        self.lines().min(MAX_LINES)
    }

    /// The rows the editor takes, its box included: a line per item and the add line, at most [`MAX_LINES`].
    pub fn height(&self) -> u16 {
        // At most MAX_LINES, so it fits.
        u16::try_from(self.view()).unwrap_or(u16::MAX) + 2 * BORDER
    }

    /// The highlighted line's row (the open input's while editing), counted from the editor's top.
    pub fn highlighted_rows(&self) -> Range<u16> {
        let row = BORDER + u16::try_from(self.highlighted.saturating_sub(self.scroll)).unwrap_or(u16::MAX);
        row..row + 1
    }

    /// Highlights line `line`, clamped to the add line, and scrolls it into view.
    fn highlight(&mut self, line: usize) {
        self.highlighted = line.min(self.items.len());
        self.reveal();
    }

    /// Scrolls the highlighted line into view, and keeps the scroll within the lines.
    fn reveal(&mut self) {
        let view = self.view();
        self.scroll = option_nav::scroll_into_view(self.scroll, self.highlighted, 1, view).min(self.lines() - view);
    }

    /// A binding of [`LIST_EDITOR_BINDINGS`] while browsing, or of [`EDIT_KEYS`] while the input is open.
    pub fn on_binding(&mut self, id: BindingId) -> ListEditorOutcome {
        use BindingId as B;
        if self.edit.is_some() {
            return match id {
                B::SettingsListCommit => self.finish(Finish::Enter),
                B::SettingsListCancel => {
                    if let Some(Edit {
                        target: EditTarget::New { from },
                        ..
                    }) = self.edit.take()
                    {
                        self.highlight(from);
                    }
                    ListEditorOutcome::Cancelled
                }
                _ => ListEditorOutcome::Ignored,
            };
        }
        let last = self.items.len();
        let at = self.highlighted;
        match id {
            B::TextualOptionListCursorDown => self.highlight(at + 1),
            B::TextualOptionListCursorUp => self.highlight(at.saturating_sub(1)),
            B::TextualOptionListFirst => self.highlight(0),
            B::TextualOptionListLast => self.highlight(last),
            B::TextualOptionListPageDown => self.highlight(option_nav::page_down(at, self.lines(), 1, self.view())),
            B::TextualOptionListPageUp => self.highlight(option_nav::page_up(at, 1, self.view())),
            B::SettingsListEdit if at == last => return self.start_add(""),
            B::SettingsListEdit => return self.start_edit(at),
            B::SettingsListAdd => return self.start_add(""),
            B::SettingsListRemove if at < last => {
                self.items.remove(at);
                self.highlight(at);
                return ListEditorOutcome::Changed;
            }
            B::SettingsListMoveUp if at < last && at > 0 => {
                self.items.swap(at, at - 1);
                self.highlight(at - 1);
                return ListEditorOutcome::Changed;
            }
            B::SettingsListMoveDown if at + 1 < last => {
                self.items.swap(at, at + 1);
                self.highlight(at + 1);
                return ListEditorOutcome::Changed;
            }
            // On the add line, or at an end: nothing to do, but the key is the editor's.
            B::SettingsListRemove | B::SettingsListMoveUp | B::SettingsListMoveDown => {}
            _ => return ListEditorOutcome::Ignored,
        }
        ListEditorOutcome::Moved
    }

    /// Opens item `index` for editing, its text selected so typing replaces it.
    fn start_edit(&mut self, index: usize) -> ListEditorOutcome {
        let Some(item) = self.items.get(index) else {
            return ListEditorOutcome::Ignored;
        };
        let mut input = TextInput::new("").borderless();
        input.set_value(item.clone());
        input.select_all();
        self.edit = Some(Edit {
            target: EditTarget::Item(index),
            input,
        });
        self.highlight(index);
        ListEditorOutcome::Editing
    }

    /// Turns the add line into an input holding `text`, and highlights it.
    fn start_add(&mut self, text: &str) -> ListEditorOutcome {
        let mut input = TextInput::new(self.placeholder).borderless();
        if !text.is_empty() {
            input.handle_paste(text);
        }
        self.edit = Some(Edit {
            target: EditTarget::New { from: self.highlighted },
            input,
        });
        self.highlight(self.items.len());
        ListEditorOutcome::Editing
    }

    /// Closes the open input by `finish`, taking its stripped text: blank removes an edited item or adds nothing;
    /// text equal to another item is refused; anything else replaces the item or is appended.
    fn finish(&mut self, finish: Finish) -> ListEditorOutcome {
        let Some(edit) = &self.edit else {
            return ListEditorOutcome::Ignored;
        };
        let text = py_strip(edit.input.value()).to_owned();
        let target = edit.target;
        let listed = |skip: Option<usize>| {
            self.items
                .iter()
                .enumerate()
                .any(|(index, item)| Some(index) != skip && *item == text)
        };
        let refused = match target {
            EditTarget::Item(index) => !text.is_empty() && listed(Some(index)),
            EditTarget::New { .. } => !text.is_empty() && listed(None),
        };
        if refused {
            if finish == Finish::Blur {
                self.edit = None;
            }
            return ListEditorOutcome::Refused { text };
        }
        self.edit = None;
        match target {
            EditTarget::Item(index) if text.is_empty() => {
                self.items.remove(index);
                self.highlight(index);
            }
            EditTarget::Item(index) => self.items[index] = text,
            EditTarget::New { .. } if text.is_empty() => return ListEditorOutcome::Cancelled,
            EditTarget::New { .. } => {
                self.items.push(text);
                if finish == Finish::Enter {
                    let _ = self.start_add("");
                } else {
                    self.highlight(self.items.len());
                }
            }
        }
        ListEditorOutcome::Changed
    }

    /// Focus is leaving, or the owner is saving: the open input commits as `enter` does, but no fresh add input
    /// opens, and refused text is dropped (still [`ListEditorOutcome::Refused`], so the owner can say so).
    pub fn commit(&mut self) -> ListEditorOutcome {
        self.finish(Finish::Blur)
    }

    /// Whether the open input takes `key` before any binding: what it types and its own keys, but not commit and
    /// cancel, which go through the chain. Nothing while browsing.
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|edit| keymap.match_in(EDIT_KEYS, key).is_none() && edit.input.consumes_key(key, keymap))
    }

    /// A key [`Self::consumes_key`] claimed, for the open input; `clipboard` is what its paste key types.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str) -> ListEditorOutcome {
        let Some(edit) = &mut self.edit else {
            return ListEditorOutcome::Ignored;
        };
        match edit.input.handle_key(key, keymap, clipboard) {
            InputOutcome::Changed => ListEditorOutcome::Editing,
            InputOutcome::Submitted => self.finish(Finish::Enter),
            InputOutcome::Copied(text) => ListEditorOutcome::Copied(text),
            InputOutcome::Ignored => ListEditorOutcome::Ignored,
        }
    }

    /// A bracketed paste. The open input takes its first line. While browsing, text with several non-blank lines
    /// adds each stripped line that isn't listed yet, at the end; one line starts adding with it in the input.
    pub fn handle_paste(&mut self, text: &str) -> ListEditorOutcome {
        if let Some(edit) = &mut self.edit {
            edit.input.handle_paste(text);
            return ListEditorOutcome::Editing;
        }
        let lines: Vec<&str> = text.lines().map(py_strip).filter(|line| !line.is_empty()).collect();
        match lines.as_slice() {
            [] => ListEditorOutcome::Ignored,
            [line] => self.start_add(line),
            lines => {
                let before = self.items.len();
                for line in lines {
                    if !self.items.iter().any(|item| item == line) {
                        self.items.push((*line).to_owned());
                    }
                }
                if self.items.len() == before {
                    return ListEditorOutcome::Moved;
                }
                // The add line moved down with the items: keep it highlighted if it was.
                if self.highlighted == before {
                    self.highlight(self.items.len());
                }
                ListEditorOutcome::Changed
            }
        }
    }

    /// A left press at `event`, in the coordinates the editor was drawn in. `focused` says whether the editor had
    /// focus before the press. On an item it highlights it, or opens it when it was highlighted already and focused;
    /// on the add line it starts adding. While editing, a press in the input places its cursor, and a press on
    /// another line commits (as on blur) and highlights that line.
    pub fn handle_mouse(&mut self, event: &MouseEvent, focused: bool) -> ListEditorOutcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            || !self.lines_area.contains(Position::new(event.column, event.row))
        {
            return ListEditorOutcome::Ignored;
        }
        let line = self.scroll + usize::from(event.row - self.lines_area.y);
        if line > self.items.len() {
            return ListEditorOutcome::Ignored;
        }
        if let Some(edit) = &mut self.edit {
            if line == self.highlighted {
                let row = Rect {
                    y: event.row,
                    height: 1,
                    ..self.lines_area
                };
                edit.input.press(row, event.column);
                return ListEditorOutcome::Editing;
            }
            let outcome = self.finish(Finish::Blur);
            self.highlight(line);
            return match outcome {
                ListEditorOutcome::Refused { .. } | ListEditorOutcome::Changed => outcome,
                _ => ListEditorOutcome::Moved,
            };
        }
        if line == self.items.len() {
            self.start_add("")
        } else if focused && line == self.highlighted {
            self.start_edit(line)
        } else {
            self.highlight(line);
            ListEditorOutcome::Moved
        }
    }

    /// Whether `column`, `row` (in the coordinates the editor was drawn in) is on its box.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        self.box_area.contains(Position::new(column, row))
    }

    /// A wheel notch over the box: it scrolls, and the highlight stays where it is.
    pub fn wheel(&mut self, wheel: Wheel) {
        let max = self.lines() - self.view();
        self.scroll = match wheel {
            Wheel::Up => self.scroll.saturating_sub(WHEEL_ROWS),
            Wheel::Down => self.scroll + WHEEL_ROWS,
            Wheel::Left | Wheel::Right => self.scroll,
        }
        .min(max);
    }

    /// Whether a press in the open input started a drag that hasn't been released.
    pub fn is_selecting(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| edit.input.is_selecting())
    }

    /// A mouse event while the open input has the mouse ([`TextInput::captured_mouse`]).
    pub fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        self.edit.as_mut().is_some_and(|edit| edit.input.captured_mouse(event))
    }

    /// Draws the editor into the top of `area` over `under`: its box, [`MAX_WIDTH`] wide at most and
    /// [`Self::height`] tall, with a scrollbar inside when the lines overflow. `focused` draws the highlight (a
    /// focused `OptionList`'s) and the open input's cursor; without focus there's no highlight.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let boxed = Rect {
            width: area.width.min(MAX_WIDTH),
            height: area.height.min(self.height()),
            ..area
        }
        .intersection(buf.area);
        self.box_area = boxed;
        self.lines_area = Rect::default();
        if boxed.width <= 2 * (BORDER + PADDING) || boxed.height <= 2 * BORDER {
            return;
        }
        let r = theme.roles();
        let surface = r.surface.pane.over(under);
        let bg = if focused {
            theme.tint(surface, r.surface.focus_tint, 100.0)
        } else {
            surface
        };
        let plain = Style::new().fg(theme.color(r.text.plain, bg)).bg(theme.paint(bg));
        draw_solid_box(buf, boxed, plain.fg(theme.color(r.brand.primary, bg)), plain);

        let view = usize::from(boxed.height - 2 * BORDER);
        let overflows = self.lines() > view;
        let inner_width = boxed.width - 2 * (BORDER + PADDING);
        let lines_area = Rect {
            x: boxed.x + BORDER + PADDING,
            y: boxed.y + BORDER,
            width: if overflows {
                inner_width.saturating_sub(scrollbar::VERTICAL_WIDTH)
            } else {
                inner_width
            },
            height: boxed.height - 2 * BORDER,
        };
        self.lines_area = lines_area;
        let cursor = r.cursor.focused;
        let cursor_bg = cursor.bg.over(bg);
        let highlight = cursor.style.apply(
            Style::new()
                .fg(theme.color(cursor.fg, cursor_bg))
                .bg(theme.paint(cursor_bg)),
        );
        let width = usize::from(lines_area.width);
        for (dy, line) in (0..lines_area.height).zip(self.scroll..) {
            let y = lines_area.y + dy;
            let row = Rect {
                y,
                height: 1,
                ..lines_area
            };
            if let Some(edit) = self.edit.as_mut().filter(|_| line == self.highlighted) {
                edit.input.render(buf, row, theme, bg, focused);
                continue;
            }
            let highlighted = focused && line == self.highlighted;
            let (text, style) = match self.items.get(line) {
                Some(item) => (display::sanitize(item), if highlighted { highlight } else { plain }),
                None if line == self.items.len() => {
                    let style = if highlighted {
                        highlight
                    } else {
                        theme.rich_dim(plain, r.text.plain, bg)
                    };
                    (ADD_LINE.into(), style)
                }
                None => break,
            };
            for x in row.left()..row.right() {
                buf[(x, y)].set_symbol(" ").set_style(style);
            }
            buf.set_stringn(row.x, y, display::truncate(&text, width), width, style);
        }
        // Against the right border, outside the padding, as Textual places a scrollbar.
        if overflows && inner_width > scrollbar::VERTICAL_WIDTH {
            let bar = Rect {
                x: boxed.right() - BORDER - scrollbar::VERTICAL_WIDTH,
                width: scrollbar::VERTICAL_WIDTH,
                ..lines_area
            };
            let back = r.scrollbar.track.over(bg);
            let colors = ScrollbarColors {
                bar: theme.color(r.scrollbar.thumb, back),
                back: theme.paint(back),
                fg: theme.color(r.text.plain, bg),
                bg: theme.paint(bg),
            };
            let state = ScrollbarState {
                content: self.lines(),
                window: view,
                position: self.scroll,
            };
            scrollbar::render(buf, bar, Axis::Vertical, state, colors);
        }
    }
}

#[cfg(test)]
mod tests;
