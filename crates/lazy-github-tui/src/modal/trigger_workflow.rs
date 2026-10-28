//! The Trigger workflow dialog: a heading, a "Workflow" select, a "Branch" input that completes the repo's branch
//! names, and Trigger / Cancel. Trigger asks GitHub to dispatch the picked workflow on the branch. Python's
//! `TriggerWorkflowModal` and `TriggerWorkflowContainer`.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::gh::GhError;
use lazy_github_core::models::{Branch, Repository, Workflow};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::select::{self, Select, SelectOutcome};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The frame's outer size, border included: `width: 60; max-height: 25` on a container that's `1fr` tall.
const SIZE: (u16, u16) = (60, 25);

/// The heading's block: `MarkdownH1`'s two rows of margin above and one below.
const HEADING_ROWS: u16 = 4;
const HEADING_MARGIN_TOP: u16 = 2;

/// The `Markdown` widget's `padding: 0 2`, inside which the heading is centered.
const HEADING_PADDING: u16 = 2;

const HEADING: &str = "Trigger Workflow";
const PROMPT: &str = "Select a workflow";
const PLACEHOLDER: &str = "Choose a branch";
/// The branch prefilled for a repo without a default branch.
const FALLBACK_BRANCH: &str = "main";

/// The dialog's screen bindings: `modal.submit`, then `modal.close`.
const TRIGGER_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// The Trigger workflow dialog's own action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerWorkflowAction {
    /// `modal.submit` or Trigger: check the fields and dispatch the workflow.
    Submit,
    /// `modal.close` or Cancel: close without dispatching.
    Close,
}

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerField {
    Workflow,
    Branch,
    Trigger,
    Cancel,
}

impl TriggerField {
    fn button(self) -> Option<Pressed> {
        match self {
            TriggerField::Trigger => Some(Pressed::Submit),
            TriggerField::Cancel => Some(Pressed::Cancel),
            TriggerField::Workflow | TriggerField::Branch => None,
        }
    }
}

/// Where each part goes in the interior: stacked top to bottom and centered vertically (the container's
/// `align: center middle`).
struct Areas {
    heading: Rect,
    workflow_label: Rect,
    workflow: Rect,
    branch_label: Rect,
    branch: Rect,
    buttons: Rect,
}

impl Areas {
    fn new(area: Rect) -> Self {
        let total = HEADING_ROWS + 1 + select::HEIGHT + 1 + text_input::HEIGHT + ButtonRow::HEIGHT;
        let mut y = area.y + area.height.saturating_sub(total) / 2;
        let mut next = |height: u16| {
            let rect = Rect { y, height, ..area }.intersection(area);
            y = y.saturating_add(height);
            rect
        };
        let heading_block = next(HEADING_ROWS);
        let heading = Rect {
            x: area.x + HEADING_PADDING.min(area.width),
            y: heading_block.y.saturating_add(HEADING_MARGIN_TOP),
            width: area.width.saturating_sub(2 * HEADING_PADDING),
            height: 1,
        }
        .intersection(area);
        Self {
            heading,
            workflow_label: next(1),
            workflow: next(select::HEIGHT),
            branch_label: next(1),
            branch: next(text_input::HEIGHT),
            buttons: next(ButtonRow::HEIGHT),
        }
    }
}

/// The Trigger workflow dialog. Its output is whether a workflow was dispatched.
#[derive(Debug)]
pub struct TriggerWorkflowDialog {
    repo: Repository,
    ring: FocusRing<TriggerField>,
    workflow: Select<u64>,
    branch: TextInput,
    buttons: ButtonRow,
    /// The branches request, until it answers.
    branches_request: Option<RequestId>,
    /// The dispatch in flight.
    dispatch: Option<RequestId>,
    output: Option<bool>,
}

impl TriggerWorkflowDialog {
    /// The dialog for dispatching one of `workflows` in `repo`: no workflow picked, the branch input holding the
    /// repo's default branch (or `main`), the select focused. It asks for the repo's branches at once, for the
    /// input's suggestions.
    pub fn new(repo: Repository, workflows: &[Workflow], cx: &mut Cx<'_>) -> Self {
        let mut branch = TextInput::new(PLACEHOLDER);
        branch.set_value(repo.default_branch.as_deref().unwrap_or(FALLBACK_BRANCH));
        let branches_request = Some(cx.request(GithubRequest::ListBranches {
            repo: Box::new(repo.clone()),
        }));
        Self {
            ring: FocusRing::new([
                TriggerField::Workflow,
                TriggerField::Branch,
                TriggerField::Trigger,
                TriggerField::Cancel,
            ]),
            workflow: Select::new(
                PROMPT,
                workflows.iter().map(|workflow| (workflow.name.clone(), workflow.id)),
                None,
            ),
            branch,
            buttons: ButtonRow::new("Trigger"),
            repo,
            branches_request,
            dispatch: None,
            output: None,
        }
    }

