//! The compose sheet New issue and Edit issue share (decision compose-dialogs): a heading, a one-row `Title` field and
//! Edit issue's one-row `Status` select, then the description under its `Description  Write  Preview` tabs. Python's
//! `NewIssueContainer` and `EditIssueContainer`, whose scroll area, labels above tall fields, fixed-height editor
//! and Save / Cancel row the sheet replaces.
//!
//! The dialogs own the request and the result; the form owns focus, keys, the mouse, pastes, the sheet's size and the
//! drawing. Its methods mirror the [`super::Dialog`] and [`crate::component::Component`] methods the dialogs forward
//! to it.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::models::IssueState;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use super::form::FormScroll;
use super::{FieldKind, FocusMove, ModalKind};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Cx, RenderCx};
use crate::components::command_log::Wheel;
use crate::components::compose::{
    BodyMode, BodyMouse, ComposeAction, ComposeBody, ComposeLayout, ComposeParts, LabelState, SheetGeometry,
    SheetPlacement, field_area, label_column, render_heading, render_label, sheet_geometry,
};
use crate::components::markdown::LinkSpan;
use crate::components::select::{DEFAULT_PROMPT, Select, SelectOutcome};
use crate::components::text_editor::{EditorOutcome, EditorVariant};
use crate::components::text_input::{InputOutcome, TextInput};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

/// The sheet's widest.
const MAX_WIDTH: u16 = 100;
/// A description is at least this tall.
const BODY_MIN_ROWS: u16 = 5;
/// The heading's tail while the request is in flight (Py bug B69's recommendation, moved from the Save button).
const SAVING: &str = "Saving…";
const TITLE: &str = "Title";
const STATUS: &str = "Status";
const DESCRIPTION: &str = "Description";
/// Where the tabs start on the tabs row: after `Description` and a space, which with the tab's own padding leaves two
/// cells before `Write`.
const TABS_COLUMN: u16 = 12;

/// The form's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IssueField {
    Title,
    /// Edit issue's Status select.
    Status,
    Description,
}

/// Where the fields were last drawn, in the offscreen form's coordinates ([`FormScroll::content`]), for the mouse.
#[derive(Debug, Clone, Copy, Default)]
struct Drawn {
    title_row: Rect,
    title_input: Rect,
    status_row: Rect,
    status_box: Rect,
    tabs_row: Rect,
}

/// The shared form. See the module docs.
#[derive(Debug)]
pub(super) struct IssueForm {
    kind: ModalKind,
    heading: String,
    title: TextInput,
    status: Option<Select<IssueState>>,
    body: ComposeBody,
    /// The focused field, or `None` for the sheet itself after `escape` left a field.
    focus: Option<IssueField>,
    /// `compose.expand`: the sheet fills the screen above the footer.
    expanded: bool,
    /// The dialog's request is in flight: the heading ends with `Saving…` and the submit keys are disabled.
    saving: bool,
    placement: SheetPlacement,
    /// The interior, which scrolls (without a bar) only on a screen too short for even the body's minimum.
    scroll: FormScroll,
    /// Scroll the focused field into view at the next draw, after an edit or a focus change.
    reveal: bool,
    /// The theme of the last draw, which [`Self::size`] measures the preview in.
    theme: Option<Theme>,
    drawn: Drawn,
}

