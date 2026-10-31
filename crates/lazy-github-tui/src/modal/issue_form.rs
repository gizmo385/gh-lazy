//! The fields New issue and Edit issue share, and everything they do with them: a scroll area holding a "Title"
//! input, Edit issue's "Status" select and a "Description" code editor, then a flush Save / Cancel row. Python's
//! `NewIssueContainer` and `EditIssueContainer`, which differ only in the Status select, the editor's height and what
//! Save does.
//!
//! The dialogs own the request and the result; the form owns focus, keys, the mouse, pastes and the drawing. Its
//! methods mirror the [`super::Dialog`] and [`crate::component::Component`] methods the dialogs forward to it.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::models::IssueState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::form::{Block, FormLayout, FormScroll, split};
use super::{FieldKind, FocusMove, FocusRing, ModalKind};
use crate::action::Action;
use crate::component::{ChainLink, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::info::Scroll;
use crate::components::select::{self, DEFAULT_PROMPT, Select, SelectOutcome};
use crate::components::text_editor::{EditorOutcome, EditorVariant, TextEditor};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::Paint;

/// The form's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IssueField {
    /// The scroll area itself, which Textual's `ScrollableContainer` makes focusable. It has focus when the dialog
    /// opens.
    Scroll,
    Title,
    /// Edit issue's Status select.
    Status,
    Description,
    Save,
    Cancel,
}

impl IssueField {
    fn button(self) -> Option<Pressed> {
        match self {
            IssueField::Save => Some(Pressed::Submit),
            IssueField::Cancel => Some(Pressed::Cancel),
            IssueField::Scroll | IssueField::Title | IssueField::Status | IssueField::Description => None,
        }
    }
}

/// How tall the description editor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditorHeight {
    /// This share of the scroll area's height, rounded down (New issue's `height: 75%`).
    Percent(u16),
    /// This many rows (Edit issue's `height: 15`).
    Rows(u16),
}

/// What Save reads while a request is in flight (Py bug B69's recommendation).
const SAVING: &str = "Saving...";
const SAVE: &str = "Save";

/// The shared form. See the module docs.
#[derive(Debug)]
pub(super) struct IssueForm {
    kind: ModalKind,
    /// What a counted press of Save or Cancel does: the dialog's submit or close.
    pressed: fn(Pressed) -> Action,
    ring: FocusRing<IssueField>,
    title: TextInput,
    status: Option<Select<IssueState>>,
    body: TextEditor,
    buttons: ButtonRow,
    scroll: FormScroll,
    editor_height: EditorHeight,
}

impl IssueForm {
    /// A form for dialog `kind` with `title` and `body` filled in and no Status select, the scroll area focused.
    pub(super) fn new(
        kind: ModalKind,
        pressed: fn(Pressed) -> Action,
        title: &str,
        body: &str,
        editor_height: EditorHeight,
    ) -> Self {
        let mut input = TextInput::new("Title");
        input.set_value(title);
        Self {
            kind,
            pressed,
            ring: FocusRing::new([
                IssueField::Scroll,
                IssueField::Title,
                IssueField::Description,
                IssueField::Save,
                IssueField::Cancel,
            ]),
            title: input,
            status: None,
            body: TextEditor::new(EditorVariant::Code, body),
            buttons: ButtonRow::flush(SAVE),
            scroll: FormScroll::default(),
            editor_height,
        }
    }

    /// The same form with a Status select after the title, set to `state`: "Open" and "Closed" after the blank row.
    #[must_use]
    pub(super) fn with_status(mut self, state: IssueState) -> Self {
        self.status = Some(Select::new(
            DEFAULT_PROMPT,
            [("Open", IssueState::Open), ("Closed", IssueState::Closed)],
            Some(&state),
        ));
        self.ring.set_order([
            IssueField::Scroll,
            IssueField::Title,
            IssueField::Status,
            IssueField::Description,
            IssueField::Save,
            IssueField::Cancel,
        ]);
        self.ring.focus(IssueField::Scroll);
        self
    }

