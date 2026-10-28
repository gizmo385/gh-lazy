//! The New issue dialog (`issue.new`): a title input and a description code editor in a scroll area, then Save and
//! Cancel. It creates the issue itself and closes with it. Python's `NewIssueModal` around a `NewIssueContainer`.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::github::NewIssue;
use lazy_github_core::models::{Issue, Repository};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::form::{Block, FormLayout, FormScroll, split};
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::info::Scroll;
use crate::components::text_editor::{EditorOutcome, EditorVariant, TextEditor};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The dialog's screen bindings: the container's `modal.submit`, then the modal's `modal.close`.
const NEW_ISSUE_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// `NewIssueContainer { width: 100 }`, border included.
const WIDTH: u16 = 100;

/// `height: 80%` of the screen above the footer, rounded down as Textual does.
const HEIGHT_PERCENT: u16 = 80;

/// `#new_issue_body { height: 75% }` of the scroll area.
const EDITOR_PERCENT: u16 = 75;

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewIssueAction {
    /// `modal.submit` or Save: check the title and create the issue.
    Submit,
    /// `modal.close` or Cancel: close with nothing, cancelling a request in flight.
    Close,
}

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewIssueField {
    /// The scroll area itself, which Textual's `ScrollableContainer` makes focusable. It has focus when the dialog
    /// opens.
    Scroll,
    Title,
    Description,
    Save,
    Cancel,
}

impl NewIssueField {
    fn button(self) -> Option<Pressed> {
        match self {
            NewIssueField::Save => Some(Pressed::Submit),
            NewIssueField::Cancel => Some(Pressed::Cancel),
            NewIssueField::Scroll | NewIssueField::Title | NewIssueField::Description => None,
        }
    }
}

/// The New issue dialog for one repo. Its output is the created issue, or `None` when it was closed.
#[derive(Debug)]
pub struct NewIssueDialog {
    repo: Repository,
    ring: FocusRing<NewIssueField>,
    title: TextInput,
    body: TextEditor,
    buttons: ButtonRow,
    scroll: FormScroll,
    /// The create request in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<Issue>>>,
}

impl NewIssueDialog {
    /// An empty dialog for `repo`, its description holding `prefilled` (the M4 bug-report flow passes a template),
    /// with the scroll area focused.
    pub fn new(repo: Repository, prefilled: Option<&str>) -> Self {
        Self {
            repo,
            ring: FocusRing::new([
                NewIssueField::Scroll,
                NewIssueField::Title,
                NewIssueField::Description,
                NewIssueField::Save,
                NewIssueField::Cancel,
            ]),
            title: TextInput::new("Title"),
            body: TextEditor::new(EditorVariant::Code, prefilled.unwrap_or_default()),
            buttons: ButtonRow::flush("Save"),
            scroll: FormScroll::default(),
            pending: None,
            output: None,
        }
    }

    /// The title as typed.
    pub fn title(&self) -> &str {
        self.title.value()
    }

    /// The description as typed.
    pub fn description(&self) -> String {
        self.body.text()
    }

    /// Whether a create request is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn action(action: NewIssueAction) -> Action {
        Action::Dialog(DialogAction::NewIssue(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => NewIssueAction::Submit,
            Pressed::Cancel => NewIssueAction::Close,
        })
    }

    /// The form at a scroll area `height` rows tall: the editor takes 75% of it.
    fn layout(height: u16) -> FormLayout<NewIssueField> {
        FormLayout::new([
            Block::Label("Title"),
            Block::Field(NewIssueField::Title, text_input::HEIGHT),
            Block::Rule,
            Block::Label("Description"),
            Block::Field(NewIssueField::Description, height * EDITOR_PERCENT / 100),
        ])
    }

    /// The form as of the last draw.
    fn last_layout(&self) -> FormLayout<NewIssueField> {
        Self::layout(self.scroll.area().height)
    }

    /// Moves focus with `f`. A field that gains it is scrolled into view; the title selects its text (Textual's
    /// `select_on_focus`, so typing replaces it) and the editor starts a new undo batch.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<NewIssueField>)) {
        let before = self.ring.focused();
        f(&mut self.ring);
        let Some(field) = self.ring.focused().filter(|&field| Some(field) != before) else {
            return;
        };
        match field {
            NewIssueField::Title => self.title.select_all(),
            NewIssueField::Description => self.body.focus_gained(),
            NewIssueField::Scroll | NewIssueField::Save | NewIssueField::Cancel => {}
        }
        if let Some(rows) = self.last_layout().rows(field) {
            self.scroll.reveal(rows);
        }
    }

    fn focus(&mut self, field: NewIssueField) {
        self.refocus(|ring| {
            ring.focus(field);
        });
    }

    /// Python's `create_issue`: an empty title is refused (a blank one isn't), an empty description is allowed (Py
    /// bug B25's recommendation), and a second submit waits for the first.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.pending.is_some() {
            tracing::debug!("Ignoring a New issue submit: one is in flight");
            return;
        }
        let title = self.title.value();
        if title.is_empty() {
            cx.notify(Toast::error("Must have a non-empty issue title!"));
            return;
        }
        cx.notify(Toast::info("Creating new issue..."));
        let issue = NewIssue {
            title: title.to_owned(),
            body: self.body.text(),
        };
        self.pending = Some(cx.request(GithubRequest::CreateIssue {
            repo: Box::new(self.repo.clone()),
            issue,
        }));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
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
            .rows(NewIssueField::Description)
            .is_some_and(|editor| self.scroll.shows_at(editor, column, row));
        if !(over_editor && self.body.scroll(rows)) {
            self.scroll.wheel(wheel);
        }
    }

    /// A left press in the scroll area: the field under it takes focus (and the press, for the text fields), or the
    /// area itself does.
    fn press(&mut self, event: MouseEvent) {
        let Some(form_row) = self.scroll.content_row(event.row) else {
            return;
        };
        let layout = self.last_layout();
        let content = self.scroll.content();
        // The scrollbar's columns aren't the fields'.
        let in_content = event.column < content.right();
        match layout.field_at(form_row).filter(|_| in_content) {
            Some(NewIssueField::Title) => {
                self.focus(NewIssueField::Title);
                self.title
                    .press(layout.area(NewIssueField::Title, content), event.column);
            }
            Some(NewIssueField::Description) => {
                self.focus(NewIssueField::Description);
                self.body.press(event.column, form_row);
            }
            _ => self.focus(NewIssueField::Scroll),
        }
    }
}