impl IssueForm {
    /// A form for dialog `kind` under `heading`, with `title` and `body` filled in and no Status select, `Title`
    /// focused with its text selected (Textual's `select_on_focus`, so typing replaces it).
    pub(super) fn new(kind: ModalKind, heading: String, title: &str, body: &str) -> Self {
        let mut input = TextInput::new(TITLE).one_row();
        input.set_value(title);
        input.select_all();
        Self {
            kind,
            heading,
            title: input,
            status: None,
            body: ComposeBody::new(EditorVariant::Code, "Describe the issue (markdown)", BODY_MIN_ROWS).with_text(body),
            focus: Some(IssueField::Title),
            expanded: false,
            saving: false,
            placement: SheetPlacement::default(),
            scroll: FormScroll::without_scrollbar(),
            reveal: false,
            theme: None,
            drawn: Drawn::default(),
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

    /// The focused field, `None` for the sheet.
    #[cfg(test)]
    pub(super) fn focused(&self) -> Option<IssueField> {
        self.focus
    }

    /// Whether the title was refused and not changed since.
    #[cfg(test)]
    pub(super) fn is_title_invalid(&self) -> bool {
        self.title.is_invalid()
    }

    /// Whether the request is in flight, as the dialog last said.
    #[cfg(test)]
    pub(super) fn is_saving(&self) -> bool {
        self.saving
    }

    /// The dialog's request went out (`saving`) or came back.
    pub(super) fn set_saving(&mut self, saving: bool) {
        self.saving = saving;
    }

    /// An empty title was refused: `Title` is drawn in the error color until it changes, and takes focus.
    pub(super) fn refuse_title(&mut self) {
        self.title.set_invalid(true);
        self.set_focus(Some(IssueField::Title));
    }

    /// The fields `tab` cycles, in screen order.
    fn order(&self) -> &'static [IssueField] {
        if self.status.is_some() {
            &[IssueField::Title, IssueField::Status, IssueField::Description]
        } else {
            &[IssueField::Title, IssueField::Description]
        }
    }

    /// Moves focus to `focus`. Status's list closes when focus moves; the title selects its text and the editor
    /// starts a new undo batch as they gain it. The focused field is scrolled into view at the next draw.
    fn set_focus(&mut self, focus: Option<IssueField>) {
        self.reveal = true;
        if focus == self.focus {
            return;
        }
        if let Some(status) = &mut self.status {
            status.close();
        }
        self.focus = focus;
        match focus {
            Some(IssueField::Title) => self.title.select_all(),
            Some(IssueField::Description) => self.body.focus_gained(),
            Some(IssueField::Status) | None => {}
        }
    }

    /// `tab` or `shift+tab`: the next or previous field, wrapping. From the sheet, `tab` focuses the first and
    /// `shift+tab` the last. An open list closes as focus leaves it.
    pub(super) fn move_focus(&mut self, direction: FocusMove) {
        let order = self.order();
        let at = self
            .focus
            .and_then(|focus| order.iter().position(|&field| field == focus));
        let index = match (at, direction) {
            (None, FocusMove::Next) => 0,
            (None, FocusMove::Previous) => order.len() - 1,
            (Some(at), FocusMove::Next) => (at + 1) % order.len(),
            (Some(at), FocusMove::Previous) => (at + order.len() - 1) % order.len(),
        };
        self.set_focus(Some(order[index]));
    }

    /// What one of the sheet's keys does here: `modal.close` leaves a field, or closes from the sheet.
    pub(super) fn compose_action(&self, id: BindingId) -> Option<ComposeAction> {
        Some(match id {
            BindingId::ComposeSubmit | BindingId::ModalSubmit => ComposeAction::Submit,
            BindingId::ComposeTogglePreview => ComposeAction::TogglePreview,
            BindingId::ComposeExpand => ComposeAction::ToggleExpand,
            BindingId::ModalClose if self.focus.is_some() => ComposeAction::Leave,
            BindingId::ModalClose => ComposeAction::Close,
            _ => return None,
        })
    }

    /// How the footer shows the sheet's keys: the submit keys disabled while saving, and the toggles and
    /// `modal.close` named for what they'd do now.
    pub(super) fn binding_state(&self, id: BindingId) -> BindingState {
        match id {
            BindingId::ComposeSubmit | BindingId::ModalSubmit if self.saving => BindingState::Disabled,
            BindingId::ComposeTogglePreview if self.body.mode() == BodyMode::Preview => {
                BindingState::Relabeled("Write")
            }
            BindingId::ComposeExpand if self.expanded => BindingState::Relabeled("Collapse"),
            BindingId::ModalClose if self.focus.is_some() => BindingState::Relabeled("Leave"),
            _ => BindingState::Enabled,
        }
    }

    /// The sheet's own actions; the dialog does `Submit` and `Close`.
    pub(super) fn update(&mut self, action: ComposeAction) {
        match action {
            ComposeAction::Leave => self.set_focus(None),
            ComposeAction::TogglePreview => {
                self.body.toggle_mode();
                self.set_focus(Some(IssueField::Description));
            }
            ComposeAction::ToggleExpand => {
                self.expanded = !self.expanded;
                self.reveal = true;
            }
            ComposeAction::Submit | ComposeAction::Close => {}
        }
    }

    /// The focused field's link.
    pub(super) fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.focus {
            Some(IssueField::Title) => FieldKind::TextInput,
            Some(IssueField::Status) if self.is_status_open() => FieldKind::SelectOverlay,
            Some(IssueField::Status) => FieldKind::Select,
            // The preview keeps the editor's link: the keys it would type do nothing there instead of reaching the
            // sheet's bindings.
            Some(IssueField::Description) => FieldKind::TextEditor,
            None => return Vec::new(),
        };
        vec![field.link(self.kind)]
    }

