//! The Confirm dialog: a question with Yes and No buttons. Python's `ConfirmDialog`, opened by Create PR to ask
//! whether to push a branch the remote may not have.

use std::borrow::Cow;

use crossterm::event::MouseEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{self, Button, ButtonMouse, ButtonVariant};
use crate::components::markdown::{self, TextRun};
use crate::keymap::BindingId;

/// The frame's outer size, border included (`width: 50; height: 11`).
const SIZE: (u16, u16) = (50, 11);

/// The `#dialog` grid's `padding: 0 1`: a column inside the border on each side.
const PADDING: u16 = 1;

/// The grid's `grid-gutter: 1 2`: a row between the question and the buttons, two columns between the buttons' cells.
const ROW_GUTTER: u16 = 1;
const COLUMN_GUTTER: u16 = 2;

/// Confirm's screen bindings, which also give the footer's order.
const CONFIRM_BINDINGS: &[BindingId] = dialog_bindings![ConfirmYes, ConfirmNo, ModalClose];

/// Confirm's own action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Yes (`true`) or no (`false`): the dialog is done with that answer.
    Answer(bool),
}

/// Confirm's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmField {
    Yes,
    No,
}

/// A yes/no question. Its output is `true` for Yes (`confirm.yes` or the Yes button) and `false` for No
/// (`confirm.no`, `modal.close` or the No button).
#[derive(Debug)]
pub struct ConfirmDialog {
    question: Line<'static>,
    yes: Button,
    no: Button,
    ring: FocusRing<ConfirmField>,
    answer: Option<bool>,
}

impl ConfirmDialog {
    /// Asks `question`, with "Yes" and "No" buttons and Yes focused. Spans without a foreground color take the
    /// dialog's text color.
    pub fn new(question: impl Into<Line<'static>>) -> Self {
        Self {
            question: question.into(),
            yes: Button::new("Yes", ButtonVariant::Primary),
            no: Button::new("No", ButtonVariant::Error),
            ring: FocusRing::new([ConfirmField::Yes, ConfirmField::No]),
            answer: None,
        }
    }

    /// This dialog with its buttons labeled `yes` and `no` (Python's `yes_text` and `no_text`).
    #[must_use]
    pub fn with_labels(mut self, yes: impl Into<Cow<'static, str>>, no: impl Into<Cow<'static, str>>) -> Self {
        self.yes.set_label(yes);
        self.no.set_label(no);
        self
    }

    fn answer(yes: bool) -> Action {
        Action::Dialog(DialogAction::Confirm(ConfirmAction::Answer(yes)))
    }

    fn button_mut(&mut self, field: ConfirmField) -> &mut Button {
        match field {
            ConfirmField::Yes => &mut self.yes,
            ConfirmField::No => &mut self.no,
        }
    }
}

impl Component for ConfirmDialog {
    type Action = ConfirmAction;

    fn bindings(&self) -> &'static [BindingId] {
        CONFIRM_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ConfirmYes => Some(Self::answer(true)),
            BindingId::ConfirmNo | BindingId::ModalClose => Some(Self::answer(false)),
            _ => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        for field in [ConfirmField::Yes, ConfirmField::No] {
            match self.button_mut(field).handle_mouse(&event, cx.now()) {
                ButtonMouse::Missed => {}
                ButtonMouse::Focus => {
                    self.ring.focus(field);
                    return Some(Action::Consumed);
                }
                ButtonMouse::Pressed => return Some(Self::answer(field == ConfirmField::Yes)),
            }
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: ConfirmAction, _cx: &mut Cx<'_>) {
        match action {
            ConfirmAction::Answer(yes) => self.answer = Some(yes),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        let text = Style::new().fg(theme.color(theme.palette.foreground, under));
        let content = Rect {
            x: area.x.saturating_add(PADDING),
            width: area.width.saturating_sub(2 * PADDING),
            ..area
        };
        // `grid-rows: 1fr 3`: the buttons take the last rows, and the question the rest above the gutter.
        let question_rows = content.height.saturating_sub(button::HEIGHT + ROW_GUTTER);

        let runs: Vec<TextRun> = self
            .question
            .spans
            .iter()
            .map(|span| TextRun::new(span.content.as_ref(), text.patch(self.question.style).patch(span.style)))
            .collect();
        let mut lines = markdown::wrap_text(&runs, content.width).lines;
        lines.truncate(usize::from(question_rows));
        // `content-align: center middle`, as a block: every line starts where the longest one does.
        let longest = lines.iter().map(Line::width).max().unwrap_or(0);
        let longest = u16::try_from(longest).unwrap_or(u16::MAX);
        let left = content.x + content.width.saturating_sub(longest) / 2;
        // At most `question_rows` lines, which is a u16.
        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let top = content.y + question_rows.saturating_sub(height) / 2;
        let buf = frame.buffer_mut();
        for (line, y) in lines.iter().zip(top..) {
            let row = Rect::new(left, y, content.right().saturating_sub(left), 1).intersection(area);
            buf.set_line(row.x, row.y, line, row.width);
        }

        // Two equal columns with the gutter between; each button fills its column (`Button { width: 100% }`).
        let column = content.width.saturating_sub(COLUMN_GUTTER) / 2;
        let y = content.y + question_rows + ROW_GUTTER;
        let focused = self.ring.focused();
        for (field, x) in [
            (ConfirmField::Yes, content.x),
            (ConfirmField::No, content.x + column + COLUMN_GUTTER),
        ] {
            let button = self.button_mut(field);
            let button_area = Rect::new(x, y, column, button::HEIGHT).intersection(content);
            button.render(buf, button_area, theme, focused == Some(field));
        }
    }
}

impl Dialog for ConfirmDialog {
    type Output = bool;

    fn kind(&self) -> ModalKind {
        ModalKind::Confirm
    }

    fn own_action(&self, action: DialogAction) -> Option<ConfirmAction> {
        match action {
            DialogAction::Confirm(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.ring.step(direction);
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn take_output(&mut self) -> Option<bool> {
        self.answer.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        self.ring
            .focused()
            .map(|_| FieldKind::Button.link(self.kind()))
            .into_iter()
            .collect()
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        if (field, id) != (FieldKind::Button, BindingId::TextualButtonPress) {
            return None;
        }
        let focused = self.ring.focused()?;
        // A press inside the debounce still takes the key.
        Some(if self.button_mut(focused).press(cx.now()) {
            Self::answer(focused == ConfirmField::Yes)
        } else {
            Action::Consumed
        })
    }
}

#[cfg(test)]
mod tests;
