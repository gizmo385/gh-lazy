//! Add Comment (`diff.add_comment` on a hunk pane): the diff line being commented on, shown in a read-only input, a
//! comment editor, then Add Comment and Cancel. It sends nothing: it closes with the comment's text, which the diff
//! viewer keeps as a pending comment until the review is submitted. Python's `AddCommentModal`.
//!
//! The dialog is 80 wide and as tall as its content, in a `thick $accent` border on `$surface`. When the screen is too
//! short for it, the stack clips the frame and the interior scrolls (without a bar) to keep the focused field in
//! view, where Python scrolls the modal screen.

use std::ops::Range;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::form::FormScroll;
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, ButtonVariant, Pressed};
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::text_editor::{EditorOutcome, EditorVariant, TextEditor};
use crate::components::text_input::{self, TextInput};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Palette, ThemeColor};
use crate::toast::Toast;

/// The dialog's screen bindings: `modal.submit`, then `modal.close`.
const ADD_COMMENT_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// `width: 80`, border included.
const WIDTH: u16 = 80;

/// The container's `padding: 1`.
const PADDING: u16 = 1;

/// The editor's `height: 10`.
const EDITOR_HEIGHT: u16 = 10;

/// Where each part goes, in rows from the top of the padded content. Each label has `margin: 1 0`, collapsing with its
/// neighbours' margins; the input and the editor have `margin: 0 0 1 0`.
const TITLE_ROW: u16 = 1;
const COMMENTING_ROW: u16 = 3;
const LINE_ROWS: Range<u16> = 5..5 + text_input::HEIGHT;
const COMMENT_ROW: u16 = LINE_ROWS.end + 1;
const EDITOR_ROWS: Range<u16> = COMMENT_ROW + 2..COMMENT_ROW + 2 + EDITOR_HEIGHT;
const BUTTON_ROWS: Range<u16> = EDITOR_ROWS.end + 1..EDITOR_ROWS.end + 1 + ButtonRow::FLUSH_HEIGHT;

/// The interior's rows: the content and its padding above and below.
const INTERIOR_HEIGHT: u16 = BUTTON_ROWS.end + 2 * PADDING;

/// The text a whitespace-only comment toasts.
const EMPTY_COMMENT: &str = "Comment cannot be empty!";

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddCommentAction {
    /// `modal.submit` or Add Comment: close with the comment, unless it's blank.
    Submit,
    /// `modal.close` or Cancel: close with nothing.
    Close,
}

/// The dialog's focusable fields, in screen order. The line's input is never focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddField {
    Editor,
    Add,
    Cancel,
}

impl AddField {
    fn button(self) -> Option<Pressed> {
        match self {
            AddField::Add => Some(Pressed::Submit),
            AddField::Cancel => Some(Pressed::Cancel),
            AddField::Editor => None,
        }
    }

    /// Its rows, from the interior's top.
    fn rows(self) -> Range<u16> {
        let rows = match self {
            AddField::Editor => EDITOR_ROWS,
            AddField::Add | AddField::Cancel => BUTTON_ROWS,
        };
        rows.start + PADDING..rows.end + PADDING
    }
}

/// Add Comment for one diff line. Its output is the comment as typed, or `None` when it was closed.
#[derive(Debug)]
pub struct AddCommentDialog {
    /// The line, read-only.
    line: TextInput,
    ring: FocusRing<AddField>,
    editor: TextEditor,
    buttons: ButtonRow,
    scroll: FormScroll,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<String>>,
}

impl AddCommentDialog {
    /// The dialog for the line whose diff text (its `+`, `-` or space marker first, trimmed) is `line_text`, with an
    /// empty editor focused.
    pub fn new(line_text: impl Into<String>) -> Self {
        let mut line = TextInput::new("");
        line.set_value(line_text.into());
        Self {
            line,
            ring: FocusRing::new([AddField::Editor, AddField::Add, AddField::Cancel]),
            editor: TextEditor::new(EditorVariant::Plain, ""),
            buttons: ButtonRow::spaced("Add Comment").with_cancel_variant(ButtonVariant::Default),
            scroll: FormScroll::without_scrollbar(),
            output: None,
        }
    }

    /// The line shown.
    pub fn line_text(&self) -> &str {
        self.line.value()
    }

    /// The comment as typed.
    pub fn text(&self) -> String {
        self.editor.text()
    }

    fn action(action: AddCommentAction) -> Action {
        Action::Dialog(DialogAction::AddComment(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => AddCommentAction::Submit,
            Pressed::Cancel => AddCommentAction::Close,
        })
    }

    /// Moves focus with `f`: a field that gains it is scrolled into view, and the editor starts a new undo batch.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<AddField>)) {
        let before = self.ring.focused();
        f(&mut self.ring);
        let Some(field) = self.ring.focused().filter(|&field| Some(field) != before) else {
            return;
        };
        if field == AddField::Editor {
            self.editor.focus_gained();
        }
        self.scroll.reveal(field.rows());
    }

    /// Python's `_submit_comment`: a comment that's only whitespace is refused with a toast, and the dialog stays.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        let text = self.editor.text();
        if text.trim().is_empty() {
            cx.notify(Toast::warning(EMPTY_COMMENT));
            return;
        }
        self.output = Some(Some(text));
    }

    /// A wheel notch at `(column, row)`: the editor scrolls if it's under the pointer and can, else the interior does.
    fn wheel(&mut self, wheel: Wheel, column: u16, row: u16) {
        let rows = match wheel {
            Wheel::Up => -WHEEL_ROWS.cast_signed(),
            Wheel::Down => WHEEL_ROWS.cast_signed(),
            Wheel::Left | Wheel::Right => return,
        };
        let over_editor = self.scroll.shows_at(AddField::Editor.rows(), column, row);
        if !(over_editor && self.editor.scroll(rows)) {
            self.scroll.wheel(wheel);
        }
    }
}