    /// Whether Status's list is open.
    fn is_status_open(&self) -> bool {
        self.status.as_ref().is_some_and(Select::is_open)
    }

    pub(super) fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match (self.focus, &self.status) {
            (Some(IssueField::Title), _) => self.title.consumes_key(key, keymap),
            (Some(IssueField::Description), _) => self.body.consumes_key(key, keymap),
            (Some(IssueField::Status), Some(status)) => status.consumes_key(key, keymap),
            _ => false,
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        match (self.focus, &mut self.status) {
            (Some(IssueField::Title), _) => match self.title.handle_key(&chord, cx.keymap(), cx.clipboard()) {
                InputOutcome::Changed => {
                    self.title_edited();
                    Some(Action::Consumed)
                }
                // `enter` in the title doesn't submit, as in Python.
                InputOutcome::Submitted => Some(Action::Consumed),
                InputOutcome::Copied(text) => {
                    self.title_edited();
                    Some(Action::Copy(text))
                }
                InputOutcome::Ignored => None,
            },
            (Some(IssueField::Description), _) => Some(
                match self.body.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
                    EditorOutcome::Changed => {
                        self.reveal = true;
                        Action::Consumed
                    }
                    EditorOutcome::Copied { text, changed } => {
                        self.reveal |= changed;
                        Action::Copy(text)
                    }
                    EditorOutcome::Moved | EditorOutcome::Ignored => Action::Consumed,
                },
            ),
            (Some(IssueField::Status), Some(status)) => match status.handle_key(&chord, cx.keymap(), cx.now()) {
                SelectOutcome::Ignored => None,
                _ => Some(Action::Consumed),
            },
            _ => None,
        }
    }

    /// The title may have changed: a refused one is drawn plain again once it's not empty.
    fn title_edited(&mut self) {
        if !self.title.value().is_empty() {
            self.title.set_invalid(false);
        }
    }

    pub(super) fn on_field_binding(&mut self, field: FieldKind, id: BindingId) -> Option<Action> {
        match field {
            FieldKind::Select | FieldKind::SelectOverlay => {
                let outcome = self.status.as_mut()?.on_binding(id);
                (outcome != SelectOutcome::Ignored).then_some(Action::Consumed)
            }
            // `consumes_key` already gave the text fields every key they bind.
            _ => Some(Action::Consumed),
        }
    }

    pub(super) fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        // The parts are drawn offscreen and copied with the interior's scroll: their coordinates are the content's.
        let moved = self.scroll.to_content(event);
        if self.title.captured_mouse(moved) {
            return Action::Consumed;
        }
        let press = event.kind == MouseEventKind::Down(MouseButton::Left);
        let on_box = self.drawn.status_box.contains(Position::new(moved.column, moved.row));
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
        match self.body.handle_mouse(moved) {
            BodyMouse::Focused => {
                self.set_focus(Some(IssueField::Description));
                return Action::Consumed;
            }
            BodyMouse::Handled => return Action::Consumed,
            BodyMouse::Missed => {}
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.scroll.wheel(wheel);
        } else if press {
            self.press(moved, on_box);
        }
        Action::Consumed
    }

    /// A left press at `at` in the form's coordinates, after the body missed it: on a field's row (its label
    /// included) the field takes focus, the title the press, and Status's box opens or closes its list.
    fn press(&mut self, at: MouseEvent, on_box: bool) {
        let point = Position::new(at.column, at.row);
        let drawn = self.drawn;
        if drawn.title_row.contains(point) {
            self.set_focus(Some(IssueField::Title));
            self.title.press(drawn.title_input, at.column);
        } else if drawn.status_row.contains(point) {
            self.set_focus(Some(IssueField::Status));
            if on_box && let Some(status) = &mut self.status {
                let _ = status.toggle();
            }
        } else if drawn.tabs_row.contains(point) {
            self.set_focus(Some(IssueField::Description));
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
        match self.focus {
            Some(IssueField::Title) => {
                self.title.handle_paste(text);
                self.title_edited();
            }
            Some(IssueField::Description) => {
                if self.body.handle_paste(text, cx.now()) == EditorOutcome::Changed {
                    self.reveal = true;
                }
            }
            Some(IssueField::Status) | None => return None,
        }
        Some(Action::Consumed)
    }

    /// The sheet's background, which its text and markdown sit on.
    fn under(theme: &Theme) -> Paint {
        theme.roles().surface.dialog.over(theme.background())
    }

    /// The fields' rows.
    fn field_rows(&self) -> u16 {
        if self.status.is_some() { 2 } else { 1 }
    }

    /// Where the fields start: after the widest of their labels.
    fn label_column(&self) -> u16 {
        let status = self.status.as_ref().map(|_| STATUS);
        label_column([TITLE].into_iter().chain(status))
    }

    /// The sheet's limits on `screen` and its parts' rows, measured in `theme`.
    fn sheet(&self, screen: Rect, theme: &Theme) -> (SheetGeometry, ComposeLayout) {
        let geometry = sheet_geometry(screen, MAX_WIDTH, self.expanded);
        let parts = ComposeParts {
            fields: self.field_rows(),
            body_needed: self.body.rows_needed(geometry.content_width, theme, Self::under(theme)),
            body_min: self.body.min_rows(),
            ..ComposeParts::default()
        };
        (geometry, ComposeLayout::new(&parts, geometry.room(), self.expanded))
    }

    /// The frame's outer size on `screen`: as tall as the parts, up to the most the screen allows.
    fn sheet_size(&self, screen: Rect) -> (u16, u16) {
        let theme = self.theme.unwrap_or_default();
        let (geometry, layout) = self.sheet(screen, &theme);
        (geometry.width, layout.height.min(geometry.room()) + 2)
    }

    /// [`super::Dialog::size`]: `available` is the screen above the footer row.
    pub(super) fn size(&self, available: Rect) -> (u16, u16) {
        self.sheet_size(Rect {
            height: available.height + 1,
            ..available
        })
    }

    /// [`super::Dialog::frame`] on a screen of `area`, the footer row included.
    pub(super) fn frame(&self, area: Rect) -> Rect {
        self.placement.place(area, self.sheet_size(area), self.expanded)
    }

    /// The rows the focused field takes in `layout`.
    fn focused_rows(&self, layout: &ComposeLayout) -> Option<std::ops::Range<u16>> {
        let first = layout.fields.start;
        Some(match self.focus? {
            IssueField::Title => first..first + 1,
            IssueField::Status => first + 1..first + 2,
            IssueField::Description => layout.tabs..layout.body.end,
        })
    }

    fn label_state(&self, field: IssueField) -> LabelState {
        if field == IssueField::Title && self.title.is_invalid() {
            LabelState::Invalid
        } else if self.focus == Some(field) {
            LabelState::Focused
        } else {
            LabelState::Plain
        }
    }

    /// Draws the sheet into the dialog's interior `area`, and Status's open list last, over everything below it.
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        self.theme = Some(*theme);
        let screen = frame.area();
        let outer = Rect {
            x: area.x.saturating_sub(1),
            y: area.y.saturating_sub(1),
            width: area.width + 2,
            height: area.height + 2,
        };
        self.placement.record(screen, outer, self.expanded);
        let (_, layout) = self.sheet(screen, theme);
        let roles = theme.roles();
        let under = Self::under(theme);
        let text = Style::new().fg(theme.color(roles.text.plain, under));
        let dim = Style::new().fg(theme.color(roles.text.muted, under));
        if std::mem::take(&mut self.reveal)
            && let Some(rows) = self.focused_rows(&layout)
        {
            self.scroll.reveal(rows);
        }
        let column = self.label_column();
        let label_states =
            [IssueField::Title, IssueField::Status, IssueField::Description].map(|f| self.label_state(f));
        let focus = self.focus;
        let tail = self.saving.then_some(SAVING);
        let (heading, title, status, body) = (&self.heading, &mut self.title, &self.status, &mut self.body);
        let mut drawn = Drawn::default();
        // The preview's links drawn, in rows from the interior's top and columns from its left edge.
        let mut links: Vec<LinkSpan> = Vec::new();
        self.scroll.render(
            frame.buffer_mut(),
            area,
            theme,
            text.bg(theme.paint(under)),
            layout.height,
            |buf, content| {
                let row = |y: u16, height: u16| Rect {
                    x: content.x + 1,
                    y: content.y + y,
                    width: content.width.saturating_sub(2),
                    height,
                };
                render_heading(buf, row(layout.heading, 1), heading, tail, text, dim);
                drawn.title_row = row(layout.fields.start, 1);
                render_label(buf, drawn.title_row, column, TITLE, label_states[0], theme, under);
                drawn.title_input = field_area(drawn.title_row, column);
                title.render(buf, drawn.title_input, theme, under, focus == Some(IssueField::Title));
                if let Some(status) = status {
                    drawn.status_row = row(layout.fields.start + 1, 1);
                    render_label(buf, drawn.status_row, column, STATUS, label_states[1], theme, under);
                    let field = field_area(drawn.status_row, column);
                    drawn.status_box = Rect {
                        width: status.row_width().min(field.width),
                        ..field
                    };
                    // The box loses its focus to the open list.
                    let focused = focus == Some(IssueField::Status) && !status.is_open();
                    status.render_row(buf, field, theme, under, focused);
                }
                drawn.tabs_row = row(layout.tabs, 1);
                render_label(
                    buf,
                    drawn.tabs_row,
                    TABS_COLUMN,
                    DESCRIPTION,
                    label_states[2],
                    theme,
                    under,
                );
                let tabs = field_area(drawn.tabs_row, TABS_COLUMN);
                let rect = row(layout.body.start, layout.body_rows());
                let spans = body.render(buf, tabs, rect, theme, under, focus == Some(IssueField::Description));
                let shift = rect.x - content.x;
                links.extend(spans.into_iter().map(|mut span| {
                    span.row += usize::from(rect.y);
                    span.columns = span.columns.start + shift..span.columns.end + shift;
                    span
                }));
            },
        );
        self.drawn = drawn;
        // Textual's `Markdown` keeps `open_links` in a dialog, so a click on the preview's links opens them.
        if let Some(sink) = cx.links {
            self.scroll.record_links(sink, &links);
        }
        let status_rows = layout.fields.start + 1..layout.fields.start + 2;
        if let Some(status) = &mut self.status
            && let Some(rows) = self.scroll.screen_rows(status_rows)
        {
            let below = Rect {
                x: drawn.status_box.x,
                width: status.row_overlay_width(),
                ..rows
            };
            status.render_overlay(frame.buffer_mut(), below, screen, theme, under);
        }
    }
}
