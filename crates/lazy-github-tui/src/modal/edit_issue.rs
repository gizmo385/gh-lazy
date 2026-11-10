//! The Edit issue dialog (`issue.edit`), on the compose sheet (decision compose-dialogs): a heading, the issue's
//! title and a Status select as one-row fields, and its description under the `Description  Write  Preview` tabs,
//! with the footer's keys for Save and Close. It saves the issue itself and closes with the updated one. Python's
//! `EditIssueModal` around an `EditIssueContainer`.

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::github::IssueUpdate;
use lazy_github_core::models::Issue;
use ratatui::Frame;
use ratatui::layout::Rect;

use super::issue_form::IssueForm;
use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, RenderCx};
use crate::components::compose::{COMPOSE_BINDINGS, ComposeAction};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// How long "Updating issue #N..." shows: Python's `timeout=1`.
const UPDATING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditIssueAction {
    /// One of the sheet's shared keys.
    Compose(ComposeAction),
}

/// The Edit issue dialog for one issue. Its output is the updated issue, or `None` when it was closed.
#[derive(Debug)]
pub struct EditIssueDialog {
    issue: Box<Issue>,
    form: IssueForm,
    /// The update request in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<Issue>>>,
}

impl EditIssueDialog {
    /// A dialog for `issue`, its title, state and body filled in (an issue without a body gives an empty editor),
    /// with the title focused.
    pub fn new(issue: Box<Issue>) -> Self {
        let heading = format!("Edit issue #{} · {}", issue.number, issue.repo.full_name);
        let form = IssueForm::new(
            ModalKind::EditIssue,
            heading,
            &issue.title,
            issue.body.as_deref().unwrap_or_default(),
        )
        .with_status(issue.state);
        Self {
            issue,
            form,
            pending: None,
            output: None,
        }
    }

    /// Whether an update request is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn action(action: ComposeAction) -> Action {
        Action::Dialog(DialogAction::EditIssue(EditIssueAction::Compose(action)))
    }

    /// Python's `submit_updated_issue`, with no checks: the toast, `Saving…` in the heading with the submit keys
    /// disabled (Py bug B69's recommendation), and the request. A second submit waits for the first.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.pending.is_some() {
            tracing::debug!("Ignoring an Edit issue submit: one is in flight");
            return;
        }
        let number = self.issue.number;
        cx.notify(Toast::info(format!("Updating issue #{number}...")).with_timeout(UPDATING_TIMEOUT));
        self.form.set_saving(true);
        let update = IssueUpdate {
            title: self.form.title().to_owned(),
            body: self.form.body(),
            state: self.form.status(),
        };
        self.pending = Some(cx.request(GithubRequest::UpdateIssue {
            repo: Box::new(self.issue.repo.clone()),
            number,
            update,
        }));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }
}

impl Component for EditIssueDialog {
    type Action = EditIssueAction;

    fn bindings(&self) -> &'static [BindingId] {
        COMPOSE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        self.form.compose_action(id).map(Self::action)
    }

    fn binding_state(&self, id: BindingId, _cx: &Cx<'_>) -> BindingState {
        self.form.binding_state(id)
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.form.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        self.form.handle_key(key, cx)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        Some(self.form.handle_mouse(event))
    }

    fn update(&mut self, action: EditIssueAction, cx: &mut Cx<'_>) {
        let EditIssueAction::Compose(action) = action;
        match action {
            ComposeAction::Submit => self.submit(cx),
            ComposeAction::Close => self.close(cx),
            other => self.form.update(other),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        self.form.render(frame, area, cx);
    }
}

impl Dialog for EditIssueDialog {
    type Output = Option<Box<Issue>>;

    fn kind(&self) -> ModalKind {
        ModalKind::EditIssue
    }

    fn own_action(&self, action: DialogAction) -> Option<EditIssueAction> {
        match action {
            DialogAction::EditIssue(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.form.move_focus(direction);
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        self.form.size(available)
    }

    fn frame(&self, area: Rect) -> Rect {
        self.form.frame(area)
    }

    fn take_output(&mut self) -> Option<Option<Box<Issue>>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        self.form.field_chain()
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        self.form.on_field_binding(field, id)
    }

    fn captures_mouse(&self) -> bool {
        self.form.captures_mouse()
    }

    fn popup_contains(&self, column: u16, row: u16) -> bool {
        self.form.popup_contains(column, row)
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        self.form.handle_paste(text, cx)
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let GithubResponse::UpdateIssue(result) = response else {
            return Some(response);
        };
        self.pending = None;
        self.form.set_saving(false);
        let number = self.issue.number;
        match result {
            Ok(issue) => {
                cx.notify(Toast::info(format!("Successfully updated issue #{number}")));
                self.output = Some(Some(issue));
            }
            // Python has no handling and crashes (Py bug B70): the submit keys come back and the dialog keeps the
            // edits.
            Err(error) => {
                tracing::error!("Error updating issue #{number}: {error}");
                cx.notify(Toast::error(format!("Could not update issue #{number}")).with_title("Error updating issue"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