impl Component for AddCommentDialog {
    type Action = AddCommentAction;

    fn bindings(&self) -> &'static [BindingId] {
        ADD_COMMENT_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(AddCommentAction::Submit)),
            BindingId::ModalClose => Some(Self::action(AddCommentAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.ring.is_focused(AddField::Editor) && self.editor.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        if !self.ring.is_focused(AddField::Editor) {
            return None;
        }
        match self.editor.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
            EditorOutcome::Copied { text, .. } => Some(Action::Copy(text)),
            EditorOutcome::Changed | EditorOutcome::Moved | EditorOutcome::Ignored => Some(Action::Consumed),
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // The fields are drawn offscreen and copied with the scroll offset: their mouse coordinates are the form's.
        let moved = self.scroll.to_content(event);
        if self.editor.captured_mouse(moved) {
            return Some(Action::Consumed);
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&moved, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    let field = match button {
                        Pressed::Submit => AddField::Add,
                        Pressed::Cancel => AddField::Cancel,
                    };
                    self.refocus(|ring| {
                        ring.focus(field);
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.wheel(wheel, event.column, event.row);
        } else if event.kind == MouseEventKind::Down(MouseButton::Left)
            && self.scroll.shows_at(AddField::Editor.rows(), event.column, event.row)
        {
            self.refocus(|ring| {
                ring.focus(AddField::Editor);
            });
            self.editor.press(moved.column, moved.row);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: AddCommentAction, cx: &mut Cx<'_>) {
        match action {
            AddCommentAction::Submit => self.submit(cx),
            AddCommentAction::Close => self.output = Some(None),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        let text = Style::new().fg(theme.color(theme.palette.foreground, under));
        let fill = text.bg(theme.paint(under));
        // `Label { color: $text; text-style: bold }`.
        let label = Style::new()
            .fg(theme.color(theme.palette.text, under))
            .add_modifier(Modifier::BOLD);
        let focused = self.ring.focused();
        let (line, editor, buttons) = (&mut self.line, &mut self.editor, &mut self.buttons);
        self.scroll
            .render(frame.buffer_mut(), area, theme, fill, INTERIOR_HEIGHT, |buf, form| {
                let content = Rect {
                    x: form.x + PADDING,
                    y: form.y + PADDING,
                    width: form.width.saturating_sub(2 * PADDING),
                    height: form.height.saturating_sub(2 * PADDING),
                };
                let rect = |rows: Range<u16>| Rect {
                    y: content.y + rows.start,
                    height: rows.end - rows.start,
                    ..content
                };
                let width = usize::from(content.width);
                for (row, text) in [
                    (TITLE_ROW, "Add Comment"),
                    (COMMENTING_ROW, "Commenting on:"),
                    (COMMENT_ROW, "Comment:"),
                ] {
                    buf.set_stringn(content.x, content.y + row, text, width, label);
                }
                // Python's input is disabled and can't take focus; Textual draws it as an unfocused one.
                line.render(buf, rect(LINE_ROWS), theme, under, false);
                editor.render(buf, rect(EDITOR_ROWS), theme, under, focused == Some(AddField::Editor));
                buttons.render(buf, rect(BUTTON_ROWS), theme, focused.and_then(AddField::button));
            });
    }
}

impl Dialog for AddCommentDialog {
    type Output = Option<String>;

    fn kind(&self) -> ModalKind {
        ModalKind::AddComment
    }

    fn own_action(&self, action: DialogAction) -> Option<AddCommentAction> {
        match action {
            DialogAction::AddComment(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.refocus(|ring| ring.step(direction));
    }

    /// 80 wide, as tall as the content with its padding and the border.
    fn size(&self, _available: Rect) -> (u16, u16) {
        (WIDTH, INTERIOR_HEIGHT + 2)
    }

    /// `background: $surface`.
    fn background(&self, palette: &Palette) -> ThemeColor {
        palette.surface
    }

    /// `border: thick $accent`.
    fn border(&self, palette: &Palette) -> ThemeColor {
        palette.accent
    }

    /// Python's modal has no footer of its own: the screen's shows under the dim.
    fn shows_footer(&self) -> bool {
        false
    }

    fn take_output(&mut self) -> Option<Option<String>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(AddField::Editor) => FieldKind::TextEditor,
            Some(AddField::Add | AddField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(AddField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            // `consumes_key` already gave the editor every key it binds.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.editor.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        self.ring.is_focused(AddField::Editor).then(|| {
            let _ = self.editor.handle_paste(text, cx.now());
            Action::Consumed
        })
    }
}

#[cfg(test)]
mod tests;