    /// The branch as typed.
    pub fn branch(&self) -> &str {
        self.branch.value()
    }

    /// The picked workflow's id, `None` for the blank.
    pub fn workflow(&self) -> Option<u64> {
        self.workflow.value().copied()
    }

    fn action(action: TriggerWorkflowAction) -> Action {
        Action::Dialog(DialogAction::TriggerWorkflow(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => TriggerWorkflowAction::Submit,
            Pressed::Cancel => TriggerWorkflowAction::Close,
        })
    }

    /// Textual's `Length(minimum=1)` validator: the red border follows the latest check.
    fn validate(&mut self) -> bool {
        let valid = !self.branch.value().is_empty();
        self.branch.set_invalid(!valid);
        valid
    }

    /// Moves focus by `f`. The select's list closes when it loses focus; the input is checked when it loses focus
    /// (Textual validates on blur) and selects its text when it gains it (`select_on_focus`).
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<TriggerField>)) {
        let before = self.ring.focused();
        f(&mut self.ring);
        let after = self.ring.focused();
        if before == after {
            return;
        }
        self.workflow.close();
        if before == Some(TriggerField::Branch) {
            self.validate();
        }
        if after == Some(TriggerField::Branch) {
            self.branch.select_all();
        }
    }

    fn focus(&mut self, field: TriggerField) {
        self.refocus(|ring| {
            ring.focus(field);
        });
    }

    /// Edits the input with `edit`, checking it if the text changed (Textual's `Input.Changed`) or was submitted.
    fn edit(&mut self, edit: impl FnOnce(&mut TextInput) -> InputOutcome) -> InputOutcome {
        let before = self.branch.value().to_owned();
        let outcome = edit(&mut self.branch);
        match outcome {
            InputOutcome::Changed | InputOutcome::Copied(_) if self.branch.value() != before => {
                self.validate();
            }
            InputOutcome::Submitted => {
                self.validate();
            }
            InputOutcome::Changed | InputOutcome::Copied(_) | InputOutcome::Ignored => {}
        }
        outcome
    }

    /// Python's `action_submit`: a workflow must be picked (Py bug B67's recommendation: Python crashes) and a branch
    /// typed, then the dispatch goes out.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.dispatch.is_some() {
            tracing::debug!("Ignoring a trigger: a dispatch is in flight");
            return;
        }
        let Some(workflow_id) = self.workflow() else {
            cx.notify(Toast::error("You must select a workflow!").with_title("Validation Error"));
            return;
        };
        if !self.validate() {
            cx.notify(Toast::error("You must enter a branch!").with_title("Validation Error"));
            return;
        }
        self.dispatch = Some(cx.request(GithubRequest::DispatchWorkflow {
            repo: Box::new(self.repo.clone()),
            workflow_id,
            git_ref: self.branch.value().to_owned(),
        }));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        for id in self.branches_request.take().into_iter().chain(self.dispatch.take()) {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(false);
    }

    fn on_branches(&mut self, result: Result<Vec<Branch>, GhError>) {
        match result {
            Ok(branches) => {
                let names: Vec<String> = branches.into_iter().map(|branch| branch.name).collect();
                self.branch.set_suggestions(names);
            }
            // Python's worker raises and Textual logs it; the input just gets no suggestions.
            Err(error) => tracing::debug!("Fetching the branches for the trigger dialog failed: {error}"),
        }
    }

    /// A dispatch GitHub refused is reported and the dialog stays open (Py bug B68's recommendation: Python reports
    /// success).
    fn on_dispatched(&mut self, result: Result<(), GhError>, cx: &mut Cx<'_>) {
        match result {
            Ok(()) => self.output = Some(true),
            Err(error) => {
                tracing::error!("Error creating workflow dispatch event!");
                tracing::debug!("The dispatch failed: {error}");
                cx.notify(
                    Toast::error("Could not trigger build - are you sure this workflow supports dispatch events?")
                        .with_title("Error Triggering Build"),
                );
            }
        }
    }
}

impl Component for TriggerWorkflowDialog {
    type Action = TriggerWorkflowAction;