impl Component for NewIssueDialog {
    type Action = NewIssueAction;

    fn bindings(&self) -> &'static [BindingId] {
        NEW_ISSUE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(NewIssueAction::Submit)),
            BindingId::ModalClose => Some(Self::action(NewIssueAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match self.ring.focused() {
            Some(NewIssueField::Title) => self.title.consumes_key(key, keymap),
            Some(NewIssueField::Description) => self.body.consumes_key(key, keymap),
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        match self.ring.focused() {
            Some(NewIssueField::Title) => match self.title.handle_key(&chord, cx.keymap(), cx.clipboard()) {
                // `enter` in the title doesn't submit, as in Python.
                InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
                InputOutcome::Copied(text) => Some(Action::Copy(text)),
                InputOutcome::Ignored => None,
            },
            Some(NewIssueField::Description) => {
                match self.body.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
                    EditorOutcome::Copied { text, .. } => Some(Action::Copy(text)),
                    EditorOutcome::Changed | EditorOutcome::Moved | EditorOutcome::Ignored => Some(Action::Consumed),
                }
            }
            _ => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // A drag that started in a text field, wherever the pointer is now.
        let moved = self.scroll.to_content(event);
        if self.title.captured_mouse(moved) || self.body.captured_mouse(moved) {
            return Some(Action::Consumed);
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    self.focus(match button {
                        Pressed::Submit => NewIssueField::Save,
                        Pressed::Cancel => NewIssueField::Cancel,
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.wheel(wheel, event.column, event.row);
        } else if event.kind == MouseEventKind::Down(MouseButton::Left) {
            self.press(event);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: NewIssueAction, cx: &mut Cx<'_>) {
        match action {
            NewIssueAction::Submit => self.submit(cx),
            NewIssueAction::Close => self.close(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        let fill = Style::new()
            .fg(theme.color(theme.palette.foreground, under))
            .bg(theme.paint(under));
        let (scroll_area, buttons_area) = split(area, self.buttons.height());
        let layout = Self::layout(scroll_area.height);
        let (title, body, ring) = (&mut self.title, &mut self.body, &self.ring);
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
                    layout.area(NewIssueField::Title, content),
                    theme,
                    under,
                    ring.is_focused(NewIssueField::Title),
                );
                body.render(
                    content_buf,
                    layout.area(NewIssueField::Description, content),
                    theme,
                    under,
                    ring.is_focused(NewIssueField::Description),
                );
            },
        );
        let focused = self.ring.focused().and_then(NewIssueField::button);
        self.buttons.render(buf, buttons_area, theme, focused);
    }
}

impl Dialog for NewIssueDialog {
    type Output = Option<Box<Issue>>;

    fn kind(&self) -> ModalKind {
        ModalKind::NewIssue
    }

    fn own_action(&self, action: DialogAction) -> Option<NewIssueAction> {
        match action {
            DialogAction::NewIssue(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.refocus(|ring| ring.step(direction));
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        (WIDTH, available.height * HEIGHT_PERCENT / 100)
    }

    fn take_output(&mut self) -> Option<Option<Box<Issue>>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(NewIssueField::Scroll) => FieldKind::Scroll,
            Some(NewIssueField::Title) => FieldKind::TextInput,
            Some(NewIssueField::Description) => FieldKind::TextEditor,
            Some(NewIssueField::Save | NewIssueField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(NewIssueField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            (FieldKind::Scroll, id) => {
                let scroll = Scroll::from_binding(id)?;
                self.scroll.scroll(scroll);
                Some(Action::Consumed)
            }
            // `consumes_key` already gave the text fields every key they bind.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.title.is_selecting() || self.body.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        match self.ring.focused() {
            Some(NewIssueField::Title) => self.title.handle_paste(text),
            Some(NewIssueField::Description) => {
                let _ = self.body.handle_paste(text, cx.now());
            }
            _ => return None,
        }
        Some(Action::Consumed)
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let GithubResponse::CreateIssue(result) = response else {
            return Some(response);
        };
        self.pending = None;
        match result {
            Ok(issue) => {
                // Raised before the dialog closes, as Python's is.
                cx.notify(Toast::info(format!("Successfully created issue #{}", issue.number)));
                self.output = Some(Some(issue));
            }
            // Python has no handling and crashes (Py bug B70): the dialog stays open with everything typed.
            Err(error) => {
                tracing::error!("Error creating issue: {error}");
                cx.notify(Toast::error("Could not create the issue").with_title("Error creating issue"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