    /// The title as typed.
    pub(super) fn title(&self) -> &str {
        self.title.value()
    }

    /// Replaces the title, for tests that drive a dialog without keys.
    #[cfg(test)]
    pub(super) fn set_title(&mut self, title: &str) {
        self.title.set_value(title);
    }

    /// The description as typed.
    pub(super) fn body(&self) -> String {
        self.body.text()
    }

    /// The Status select's value: `None` for the blank row, or without a select.
    pub(super) fn status(&self) -> Option<IssueState> {
        self.status.as_ref().and_then(Select::value).copied()
    }

    /// The focused field.
    #[cfg(test)]
    pub(super) fn focused(&self) -> Option<IssueField> {
        self.ring.focused()
    }

    /// Whether Status's list is open.
    pub(super) fn is_status_open(&self) -> bool {
        self.status.as_ref().is_some_and(Select::is_open)
    }

    /// Save, as the dialog's request goes out (`busy`) and comes back: "Saving..." and disabled, then "Save" again.
    /// A disabled Save loses focus, as Textual blurs a disabled widget.
    pub(super) fn set_saving(&mut self, busy: bool) {
        let save = self.buttons.button_mut(Pressed::Submit);
        save.set_label(if busy { SAVING } else { SAVE });
        save.set_disabled(busy);
        self.ring.set_disabled(IssueField::Save, busy);
    }

    /// Save's label and whether it's disabled.
    #[cfg(test)]
    pub(super) fn save_state(&self) -> (&str, bool) {
        let save = self.buttons.button(Pressed::Submit);
        (save.label(), save.disabled())
    }

    /// The form at a scroll area `height` rows tall.
    fn layout(&self, height: u16) -> FormLayout<IssueField> {
        let editor = match self.editor_height {
            EditorHeight::Percent(percent) => height * percent / 100,
            EditorHeight::Rows(rows) => rows,
        };
        let status = self.status.is_some().then_some([
            Block::Rule,
            Block::Label("Status"),
            Block::Field(IssueField::Status, select::HEIGHT),
        ]);
        FormLayout::new(
            [
                Block::Label("Title"),
                Block::Field(IssueField::Title, text_input::HEIGHT),
            ]
            .into_iter()
            .chain(status.into_iter().flatten())
            .chain([
                Block::Rule,
                Block::Label("Description"),
                Block::Field(IssueField::Description, editor),
            ]),
        )
    }

    /// The form as of the last draw.
    fn last_layout(&self) -> FormLayout<IssueField> {
        self.layout(self.scroll.area().height)
    }