    fn bindings(&self) -> &'static [BindingId] {
        TRIGGER_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(TriggerWorkflowAction::Submit)),
            BindingId::ModalClose => Some(Self::action(TriggerWorkflowAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match self.ring.focused() {
            Some(TriggerField::Workflow) => self.workflow.consumes_key(key, keymap),
            Some(TriggerField::Branch) => self.branch.consumes_key(key, keymap),
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        match self.ring.focused() {
            Some(TriggerField::Workflow) => match self.workflow.handle_key(&chord, cx.keymap(), cx.now()) {
                SelectOutcome::Ignored => None,
                _ => Some(Action::Consumed),
            },
            // `enter` in the input only checks it; triggering is `modal.submit` or the Trigger button.
            Some(TriggerField::Branch) => {
                match self.edit(|input| input.handle_key(&chord, cx.keymap(), cx.clipboard())) {
                    InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
                    InputOutcome::Copied(text) => Some(Action::Copy(text)),
                    InputOutcome::Ignored => None,
                }
            }
            _ => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // A drag that started on the input, wherever the pointer is now.
        if self.branch.captured_mouse(event) {
            return Some(Action::Consumed);
        }
        let areas = Areas::new(area);
        let press = event.kind == MouseEventKind::Down(MouseButton::Left);
        let at = Position::new(event.column, event.row);
        let on_box = areas.workflow.contains(at);
        if self.workflow.is_open() {
            if self.workflow.overlay_contains(event.column, event.row) {
                let _ = self.workflow.handle_overlay_mouse(&event);
                return Some(Action::Consumed);
            }
            // A press elsewhere takes the focus from the list, which closes it; a press on the box toggles it.
            if press && !on_box {
                self.workflow.close();
            }
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    self.focus(match button {
                        Pressed::Submit => TriggerField::Trigger,
                        Pressed::Cancel => TriggerField::Cancel,
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if !press {
            return Some(Action::Consumed);
        }
        if on_box {
            self.focus(TriggerField::Workflow);
            let _ = self.workflow.toggle();
        } else if areas.branch.contains(at) {
            self.focus(TriggerField::Branch);
            self.branch.press(areas.branch, event.column);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: TriggerWorkflowAction, cx: &mut Cx<'_>) {
        match action {
            TriggerWorkflowAction::Submit => self.submit(cx),
            TriggerWorkflowAction::Close => self.close(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let p = theme.palette;
        let under = self.background(p).over(theme.background());
        let bold = Style::new()
            .fg(theme.color(p.foreground, under))
            .add_modifier(Modifier::BOLD);
        let areas = Areas::new(area);
        let screen = frame.area();
        let buf = frame.buffer_mut();

        // `MarkdownH1`: centered, in `$markdown-h1-color` and its text style.
        let heading = p
            .markdown_h1_text_style
            .apply(Style::new().fg(theme.color(p.markdown_h1_color, under)));
        let width = usize::from(areas.heading.width);
        buf.set_stringn(
            areas.heading.x,
            areas.heading.y,
            format!("{HEADING:^width$}"),
            width,
            heading,
        );

        for (label, rect) in [("Workflow", areas.workflow_label), ("Branch", areas.branch_label)] {
            buf.set_span(rect.x, rect.y, &Span::styled(label, bold), rect.width);
        }

        // The box loses its focus to the open list.
        let select_focused = self.ring.is_focused(TriggerField::Workflow) && !self.workflow.is_open();
        self.workflow
            .render_box(buf, areas.workflow, theme, under, select_focused);
        self.branch.render(
            buf,
            areas.branch,
            theme,
            under,
            self.ring.is_focused(TriggerField::Branch),
        );
        let focused = self.ring.focused().and_then(TriggerField::button);
        self.buttons.render(buf, areas.buttons, theme, focused);
        // The open list last, over everything below it.
        self.workflow.render_overlay(buf, areas.workflow, screen, theme, under);
    }
}

impl Dialog for TriggerWorkflowDialog {
    type Output = bool;

    fn kind(&self) -> ModalKind {
        ModalKind::TriggerWorkflow
    }

    fn own_action(&self, action: DialogAction) -> Option<TriggerWorkflowAction> {
        match action {
            DialogAction::TriggerWorkflow(action) => Some(action),
            _ => None,
        }
    }

    /// `tab` or `shift+tab`: an open list closes (it loses focus), then focus moves on from the select.
    fn move_focus(&mut self, direction: FocusMove) {
        self.workflow.close();
        self.refocus(|ring| ring.step(direction));
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn take_output(&mut self) -> Option<bool> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(TriggerField::Workflow) if self.workflow.is_open() => FieldKind::SelectOverlay,
            Some(TriggerField::Workflow) => FieldKind::Select,
            Some(TriggerField::Branch) => FieldKind::TextInput,
            Some(TriggerField::Trigger | TriggerField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(TriggerField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            (FieldKind::Select | FieldKind::SelectOverlay, id) => {
                let outcome = self.workflow.on_binding(id);
                (outcome != SelectOutcome::Ignored).then_some(Action::Consumed)
            }
            // `consumes_key` already gave the input every key it binds.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.branch.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        self.ring.is_focused(TriggerField::Branch).then(|| {
            let _ = self.edit(|input| {
                input.handle_paste(text);
                InputOutcome::Changed
            });
            Action::Consumed
        })
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        match response {
            GithubResponse::ListBranches(result) if self.branches_request == Some(id) => {
                self.branches_request = None;
                self.on_branches(result);
            }
            GithubResponse::DispatchWorkflow(result) if self.dispatch == Some(id) => {
                self.dispatch = None;
                self.on_dispatched(result, cx);
            }
            other => return Some(other),
        }
        None
    }
}

#[cfg(test)]
mod tests;