    /// Moves focus with `f`. Status's list closes when focus leaves it. A field that gains focus is scrolled into
    /// view; the title selects its text (Textual's `select_on_focus`, so typing replaces it) and the editor starts a
    /// new undo batch.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<IssueField>)) {
        let before = self.ring.focused();
        f(&mut self.ring);
        let Some(field) = self.ring.focused().filter(|&field| Some(field) != before) else {
            return;
        };
        if let Some(status) = &mut self.status {
            status.close();
        }
        match field {
            IssueField::Title => self.title.select_all(),
            IssueField::Description => self.body.focus_gained(),
            IssueField::Scroll | IssueField::Status | IssueField::Save | IssueField::Cancel => {}
        }
        if let Some(rows) = self.last_layout().rows(field) {
            self.scroll.reveal(rows);
        }
    }

    fn focus(&mut self, field: IssueField) {
        self.refocus(|ring| {
            ring.focus(field);
        });
    }

    /// `tab` or `shift+tab`: an open list closes (it loses focus), then focus moves on from the select.
    pub(super) fn move_focus(&mut self, direction: FocusMove) {
        if let Some(status) = &mut self.status {
            status.close();
        }
        self.refocus(|ring| ring.step(direction));
    }

    /// The focused field's link.
    pub(super) fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(IssueField::Scroll) => FieldKind::Scroll,
            Some(IssueField::Title) => FieldKind::TextInput,
            Some(IssueField::Status) if self.is_status_open() => FieldKind::SelectOverlay,
            Some(IssueField::Status) => FieldKind::Select,
            Some(IssueField::Description) => FieldKind::TextEditor,
            Some(IssueField::Save | IssueField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind)]
    }

    pub(super) fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match (self.ring.focused(), &self.status) {
            (Some(IssueField::Title), _) => self.title.consumes_key(key, keymap),
            (Some(IssueField::Description), _) => self.body.consumes_key(key, keymap),
            (Some(IssueField::Status), Some(status)) => status.consumes_key(key, keymap),
            _ => false,
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        match (self.ring.focused(), &mut self.status) {
            (Some(IssueField::Title), _) => match self.title.handle_key(&chord, cx.keymap(), cx.clipboard()) {
                // `enter` in the title doesn't submit, as in Python.
                InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
                InputOutcome::Copied(text) => Some(Action::Copy(text)),
                InputOutcome::Ignored => None,
            },
            (Some(IssueField::Description), _) => {
                match self.body.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
                    EditorOutcome::Copied { text, .. } => Some(Action::Copy(text)),
                    EditorOutcome::Changed | EditorOutcome::Moved | EditorOutcome::Ignored => Some(Action::Consumed),
                }
            }
            (Some(IssueField::Status), Some(status)) => match status.handle_key(&chord, cx.keymap(), cx.now()) {
                SelectOutcome::Ignored => None,
                _ => Some(Action::Consumed),
            },
            _ => None,
        }
    }

    pub(super) fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(IssueField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, self.pressed),
                )
            }
            (FieldKind::Scroll, id) => {
                let scroll = Scroll::from_binding(id)?;
                self.scroll.scroll(scroll);
                Some(Action::Consumed)
            }
            (FieldKind::Select | FieldKind::SelectOverlay, id) => {
                let outcome = self.status.as_mut()?.on_binding(id);
                (outcome != SelectOutcome::Ignored).then_some(Action::Consumed)
            }
            // `consumes_key` already gave the text fields every key they bind.
            _ => Some(Action::Consumed),
        }
    }

    pub(super) fn handle_mouse(&mut self, event: MouseEvent, cx: &Cx<'_>) -> Action {
        // A drag that started in a text field, wherever the pointer is now.
        let moved = self.scroll.to_content(event);
        if self.title.captured_mouse(moved) || self.body.captured_mouse(moved) {
            return Action::Consumed;
        }
        let press = event.kind == MouseEventKind::Down(MouseButton::Left);
        let on_box = self.field_at(event.column, event.row) == Some(IssueField::Status);
        if let Some(status) = &mut self.status
            && status.is_open()
        {
            if status.overlay_contains(event.column, event.row) {
                let _ = status.handle_overlay_mouse(&event);
                return Action::Consumed;
            }
            // A press elsewhere takes the focus from the list, which closes it; a press on the box toggles it.
            if press && !on_box {
                status.close();
            }
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            return match outcome {
                ButtonMouse::Focus => {
                    self.focus(match button {
                        Pressed::Submit => IssueField::Save,
                        Pressed::Cancel => IssueField::Cancel,
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => (self.pressed)(button),
                ButtonMouse::Missed => Action::Consumed,
            };
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.wheel(wheel, event.column, event.row);
        } else if press {
            self.press(event);
        }
        Action::Consumed
    }

    /// The field under screen cell `(column, row)` in the scroll area, if any.
    fn field_at(&self, column: u16, row: u16) -> Option<IssueField> {
        let form_row = self.scroll.content_row(row)?;
        (column < self.scroll.content().right())
            .then(|| self.last_layout().field_at(form_row))
            .flatten()
    }

    /// A wheel notch at `(column, row)`: the editor scrolls if it's under the pointer and can, else the form does,
    /// as Textual passes the wheel on from a widget that can't scroll.
    fn wheel(&mut self, wheel: Wheel, column: u16, row: u16) {
        let rows = match wheel {
            Wheel::Up => -WHEEL_ROWS.cast_signed(),
            Wheel::Down => WHEEL_ROWS.cast_signed(),
            Wheel::Left | Wheel::Right => return,
        };
        let over_editor = self
            .last_layout()
            .rows(IssueField::Description)
            .is_some_and(|editor| self.scroll.shows_at(editor, column, row));
        if !(over_editor && self.body.scroll(rows)) {
            self.scroll.wheel(wheel);
        }
    }

    /// A left press in the scroll area: the field under it takes focus (and the press, for the text fields; a click
    /// on Status's box opens or closes its list), or the area itself does.
    fn press(&mut self, event: MouseEvent) {
        let Some(form_row) = self.scroll.content_row(event.row) else {
            return;
        };
        let layout = self.last_layout();
        let content = self.scroll.content();
        match self.field_at(event.column, event.row) {
            Some(IssueField::Title) => {
                self.focus(IssueField::Title);
                self.title.press(layout.area(IssueField::Title, content), event.column);
            }
            Some(IssueField::Status) => {
                self.focus(IssueField::Status);
                if let Some(status) = &mut self.status {
                    let _ = status.toggle();
                }
            }
            Some(IssueField::Description) => {
                self.focus(IssueField::Description);
                self.body.press(event.column, form_row);
            }
            _ => self.focus(IssueField::Scroll),
        }
    }

    pub(super) fn captures_mouse(&self) -> bool {
        self.title.is_selecting() || self.body.is_selecting()
    }

    /// Whether screen cell `(column, row)` is on the open status list.
    pub(super) fn popup_contains(&self, column: u16, row: u16) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.overlay_contains(column, row))
    }

    pub(super) fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        match self.ring.focused() {
            Some(IssueField::Title) => self.title.handle_paste(text),
            Some(IssueField::Description) => {
                let _ = self.body.handle_paste(text, cx.now());
            }
            _ => return None,
        }
        Some(Action::Consumed)
    }

    /// Draws the form into the dialog's interior `area`, over its background `under`, and Status's open list last,
    /// over everything below it.
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>, under: Paint) {
        let theme = cx.theme;
        let fill = Style::new()
            .fg(theme.color(theme.palette.foreground, under))
            .bg(theme.paint(under));
        let (scroll_area, buttons_area) = split(area, self.buttons.height());
        let layout = self.layout(scroll_area.height);
        let screen = frame.area();
        let (title, status, body, ring) = (&mut self.title, &self.status, &mut self.body, &self.ring);
        let buf = frame.buffer_mut();
        self.scroll.render(
            buf,
            scroll_area,
            theme,
            fill,
            layout.height(),
            |content_buf, content| {
                layout.render_static(content_buf, content, theme, under);
                title.render(
                    content_buf,
                    layout.area(IssueField::Title, content),
                    theme,
                    under,
                    ring.is_focused(IssueField::Title),
                );
                if let Some(status) = status {
                    // The box loses its focus to the open list.
                    let focused = ring.is_focused(IssueField::Status) && !status.is_open();
                    status.render_box(
                        content_buf,
                        layout.area(IssueField::Status, content),
                        theme,
                        under,
                        focused,
                    );
                }
                body.render(
                    content_buf,
                    layout.area(IssueField::Description, content),
                    theme,
                    under,
                    ring.is_focused(IssueField::Description),
                );
            },
        );
        let focused = self.ring.focused().and_then(IssueField::button);
        self.buttons.render(buf, buttons_area, theme, focused);
        if let (Some(status), Some(rows)) = (&mut self.status, layout.rows(IssueField::Status))
            && let Some(below) = self.scroll.screen_rows(rows)
        {
            status.render_overlay(buf, below, screen, theme, under);
        }
    }
}
